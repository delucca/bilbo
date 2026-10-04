mod common;

use std::path::PathBuf;

use common::{Run, TempDir, bilbo, bilbo_input, note_text, snapshot};

const ID_PLAIN: &str = "01M3EZ8NVEC2KJQNGK5DTK349R";
const ID_BOOK: &str = "01M3EZ8NVEC2KJQNGK5DTK3400";
const ID_LONG: &str = "01M3EZ8NVEC2KJQNGK5DTK3401";
const ID_NOTE: &str = "01M3EZ8NVEC2KJQNGK5DTK3402";
const ID_GUIDE: &str = "01M3EZ8NBEVNHZRTQ6T60171J2";
const ID_GONE: &str = "01M3EZ8NVEC2KJQNGK5DTK3499";
const ZERO: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

const PLAIN: &str =
    "# Plain\n\nA goroutine has a simple model: it is a function executing concurrently.\n";
const BOOK: &str = "# Book

Intro text before any heading is cited without an anchor.

## Concurrency

### Goroutines

A goroutine has a simple model: it is a function executing concurrently with other goroutines.

### Channels

Channels are the pipes that connect concurrent goroutines together.

## The `Option` type

Option values wrap something or nothing at all here.

## Lints

### needless_return

#### What it does

Checks for return statements at the end of a function body.

### redundant_clone

#### What it does

Checks for redundant clone calls on values never reused later.
";

struct Lab {
    dir: TempDir,
    root: PathBuf,
    state: PathBuf,
}

fn source(id: &str, body: &str) -> String {
    format!(
        "---\nid: {id}\nfetched: 2026-08-23\norigin: \"url: https://go.dev/doc\"\ndigest: {ZERO}\n---\n{body}"
    )
}

fn guide() -> String {
    format!(
        "---\nid: {ID_GUIDE}\ncreated: 2026-09-26T13:16-03:00\n---\n\n# Go\n\nThe lead says that guides are cited like any other file.\n"
    )
}

/// Sixty numbered sentences under a title; the title sits on line 7.
fn long_body() -> String {
    let mut body = String::from("# Long\n");
    for n in 1..=60 {
        body.push_str(&format!("Line {n} says the quick brown fox jumps over.\n"));
    }
    body
}

impl Lab {
    fn new(name: &str) -> Lab {
        let dir = TempDir::new(name);
        let root = dir.path().join("store");
        let state = dir.path().join("state");
        let lab = Lab { dir, root, state };
        lab.put("library/go/guide.md", &guide());
        lab.put("library/go/plain.md", &source(ID_PLAIN, PLAIN));
        lab.put("library/go/book.md", &source(ID_BOOK, BOOK));
        lab.put(
            "notes/decision-release-tags.md",
            &format!(
                "{}\nUse release tags for every published version of the tool.\n",
                note_text(ID_NOTE, "Release tags")
            ),
        );
        lab
    }

