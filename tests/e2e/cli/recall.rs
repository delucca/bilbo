use std::fs;
use std::os::unix::fs::PermissionsExt;

use super::scratch::{CLAUDE, MAIN, NOTE, SUBAGENT, Scratch, UUID, remembered};

fn hit_refs(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix("=== "))
        .filter_map(|header| header.split('\t').next())
        .collect()
}

fn capture_label_of(scratch: &Scratch, reference: &str) -> String {
    let entry = reference.split('#').next().expect("an entry");
    let read = scratch.run(&["read", entry]);
    assert_eq!(read.code, 0, "{}", read.stderr);
    read.stdout
        .lines()
        .find_map(|line| line.strip_prefix("label: "))
        .and_then(|label| label.strip_suffix(" (capture)"))
        .expect("a capture label")
        .to_owned()
}

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
    assert_eq!(
        reference.split('#').next().map(str::len),
        Some(10),
        "{header}"
    );
    assert_eq!(fields.len(), 2, "no kind, no date: {header}");
    assert!(
        !fields.iter().any(|field| field.starts_with("part ")),
        "the fixtures hold no record too long for a chunk: {header}"
    );
    let count: usize = fields[1]
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
        [("q2", "hits"), ("q2", "nothing"), ("q2", "hits")]
    );
    assert_eq!(fields[2][4], "markdown");
    assert_eq!(fields[2][5], "-");
    assert_eq!(fields[2][8], "heading");
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

