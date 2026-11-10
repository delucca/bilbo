use std::fs;

use super::scratch::Scratch;

const UUID: &str = "8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b";
const SESSION: &[u8] = include_bytes!(
    "../../../libs/rust/kinds/session/fixtures/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl"
);
const NOTE: &str = include_str!("../../../libs/rust/kinds/markdown/fixtures/sections.md");
const NOTE_ID: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DG";

/// A scratch home with one session and one note remembered.
fn remembered(test: &str) -> Scratch {
    let scratch = Scratch::new(test);
    let session = scratch.dir.join(format!(
        "home/.claude/projects/-home-me-src-garden/{UUID}.jsonl"
    ));
    fs::create_dir_all(session.parent().expect("a parent")).expect("folders");
    fs::write(&session, SESSION).expect("a session");
    scratch.write("sections.md", NOTE);
    for target in [
        session.to_str().expect("UTF-8").to_owned(),
        scratch.path("notes"),
    ] {
        let run = scratch.run(&["remember", &target]);
        assert_eq!(run.code, 0, "{}", run.stderr);
    }
    scratch
}

/// The record each block of a page starts at: the lines `#r<n> ...`.
fn records(page: &str) -> Vec<u64> {
    page.lines()
        .filter_map(|line| line.strip_prefix("#r")?.split_once(' '))
        .filter_map(|(number, _)| number.parse().ok())
        .collect()
}

#[test]
fn a_remembered_session_reads_as_its_outline_then_a_page_at_a_time_to_its_end() {
    let scratch = remembered("read_session");
    let outline = scratch.run(&["read", UUID]);
    assert_eq!((outline.code, outline.stderr.as_str()), (0, ""));
    let mut lines = outline.stdout.lines();
    let id = lines
        .next()
        .and_then(|line| line.strip_prefix("bilbo:"))
        .expect("the entry's id")
        .to_owned();
    assert_eq!(lines.next(), Some("kind: session"));
    assert!(
        outline.stdout.ends_with(
            "started: 2026-11-07 08:01 UTC\nrecords: 23\ngeneration: 1\noutline:\n\
             #r3 user: Thin the carrot rows, then tell me how many seedlings are left.\n\
             #r17 compact summary: This session is being continued. The carrots in the north bed were thinned from 42…\n\
             #r18 user: Here is the bed. What is the weed in the corner?\nend: #r1-23 of 23 records\n"
        ),
        "{}",
        outline.stdout
    );

    let mut seen = Vec::new();
    let mut cuts = 0;
    let mut command = format!("bilbo read {id}#r1-23 --budget 256");
    for page in 1.. {
        assert!(page <= 23, "the chain does not end");
        let args: Vec<&str> = command.split(' ').skip(1).collect();
        let run = scratch.run(&args);
        assert_eq!((run.code, run.stderr.as_str()), (0, ""), "{command}");
        seen.extend(records(&run.stdout));
        let last = run.stdout.lines().last().expect("a last line");
        // A cut is said on the last line, before what follows it.
        let last = match last
            .strip_prefix("cut: ")
            .and_then(|cut| cut.rsplit_once("; "))
        {
            Some((_, trailer)) => {
                cuts += 1;
                trailer
            }
            None => last,
        };
        match last.strip_prefix("more: ") {
            Some(next) => command = next.to_owned(),
            None => {
                assert!(
                    last.starts_with("end: #r") && last.ends_with(" of 23 records"),
                    "{last}"
                );
                break;
            }
        }
    }
    assert_eq!(
        seen,
        [3, 5, 6, 8, 9, 17, 18, 19, 20, 21],
        "each record that renders, once"
    );
    assert_eq!(cuts, 1, "the long tool result");

    let page = scratch.run(&["read", &format!("{}#r5-6", &UUID[..8])]);
    assert_eq!(
        page.stdout,
        "#r5 assistant: I'll count the rows first — então desbasto.\ntool_use Bash:\n\
         command: wc -l beds/north.csv\ndescription: Count the rows\ntimeout: 5000\n\
         run_in_background: false\n#r6 tool_result Bash:\n42 beds/north.csv\nend: #r5-6 of 23 records\n"
    );
}

#[test]
fn a_remembered_note_reads_back_by_heading() {
    let scratch = remembered("read_note");
    let outline = scratch.run(&["read", &format!("bilbo:{NOTE_ID}")]);
    assert_eq!((outline.code, outline.stderr.as_str()), (0, ""));
    assert!(
        outline.stdout.ends_with(
            "outline:\n#r1-7 frontmatter\n#r8-11 The garden log\n#r12-21 The garden log > Beds\n\
             #r22-27 The garden log > Watering\n#r28-31 The garden log > Watering > Tools\n\
             #r32-34 The garden log > Harvest\nend: #r1-34 of 34 records\n"
        ),
        "{}",
        outline.stdout
    );
    let section = scratch.run(&["read", &format!("{NOTE_ID}#r32-34")]);
    assert_eq!(
        section.stdout,
        "#r32-34 ## Harvest\n\nNothing yet.\nend: #r32-34 of 34 records\n"
    );
}

#[test]
fn read_exits_1_for_what_is_not_there_and_2_for_what_cannot_be_meant() {
    let scratch = remembered("read_exits");
    for (args, code, stderr) in [
        (
            vec!["read", "01JC5Q3V7XKM4P8R2T6W9YB0ZZ"],
            1,
            "bilbo: no entry 01JC5Q3V7XKM4P8R2T6W9YB0ZZ\n".to_owned(),
        ),
        (
            vec!["read", "01JC5Q3V7XKM4P8R2T6W9YB0DG#r35"],
            1,
            "bilbo: entry 01JC5Q3V7XKM4P8R2T6W9YB0DG has 34 records: #r35 is outside it\n"
                .to_owned(),
        ),
        (
            vec!["read", "0e1d2c3b"],
            1,
            "bilbo: no entry for the session 0e1d2c3b\n".to_owned(),
        ),
        (
            vec!["read", "nope"],
            2,
            "bilbo: 'nope' is not an entry id, a session id or 8 or more characters of one\n\
             bilbo: usage: bilbo read <ref> [--budget <tokens>]\n\
             bilbo:        bilbo read <ref> --outline [--budget <tokens>]\n\
             bilbo: see 'bilbo read --help'\n"
                .to_owned(),
        ),
        (
            vec!["read", NOTE_ID, "--budget", "many"],
            2,
            "bilbo: --budget must be a whole number of 256 or more, got \"many\"\n".to_owned(),
        ),
    ] {
        let run = scratch.run(&args);
        assert_eq!((run.code, run.stdout.as_str()), (code, ""), "{args:?}");
        assert_eq!(run.stderr, stderr, "{args:?}");
    }
    let none = Scratch::new("read_no_store");
    let run = none.run(&["read", NOTE_ID]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!("bilbo: no store at {}\n", none.path("store"))
    );
    assert!(!none.dir.join("store").exists());
}