    fn put(&self, rel: &str, text: &str) {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn env(&self) -> Vec<(&str, &str)> {
        vec![
            ("BILBO_HOME", self.root.to_str().unwrap()),
            ("HOME", self.dir.path().to_str().unwrap()),
            ("XDG_STATE_HOME", self.state.to_str().unwrap()),
        ]
    }

    fn cite(&self, args: &[&str], draft: &str) -> Run {
        let mut full = vec!["cite"];
        full.extend(args);
        bilbo_input(self.dir.path(), &self.env(), &full, draft)
    }

    fn run(&self, args: &[&str]) -> Run {
        bilbo(self.dir.path(), &self.env(), args)
    }

    fn path(&self, rel: &str) -> String {
        self.root.join(rel).display().to_string()
    }

    fn plan(&self, refs: &[&str], extra: &[&str]) -> String {
        let mut args = vec!["library", "plan"];
        args.extend(refs);
        args.extend(extra);
        let run = self.run(&args);
        assert_eq!(run.code, 0, "{}", run.stderr);
        run.stdout
            .lines()
            .find_map(|l| l.strip_prefix("plan: "))
            .unwrap()
            .to_string()
    }

    fn read(&self, plan: &str, slices: &[&str]) {
        let mut args = vec!["library", "read", plan];
        args.extend(slices);
        let run = self.run(&args);
        assert_eq!(run.code, 0, "{}", run.stderr);
    }
}

fn cited(id: &str, anchor: Option<&str>, quote: &str) -> String {
    match anchor {
        Some(anchor) => format!("bilbo:{id}#{anchor} \"{quote}\"\n"),
        None => format!("bilbo:{id} \"{quote}\"\n"),
    }
}

fn line_quote(n: usize) -> String {
    cited(
        ID_LONG,
        None,
        &format!("Line {n} says the quick brown fox jumps over"),
    )
}

/// The rows before the `citations:` line, each split at its tabs.
fn rows(stdout: &str) -> Vec<Vec<&str>> {
    stdout
        .lines()
        .take_while(|l| !l.starts_with("citations: "))
        .map(|l| l.split('\t').collect())
        .collect()
}

fn lines_with<'a>(stdout: &'a str, prefix: &str) -> Vec<&'a str> {
    stdout.lines().filter(|l| l.starts_with(prefix)).collect()
}

const GOROUTINE: &str = "A goroutine has a simple model: it is a function executing concurrently";

// Verdicts

#[test]
fn a_quote_under_its_anchor_is_ok() {
    let lab = Lab::new("cite-ok");
    let run = lab.cite(
        &[],
        &cited(ID_BOOK, Some("Concurrency > Goroutines"), GOROUTINE),
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty());
    let found = rows(&run.stdout);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0][..4],
        [
            "1",
            "ok",
            &format!("{ID_BOOK}#Concurrency > Goroutines"),
            &lab.path("library/go/book.md")
        ]
    );
    assert!(found[0][4].starts_with("lines "), "{}", found[0][4]);
    assert!(run.stdout.ends_with("citations: 1 checked, 1 ok\n"));
}

#[test]
fn a_trailing_part_of_the_path_and_a_section_with_subsections() {
    let lab = Lab::new("cite-trailing");
    let draft = [
        cited(
            ID_BOOK,
            Some("needless_return > What it does"),
            "Checks for return statements at the end of a function body",
        ),
        cited(
            ID_BOOK,
            Some("Concurrency"),
            "Channels are the pipes that connect concurrent goroutines",
        ),
    ]
    .concat();
    let run = lab.cite(&[], &draft);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert_eq!(rows(&run.stdout).iter().filter(|r| r[1] == "ok").count(), 2);
}

#[test]
fn markup_in_a_heading_does_not_break_the_anchor() {
    let lab = Lab::new("cite-markup");
    let run = lab.cite(
        &[],
        &cited(
            ID_BOOK,
            Some("The Option type"),
            "Option values wrap something or nothing at all here",
        ),
    );
    assert_eq!(rows(&run.stdout)[0][1], "ok", "{}", run.stdout);
}

#[test]
fn the_title_is_not_an_anchor() {
    let lab = Lab::new("cite-title");
    let run = lab.cite(&[], &cited(ID_BOOK, Some("Book"), GOROUTINE));
    assert_eq!(run.code, 1);
    let row = &rows(&run.stdout)[0];
    assert_eq!(row[1], "anchor_missing");
    assert!(row[4].contains("Concurrency > Goroutines"), "{}", row[4]);
}

#[test]
fn a_wrong_anchor_is_quote_elsewhere_and_names_the_section() {
    let lab = Lab::new("cite-elsewhere");
    let run = lab.cite(
        &[],
        &cited(
            ID_BOOK,
            Some("Goroutines"),
            "Channels are the pipes that connect concurrent goroutines",
        ),
    );
    assert_eq!(run.code, 0, "{}", run.stdout);
    let row = &rows(&run.stdout)[0];
    assert_eq!(row[1], "quote_elsewhere");
    assert!(row[4].contains("Concurrency > Channels"), "{}", row[4]);
}