#[test]
fn a_text_kept_twice_is_shown_once() {
    let scratch = Scratch::new("recall_copies");
    let note = "# Beds\n\nThe turnips grew well in the east bed this year.\n";
    scratch.write("a.md", note);
    scratch.write("b.md", note);
    let kept = scratch.run(&["remember", &scratch.path("notes")]);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    let run = scratch.run(&["recall", "turnips"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout.matches("\n=== ").count() + 1,
        1,
        "{}",
        run.stdout
    );
    assert!(
        run.stderr.contains("bilbo: passed over: 1 copy\n"),
        "{}",
        run.stderr
    );
}

#[test]
fn a_small_collection_among_a_large_one_still_fills_its_list_with_in() {
    let scratch = Scratch::new("recall_in_fills");
    for n in 0..5 {
        scratch.write(
            &format!("tillage/row{n}.md"),
            &format!("# Row {n}\n\nturnips turnips turnips turnips lie in furrow number{n} beside marker{n}\n"),
        );
    }
    for n in 0..3 {
        scratch.write(
            &format!("glasshouse/pane{n}.md"),
            &format!(
                "# Pane {n}\n\nOne turnips seedling sits on shelf{n} under lamp{n} and fan{n}\n"
            ),
        );
    }
    let kept = scratch.run(&["remember", &scratch.path("notes")]);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    let few = [("BILBO_RECALL_CANDIDATES", "3")];
    let all = scratch.run_with(&["recall", "turnips"], &few);
    assert_eq!(all.code, 0, "{}", all.stderr);
    let refs = hit_refs(&all.stdout);
    assert_eq!(refs.len(), 3, "{}", all.stdout);
    for reference in refs {
        assert!(capture_label_of(&scratch, reference).contains("/tillage/"));
    }
    let scoped = scratch.run_with(&["recall", "--in", "glasshouse", "turnips"], &few);
    assert_eq!(scoped.code, 0, "{}", scoped.stderr);
    let refs = hit_refs(&scoped.stdout);
    assert_eq!(refs.len(), 3, "{}", scoped.stdout);
    for reference in refs {
        assert!(capture_label_of(&scratch, reference).contains("/glasshouse/"));
    }
}

#[test]
fn a_scope_that_drops_a_kind_says_so() {
    let scratch = remembered("recall_in_kind");
    let run = scratch.run(&["recall", "--in=-home-me-src-garden", "carrots"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let line = run
        .stderr
        .lines()
        .find(|line| line.starts_with("bilbo: --in kept "))
        .unwrap_or_else(|| panic!("a scope line in {}", run.stderr));
    assert!(
        line.starts_with("bilbo: --in kept 1 entry and left out ")
            && line.ends_with(": markdown 1"),
        "{line}"
    );
}

#[test]
fn in_with_an_id_takes_a_sessions_subagents() {
    let scratch = Scratch::new("recall_in_subagents");
    let project = scratch
        .dir
        .join("home/.claude/projects/-home-me-src-garden");
    let files: [(String, &[u8]); 3] = [
        (format!("{MAIN}.jsonl"), CLAUDE),
        (
            format!("{MAIN}/subagents/agent-ad63c2b3c9e7a4b3d.jsonl"),
            SUBAGENT,
        ),
        (format!("{UUID}.jsonl"), super::scratch::SESSION),
    ];
    for (name, bytes) in files {
        let path = project.join(name);
        fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
        fs::write(path, bytes).expect("a session");
    }
    let kept = scratch.run(&["remember", project.to_str().expect("UTF-8")]);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    let main = scratch.run(&["read", MAIN]);
    assert_eq!(main.code, 0, "{}", main.stderr);
    let id = main
        .stdout
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("bilbo:"))
        .expect("the id");
    let short = &id[16..];
    let run = scratch.run(&["recall", "--in", short, "packets"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let refs = hit_refs(&run.stdout);
    assert!(!refs.is_empty(), "{}", run.stdout);
    for reference in refs {
        assert!(capture_label_of(&scratch, reference).contains("/subagents/agent-"));
    }
    let other = scratch.run(&["recall", "--in", "8f2b6d4e-1a3c", "packets"]);
    assert_eq!(
        (other.code, other.stdout.as_str()),
        (1, ""),
        "{}",
        other.stderr
    );
}

#[test]
fn a_value_that_names_nothing_never_searches_everything() {
    let scratch = remembered("recall_in_nothing");
    let alone = scratch.run(&["recall", "--in", "zzqqxx", "carrots"]);
    assert_eq!((alone.code, alone.stdout.as_str()), (1, ""));
    assert_eq!(
        alone.stderr,
        "bilbo: --in 'zzqqxx' names no entry\nbilbo: nothing matched\n"
    );
    let beside = scratch.run(&["recall", "--in", "zzqqxx", "--in", "notes", "carrots"]);
    assert_eq!(beside.code, 0, "{}", beside.stderr);
    assert!(
        beside
            .stderr
            .contains("bilbo: --in 'zzqqxx' names no entry\n"),
        "{}",
        beside.stderr
    );
    assert!(
        beside.stderr.contains("bilbo: --in kept 1 entry"),
        "{}",
        beside.stderr
    );
    assert!(!hit_refs(&beside.stdout).is_empty());
}

#[test]
fn in_with_a_folder_keeps_the_sessions_that_worked_there() {
    let scratch = Scratch::new("recall_in_folder");
    let claude = scratch
        .dir
        .join("home/.claude/projects/-home-me-src-garden");
    let codex = scratch.dir.join("home/.codex/sessions/2026/11/08");
    let files = [
        (
            claude.join("7d1e4b9a-3c5f-4a2e-8b6d-0f9e1c2a3b4d.jsonl"),
            include_bytes!(
                "../../../libs/rust/kinds/session/fixtures/7d1e4b9a-3c5f-4a2e-8b6d-0f9e1c2a3b4d.jsonl"
            )
            .as_slice(),
        ),
        (
            claude.join(format!("{UUID}.jsonl")),
            super::scratch::SESSION,
        ),
        (
            codex.join("rollout-2026-11-08T10-00-00-019a6c4d-3e5f-7a71-9b2c-4d6e8fa0b1c3.jsonl"),
            include_bytes!(
                "../../../libs/rust/kinds/session/fixtures/rollout-2026-11-08T10-00-00-019a6c4d-3e5f-7a71-9b2c-4d6e8fa0b1c3.jsonl"
            )
            .as_slice(),
        ),
    ];
    for (path, bytes) in files {
        fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
        fs::write(path, bytes).expect("a session");
    }
    for folder in [&claude, &codex] {
        let kept = scratch.run(&["remember", folder.to_str().expect("UTF-8")]);
        assert_eq!(kept.code, 0, "{}", kept.stderr);
    }
    let run = scratch.run(&["recall", "--in", "/home/me/src/shed", "frost", "compost"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stderr.contains("bilbo: --in kept 2 entries"),
        "{}",
        run.stderr
    );
    let refs = hit_refs(&run.stdout);
    assert!(!refs.is_empty(), "{}", run.stdout);
    for reference in refs {
        let capture = capture_label_of(&scratch, reference);
        assert!(
            capture.contains("7d1e4b9a-3c5f") || capture.contains("019a6c4d-3e5f"),
            "{capture}"
        );
    }
    let one = scratch.run(&["recall", "--in", "/home/me/src/greenhouse", "frost"]);
    assert_eq!(one.code, 0, "{}", one.stderr);
    assert!(
        one.stderr.contains("bilbo: --in kept 1 entry"),
        "{}",
        one.stderr
    );
}
