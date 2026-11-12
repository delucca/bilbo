use std::fs;

use super::scratch::Scratch;

#[test]
fn remembering_a_folder_twice_keeps_everything_once() {
    let scratch = Scratch::new("twice");
    scratch.write(
        "a.md",
        "---\nid: 01JC5Q3V7XKM4P8R2T6W9YB0DE\n---\n# A\nline\n",
    );
    scratch.write("b.md", "no final newline");
    scratch.write("sub/c.md", "c\n");
    scratch.write(".hidden.md", "h\n");
    scratch.write(".git/d.md", "d\n");
    scratch.write("x.txt", "x\n");
    let before = scratch.notes_state();
    let notes = scratch.path("notes");

    let first = scratch.run(&["remember", &notes]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    assert_eq!(first.stderr, "");
    let mut lines = first.stdout.lines();
    assert_eq!(
        lines.next(),
        Some("new 3, appended 0, new generation 0, unchanged 0, refused 0")
    );
    let totals = lines.next().expect("a second line");
    assert!(
        totals.starts_with("records 7, raw bytes 66, stored bytes "),
        "{totals}"
    );
    assert!(!totals.ends_with(" 0"), "{totals}");
    assert_eq!(scratch.segments(), 0, "a young open.seg is not sealed");
    let open_seg = fs::read(scratch.dir.join("state/bilbo/open.seg")).expect("open.seg");
    assert!(open_seg.len() > 12);
    assert!(scratch.dir.join("state/bilbo/open.since").exists());
    assert!(scratch.dir.join("cache/bilbo/catalog.sqlite").exists());

    let second = scratch.run(&["remember", &notes]);
    assert_eq!(second.code, 0, "{}", second.stderr);
    assert_eq!(
        second.stdout,
        "new 0, appended 0, new generation 0, unchanged 3, refused 0\nrecords 0, raw bytes 0, stored bytes 0\n"
    );
    assert_eq!(scratch.segments(), 0);
    assert_eq!(scratch.notes_state(), before);
}

#[test]
fn remember_usage_errors_name_the_problem_and_touch_no_store() {
    const USAGE: &str = "bilbo: usage: bilbo remember <file|folder> [--kind <kind>] [--follow]\n\
                         bilbo:        bilbo remember\n\
                         bilbo: see 'bilbo remember --help'\n";
    let scratch = Scratch::new("usage");
    let cases: [(&[&str], &str); 8] = [
        (&["remember", "--follow"], "bilbo: missing <file|folder>\n"),
        (
            &["remember", "--kind", "session"],
            "bilbo: missing <file|folder>\n",
        ),
        (&["remember", ""], "bilbo: [file|folder] needs a value\n"),
        (
            &["remember", "x", "--kind"],
            "bilbo: --kind needs a value\n",
        ),
        (
            &["remember", "x", "--kind="],
            "bilbo: --kind needs a value\n",
        ),
        (&["remember", "--hook"], "bilbo: unknown option '--hook'\n"),
        (&["remember", "a", "b"], "bilbo: unexpected argument 'b'\n"),
        (
            &["remember", "x", "--kind", "nope"],
            "bilbo: unknown kind 'nope'; kinds: markdown, session\n",
        ),
    ];
    for (args, reason) in cases {
        let run = scratch.run(args);
        assert_eq!(run.code, 2, "{args:?}");
        assert_eq!(run.stdout, "", "{args:?}");
        assert_eq!(run.stderr, format!("{reason}{USAGE}"), "{args:?}");
    }
    for folder in ["store", "state/bilbo", "cache/bilbo"] {
        assert!(!scratch.dir.join(folder).exists(), "{folder}");
    }
}

#[test]
fn a_root_that_holds_0_20s_library_exits_2_and_writes_nothing_even_with_nothing_to_keep() {
    for files in [false, true] {
        let scratch = Scratch::new("older_root");
        let library = scratch.dir.join("store/library");
        fs::create_dir_all(&library).expect("a 0.20 library folder");
        if files {
            scratch.write("a.md", "a\n");
        }
        let run = scratch.run(&["remember", &scratch.path("notes")]);
        assert_eq!(run.code, 2, "files: {files}");
        assert_eq!(run.stdout, "", "files: {files}");
        assert_eq!(
            run.stderr,
            format!(
                "bilbo: {} belongs to bilbo 0.20; set BILBO_HOME or paths.root to another folder\n",
                library.display()
            ),
            "files: {files}"
        );
        for folder in ["state", "cache"] {
            assert!(!scratch.dir.join(folder).exists(), "{folder}");
        }
        assert_eq!(
            fs::read_dir(scratch.dir.join("store"))
                .expect("store")
                .count(),
            1
        );
    }
}