#[test]
fn no_anchor_in_a_file_with_sections() {
    let lab = Lab::new("cite-noanchor");
    let draft = [
        cited(
            ID_BOOK,
            None,
            "Intro text before any heading is cited without an anchor",
        ),
        cited(ID_BOOK, None, GOROUTINE),
        cited(ID_PLAIN, None, GOROUTINE),
    ]
    .concat();
    let run = lab.cite(&[], &draft);
    assert_eq!(run.code, 0, "{}", run.stdout);
    let found = rows(&run.stdout);
    assert_eq!(found[0][1], "ok");
    assert_eq!(found[1][1], "quote_elsewhere");
    assert!(
        found[1][4].contains("Concurrency > Goroutines"),
        "{}",
        found[1][4]
    );
    assert_eq!(found[2][1], "ok");
}

#[test]
fn a_repeated_heading_is_ambiguous() {
    let lab = Lab::new("cite-ambiguous");
    let run = lab.cite(
        &[],
        &cited(
            ID_BOOK,
            Some("What it does"),
            "Checks for return statements at the end of a function body",
        ),
    );
    assert_eq!(run.code, 0, "{}", run.stdout);
    let row = &rows(&run.stdout)[0];
    assert_eq!(row[1], "ambiguous");
    assert!(
        row[4].contains("Lints > needless_return > What it does"),
        "{}",
        row[4]
    );
}

#[test]
fn a_retyped_quote_is_missing_with_a_hint() {
    let lab = Lab::new("cite-missing");
    let run = lab.cite(
        &[],
        &cited(
            ID_BOOK,
            Some("Concurrency > Goroutines"),
            "A goroutine has a complicated model: it is a function running concurrently",
        ),
    );
    assert_eq!(run.code, 1);
    let row = &rows(&run.stdout)[0];
    assert_eq!(row[1], "quote_missing");
    assert!(row[4].starts_with("nearest passage: "), "{}", row[4]);
    assert!(row[4].contains("goroutine"), "{}", row[4]);
}

#[test]
fn an_unknown_anchor_and_an_unknown_id_fail() {
    let lab = Lab::new("cite-fail-kinds");
    let draft = [
        cited(ID_BOOK, Some("Nope"), GOROUTINE),
        cited(ID_GONE, None, GOROUTINE),
    ]
    .concat();
    let run = lab.cite(&[], &draft);
    assert_eq!(run.code, 1);
    let found = rows(&run.stdout);
    assert_eq!(found[0][1], "anchor_missing");
    assert_eq!(found[1][1], "id_missing");
    assert_eq!(found[1][3], "-");
}

#[test]
fn five_words_are_too_short_and_pass() {
    let lab = Lab::new("cite-short");
    let run = lab.cite(
        &[],
        &cited(ID_BOOK, Some("Goroutines"), "A goroutine has a simple"),
    );
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert_eq!(rows(&run.stdout)[0][1], "too_short");
}

#[test]
fn a_note_and_a_guide_resolve() {
    let lab = Lab::new("cite-note-guide");
    let draft = [
        cited(
            ID_NOTE,
            None,
            "Use release tags for every published version of the tool",
        ),
        cited(
            ID_GUIDE,
            None,
            "The lead says that guides are cited like any other file",
        ),
    ]
    .concat();
    let run = lab.cite(&[], &draft);
    assert_eq!(run.code, 0, "{}", run.stdout);
    let found = rows(&run.stdout);
    assert_eq!(found[0][1], "ok");
    assert_eq!(found[0][3], lab.path("notes/decision-release-tags.md"));
    assert_eq!(found[1][1], "ok");
    assert_eq!(found[1][3], lab.path("library/go/guide.md"));
}

// The draft

