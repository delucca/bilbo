use std::fs;
use std::os::unix::fs::PermissionsExt;

use super::scratch::{NOTE, Scratch, remembered};

const EVERY_PASSAGE_ENDS_IN_A_LINE_FEED: bool = true;
const BETWEEN_PASSAGES: &str = "\n";

#[test]
fn a_remembered_store_is_recalled_and_a_hits_ref_reads() {
    let scratch = remembered("recall_reads");
    let run = scratch.run(&["recall", "carrot", "beans"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let mut lines = run.stdout.split('\n');
    let header = lines.next().expect("a header");
    let fields: Vec<&str> = header.split('\t').collect();
    let reference = fields[0].strip_prefix("=== ").expect("a header");
    assert!(reference.starts_with("bilbo:"), "{header}");
    assert!(
        fields[2].len() == 10 && fields[2].starts_with("20"),
        "{header}"
    );
    assert!(
        !fields.iter().any(|field| field.starts_with("part ")),
        "the fixtures hold no record too long for a chunk: {header}"
    );
    let count: usize = fields[3]
        .split(' ')
        .next()
        .and_then(|n| n.parse().ok())
        .expect("a line count");
    let content: Vec<&str> = lines.take(count).collect();
    assert_eq!(content.len(), count, "{}", run.stdout);
    assert!(
        run.stderr.starts_with("bilbo: searched ") && run.stderr.contains(" entries\n"),
        "{}",
        run.stderr
    );

    assert_eq!(fields[1], "markdown", "{header}");
    assert!(NOTE.ends_with('\n'));
    assert!(NOTE.lines().all(|line| !line.starts_with("#r")));
    let (_, records) = reference.split_once("#r").expect("records in the ref");
    let (first, last) = records.split_once('-').expect("a range");
    let (first, last): (u64, u64) = (
        first.parse().expect("a record"),
        last.parse().expect("a record"),
    );
    let read = scratch.run(&["read", reference, "--budget", "100000"]);
    assert_eq!((read.code, read.stderr.as_str()), (0, ""));
    let whole = read.stdout.strip_suffix('\n').expect("a trailer");
    let at = whole.rfind('\n').expect("a trailer") + 1;
    let (body, trailer) = (&read.stdout[..at], &whole[at..]);
    assert!(trailer.starts_with("end: "), "{trailer}");

    let mut blocks: Vec<(u64, u64, String)> = Vec::new();
    for line in body.split_inclusive('\n') {
        let located = line.strip_prefix("#r").and_then(|rest| {
            let (locator, text) = rest.split_once(' ')?;
            let (from, to) = locator.split_once('-').unwrap_or((locator, locator));
            Some((from.parse::<u64>().ok()?, to.parse::<u64>().ok()?, text))
        });
        match located {
            Some((from, to, text)) => blocks.push((from, to, text.to_owned())),
            None => blocks.last_mut().expect("a locator first").2.push_str(line),
        }
    }
    let mut next = first;
    for (from, to, _) in &blocks {
        assert_eq!(
            *from, next,
            "a block starts one past the last: {}",
            read.stdout
        );
        next = to + 1;
    }
    assert_eq!(next, last + 1, "{}", read.stdout);
    assert!(
        blocks.len() >= 2,
        "the hit joins two passages or more: {}",
        read.stdout
    );
    let passages: Vec<String> = blocks
        .into_iter()
        .map(|(_, _, mut text)| {
            if !EVERY_PASSAGE_ENDS_IN_A_LINE_FEED {
                text.pop();
            }
            text
        })
        .collect();
    let expected = passages.join(BETWEEN_PASSAGES);
    let shown = content.join("\n");
    if shown != expected {
        let mut at = 0;
        for (number, passage) in passages.iter().enumerate() {
            assert!(
                shown[at..].starts_with(passage.as_str()),
                "passage {number} differs: {passage:?} against {:?}",
                &shown[at..]
            );
            at += passage.len() + BETWEEN_PASSAGES.len();
        }
    }
    assert_eq!(shown, expected);
}

#[test]
fn recall_exits_1_when_nothing_matches() {
    let scratch = remembered("recall_nothing");
    let none = scratch.run(&["recall", "xyzzy"]);
    assert_eq!((none.code, none.stdout.as_str()), (1, ""));
    assert!(
        none.stderr.ends_with("bilbo: nothing matched\n"),
        "{}",
        none.stderr
    );
    let stopwords = scratch.run(&["recall", "the", "of"]);
    assert_eq!((stopwords.code, stopwords.stdout.as_str()), (1, ""));
    assert!(
        stopwords.stderr.ends_with("bilbo: nothing matched\n"),
        "{}",
        stopwords.stderr
    );

    let empty = Scratch::new("recall_no_store");
    let run = empty.run(&["recall", "carrot"]);
    assert_eq!((run.code, run.stdout.as_str()), (1, ""));
    assert!(
        run.stderr.starts_with("bilbo: no store at "),
        "{}",
        run.stderr
    );

    for args in [
        &["recall"][..],
        &["recall", "--frobnicate", "x"],
        &["recall", "--kind", "nope", "x"],
        &["recall", "--budget", "1", "x"],
    ] {
        let run = scratch.run(args);
        assert_eq!((run.code, run.stdout.as_str()), (2, ""), "{args:?}");
    }
}

#[test]
fn every_recall_adds_one_line_to_the_query_log() {
    let scratch = remembered("recall_log");
    let log = scratch.dir.join("state/bilbo/queries.log");
    assert!(!log.exists());
    for (count, args) in [
        &["recall", "carrot"][..],
        &["recall", "xyzzy"],
        &["recall", "--kind", "markdown", "heading"],
    ]
    .into_iter()
    .enumerate()
    {
        scratch.run(args);
        let text = fs::read_to_string(&log).expect("the query log");
        assert_eq!(text.lines().count(), count + 1, "{text}");
        assert!(text.ends_with('\n'));
    }
    let text = fs::read_to_string(&log).expect("the query log");
    let fields: Vec<Vec<&str>> = text
        .lines()
        .map(|line| line.split('\t').collect())
        .collect();
    assert_eq!(
        fields.iter().map(|f| (f[0], f[2])).collect::<Vec<_>>(),
        [("q1", "hits"), ("q1", "nothing"), ("q1", "hits")]
    );
    assert_eq!(fields[2][4], "markdown");
    assert_eq!(fields[2][6], "heading");
    assert_eq!(
        fs::metadata(&log).expect("the log").permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn recall_help_is_its_page() {
    let page = Scratch::new("recall_help").run(&["recall", "--help"]);
    assert_eq!((page.code, page.stderr.as_str()), (0, ""));
    assert!(
        page.stdout
            .starts_with("bilbo recall: search what bilbo keeps"),
        "{}",
        page.stdout
    );
    for part in [
        "Usage:\n  bilbo recall <query>...",
        "\nOutput:\n  === <ref>",
        "\nExit:\n  0  at least one hit\n",
    ] {
        assert!(page.stdout.contains(part), "{part:?} in {}", page.stdout);
    }
    assert!(
        page.stdout
            .ends_with("Docs: https://github.com/delucca/bilbo/wiki/Commands#recall\n")
    );
    for form in [&["recall", "-h"][..], &["help", "recall"]] {
        assert_eq!(
            Scratch::new("recall_help_form").run(form).stdout,
            page.stdout,
            "{form:?}"
        );
    }
}

#[test]
fn a_run_that_never_opens_a_store_is_not_logged() {
    let scratch = Scratch::new("recall_unlogged");
    let state = scratch.dir.join("state/bilbo");
    fs::create_dir_all(&state).expect("a state folder");
    let log = state.join("queries.log");
    fs::write(&log, "q1\t1\thits\t1\t-\t-\ta\nq1\t2\thits\t1\t-\t-\tb\n").expect("a log");
    fs::set_permissions(&log, fs::Permissions::from_mode(0o600)).expect("a mode");
    let before = fs::read(&log).expect("the log");
    let run = scratch.run(&["recall", "carrot"]);
    assert_eq!((run.code, run.stdout.as_str()), (1, ""));
    assert!(
        run.stderr.starts_with("bilbo: no store at "),
        "{}",
        run.stderr
    );
    assert_eq!(fs::read(&log).expect("the log"), before);
}

#[test]
fn a_portuguese_query_is_searched_without_its_function_words() {
    let scratch = Scratch::new("recall_portuguese_drop");
    scratch.write(
        "tarifa.md",
        "# Tarifa\n\nA nota da tarifa de energia: 0,87 por kwh.\n",
    );
    scratch.write("filler.md", "# Filler\n\ntem de de tem\n");
    let notes = scratch.path("notes");
    assert_eq!(scratch.run(&["remember", &notes]).code, 0);
    let query = [
        "recall", "tem", "alguma", "nota", "sobre", "a", "tarifa", "de", "energia",
    ];
    let by_default = scratch.run(&query);
    assert_eq!(by_default.code, 0, "{}", by_default.stderr);
    assert!(
        by_default.stdout.contains("0,87 por kwh"),
        "{}",
        by_default.stdout
    );
    assert!(
        !by_default.stdout.contains("tem de de tem"),
        "{}",
        by_default.stdout
    );
    scratch.config("[recall]\nlanguages = []\n");
    let off = scratch.run(&query);
    assert_eq!(off.code, 0, "{}", off.stderr);
    assert!(off.stdout.contains("tem de de tem"), "{}", off.stdout);
}

#[test]
fn a_lone_listed_word_is_still_searched() {
    let scratch = Scratch::new("recall_lone_listed_word");
    scratch.write("nas.md", "# Rede\n\nO nas da sala guarda os backups.\n");
    let notes = scratch.path("notes");
    assert_eq!(scratch.run(&["remember", &notes]).code, 0);
    let run = scratch.run(&["recall", "nas"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("guarda os backups"), "{}", run.stdout);
}