#[test]
fn rows_come_in_draft_order_with_their_lines() {
    let lab = Lab::new("cite-rows");
    let draft = format!(
        "Intro\n\n{}Some prose.\n\n\n\n{}",
        cited(ID_BOOK, Some("Concurrency > Goroutines"), GOROUTINE),
        cited(
            ID_BOOK,
            Some("Goroutines"),
            "Channels are the pipes that connect concurrent goroutines"
        ),
    );
    let run = lab.cite(&[], &draft);
    assert_eq!(run.code, 0, "{}", run.stdout);
    let found = rows(&run.stdout);
    assert_eq!((found[0][0], found[0][1]), ("3", "ok"));
    assert_eq!((found[1][0], found[1][1]), ("8", "quote_elsewhere"));
    assert!(run.stdout.ends_with("citations: 2 checked, 1 ok\n"));
}

#[test]
fn a_draft_in_a_file_and_on_stdin_with_a_dash() {
    let lab = Lab::new("cite-file");
    let draft = cited(ID_BOOK, Some("Goroutines"), GOROUTINE);
    let file = lab.dir.path().join("draft.md");
    std::fs::write(&file, &draft).unwrap();
    let from_file = lab.cite(&[file.to_str().unwrap()], "ignored");
    let from_stdin = lab.cite(&[], &draft);
    let dashed = lab.cite(&["-"], &draft);
    assert_eq!(from_file.code, 0, "{}", from_file.stderr);
    assert_eq!(from_file.stdout, from_stdin.stdout);
    assert_eq!(dashed.stdout, from_stdin.stdout);
}

#[test]
fn two_citations_on_one_line_get_two_rows() {
    let lab = Lab::new("cite-oneline");
    let draft = format!(
        "bilbo:{ID_BOOK}#Goroutines \"{GOROUTINE}\" and bilbo:{ID_PLAIN} \"{GOROUTINE}\"\n"
    );
    let run = lab.cite(&[], &draft);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert_eq!(rows(&run.stdout).len(), 2);
    assert!(run.stdout.ends_with("citations: 2 checked, 2 ok\n"));
}

#[test]
fn no_citations_is_a_warning_and_exit_0() {
    let lab = Lab::new("cite-none");
    let run = lab.cite(&[], "bilbo: no store at /tmp/x\nplain prose\n");
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, "citations: 0 checked, 0 ok\n");
    assert_eq!(run.stderr, "bilbo: no citations found\n");
}

#[test]
fn a_citation_without_a_quote_and_the_old_form_are_noticed() {
    let lab = Lab::new("cite-notices");
    let draft = format!(
        "intro\n\n\n\nbilbo:{ID_BOOK}#Concurrency\nnote: /Users/a/Notebooks/x/library/go.md#Errors \"some quoted words here\"\nnote: ~/Notebooks/x/library/go.md#Errors \"some quoted words here\"\nnote: ./library/go.md#Errors \"some quoted words here\"\n{}",
        cited(ID_PLAIN, None, GOROUTINE)
    );
    let run = lab.cite(&[], &draft);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(rows(&run.stdout).len(), 1);
    assert!(run.stderr.contains("line 5: "), "{}", run.stderr);
    assert!(run.stderr.contains("has no quote"), "{}", run.stderr);
    for n in [6, 7, 8] {
        assert!(
            run.stderr.contains(&format!("line {n}: ")) && run.stderr.contains("note:"),
            "{}",
            run.stderr
        );
    }
    assert!(run.stderr.lines().all(|l| l.starts_with("bilbo: ")));
}

#[test]
fn warnings_pass() {
    let lab = Lab::new("cite-warnings");
    let draft = [
        cited(ID_BOOK, Some("Goroutines"), GOROUTINE),
        cited(ID_BOOK, Some("Goroutines"), "A goroutine has a simple"),
        cited(
            ID_BOOK,
            Some("Goroutines"),
            "Channels are the pipes that connect concurrent goroutines",
        ),
        cited(
            ID_BOOK,
            Some("What it does"),
            "Checks for return statements at the end of a function body",
        ),
    ]
    .concat();
    let run = lab.cite(&[], &draft);
    assert_eq!(run.code, 0, "{}", run.stdout);
    let verdicts: Vec<&str> = rows(&run.stdout).iter().map(|r| r[1]).collect();
    assert_eq!(
        verdicts,
        ["ok", "too_short", "quote_elsewhere", "ambiguous"]
    );
}

#[test]
fn one_failure_among_ten_fails_with_every_row_printed() {
    let lab = Lab::new("cite-ten");
    let mut draft = String::new();
    for i in 0..10 {
        draft.push_str(&if i == 6 {
            cited(
                ID_BOOK,
                Some("Goroutines"),
                "A quote that the source never held at all",
            )
        } else {
            cited(ID_PLAIN, None, GOROUTINE)
        });
    }
    let run = lab.cite(&[], &draft);
    assert_eq!(run.code, 1);
    assert_eq!(rows(&run.stdout).len(), 10);
    assert!(run.stdout.ends_with("citations: 10 checked, 9 ok\n"));
}

#[test]
fn a_missing_draft_exits_1() {
    let lab = Lab::new("cite-nodraft");
    let run = lab.cite(&["/tmp/bilbo-no-such-draft.md"], "");
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.contains("/tmp/bilbo-no-such-draft.md"),
        "{}",
        run.stderr
    );
}

#[test]
fn no_store_exits_1() {
    let dir = TempDir::new("cite-nostore");
    let root = dir.path().join("none");
    let env = [
        ("BILBO_HOME", root.to_str().unwrap()),
        ("HOME", dir.path().to_str().unwrap()),
    ];
    let run = bilbo_input(
        dir.path(),
        &env,
        &["cite"],
        &cited(ID_BOOK, None, GOROUTINE),
    );
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!("bilbo: no store at {}\n", root.display())
    );
}

#[test]
fn usage_errors_exit_2() {
    let lab = Lab::new("cite-usage");
    for args in [vec!["--frob"], vec!["a.md", "b.md"], vec!["--plan"]] {
        let run = lab.cite(&args, "");
        assert_eq!(run.code, 2, "{args:?}: {}", run.stderr);
        assert!(run.stdout.is_empty());
    }
}

#[test]
fn cite_reads_and_writes_nothing_of_the_store() {
    let lab = Lab::new("cite-readonly");
    let before = snapshot(&lab.root);
    let run = lab.cite(&[], &cited(ID_BOOK, Some("Goroutines"), GOROUTINE));
    assert_eq!(run.code, 0);
    assert_eq!(snapshot(&lab.root), before);
    assert!(!lab.state.exists());
}

#[test]
fn cite_ignores_the_config() {
    let lab = Lab::new("cite-config");
    let missing = lab.dir.path().join("no-such-config");
    let mut env = lab.env();
    env.push(("BILBO_CONFIG", missing.to_str().unwrap()));
    let run = bilbo_input(
        lab.dir.path(),
        &env,
        &["cite"],
        &cited(ID_BOOK, Some("Goroutines"), GOROUTINE),
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty());
}

// Plans

fn long_lab(name: &str) -> Lab {
    let lab = Lab::new(name);
    lab.put("library/go/long.md", &source(ID_LONG, &long_body()));
    lab
}

#[test]
fn a_quote_from_a_read_slice_is_ok_and_from_an_unread_one_fails() {
    let lab = long_lab("cite-plan-read");
    let plan = lab.plan(&["go/long"], &["--slice-lines", "20"]);
    lab.read(&plan, &["3"]);
    let read = lab.run(&["library", "read", &plan, "3", "--part", "2/2"]);
    let last = read
        .stdout
        .lines()
        .rev()
        .nth(1)
        .unwrap()
        .split_once('\t')
        .unwrap()
        .1
        .to_string();
    let n: usize = last.split(' ').nth(1).unwrap().parse().unwrap();
    let ok = lab.cite(&["--plan", &plan], &line_quote(n));
    assert_eq!(ok.code, 0, "{}{}", ok.stdout, ok.stderr);
    assert_eq!(rows(&ok.stdout)[0][1], "ok");
    let unread = lab.cite(&["--plan", &plan], &line_quote(1));
    assert_eq!(unread.code, 1);
    let row = &rows(&unread.stdout)[0];
    assert_eq!(row[1], "unread");
    assert!(row[4].contains("(slice 1)"), "{}", row[4]);
    assert!(row[4].starts_with("lines 8-8 "), "{}", row[4]);
}

#[test]
fn a_source_outside_the_plan_is_unread() {
    let lab = long_lab("cite-plan-outside");
    let plan = lab.plan(&["go/book"], &[]);
    lab.read(&plan, &["1"]);
    let run = lab.cite(&["--plan", &plan], &line_quote(3));
    assert_eq!(run.code, 1);
    let row = &rows(&run.stdout)[0];
    assert_eq!(row[1], "unread");
    assert!(row[4].contains("no plan"), "{}", row[4]);
}

#[test]
fn a_source_that_changed_since_its_plan_is_unread() {
    let lab = long_lab("cite-plan-changed");
    let plan = lab.plan(&["go/long"], &[]);
    lab.read(&plan, &["1"]);
    let changed = source(ID_LONG, &long_body()).replace(ZERO, &ZERO.replace('0', "1"));
    lab.put("library/go/long.md", &changed);
    let run = lab.cite(&["--plan", &plan], &line_quote(3));
    assert_eq!(run.code, 1);
    let row = &rows(&run.stdout)[0];
    assert_eq!(row[1], "unread");
    assert!(row[4].contains("changed since its plan"), "{}", row[4]);
}

#[test]
fn notes_and_guides_are_never_unread() {
    let lab = Lab::new("cite-plan-notes");
    let plan = lab.plan(&["go/plain"], &[]);
    let draft = [
        cited(
            ID_NOTE,
            None,
            "Use release tags for every published version of the tool",
        ),
        cited(
            ID_GUIDE,
            None,
            "The lead says that guides are cited like any other file",
        ),
    ]
    .concat();
    let run = lab.cite(&["--plan", &plan], &draft);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(rows(&run.stdout).iter().all(|r| r[1] == "ok"));
}

#[test]
fn an_unknown_plan_exits_1() {
    let lab = Lab::new("cite-plan-unknown");
    let run = lab.cite(&["--plan", ID_GONE], "");
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains(ID_GONE), "{}", run.stderr);
}

#[test]
fn a_plan_that_is_no_plan_id_exits_1_and_reads_nothing() {
    let lab = Lab::new("cite-plan-notid");
    for name in ["not-a-plan", "../x"] {
        let run = lab.cite(&["--plan", name], "");
        assert_eq!(run.code, 1, "{name}: {}", run.stderr);
        assert!(run.stdout.is_empty());
        assert!(
            run.stderr.contains(&format!("no plan '{name}'")),
            "{}",
            run.stderr
        );
    }
    assert_eq!(lab.cite(&["--plan="], "").code, 2);
}

#[test]
fn a_plan_needs_a_state_folder() {
    let lab = Lab::new("cite-plan-nostate");
    let env = [
        ("BILBO_HOME", lab.root.to_str().unwrap()),
        ("HOME", "relative/home"),
    ];
    let run = bilbo_input(
        lab.dir.path(),
        &env,
        &["cite", "--plan", ID_GONE],
        &cited(ID_BOOK, None, GOROUTINE),
    );
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.contains("XDG_STATE_HOME") && run.stderr.contains("HOME"),
        "{}",
        run.stderr
    );
}

#[test]
fn two_plans_are_read_as_one_and_each_gets_its_lines() {
    let lab = long_lab("cite-plan-two");
    let first = lab.plan(&["go/long"], &["--slice-lines", "20"]);
    let second = lab.plan(&["go/long"], &["--slice-lines", "20"]);
    assert_ne!(first, second);
    lab.read(&first, &["1"]);
    lab.read(&second, &["3"]);
    let read = lab.run(&["library", "read", &second, "3"]);
    let (_, text) = read
        .stdout
        .lines()
        .rev()
        .nth(1)
        .unwrap()
        .split_once('\t')
        .unwrap();
    let late: usize = text.split(' ').nth(1).unwrap().parse().unwrap();
    let draft = [line_quote(1), line_quote(late)].concat();
    let one = lab.cite(&["--plan", &first], &draft);
    assert_eq!(one.code, 1, "{}", one.stdout);
    let both = lab.cite(&["--plan", &first, "--plan", &second], &draft);
    assert_eq!(both.code, 0, "{}{}", both.stdout, both.stderr);
    assert_eq!(lines_with(&both.stdout, "coverage: ").len(), 2);
    assert_eq!(lines_with(&both.stdout, "picked: ").len(), 2);
    assert!(
        lines_with(&both.stdout, "coverage: ")[0].starts_with(&format!("coverage: plan {first}: "))
    );
}

// Coverage

#[test]
fn coverage_merges_unread_slices_and_ends_with_none_when_all_are_read() {
    let lab = long_lab("cite-coverage");
    let planned = lab.run(&["library", "plan", "go/long", "--slice-lines", "10"]);
    assert_eq!(planned.code, 0, "{}", planned.stderr);
    let plan = lab.plan(&["go/long"], &["--slice-lines", "10"]);
    let count = planned.stdout.split_once("\n\n").unwrap().1.lines().count();
    assert!(count >= 5, "{count}");
    let all: Vec<String> = (1..=count).map(|i| i.to_string()).collect();
    let first: Vec<&str> = all[..count - 2].iter().map(String::as_str).collect();
    lab.read(&plan, &first);
    let run = lab.cite(&["--plan", &plan], "");
    assert_eq!(run.code, 0, "{}", run.stderr);
    let coverage = lines_with(&run.stdout, "coverage: ");
    assert_eq!(coverage.len(), 1);
    assert!(
        coverage[0].starts_with(&format!(
            "coverage: plan {plan}: read {} of {count} slices (",
            count - 2
        )),
        "{}",
        coverage[0]
    );
    assert!(
        coverage[0].contains(" tokens); not read: go/long lines "),
        "{}",
        coverage[0]
    );
    assert!(
        coverage[0].ends_with(&format!("(slices {}-{count})", count - 1)),
        "{}",
        coverage[0]
    );
    let last = [all[count - 2].as_str()];
    lab.read(&plan, &last);
    lab.read(&plan, &[all[count - 1].as_str()]);
    let run = lab.cite(&["--plan", &plan], "");
    let coverage = lines_with(&run.stdout, "coverage: ");
    assert!(coverage[0].ends_with("not read: none"), "{}", coverage[0]);
    assert!(coverage[0].contains(&format!("read {count} of {count} slices")));
}

#[test]
fn adjacent_picks_of_one_source_merge_and_apart_ones_do_not() {
    let lab = Lab::new("cite-coverage-adjacent");
    let plan = lab.plan(
        &[
            "go/book#Concurrency > Goroutines",
            "go/book#Concurrency > Channels",
        ],
        &[],
    );
    let run = lab.cite(&["--plan", &plan], "");
    let coverage = lines_with(&run.stdout, "coverage: ")[0];
    assert!(coverage.contains("read 0 of 2 slices"), "{coverage}");
    let runs = coverage.split_once("not read: ").unwrap().1;
    assert!(
        runs.starts_with("go/book lines ") && runs.ends_with("(slices 1-2)"),
        "{runs}"
    );
    assert!(!runs.contains(", "), "{runs}");
    let plan = lab.plan(&["go/book#Goroutines", "go/book#The Option type"], &[]);
    let run = lab.cite(&["--plan", &plan], "");
    let coverage = lines_with(&run.stdout, "coverage: ")[0];
    let runs = coverage.split_once("not read: ").unwrap().1;
    assert_eq!(runs.matches("go/book lines ").count(), 2, "{runs}");
    assert!(
        runs.contains("(slice 1), ") && runs.ends_with("(slice 2)"),
        "{runs}"
    );
}

#[test]
fn picked_counts_the_corpus_as_it_is() {
    let lab = Lab::new("cite-picked");
    let section = "# T\n\n## Wrapping\n\nWrap the error with context before you return it.\n";
    for i in 1..=14 {
        lab.put(
            &format!("library/big/s{i:02}.md"),
            &source(&format!("01M3EZ8NVEC2KJQNGK5DTK36{i:02}"), section),
        );
    }
    let plan = lab.plan(&["big/s01", "big/s02#Wrapping"], &[]);
    let run = lab.cite(&["--plan", &plan], "");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        lines_with(&run.stdout, "picked: "),
        [format!(
            "picked: plan {plan}: big 2 of 14 sources (s01, s02#Wrapping)"
        )]
    );
}

#[test]
fn a_moved_source_is_named_by_its_current_corpus() {
    let lab = long_lab("cite-moved");
    let plan = lab.plan(&["go/long"], &["--slice-lines", "20"]);
    std::fs::create_dir_all(lab.root.join("library/rust")).unwrap();
    std::fs::rename(
        lab.root.join("library/go/long.md"),
        lab.root.join("library/rust/long-2.md"),
    )
    .unwrap();
    let run = lab.cite(&["--plan", &plan], "");
    assert_eq!(run.code, 0, "{}", run.stderr);
    let coverage = lines_with(&run.stdout, "coverage: ")[0];
    assert!(coverage.contains("rust/long-2 lines "), "{coverage}");
    assert_eq!(
        lines_with(&run.stdout, "picked: "),
        [format!("picked: plan {plan}: rust 1 of 1 sources (long-2)")]
    );
}

#[test]
fn no_plan_no_coverage() {
    let lab = long_lab("cite-nocoverage");
    let run = lab.cite(&[], &line_quote(3));
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(!run.stdout.contains("coverage:"));
    assert!(!run.stdout.contains("picked:"));
}

// Normalizing quotes

#[test]
fn markup_and_a_line_break_do_not_stop_a_match() {
    let lab = Lab::new("cite-norm-markup");
    lab.put(
        "library/go/marked.md",
        &source(
            ID_LONG,
            "# M\n\nIn Go the **zero value** is\nuseful, and the `bytes.Buffer` type shows it.\n",
        ),
    );
    let run = lab.cite(
        &[],
        &cited(
            ID_LONG,
            None,
            "the zero value is useful, and the bytes.Buffer type",
        ),
    );
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert_eq!(rows(&run.stdout)[0][1], "ok");
}

#[test]
fn a_pipe_in_a_table_cell_matches_its_entity() {
    let lab = Lab::new("cite-norm-pipe");
    lab.put(
        "library/go/table.md",
        &source(ID_LONG, "# T\n\n| Form | Meaning |\n| --- | --- |\n| a &#124; b | either one of the two forms |\n"),
    );
    let run = lab.cite(
        &[],
        &cited(ID_LONG, None, "a | b | either one of the two forms"),
    );
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert_eq!(rows(&run.stdout)[0][1], "ok");
}

#[test]
fn a_quote_copied_with_its_line_number_matches() {
    let lab = long_lab("cite-norm-prefix");
    let quote = "8\tLine 1 says the quick brown fox jumps over.";
    let run = lab.cite(&[], &cited(ID_LONG, None, quote));
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert_eq!(rows(&run.stdout)[0][1], "ok");
}

#[test]
fn fragments_must_come_in_order() {
    let lab = Lab::new("cite-norm-fragments");
    let run = lab.cite(
        &[],
        &cited(
            ID_BOOK,
            Some("Concurrency > Goroutines"),
            "A goroutine has a simple model ... executing concurrently with other goroutines",
        ),
    );
    assert_eq!(rows(&run.stdout)[0][1], "ok", "{}", run.stdout);
    let run = lab.cite(
        &[],
        &cited(
            ID_BOOK,
            Some("Concurrency > Goroutines"),
            "executing concurrently with other goroutines ... A goroutine has a simple model",
        ),
    );
    assert_eq!(run.code, 1);
    assert_eq!(rows(&run.stdout)[0][1], "quote_missing");
}
