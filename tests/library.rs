mod common;

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use common::{Pages, Route, Run, TempDir, bilbo, bilbo_input, dead_url, note_text, snapshot};

const ID_A: &str = "01M3EZ8NVEC2KJQNGK5DTK349R";
const ID_B: &str = "01M3EZ8NVEC2KJQNGK5DTK3400";
const ZERO: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
const ORIGIN: &str = "url: https://go.dev/doc/effective_go";
/// The capture most land tests use; its SHA-256 is the name of its capture folder.
const CAPTURE: &str = "# Errors\n\nWrap them.\n";
const CAPTURE_SHA: &str = "3c422834eb609821025bad23ee69ff3eec51facaa35dc7f6663d4304e1fcc34f";

struct Lab {
    dir: TempDir,
    root: PathBuf,
    state: PathBuf,
}

impl Lab {
    fn new(name: &str) -> Lab {
        let dir = TempDir::new(name);
        let root = dir.path().join("store");
        std::fs::create_dir_all(&root).unwrap();
        let state = dir.path().join("state");
        Lab { dir, root, state }
    }

    fn env(&self) -> Vec<(&str, &str)> {
        vec![
            ("BILBO_HOME", self.root.to_str().unwrap()),
            ("HOME", self.dir.path().to_str().unwrap()),
            ("XDG_STATE_HOME", self.state.to_str().unwrap()),
            ("TZ", "<-03>3"),
        ]
    }

    fn run(&self, args: &[&str]) -> Run {
        bilbo(self.dir.path(), &self.env(), args)
    }

    fn library(&self, args: &[&str]) -> Run {
        let mut full = vec!["library"];
        full.extend(args);
        self.run(&full)
    }

    fn put(&self, rel: &str, text: &str) {
        let path = self.root.join("library").join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join("library").join(rel)).unwrap()
    }

    fn path(&self, rel: &str) -> String {
        self.root.join("library").join(rel).display().to_string()
    }

    fn input(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// Stages `text` and returns the stage id.
    fn stage(&self, text: &str, origin: &str, extra: &[&str]) -> String {
        let file = self.input("input.txt", text.as_bytes());
        let mut args = vec!["stage", file.to_str().unwrap(), "--origin", origin];
        args.extend(extra);
        let run = self.library(&args);
        assert_eq!(run.code, 0, "{}", run.stderr);
        field(&run.stdout, "stage").to_string()
    }

    fn land(&self, stage: &str, target: &str, extra: &[&str]) -> Run {
        let mut args = vec!["land", stage, target];
        args.extend(extra);
        self.library(&args)
    }

    /// Stages and lands `text`, keeping `keep`.
    fn land_text(&self, text: &str, target: &str, keep: &str, extra: &[&str]) -> Run {
        let stage = self.stage(text, ORIGIN, &[]);
        let mut args = vec!["--keep", keep];
        args.extend(extra);
        let run = self.land(&stage, target, &args);
        assert_eq!(run.code, 0, "{}", run.stderr);
        run
    }

    fn staged(&self) -> Vec<String> {
        let folder = self.state.join("bilbo/staging");
        let Ok(entries) = std::fs::read_dir(folder) else {
            return Vec::new();
        };
        entries
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect()
    }

    fn captures(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.root.join(".bilbo/captures")) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| !n.starts_with('.'))
            .collect();
        names.sort();
        names
    }

    fn spawn_land(&self, stage: &str, target: &str) -> Child {
        Command::new(env!("CARGO_BIN_EXE_bilbo"))
            .env_clear()
            .envs(self.env())
            .current_dir(self.dir.path())
            .args(["library", "land", stage, target, "--keep", "3-3"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }
}

fn field<'a>(stdout: &'a str, key: &str) -> &'a str {
    let prefix = format!("{key}: ");
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("no '{prefix}' line in {stdout:?}"))
}

fn today() -> String {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::fixed(jiff::tz::offset(-3)))
        .date()
        .to_string()
}

fn tokens(bytes: usize) -> usize {
    (bytes * 2).div_ceil(5)
}

/// A source with a made-up digest: browsing never checks it.
fn source(id: &str, extra: &[&str], body: &str) -> String {
    let mut out =
        format!("---\nid: {id}\nfetched: 2026-08-23\norigin: \"{ORIGIN}\"\ndigest: {ZERO}\n");
    for line in extra {
        out.push_str(&format!("{line}\n"));
    }
    out.push_str("---\n");
    out.push_str(body);
    out
}

/// A body of exactly `bytes` bytes with no section.
fn sized(bytes: usize) -> String {
    let mut body = String::from("# T\n");
    let mut left = bytes - 4;
    while left > 0 {
        let width = left.min(80);
        body.push_str(&"a".repeat(width - 1));
        body.push('\n');
        left -= width;
    }
    body
}

fn guide(title: &str, lead: &str, entries: &[(&str, &str)]) -> String {
    let mut out = format!(
        "---\nid: 01M3EZ8NBEVNHZRTQ6T60171J2\ncreated: 2026-09-26T13:16-03:00\n---\n\n# {title}\n\n{lead}\n"
    );
    for (name, prose) in entries {
        out.push_str(&format!("\n## {name}\n\n{prose}\n"));
    }
    out
}

const ERRORS_BODY: &str = "# Errors\n\n## Wrapping\naaaa\naaaa\naaaa\naaaa\n### Is and As\naaaa\naaaa\naaaa\naaaa\naaaa\naaaa\n";

fn lines_of(stdout: &str) -> Vec<&str> {
    stdout.lines().collect()
}

// Browsing

#[test]
fn two_corpora_list_their_rows() {
    let lab = Lab::new("lib-list");
    lab.put("go/guide.md", &guide("Go", "Lead.", &[]));
    lab.put("go/a.md", &source(ID_A, &[], &sized(1000)));
    lab.put("go/b.md", &source(ID_B, &[], &sized(1500)));
    lab.put("rust/guide.md", &guide("Rust", "Lead.", &[]));
    lab.put(
        "rust/c.md",
        &source("01M3EZ8NVEC2KJQNGK5DTK3401", &[], &sized(4000)),
    );
    let run = lab.library(&[]);
    assert_eq!(run.code, 0);
    assert_eq!(
        run.stdout,
        "go\t2 sources\t3 KB\t1000 tokens\tGo\nrust\t1 source\t4 KB\t1600 tokens\tRust\n"
    );
    assert!(run.stderr.is_empty());
}

#[test]
fn a_corpus_without_a_guide_has_a_dash_for_its_title() {
    let lab = Lab::new("lib-list-dash");
    lab.put("go/a.md", &source(ID_A, &[], &sized(1000)));
    assert_eq!(
        lab.library(&[]).stdout,
        "go\t1 source\t1 KB\t400 tokens\t-\n"
    );
}

#[test]
fn invalid_folders_and_hidden_entries_are_skipped() {
    let lab = Lab::new("lib-list-skip");
    lab.put("go/guide.md", &guide("Go", "Lead.", &[]));
    lab.put("Go-Old/guide.md", &guide("Old", "Lead.", &[]));
    lab.put("plan/guide.md", &guide("Plan", "Lead.", &[]));
    lab.put(".lock", "");
    lab.put("loose.md", "x");
    let run = lab.library(&[]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, "go\t0 sources\t0 KB\t0 tokens\tGo\n");
    assert!(run.stderr.is_empty());
}

#[test]
fn no_library_prints_nothing() {
    let lab = Lab::new("lib-list-none");
    for _ in 0..2 {
        let run = lab.library(&[]);
        assert_eq!(run.code, 0);
        assert!(run.stdout.is_empty() && run.stderr.is_empty());
        std::fs::create_dir_all(lab.root.join("library")).unwrap();
    }
}

#[test]
fn a_corpus_prints_its_guide_with_facts() {
    let lab = Lab::new("lib-corpus");
    lab.put(
        "go/guide.md",
        &guide(
            "Go",
            "Lead line.",
            &[("effective-go", "Two sentences. Here.")],
        ),
    );
    lab.put(
        "go/effective-go.md",
        &source(ID_A, &["capture: external"], &sized(1000)),
    );
    let run = lab.library(&["go"]);
    assert_eq!(run.code, 0);
    let facts = format!(
        "`effective-go.md` · {ID_A} · 1 KB · 400 tokens · fetched 2026-08-23 · 0 headings · capture external"
    );
    let expected = [
        &lab.path("go/guide.md"),
        "",
        "# Go",
        "",
        "Lead line.",
        "",
        "## effective-go",
        &facts,
        "",
        "Two sentences. Here.",
    ];
    assert_eq!(lines_of(&run.stdout), expected);
}

#[test]
fn a_source_without_an_entry_comes_last() {
    let lab = Lab::new("lib-corpus-noentry");
    lab.put("go/guide.md", &guide("Go", "Lead.", &[]));
    lab.put("go/inspecting-errors.md", &source(ID_A, &[], &sized(1000)));
    let run = lab.library(&["go"]);
    let lines = lines_of(&run.stdout);
    let n = lines.len();
    assert_eq!(lines[n - 3], "## inspecting-errors");
    assert!(lines[n - 2].starts_with("`inspecting-errors.md` · "));
    assert_eq!(lines[n - 1], "(no entry in guide.md)");
}

#[test]
fn an_entry_without_a_source_is_missing() {
    let lab = Lab::new("lib-corpus-missing");
    lab.put(
        "go/guide.md",
        &guide("Go", "Lead.", &[("effective-go", "x")]),
    );
    let run = lab.library(&["go"]);
    let lines = lines_of(&run.stdout);
    let at = lines.iter().position(|l| *l == "## effective-go").unwrap();
    assert_eq!(lines[at + 1], "`effective-go.md` · missing");
}

#[test]
fn a_missing_guide_prints_the_path_and_the_sources() {
    let lab = Lab::new("lib-corpus-noguide");
    lab.put("go/a.md", &source(ID_A, &[], &sized(1000)));
    let run = lab.library(&["go"]);
    assert_eq!(run.code, 0);
    let lines = lines_of(&run.stdout);
    assert_eq!(lines[0], lab.path("go/guide.md"));
    assert_eq!(lines[1], "## a");
    assert_eq!(lines[3], "(no entry in guide.md)");
    assert_eq!(lines.len(), 4);
}

#[test]
fn stub_and_stale_lines_are_printed_as_they_are() {
    let lab = Lab::new("lib-corpus-stub");
    let stale = "stale: re-ingested 2026-10-03; re-read the source and revise this entry.";
    lab.put(
        "go/guide.md",
        &guide(
            "Go",
            "TODO: describe this corpus.",
            &[("a", "TODO: describe this source."), ("b", stale)],
        ),
    );
    let run = lab.library(&["go"]);
    for line in [
        "TODO: describe this corpus.",
        "TODO: describe this source.",
        stale,
    ] {
        assert!(run.stdout.lines().any(|l| l == line), "{line}");
    }
}

#[test]
fn a_bad_value_in_the_facts_is_a_dash() {
    let lab = Lab::new("lib-facts-bad");
    lab.put("go/guide.md", &guide("Go", "Lead.", &[("a", "x")]));
    lab.put(
        "go/a.md",
        &source(ID_A, &[], &sized(1000)).replace("2026-08-23", "yesterday"),
    );
    let run = lab.library(&["go"]);
    assert!(run.stdout.contains(" · fetched - · "), "{}", run.stdout);
    assert!(run.stdout.contains(ID_A));
}

#[test]
fn a_catalog_is_marked() {
    let lab = Lab::new("lib-facts-catalog");
    let mut body = String::from("# Lints\n");
    for i in 0..41 {
        body.push_str(&format!("\n## lint{i}\n{}\n", "a".repeat(1400)));
    }
    assert!(body.len() > 55_000);
    lab.put(
        "rust/guide.md",
        &guide("Rust", "Lead.", &[("clippy-lints", "x")]),
    );
    lab.put("rust/clippy-lints.md", &source(ID_A, &[], &body));
    let run = lab.library(&["rust"]);
    assert!(
        run.stdout
            .lines()
            .any(|l| l.ends_with(" · 41 headings · catalog")),
        "{}",
        run.stdout
    );
    let shown = lab.library(&["show", "rust/clippy-lints"]);
    assert!(shown.stdout.contains("\ncatalog: yes\n"));
}

#[test]
fn corpus_argument_errors() {
    let lab = Lab::new("lib-corpus-errors");
    lab.put("go/guide.md", &guide("Go", "Lead.", &[]));
    let run = lab.library(&["haskell"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: no corpus 'haskell' in {}\n",
            lab.root.join("library").display()
        )
    );

    let run = lab.library(&["Go"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("bilbo: invalid corpus 'Go'"));

    for (subcommand, missing) in [("plan", "<ref>"), ("read", "<plan>")] {
        let run = lab.library(&[subcommand]);
        assert_eq!(run.code, 2);
        assert!(run.stdout.is_empty());
        assert!(
            run.stderr
                .starts_with(&format!("bilbo: missing {missing}\n")),
            "{}",
            run.stderr
        );
    }
    assert!(!lab.state.join("bilbo/plans").exists());
}

fn errors_lab(name: &str) -> Lab {
    let lab = Lab::new(name);
    lab.put("go/guide.md", &guide("Go", "Lead.", &[("errors", "x")]));
    lab.put("go/errors.md", &source(ID_A, &[], ERRORS_BODY));
    lab
}

#[test]
fn show_by_name_and_by_id_agree() {
    let lab = errors_lab("lib-show-ref");
    let by_name = lab.library(&["show", "go/errors"]);
    let by_id = lab.library(&["show", ID_A]);
    assert_eq!(by_name.code, 0);
    assert_eq!(by_name.stdout, by_id.stdout);
    assert!(by_name.stderr.is_empty());
}

#[test]
fn show_prints_the_header_and_one_row_per_section() {
    let lab = errors_lab("lib-show-rows");
    let run = lab.library(&["show", "go/errors"]);
    let expected = format!(
        "path: {}\nid: {ID_A}\ntitle: Errors\norigin: {ORIGIN}\nfetched: 2026-08-23\nlines: 7-20\ntokens: 35\nheadings: 2\ncatalog: no\n\n9-20\t31 tokens\tWrapping\n14-20\t18 tokens\tWrapping > Is and As\n",
        lab.path("go/errors.md")
    );
    assert_eq!(run.stdout, expected);
}

#[test]
fn a_headingless_source_has_no_rows() {
    let lab = Lab::new("lib-show-flat");
    lab.put("go/flat.md", &source(ID_A, &[], "# Flat\n\ntext\n"));
    let run = lab.library(&["show", "go/flat"]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("\nheadings: 0\n"));
    assert!(run.stdout.ends_with("\ncatalog: no\n\n"), "{}", run.stdout);
}

#[test]
fn a_source_whose_capture_is_not_held_has_no_capture_folder() {
    let lab = Lab::new("lib-show-no-capture");
    lab.put(
        "go/saved.md",
        &source(ID_A, &["capture: external"], "# Saved\n\ntext\n"),
    );
    let run = lab.library(&["show", "go/saved"]);
    assert!(run.stdout.contains("\ncapture: external\n"));
    assert!(!run.stdout.contains("capture folder:"));
}

#[test]
fn show_prints_the_capture_folder_that_records_the_source() {
    let lab = Lab::new("lib-show-folder");
    lab.put("go/old.md", &source(ID_A, &[], "# Old\n\ntext\n"));
    let folder = lab.root.join(".bilbo/captures/abc");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("landed"),
        format!("{ID_B}\t{ZERO}\t2026-10-03\n{ID_A}\t{ZERO}\t2026-10-03\n"),
    )
    .unwrap();
    let other = lab.root.join(".bilbo/captures/def");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("landed"), format!("{ID_A}\tsha256:other\tx\n")).unwrap();
    let run = lab.library(&["show", "go/old"]);
    assert_eq!(
        field(&run.stdout, "capture folder"),
        folder.display().to_string()
    );
}

#[test]
fn an_anchor_narrows_the_outline() {
    let lab = errors_lab("lib-show-anchor");
    let run = lab.library(&["show", "go/errors#Wrapping"]);
    assert_eq!(run.code, 0);
    let rows: Vec<&str> = run
        .stdout
        .lines()
        .skip_while(|l| !l.is_empty())
        .skip(1)
        .collect();
    assert_eq!(
        rows,
        [
            "9-20\t31 tokens\tWrapping",
            "14-20\t18 tokens\tWrapping > Is and As"
        ]
    );
    let run = lab.library(&["show", "go/errors#Is and As"]);
    let rows: Vec<&str> = run
        .stdout
        .lines()
        .skip_while(|l| !l.is_empty())
        .skip(1)
        .collect();
    assert_eq!(rows, ["14-20\t18 tokens\tWrapping > Is and As"]);
}

#[test]
fn show_resolves_a_backticked_heading_through_a_plain_anchor() {
    let lab = Lab::new("lib-show-markup");
    lab.put(
        "rust/book.md",
        &source(
            ID_A,
            &[],
            "# Book\n\n## The `Option` type\n\ntext\n\n## Other\n\nmore\n",
        ),
    );
    let run = lab.library(&["show", "rust/book#The Option type"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("The `Option` type"), "{}", run.stdout);
    assert!(!run.stdout.contains("Other"));
    let run = lab.library(&["show", "rust/book#The Options type"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
}

#[test]
fn an_ambiguous_anchor_lists_its_matches() {
    let lab = Lab::new("lib-show-ambiguous");
    lab.put(
        "rust/lints.md",
        &source(
            ID_A,
            &[],
            "# Lints\n\n## needless_return\n### What it does\ntext\n## needless_range_loop\n### What it does\ntext\n",
        ),
    );
    let run = lab.library(&["show", "rust/lints#What it does"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr
            .contains("bilbo: needless_return > What it does (line 10)\n")
    );
    assert!(
        run.stderr
            .contains("bilbo: needless_range_loop > What it does (line 13)\n")
    );
    let run = lab.library(&["show", "rust/lints#needless_return > What it does"]);
    assert_eq!(run.code, 0);
    assert!(
        run.stdout
            .ends_with("\n10-11\t9 tokens\tneedless_return > What it does\n")
    );
    let run = lab.library(&["show", "rust/lints#nothing"]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("no section 'nothing'"));
}

#[test]
fn depth_limits_the_rows() {
    let lab = errors_lab("lib-show-depth");
    let run = lab.library(&["show", "go/errors", "--depth", "1"]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("\tWrapping\n"));
    assert!(!run.stdout.contains("Is and As"));
    let before = lab.library(&["show", "--depth=1", "go/errors"]);
    assert_eq!(before.stdout, run.stdout);

    for bad in ["0", "-1", "x", ""] {
        let run = lab.library(&["show", "go/errors", "--depth", bad]);
        assert_eq!(run.code, 2, "{bad:?}");
        assert!(run.stdout.is_empty());
    }
}

#[test]
fn a_note_id_is_not_a_source() {
    let lab = errors_lab("lib-show-note");
    let notes = lab.root.join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    let id = "01M3YJ7R6HK6NQ30DCDB1P4DYB";
    std::fs::write(
        notes.join("decision-note-store.md"),
        note_text(id, "Note store"),
    )
    .unwrap();
    let run = lab.library(&["show", id]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr
            .contains(&notes.join("decision-note-store.md").display().to_string())
    );
}

#[test]
fn an_unknown_source_and_a_shared_id_exit_1() {
    let lab = errors_lab("lib-show-unknown");
    let run = lab.library(&["show", "go/effective-rust"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("'go/effective-rust'"));

    lab.put("rust/b.md", &source(ID_A, &[], "# B\n"));
    let run = lab.library(&["show", ID_A]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("go/errors") && run.stderr.contains("rust/b"));
}

#[test]
fn malformed_references_are_usage_errors() {
    let lab = errors_lab("lib-show-malformed");
    for reference in [
        "effective-go",
        "go/Effective_Go",
        "go/guide/x",
        "go/errors#",
    ] {
        let run = lab.library(&["show", reference]);
        assert_eq!(run.code, 2, "{reference}");
        assert!(run.stdout.is_empty());
    }
}

#[test]
fn option_rules() {
    let lab = errors_lab("lib-options");
    let run = lab.library(&["go", "--json"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("unknown option '--json'"));

    for args in [
        vec!["show"],
        vec!["go", "extra"],
        vec!["go", "--depth", "1"],
        vec!["show", "go/errors", "--keep", "1-2"],
        vec!["show", "go/errors", "extra"],
        vec!["show", "go/errors", "--depth"],
        vec!["show", "go/errors", "--depth", "1", "--depth", "2"],
        vec!["show", "go/errors", "--replace"],
        vec!["land", ID_A],
    ] {
        assert_eq!(lab.library(&args).code, 2, "{args:?}");
    }
    let run = lab.library(&["--", "show"]);
    assert_eq!(run.code, 2);
}

#[test]
fn library_ignores_the_config() {
    let lab = errors_lab("lib-config");
    let config = lab.input("config", b"embeder.url = http://embedder.example:8081\n");
    let mut env = lab.env();
    env.push(("BILBO_CONFIG", config.to_str().unwrap()));
    for args in [
        &["library"][..],
        &["library", "go"],
        &["library", "show", "go/errors"],
    ] {
        let run = bilbo(lab.dir.path(), &env, args);
        assert_eq!(run.code, 0);
        assert!(run.stderr.is_empty());
    }
}

#[test]
fn browsing_changes_nothing() {
    let lab = errors_lab("lib-readonly");
    let folder = lab.root.join(".bilbo/captures/abc");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("landed"), format!("{ID_A}\t{ZERO}\tx\n")).unwrap();
    let before = snapshot(&lab.root);
    assert_eq!(lab.library(&[]).code, 0);
    assert_eq!(lab.library(&["go"]).code, 0);
    assert_eq!(lab.library(&["show", "go/errors"]).code, 0);
    assert_eq!(snapshot(&lab.root), before);
}

#[test]
fn a_title_value_is_not_help() {
    let lab = Lab::new("lib-title-help");
    let run = lab.land(ID_A, "go/x", &["--keep", "1-1", "--title", "-h"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    let run = lab.library(&["land", "-h"]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.starts_with("usage: bilbo new"));
}

// Stage

fn page() -> String {
    let mut text = String::from("Home\nAbout\nDocs\nBlog\n# Effective Go\n");
    for n in 6..=900 {
        text.push_str(match n {
            10 => "## Section\n",
            20 => "### Three\n",
            30 => "```\n",
            31 => "## not a heading\n",
            32 => "```\n",
            _ => "text\n",
        });
    }
    text.push_str("\n\n\n");
    text
}

#[test]
fn stage_normalizes_and_writes_nothing_under_the_root() {
    let lab = Lab::new("stage-crlf");
    let before = snapshot(&lab.root);
    let file = lab.input("spec.txt", b"\xef\xbb\xbfone\r\ntwo\rthree");
    let run = lab.library(&[
        "stage",
        file.to_str().unwrap(),
        "--origin",
        "url: https://go.dev/ref/spec",
    ]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let stage = field(&run.stdout, "stage");
    let capture = lab
        .state
        .join("bilbo/staging")
        .join(stage)
        .join("capture.md");
    assert_eq!(field(&run.stdout, "capture"), capture.display().to_string());
    assert_eq!(std::fs::read(&capture).unwrap(), b"one\ntwo\nthree\n");
    assert_eq!(snapshot(&lab.root), before);
}

#[test]
fn stage_output_for_a_page_with_navigation() {
    let lab = Lab::new("stage-page");
    let text = page();
    let run = {
        let file = lab.input("page.txt", text.as_bytes());
        lab.library(&["stage", file.to_str().unwrap(), "--origin", ORIGIN])
    };
    assert_eq!(run.code, 0);
    let lines = lines_of(&run.stdout);
    assert!(lines[0].starts_with("stage: "));
    assert!(lines[1].starts_with("capture: "));
    assert_eq!(lines[2], "lines: 903");
    assert_eq!(lines[3], format!("tokens: {}", tokens(text.len())));
    assert_eq!(lines[4], "title: Effective Go");
    assert_eq!(lines[5], "keep: 6-900");
    assert_eq!(lines[6], "");
    assert_eq!(&lines[7..], ["5\t# Effective Go", "10\t## Section"]);
}

#[test]
fn stage_output_for_text_with_no_title() {
    let lab = Lab::new("stage-notitle");
    let body: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    let stage = lab.stage(&format!("{body}\n\n"), ORIGIN, &[]);
    assert_eq!(stage.len(), 26);
    let file = lab.input("again.txt", format!("\n{body}\n").as_bytes());
    let run = lab.library(&["stage", file.to_str().unwrap(), "--origin", ORIGIN]);
    assert_eq!(field(&run.stdout, "title"), "-");
    assert_eq!(field(&run.stdout, "keep"), "2-41");
    assert!(run.stdout.ends_with("keep: 2-41\n\n"));
    let run = {
        let file = lab.input("title-only.txt", b"# Only\n\n");
        lab.library(&["stage", file.to_str().unwrap(), "--origin", ORIGIN])
    };
    assert_eq!(field(&run.stdout, "keep"), "-");
}

#[test]
fn the_origin_and_date_reach_the_source() {
    let lab = Lab::new("stage-origin");
    let origin = "doc: The Go Programming Language, chapter 8";
    let stage = lab.stage(CAPTURE, origin, &["--fetched", "2026-08-23"]);
    let run = lab.land(&stage, "go/gopl", &["--keep", "3-3"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let text = lab.read("go/gopl.md");
    assert!(text.contains(&format!("\nfetched: 2026-08-23\norigin: \"{origin}\"\n")));
}

#[test]
fn the_default_date_is_today() {
    let lab = Lab::new("stage-today");
    let stage = lab.stage(CAPTURE, ORIGIN, &[]);
    lab.land(&stage, "go/errors", &["--keep", "3-3"]);
    assert!(
        lab.read("go/errors.md")
            .contains(&format!("\nfetched: {}\n", today()))
    );
}

#[test]
fn stage_refusals() {
    let lab = Lab::new("stage-refuse");
    let pdf = lab.input("doc.pdf", b"%PDF-1.7\n\xff\xfe\x00binary");
    let run = lab.library(&["stage", pdf.to_str().unwrap(), "--origin", ORIGIN]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("not valid UTF-8"));
    assert!(lab.staged().is_empty());

    let blank = lab.input("blank.txt", b" \n\t\r\n");
    let missing = lab.dir.path().join("missing.txt");
    for path in [blank, missing, lab.dir.path().to_path_buf()] {
        let run = lab.library(&["stage", path.to_str().unwrap(), "--origin", ORIGIN]);
        assert_eq!(run.code, 1, "{}", path.display());
        assert!(run.stdout.is_empty());
        assert!(lab.staged().is_empty());
    }
}

#[test]
fn stage_usage_errors() {
    let lab = Lab::new("stage-usage");
    let file = lab.input("spec.txt", b"text\n");
    let path = file.to_str().unwrap();
    let run = lab.library(&["stage", path]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("--origin"));
    for origin in [
        "web: https://go.dev",
        "https://go.dev",
        "url: ",
        "url: a\"b",
    ] {
        let run = lab.library(&["stage", path, "--origin", origin]);
        assert_eq!(run.code, 2, "{origin}");
        assert!(run.stderr.contains("--origin"));
    }
    for date in ["2026-02-30", "today", "2026-8-3"] {
        let run = lab.library(&["stage", path, "--origin", ORIGIN, "--fetched", date]);
        assert_eq!(run.code, 2, "{date}");
        assert!(run.stderr.contains("--fetched"));
    }
    let run = lab.library(&["stage", "--origin", ORIGIN]);
    assert_eq!(run.code, 2);
    assert!(lab.staged().is_empty());
}

#[test]
fn stage_needs_a_state_folder() {
    let lab = Lab::new("stage-nostate");
    let file = lab.input("spec.txt", b"text\n");
    let run = bilbo(
        lab.dir.path(),
        &[("BILBO_HOME", lab.root.to_str().unwrap())],
        &[
            "library",
            "stage",
            file.to_str().unwrap(),
            "--origin",
            ORIGIN,
        ],
    );
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("XDG_STATE_HOME") && run.stderr.contains("HOME"));
}

// Land

fn numbered(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

#[test]
fn a_first_source_in_a_new_corpus() {
    let lab = Lab::new("land-first");
    let text = page();
    let stage = lab.stage(&text, ORIGIN, &[]);
    let run = lab.land(&stage, "go/effective-go", &["--keep", "6-900"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty());
    let id = field(&run.stdout, "id");
    assert_eq!(id.len(), 26);
    assert_eq!(field(&run.stdout, "source"), lab.path("go/effective-go.md"));
    assert_eq!(field(&run.stdout, "guide"), lab.path("go/guide.md"));
    let sha = &lab.captures()[0];
    assert_eq!(
        field(&run.stdout, "capture folder"),
        lab.root
            .join(".bilbo/captures")
            .join(sha)
            .display()
            .to_string()
    );
    assert_eq!(run.stdout.lines().count(), 4);

    let source = lab.read("go/effective-go.md");
    let front: Vec<&str> = source.lines().take(8).collect();
    assert_eq!(front[0], "---");
    assert_eq!(front[1], format!("id: {id}"));
    assert_eq!(front[2], format!("fetched: {}", today()));
    assert_eq!(front[3], format!("origin: \"{ORIGIN}\""));
    assert!(front[4].starts_with("digest: sha256:"));
    assert_eq!(&front[5..], ["kept: 6-900", "capture: external", "---"]);
    assert_eq!(source.lines().nth(8), Some("# Effective Go"));

    let guide = lab.read("go/guide.md");
    let rest = guide.split_once("---\n\n").unwrap().1;
    assert_eq!(
        rest,
        "# go\n\nTODO: describe this corpus.\n\n## effective-go\n\nTODO: describe this source.\n"
    );
    assert!(guide.starts_with("---\nid: "));
    assert!(lab.staged().is_empty());
}

#[test]
fn the_whole_capture_has_no_kept_key() {
    let lab = Lab::new("land-whole");
    lab.land_text(&numbered(40), "go/errors", "1-40", &["--title", "Errors"]);
    let source = lab.read("go/errors.md");
    assert!(!source.contains("kept:"));
    assert!(source.contains("\ncapture: external\n"));
}

#[test]
fn ranges_are_kept_in_order_and_merged_when_they_touch() {
    let lab = Lab::new("land-ranges");
    let text = format!("# Book\n{}", numbered(899));
    lab.land_text(&text, "go/two", "6-400", &["--keep", "420-900"]);
    let source = lab.read("go/two.md");
    assert!(source.contains("\nkept: 6-400,420-900\n"));
    let body = source.split_once("\n---\n").unwrap().1;
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines[0], "# Book");
    assert_eq!(lines[2], "line 5");
    assert_eq!(lines[2 + 394], "line 399");
    assert_eq!(lines[2 + 395], "line 419");
    assert_eq!(lines.last(), Some(&"line 899"));

    lab.land_text(&text, "go/touch", "6-400,401-900", &[]);
    assert!(lab.read("go/touch.md").contains("\nkept: 6-900\n"));
}

#[test]
fn bad_ranges_are_usage_errors_that_write_nothing() {
    let lab = Lab::new("land-badranges");
    let stage = lab.stage(&numbered(900), ORIGIN, &[]);
    let before = snapshot(&lab.root);
    for keep in ["6-950", "6-400,300-900", "0-5", "9-3", "a-b", "5", ""] {
        let run = lab.land(&stage, "go/x", &["--keep", keep, "--title", "X"]);
        assert_eq!(run.code, 2, "{keep:?}");
        assert!(run.stderr.contains("--keep"), "{}", run.stderr);
        assert_eq!(snapshot(&lab.root), before);
    }
    let run = lab.land(&stage, "go/x", &["--title", "X"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("--keep"));
    assert!(lab.staged().contains(&stage));
}

#[test]
fn kept_lines_are_copied_byte_for_byte() {
    let lab = Lab::new("land-bytes");
    let kept =
        "trailing   \n\ttabbed\n<!-- a comment -->\n```go\n# not a heading\n## nor this\n```\nend";
    let text = format!("# Title\n{kept}\n");
    let run = lab.land_text(&text, "go/bytes", "2-9", &["--title", "Book"]);
    assert!(run.stderr.is_empty());
    let source = lab.read("go/bytes.md");
    let body = source.split_once("\n---\n").unwrap().1;
    assert_eq!(body, format!("# Book\n\n{kept}\n"));
}

#[test]
fn the_title_comes_from_the_capture() {
    let lab = Lab::new("land-title");
    lab.land_text(CAPTURE, "go/errors", "3-3", &[]);
    let source = lab.read("go/errors.md");
    assert_eq!(
        source.split_once("\n---\n").unwrap().1,
        "# Errors\n\nWrap them.\n"
    );
}

#[test]
fn headings_are_demoted_under_a_kept_h1() {
    let lab = Lab::new("land-demote");
    let text = "# Part one\n\ntext\n## Details\nmore\n```\n# in fence\n```\n###### six\n";
    let stage = lab.stage(text, ORIGIN, &[]);
    let run = lab.land(&stage, "go/book", &["--keep", "1-9", "--title", "Book"]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stderr.lines().count(), 1, "{}", run.stderr);
    assert!(run.stderr.contains("level-1 heading"));
    let source = lab.read("go/book.md");
    assert_eq!(
        source.split_once("\n---\n").unwrap().1,
        "# Book\n\n## Part one\n\ntext\n### Details\nmore\n```\n# in fence\n```\n####### six\n"
    );
}

#[test]
fn a_missing_or_unusable_title_is_a_usage_error() {
    let lab = Lab::new("land-notitle");
    let stage = lab.stage(&numbered(5), ORIGIN, &[]);
    let before = snapshot(&lab.root);
    let run = lab.land(&stage, "go/x", &["--keep", "1-5"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("--title"));
    for title in ["", "   ", "a\nb"] {
        let run = lab.land(&stage, "go/x", &["--keep", "1-5", "--title", title]);
        assert_eq!(run.code, 2, "{title:?}");
    }
    assert_eq!(snapshot(&lab.root), before);
}

#[test]
fn a_new_entry_is_appended() {
    let lab = Lab::new("land-entry");
    let existing = guide("Go", "Lead.", &[("a", "Prose a."), ("b", "Prose b.")]);
    lab.put("go/guide.md", &existing);
    lab.land_text(CAPTURE, "go/errors", "3-3", &[]);
    assert_eq!(
        lab.read("go/guide.md"),
        format!("{existing}\n## errors\n\nTODO: describe this source.\n")
    );
}

#[test]
fn an_entry_that_outlived_its_source_goes_stale() {
    let lab = Lab::new("land-outlived");
    lab.put(
        "go/guide.md",
        &guide("Go", "Lead.", &[("errors", "Old prose.")]),
    );
    lab.land_text(CAPTURE, "go/errors", "3-3", &[]);
    let guide = lab.read("go/guide.md");
    let stale = format!(
        "## errors\nstale: re-ingested {}; re-read the source and revise this entry.\n\nOld prose.\n",
        today()
    );
    assert!(guide.ends_with(&stale), "{guide}");
    assert!(!guide.contains("TODO: describe this source."));
}

fn land_with_prose(lab: &Lab, text: &str, fetched: &str) -> String {
    let stage = lab.stage(text, ORIGIN, &["--fetched", fetched]);
    let run = lab.land(&stage, "go/effective-go", &["--keep", "3-3"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    field(&run.stdout, "id").to_string()
}

#[test]
fn a_reingest_with_new_text_marks_the_entry_stale() {
    let lab = Lab::new("land-replace");
    let id = land_with_prose(&lab, CAPTURE, "2026-08-01");
    let guide = lab
        .read("go/guide.md")
        .replace("TODO: describe this source.", "Reviewed prose.");
    lab.put("go/guide.md", &guide);
    let old_digest = lab.read("go/effective-go.md");

    let stage = lab.stage(
        "# Errors\n\nWrap them well.\n",
        ORIGIN,
        &["--fetched", "2026-08-23"],
    );
    let run = lab.land(&stage, "go/effective-go", &["--keep", "3-3", "--replace"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(field(&run.stdout, "id"), id);
    let source = lab.read("go/effective-go.md");
    assert!(source.contains(&format!("id: {id}\nfetched: 2026-08-23\n")));
    assert_ne!(source, old_digest);
    assert!(source.ends_with("\nWrap them well.\n"));
    assert!(lab.read("go/guide.md").ends_with(&format!(
        "## effective-go\nstale: re-ingested {}; re-read the source and revise this entry.\n\nReviewed prose.\n",
        today()
    )));
}

#[test]
fn a_reingest_with_the_same_text_leaves_the_guide() {
    let lab = Lab::new("land-replace-same");
    let id = land_with_prose(&lab, CAPTURE, "2026-08-01");
    let guide = lab.read("go/guide.md");
    let stage = lab.stage(CAPTURE, ORIGIN, &["--fetched", "2026-08-23"]);
    let run = lab.land(&stage, "go/effective-go", &["--keep", "3-3", "--replace"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(lab.read("go/guide.md"), guide);
    let landed = lab
        .root
        .join(".bilbo/captures")
        .join(CAPTURE_SHA)
        .join("landed");
    assert_eq!(std::fs::read_to_string(landed).unwrap().lines().count(), 1);
    assert!(
        lab.read("go/effective-go.md")
            .contains(&format!("id: {id}\nfetched: 2026-08-23\n"))
    );
}

#[test]
fn a_taken_name_needs_replace() {
    let lab = Lab::new("land-taken");
    land_with_prose(&lab, CAPTURE, "2026-08-01");
    let before = snapshot(&lab.root);
    let stage = lab.stage("# Errors\n\nOther.\n", ORIGIN, &[]);
    let run = lab.land(&stage, "go/effective-go", &["--keep", "3-3"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("library/go/effective-go.md"));
    assert!(run.stderr.contains("--replace"));
    assert_eq!(snapshot(&lab.root), before);
    assert!(lab.staged().contains(&stage));
}

#[test]
fn nothing_to_replace() {
    let lab = Lab::new("land-noreplace");
    let before = snapshot(&lab.root);
    let stage = lab.stage(CAPTURE, ORIGIN, &[]);
    let run = lab.land(&stage, "go/effective-go", &["--keep", "3-3", "--replace"]);
    assert_eq!(run.code, 1);
    assert_eq!(snapshot(&lab.root), before);
    assert!(lab.staged().contains(&stage));
}

#[test]
fn the_capture_is_kept_with_its_landed_line() {
    let lab = Lab::new("land-capture");
    let run = lab.land_text(CAPTURE, "go/errors", "3-3", &[]);
    assert_eq!(lab.captures(), [CAPTURE_SHA]);
    let folder = lab.root.join(".bilbo/captures").join(CAPTURE_SHA);
    assert_eq!(
        field(&run.stdout, "capture folder"),
        folder.display().to_string()
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("capture.md")).unwrap(),
        CAPTURE
    );
    assert!(!folder.join("stage.json").exists());
    let source = lab.read("go/errors.md");
    let digest = source
        .lines()
        .find_map(|l| l.strip_prefix("digest: "))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(folder.join("landed")).unwrap(),
        format!("{}\t{digest}\t{}\n", field(&run.stdout, "id"), today())
    );
    let shown = lab.library(&["show", "go/errors"]);
    assert_eq!(
        field(&shown.stdout, "capture folder"),
        folder.display().to_string()
    );
}

#[test]
fn the_same_text_landed_twice_is_one_capture() {
    let lab = Lab::new("land-twice");
    lab.land_text(CAPTURE, "go/a", "3-3", &[]);
    lab.land_text(CAPTURE, "go/b", "3-3", &[]);
    assert_eq!(lab.captures(), [CAPTURE_SHA]);
    let landed = std::fs::read_to_string(
        lab.root
            .join(".bilbo/captures")
            .join(CAPTURE_SHA)
            .join("landed"),
    )
    .unwrap();
    assert_eq!(landed.lines().count(), 2);
}

#[test]
fn an_existing_capture_folder_is_left_as_it_was() {
    let lab = Lab::new("land-keep-capture");
    lab.land_text(CAPTURE, "go/a", "3-3", &[]);
    let folder = lab.root.join(".bilbo/captures").join(CAPTURE_SHA);
    std::fs::write(folder.join("note.txt"), "by hand").unwrap();
    let stamp = |name: &str| {
        let path = folder.join(name);
        (
            std::fs::read(&path).unwrap(),
            std::fs::metadata(&path).unwrap().modified().unwrap(),
        )
    };
    let (capture, note) = (stamp("capture.md"), stamp("note.txt"));
    lab.land_text(CAPTURE, "go/b", "3-3", &[]);
    assert_eq!(stamp("capture.md"), capture);
    assert_eq!(stamp("note.txt"), note);
    assert_eq!(
        std::fs::read_to_string(folder.join("landed"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[test]
fn files_beside_the_capture_reach_the_capture_folder() {
    let lab = Lab::new("land-extra-files");
    let stage = lab.stage(CAPTURE, ORIGIN, &[]);
    let folder = lab.state.join("bilbo/staging").join(&stage);
    std::fs::write(folder.join("raw"), "raw bytes").unwrap();
    let run = lab.land(&stage, "go/errors", &["--keep", "3-3"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let kept = lab.root.join(".bilbo/captures").join(CAPTURE_SHA);
    assert_eq!(
        std::fs::read_to_string(kept.join("raw")).unwrap(),
        "raw bytes"
    );
}

#[test]
fn a_duplicate_origin_warns_and_still_lands() {
    let lab = Lab::new("land-dup-origin");
    lab.land_text(CAPTURE, "go/effective-go", "3-3", &[]);
    let stage = lab.stage(CAPTURE, ORIGIN, &[]);
    let run = lab.land(&stage, "go/effective-go-2026", &["--keep", "3-3"]);
    assert_eq!(run.code, 0);
    assert_eq!(
        run.stderr,
        "bilbo: url: https://go.dev/doc/effective_go is also the origin of go/effective-go\n"
    );
    let stage = lab.stage(CAPTURE, "url: https://go.dev/ref/spec", &[]);
    let run = lab.land(&stage, "go/spec", &["--keep", "3-3"]);
    assert_eq!(run.code, 0);
    assert!(!run.stderr.contains("is also the origin of"));
}

#[test]
fn an_edited_capture_is_refused_with_nothing_written() {
    let lab = Lab::new("land-edited");
    let stage = lab.stage(CAPTURE, ORIGIN, &[]);
    let capture = lab
        .state
        .join("bilbo/staging")
        .join(&stage)
        .join("capture.md");
    std::fs::write(&capture, format!("{CAPTURE}typed by hand\n")).unwrap();
    let before = snapshot(&lab.root);
    let run = lab.land(&stage, "go/errors", &["--keep", "3-4"]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("changed since it was staged"));
    assert_eq!(snapshot(&lab.root), before);
    assert!(lab.staged().contains(&stage));
}

#[test]
fn an_unknown_or_malformed_stage() {
    let lab = Lab::new("land-nostage");
    let run = lab.land(ID_A, "go/x", &["--keep", "1-2"]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains(ID_A));
    let run = lab.land("not-a-stage", "go/x", &["--keep", "1-2"]);
    assert_eq!(run.code, 2);
}

#[test]
fn bad_targets_are_usage_errors_that_write_nothing() {
    let lab = Lab::new("land-targets");
    let stage = lab.stage(CAPTURE, ORIGIN, &[]);
    let before = snapshot(&lab.root);
    for target in [
        "go/guide",
        "read/errors",
        "plan/errors",
        "show/errors",
        "Go/errors",
        "go/Errors",
        "errors",
        "go/a/b",
        "go/",
    ] {
        let run = lab.land(&stage, target, &["--keep", "3-3"]);
        assert_eq!(run.code, 2, "{target}");
        assert_eq!(snapshot(&lab.root), before);
    }
}

#[test]
fn a_landed_source_passes_check_once_the_guide_is_written() {
    let lab = Lab::new("land-check");
    lab.land_text(CAPTURE, "go/errors", "3-3", &[]);
    let run = lab.run(&["check"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.contains("TODO stub"), "{}", run.stdout);

    let guide = lab
        .read("go/guide.md")
        .replace("TODO: describe this corpus.", "Go material for agents.")
        .replace("TODO: describe this source.", "How errors wrap.");
    lab.put("go/guide.md", &guide);
    let run = lab.run(&["check"]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(run.stdout.is_empty());
}

#[test]
fn a_replaced_source_passes_check() {
    let lab = Lab::new("land-check-replace");
    land_with_prose(&lab, CAPTURE, "2026-08-01");
    let stage = lab.stage("# Errors\n\nWrap them well.\n", ORIGIN, &[]);
    lab.land(&stage, "go/effective-go", &["--keep", "3-3", "--replace"]);
    let guide = lab
        .read("go/guide.md")
        .replace("TODO: describe this corpus.", "Go material.");
    let guide = guide
        .lines()
        .filter(|l| !l.starts_with("stale: ") && !l.starts_with("TODO: "))
        .collect::<Vec<_>>()
        .join("\n");
    lab.put("go/guide.md", &format!("{guide}\nAn entry.\n"));
    let run = lab.run(&["check"]);
    assert_eq!(run.code, 0, "{}", run.stdout);
}

// Concurrency

fn finish(child: Child) -> Run {
    let output = child.wait_with_output().unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

#[test]
fn two_lands_into_one_corpus_keep_both_entries() {
    for round in 0..5 {
        let lab = Lab::new(&format!("land-race-corpus-{round}"));
        let first = lab.stage(CAPTURE, ORIGIN, &[]);
        let second = lab.stage(CAPTURE, ORIGIN, &[]);
        let a = lab.spawn_land(&first, "go/a");
        let b = lab.spawn_land(&second, "go/b");
        let (a, b) = (finish(a), finish(b));
        assert_eq!((a.code, b.code), (0, 0), "{} {}", a.stderr, b.stderr);
        let guide = lab.read("go/guide.md");
        assert!(
            guide.contains("\n## a\n") && guide.contains("\n## b\n"),
            "{guide}"
        );
        assert!(lab.staged().is_empty());
    }
}

#[test]
fn two_lands_of_one_name_have_one_winner() {
    for round in 0..5 {
        let lab = Lab::new(&format!("land-race-name-{round}"));
        let first = lab.stage(CAPTURE, ORIGIN, &[]);
        let second = lab.stage("# Errors\n\nOther.\n", ORIGIN, &[]);
        let a = lab.spawn_land(&first, "go/errors");
        let b = lab.spawn_land(&second, "go/errors");
        let (a, b) = (finish(a), finish(b));
        let mut codes = [a.code, b.code];
        codes.sort();
        assert_eq!(codes, [0, 1], "{} {}", a.stderr, b.stderr);
        let winner = if a.code == 0 { &a } else { &b };
        assert!(
            lab.read("go/errors.md")
                .contains(&format!("id: {}\n", field(&winner.stdout, "id")))
        );
        let leftovers: Vec<_> = std::fs::read_dir(lab.root.join("library/go"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(
            leftovers.iter().all(|n| !n.ends_with(".tmp")),
            "{leftovers:?}"
        );
    }
}

#[test]
fn a_same_text_replace_adds_no_landed_line() {
    let lab = Lab::new("land-landed-once");
    land_with_prose(&lab, CAPTURE, "2026-08-01");
    let landed = lab
        .root
        .join(".bilbo/captures")
        .join(CAPTURE_SHA)
        .join("landed");
    let before = std::fs::read_to_string(&landed).unwrap();
    let stage = lab.stage(CAPTURE, ORIGIN, &["--fetched", "2026-08-23"]);
    let run = lab.land(&stage, "go/effective-go", &["--keep", "3-3", "--replace"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(std::fs::read_to_string(&landed).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn a_failed_source_write_records_no_landing() {
    use std::os::unix::fs::PermissionsExt;
    let lab = Lab::new("land-landed-failed");
    lab.put("go/guide.md", &guide("Go", "Lead.", &[]));
    let corpus = lab.root.join("library/go");
    std::fs::set_permissions(&corpus, std::fs::Permissions::from_mode(0o555)).unwrap();
    let probe = corpus.join(".probe");
    if std::fs::write(&probe, "").is_ok() {
        let _ = std::fs::remove_file(&probe);
        eprintln!("skipped: the folder is writable despite 0o555 (running as root?)");
        return;
    }
    let stage = lab.stage(CAPTURE, ORIGIN, &[]);
    let run = lab.land(&stage, "go/errors", &["--keep", "3-3"]);
    std::fs::set_permissions(&corpus, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(run.code, 1);
    assert!(!corpus.join("errors.md").exists());
    let folder = lab.root.join(".bilbo/captures").join(CAPTURE_SHA);
    assert!(!folder.join("landed").exists());
}

#[test]
fn a_leftover_temp_file_neither_stops_a_land_nor_is_touched() {
    let lab = Lab::new("land-leftover-tmp");
    lab.put("go/.land-01M3EZ8NVEC2KJQNGK5DTK349R.tmp", "killed run");
    lab.put(
        "go/.land-guide-01M3EZ8NVEC2KJQNGK5DTK349R.tmp",
        "killed run",
    );
    lab.land_text(CAPTURE, "go/errors", "3-3", &[]);
    for name in [
        "go/.land-01M3EZ8NVEC2KJQNGK5DTK349R.tmp",
        "go/.land-guide-01M3EZ8NVEC2KJQNGK5DTK349R.tmp",
    ] {
        assert_eq!(lab.read(name), "killed run");
    }
}

#[test]
fn show_prints_the_kept_and_capture_headers_of_a_landed_source() {
    let lab = Lab::new("land-show-kept");
    lab.land_text(CAPTURE, "go/errors", "3-3", &[]);
    let run = lab.library(&["show", "go/errors"]);
    assert_eq!(field(&run.stdout, "kept"), "3-3");
    assert_eq!(field(&run.stdout, "capture"), "external");
}

// Plan and read

const ID_C: &str = "01M3EZ8NVEC2KJQNGK5DTK3401";

/// Frontmatter takes 6 lines, so a body's title sits on line 7.
fn plain_lab(name: &str) -> Lab {
    let lab = Lab::new(name);
    lab.put("go/guide.md", &guide("Go", "Lead.", &[]));
    lab.put("go/errors.md", &source(ID_B, &[], ERRORS_BODY));
    lab
}

/// A body of `count` lines of `width` letters under its title.
fn filler(title: &str, count: usize, width: usize) -> String {
    let mut body = format!("# {title}\n");
    for _ in 0..count {
        body.push_str(&"a".repeat(width));
        body.push('\n');
    }
    body
}

/// Nine sources, `go/s1` to `go/s9`, of 19,000 bytes each.
fn nine_lab(name: &str) -> Lab {
    let lab = Lab::new(name);
    lab.put("go/guide.md", &guide("Go", "Lead.", &[]));
    for i in 1..=9 {
        lab.put(
            &format!("go/s{i}.md"),
            &source(
                &format!("01M3EZ8NVEC2KJQNGK5DTK35{i}A"),
                &[],
                &sized(19_000),
            ),
        );
    }
    lab
}

fn nine_refs() -> Vec<String> {
    (1..=9).map(|i| format!("go/s{i}")).collect()
}

impl Lab {
    fn plan(&self, args: &[&str]) -> Run {
        let mut full = vec!["plan"];
        full.extend(args);
        self.library(&full)
    }

    fn read_slices(&self, plan: &str, args: &[&str]) -> Run {
        let mut full = vec!["read", plan];
        full.extend(args);
        self.library(&full)
    }

    fn plans(&self) -> PathBuf {
        self.state.join("bilbo/plans")
    }

    fn plan_files(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.plans()) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// The plan's read log, one entry per line, without the time.
    fn log(&self, plan: &str) -> Vec<String> {
        let Ok(text) = std::fs::read_to_string(self.plans().join(format!("{plan}.log"))) else {
            return Vec::new();
        };
        text.lines()
            .map(|l| l.rsplit_once('\t').unwrap().0.to_string())
            .collect()
    }

    fn cite(&self, args: &[&str], draft: &str) -> Run {
        let mut full = vec!["cite"];
        full.extend(args);
        bilbo_input(self.dir.path(), &self.env(), &full, draft)
    }

    /// The `coverage:` line `bilbo cite --plan` prints for the plan, from an empty draft.
    fn coverage(&self, plan: &str) -> String {
        let run = self.cite(&["--plan", plan], "");
        assert_eq!(run.code, 0, "{}", run.stderr);
        run.stdout
            .lines()
            .find(|l| l.starts_with("coverage: "))
            .unwrap_or_else(|| panic!("no coverage line in {:?}", run.stdout))
            .to_string()
    }

    fn spawn(&self, args: &[&str]) -> Child {
        Command::new(env!("CARGO_BIN_EXE_bilbo"))
            .env_clear()
            .envs(self.env())
            .current_dir(self.dir.path())
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }
}

fn plan_id(run: &Run) -> String {
    assert_eq!(run.code, 0, "{}", run.stderr);
    field(&run.stdout, "plan").to_string()
}

/// The slice rows of a plan's output, each split at its tabs.
fn rows(stdout: &str) -> Vec<Vec<&str>> {
    stdout
        .split_once("\n\n")
        .map_or("", |(_, rows)| rows)
        .lines()
        .map(|row| row.split('\t').collect())
        .collect()
}

/// The numbered lines of a read's stdout as `(line, text)`.
fn numbered_lines(stdout: &str) -> Vec<(usize, &str)> {
    stdout
        .lines()
        .filter(|l| !l.starts_with("-- "))
        .map(|l| {
            let (n, text) = l.split_once('\t').unwrap();
            (n.parse().unwrap(), text)
        })
        .collect()
}

#[test]
fn plan_picks_a_whole_source_and_a_section() {
    let lab = plain_lab("lib-plan-picks");
    lab.put(
        "go/effective-go.md",
        &source(ID_A, &[], &filler("Effective Go", 3, 10)),
    );
    let run = lab.plan(&["go/effective-go", "go/errors#Wrapping"]);
    let id = plan_id(&run);
    assert_eq!(field(&run.stdout, "picks"), "2");
    assert_eq!(
        rows(&run.stdout),
        [
            vec!["1", "1", "go/effective-go", "7-10", "20 tokens", "-"],
            vec!["2", "1", "go/errors", "9-20", "31 tokens", "Wrapping"],
        ]
    );
    assert_eq!(lab.plan_files(), [format!("{id}.json")]);
}

#[test]
fn plan_picks_by_id_as_by_name() {
    let lab = plain_lab("lib-plan-id");
    let by_name = lab.plan(&["go/errors"]);
    let by_id = lab.plan(&[ID_B]);
    assert_eq!(rows(&by_name.stdout), rows(&by_id.stdout));
    let anchored = lab.plan(&[&format!("{ID_B}#Wrapping > Is and As")]);
    assert_eq!(rows(&anchored.stdout)[0][3], "14-20");
}

#[test]
fn a_notes_id_is_not_a_pick() {
    let lab = plain_lab("lib-plan-note-id");
    std::fs::create_dir_all(lab.root.join("notes")).unwrap();
    let note = lab.root.join("notes/decision-note-store.md");
    std::fs::write(&note, note_text(ID_C, "Note store")).unwrap();
    let run = lab.plan(&[ID_C]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.contains(&note.display().to_string()),
        "{}",
        run.stderr
    );
    assert!(!lab.plans().exists());
}

fn catalog_lab(name: &str) -> Lab {
    let lab = Lab::new(name);
    let mut body = String::from("# Lints\n");
    for i in 0..41 {
        body.push_str(&format!("\n## lint{i}\n{}\n", "a".repeat(1400)));
    }
    lab.put(
        "rust/guide.md",
        &guide("Rust", "Lead.", &[("clippy-lints", "x")]),
    );
    lab.put("rust/clippy-lints.md", &source(ID_A, &[], &body));
    lab
}

#[test]
fn a_catalog_is_refused_whole_and_accepted_by_anchor() {
    let lab = catalog_lab("lib-plan-catalog");
    let run = lab.plan(&["rust/clippy-lints"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.contains("rust/clippy-lints is a catalog"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("bilbo library show"));
    assert!(!lab.plans().exists());

    let run = lab.plan(&["rust/clippy-lints#lint3"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(rows(&run.stdout)[0][3], "18-20");
    assert_eq!(rows(&run.stdout)[0][5], "lint3");
}

#[test]
fn overlapping_picks_are_a_usage_error() {
    let lab = plain_lab("lib-plan-overlap");
    for refs in [
        ["go/errors", "go/errors#Wrapping"],
        [ID_B, "go/errors#Wrapping"],
        ["go/errors#Wrapping", "go/errors#Wrapping > Is and As"],
    ] {
        let run = lab.plan(&refs);
        assert_eq!(run.code, 2, "{refs:?}");
        assert!(run.stdout.is_empty());
        assert!(
            run.stderr.contains(refs[0]) && run.stderr.contains(refs[1]),
            "{}",
            run.stderr
        );
    }
    assert!(!lab.plans().exists());
    let run = lab.plan(&["go/errors#Wrapping > Is and As", "go/errors#Wrapping"]);
    assert_eq!(run.code, 2);
    let run = lab.plan(&["go/errors#Wrapping", "go/errors"]);
    assert_eq!(run.code, 2);
}

#[test]
fn two_sections_of_one_source_do_not_overlap() {
    let lab = Lab::new("lib-plan-siblings");
    lab.put(
        "go/lints.md",
        &source(ID_A, &[], "# L\n\n## One\naaaa\n## Two\nbbbb\n"),
    );
    let run = lab.plan(&["go/lints#One", "go/lints#Two"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(rows(&run.stdout).len(), 2);
}

#[test]
fn an_ambiguous_anchor_is_refused_with_its_matches() {
    let lab = Lab::new("lib-plan-ambiguous");
    lab.put(
        "rust/lints.md",
        &source(
            ID_A,
            &[],
            "# Lints\n\n## needless_return\n### What it does\ntext\n## needless_range_loop\n### What it does\ntext\n",
        ),
    );
    let run = lab.plan(&["rust/lints#What it does"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr
            .contains("bilbo: needless_return > What it does (line 10)\n")
    );
    assert!(
        run.stderr
            .contains("bilbo: needless_range_loop > What it does (line 13)\n")
    );
    assert!(!lab.plans().exists());
}

#[test]
fn plan_reference_errors_match_show() {
    let lab = plain_lab("lib-plan-refs");
    for (args, code) in [
        (vec!["go/missing"], 1),
        (vec!["go/errors#nothing"], 1),
        (vec!["go/errors", "go/missing"], 1),
        (vec!["errors"], 2),
        (vec!["go/errors#"], 2),
        (vec!["go/errors", "--depth", "1"], 2),
        (vec!["go/errors", "--part", "1/2"], 2),
        (vec!["go/errors", "--json"], 2),
    ] {
        let run = lab.plan(&args);
        assert_eq!(run.code, code, "{args:?}: {}", run.stderr);
        assert!(run.stdout.is_empty());
    }
    assert!(!lab.plans().exists());
}

#[test]
fn plan_option_limits_are_usage_errors_that_write_nothing() {
    let lab = plain_lab("lib-plan-limits");
    for (args, named) in [
        (vec!["--slice-bytes", "500"], "--slice-bytes"),
        (vec!["--slice-bytes", "999"], "--slice-bytes"),
        (vec!["--slice-bytes", "30001"], "--slice-bytes"),
        (vec!["--slice-bytes", "5000000"], "--slice-bytes"),
        (vec!["--slice-bytes", "many"], "--slice-bytes"),
        (vec!["--slice-lines", "9"], "--slice-lines"),
        (vec!["--budget-tokens", "999"], "--budget-tokens"),
        (vec!["--budget-tokens", "-5"], "--budget-tokens"),
    ] {
        let mut full = vec!["go/errors"];
        full.extend(&args);
        let run = lab.plan(&full);
        assert_eq!(run.code, 2, "{args:?}");
        assert!(run.stdout.is_empty());
        assert!(run.stderr.contains(named), "{}", run.stderr);
    }
    let run = lab.plan(&["go/errors", "--slice-bytes", "5000000"]);
    assert!(run.stderr.contains("30,000"), "{}", run.stderr);
    assert!(!lab.plans().exists());
    for ok in [
        ["--slice-bytes", "1000"],
        ["--slice-bytes", "30000"],
        ["--slice-lines", "10"],
        ["--budget-tokens", "1000"],
    ] {
        assert_eq!(lab.plan(&["go/errors", ok[0], ok[1]]).code, 0, "{ok:?}");
    }
    assert_eq!(lab.plan(&["go/errors", "--slice-bytes=30000"]).code, 0);
    assert_eq!(lab.plan(&["--budget-tokens", "1000", "go/errors"]).code, 0);
}

#[test]
fn a_one_slice_plan_prints_its_header_and_row() {
    let lab = Lab::new("lib-plan-one");
    let mut body = String::from("# T\n");
    for _ in 0..12 {
        body.push_str(&format!("{}\n", "a".repeat(76)));
    }
    body.push_str(&format!("{}\n", "a".repeat(71)));
    assert_eq!(body.len(), 1000);
    lab.put("go/errors.md", &source(ID_A, &[], &body));
    let run = lab.plan(&["go/errors"]);
    let id = plan_id(&run);
    assert_eq!(
        run.stdout,
        format!(
            "plan: {id}\npicks: 1\nslices: 1\ntokens: 400\npartitions: 1\npartition 1: slices 1-1, 400 tokens\n\n1\t1\tgo/errors\t7-20\t400 tokens\t-\n"
        )
    );
}

#[test]
fn plans_cut_sections_into_slices_and_partitions() {
    let lab = Lab::new("lib-plan-cuts");
    let mut body = String::from("# T\n");
    for name in ["A", "B", "C"] {
        body.push_str(&format!("## {name}\n"));
        body.push_str(&("a".repeat(99) + "\n").repeat(99));
    }
    lab.put("go/book.md", &source(ID_A, &[], &body));
    let run = lab.plan(&["go/book"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let table = rows(&run.stdout);
    assert_eq!(table.len(), 2);
    assert_eq!((table[0][3], table[1][3]), ("7-207", "208-307"));
    assert_eq!(table[1][5], "C");
    assert_eq!(field(&run.stdout, "partitions"), "1");

    let run = lab.plan(&["go/book", "--budget-tokens", "1000"]);
    assert_eq!(field(&run.stdout, "partitions"), "2");
    assert!(run.stdout.contains("partition 2: slices 2-2, "));
}

#[test]
fn a_line_cap_bounds_every_slice_of_a_plan() {
    let lab = Lab::new("lib-plan-lines");
    lab.put("go/long.md", &source(ID_A, &[], &filler("L", 600, 10)));
    let run = lab.plan(&["go/long", "--slice-lines", "250"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let table = rows(&run.stdout);
    assert!(table.len() >= 3);
    for row in table {
        let (a, b) = row[3].split_once('-').unwrap();
        assert!(
            b.parse::<usize>().unwrap() - a.parse::<usize>().unwrap() < 250,
            "{row:?}"
        );
    }
}

#[test]
fn a_slice_that_starts_inside_a_section_names_its_path() {
    let lab = Lab::new("lib-plan-in");
    let body = format!(
        "# T\n## Concurrency\n### Goroutines\n{}",
        ("a".repeat(99) + "\n").repeat(299)
    );
    lab.put("go/book.md", &source(ID_A, &[], &body));
    let run = lab.plan(&["go/book", "--slice-bytes", "10000"]);
    let table = rows(&run.stdout);
    assert!(table.len() >= 3, "{}", run.stdout);
    assert_eq!(table[0][5], "-");
    assert_eq!(table[1][5], "Concurrency > Goroutines");
    let plan = plan_id(&run);
    let read = lab.read_slices(&plan, &["2"]);
    assert!(
        read.stdout.lines().nth(1) == Some("-- in: Concurrency > Goroutines --"),
        "{}",
        read.stdout
    );
}

#[test]
fn nine_sources_make_two_partitions() {
    let lab = nine_lab("lib-plan-nine");
    let refs = nine_refs();
    let run = lab.plan(&refs.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(field(&run.stdout, "slices"), "9");
    assert_eq!(field(&run.stdout, "tokens"), "68400");
    assert!(run.stdout.contains("\npartitions: 2\npartition 1: slices 1-7, 53200 tokens\npartition 2: slices 8-9, 15200 tokens\n"), "{}", run.stdout);
    let table = rows(&run.stdout);
    assert_eq!(table[6][1], "1");
    assert_eq!(table[7][1], "2");
}

#[test]
fn planning_and_reading_leave_the_store_alone() {
    let lab = plain_lab("lib-plan-readonly");
    let before = snapshot(&lab.root);
    let id = plan_id(&lab.plan(&["go/errors"]));
    assert_eq!(lab.read_slices(&id, &["1"]).code, 0);
    assert_eq!(lab.read_slices(&id, &["1", "--part", "1/2"]).code, 0);
    assert_eq!(snapshot(&lab.root), before);
    assert_eq!(
        lab.plan_files(),
        [format!("{id}.json"), format!("{id}.log")]
    );
}

#[test]
fn plan_needs_a_state_folder() {
    let lab = plain_lab("lib-plan-state");
    let env = vec![
        ("BILBO_HOME", lab.root.to_str().unwrap()),
        ("HOME", "relative/home"),
    ];
    for args in [
        vec!["library", "plan", "go/errors"],
        vec!["library", "read", ID_C, "1"],
    ] {
        let run = bilbo(lab.dir.path(), &env, &args);
        assert_eq!(run.code, 2, "{args:?}");
        assert!(run.stdout.is_empty());
        assert!(run.stderr.contains("XDG_STATE_HOME") && run.stderr.contains("HOME"));
    }
    assert!(!lab.plans().exists());
}

fn set_age(path: &std::path::Path, days: u64) {
    let when = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 24 * 60 * 60);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn an_old_plan_is_removed_and_a_recent_one_kept() {
    let lab = plain_lab("lib-plan-prune");
    let old = plan_id(&lab.plan(&["go/errors"]));
    assert_eq!(lab.read_slices(&old, &["1"]).code, 0);
    let recent = plan_id(&lab.plan(&["go/errors"]));
    set_age(&lab.plans().join(format!("{old}.json")), 31);
    set_age(&lab.plans().join(format!("{old}.log")), 31);
    set_age(&lab.plans().join(format!("{recent}.json")), 1);
    let fresh = plan_id(&lab.plan(&["go/errors"]));
    let mut expected = vec![format!("{recent}.json"), format!("{fresh}.json")];
    expected.sort();
    assert_eq!(lab.plan_files(), expected);
}

#[test]
fn a_refused_plan_prunes_nothing() {
    let lab = plain_lab("lib-plan-prune-refused");
    let old = plan_id(&lab.plan(&["go/errors"]));
    set_age(&lab.plans().join(format!("{old}.json")), 40);
    assert_eq!(lab.plan(&["go/missing"]).code, 1);
    assert_eq!(lab.plan_files(), [format!("{old}.json")]);
}

#[test]
fn read_prints_a_slice_with_its_header_numbers_and_end_marker() {
    let lab = plain_lab("lib-read-one");
    let id = plan_id(&lab.plan(&["go/errors"]));
    let run = lab.read_slices(&id, &["1"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let mut expected = format!("-- slice 1/1: go/errors {ID_B} lines 7-20 --\n-- in: - --\n");
    for (i, line) in ERRORS_BODY.lines().enumerate() {
        expected.push_str(&format!("{}\t{line}\n", i + 7));
    }
    expected.push_str("-- end slice 1/1 --\n");
    assert_eq!(run.stdout, expected);
    assert!(run.stderr.is_empty());
    assert!(!run.stdout.contains("id: "));
    assert_eq!(numbered_lines(&run.stdout)[0], (7, "# Errors"));
    assert_eq!(lab.log(&id), [format!("1\t{ID_B}\t{ZERO}\t7-20")]);
}

#[test]
fn read_prints_lines_as_they_are() {
    let lab = Lab::new("lib-read-bytes");
    let body = "# T\n\n## Code\n\tindented\ttab  \n```go\n# not a heading\n  x := 1  \n```\n\n";
    lab.put("go/code.md", &source(ID_A, &[], body));
    let id = plan_id(&lab.plan(&["go/code"]));
    let run = lab.read_slices(&id, &["1"]);
    let got: Vec<&str> = numbered_lines(&run.stdout)
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    let want: Vec<&str> = body.lines().collect();
    assert_eq!(got, want);
}

#[test]
fn read_prints_two_small_slices_in_one_call() {
    let lab = plain_lab("lib-read-two");
    lab.put("go/small.md", &source(ID_A, &[], &filler("Small", 3, 20)));
    let id = plan_id(&lab.plan(&["go/errors", "go/small"]));
    let run = lab.read_slices(&id, &["2", "1"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let headers: Vec<&str> = run
        .stdout
        .lines()
        .filter(|l| l.starts_with("-- "))
        .collect();
    assert_eq!(
        headers,
        [
            format!("-- slice 2/2: go/small {ID_A} lines 7-10 --").as_str(),
            "-- in: - --",
            "-- end slice 2/2 --",
            format!("-- slice 1/2: go/errors {ID_B} lines 7-20 --").as_str(),
            "-- in: - --",
            "-- end slice 1/2 --",
        ]
    );
    assert_eq!(lab.log(&id).len(), 2);
}

#[test]
fn one_read_of_too_much_is_refused_and_logged_nowhere() {
    let lab = nine_lab("lib-read-toomuch");
    let refs = nine_refs();
    let id = plan_id(&lab.plan(&refs.iter().map(String::as_str).collect::<Vec<_>>()));
    let run = lab.read_slices(&id, &["1", "2", "3", "4", "5", "6", "7", "8", "9"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.contains("slices 1, 2, 3, 4, 5, 6, 7, 8, 9"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("24,000 bytes"), "{}", run.stderr);
    assert!(!lab.plans().join(format!("{id}.log")).exists());
    assert!(lab.coverage(&id).contains("read 0 of 9 slices"));
    assert_eq!(lab.read_slices(&id, &["1", "2"]).code, 2);
    assert_eq!(lab.read_slices(&id, &["9"]).code, 0);
    assert!(lab.coverage(&id).contains("read 1 of 9 slices"));
}

#[test]
fn the_limit_of_one_read_is_the_plans_slice_size() {
    let lab = nine_lab("lib-read-limit");
    let id = plan_id(&lab.plan(&["go/s1", "go/s2", "--slice-bytes", "30000"]));
    assert_eq!(lab.read_slices(&id, &["1", "2"]).code, 2);
    assert_eq!(lab.read_slices(&id, &["1", "2", "--part", "1/2"]).code, 0);
    assert_eq!(lab.read_slices(&id, &["1", "2", "--part", "2/2"]).code, 0);
    assert_eq!(lab.read_slices(&id, &["1", "2", "--part", "1/8"]).code, 0);
}

#[test]
fn parts_of_several_slices_still_count_against_the_limit() {
    let lab = nine_lab("lib-read-parts-limit");
    let refs = nine_refs();
    let id = plan_id(&lab.plan(&refs.iter().map(String::as_str).collect::<Vec<_>>()));
    let run = lab.read_slices(&id, &["1", "2", "3", "4", "--part", "1/2"]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("slices 1, 2, 3, 4"), "{}", run.stderr);
    assert!(run.stderr.contains("24,000 bytes"), "{}", run.stderr);
    assert!(lab.coverage(&id).contains("read 0 of 9 slices"));
}

#[test]
fn a_plan_without_body_start_is_refused() {
    let lab = book_lab("lib-read-nobody");
    let id = plan_id(&lab.plan(&["go/book"]));
    let path = lab.plans().join(format!("{id}.json"));
    let mut plan: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    for pick in plan["picks"].as_array_mut().unwrap() {
        pick.as_object_mut().unwrap().remove("body_start").unwrap();
    }
    std::fs::write(&path, plan.to_string()).unwrap();
    let run = lab.read_slices(&id, &["1"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("body_start"), "{}", run.stderr);
}

#[test]
fn one_oversized_slice_still_prints() {
    let lab = Lab::new("lib-read-huge");
    let body = format!("# T\n{}\n", "a".repeat(30_000));
    lab.put("go/huge.md", &source(ID_A, &[], &body));
    let run = lab.plan(&["go/huge"]);
    let id = plan_id(&run);
    assert_eq!(rows(&run.stdout).len(), 2);
    let read = lab.read_slices(&id, &["2"]);
    assert_eq!(read.code, 0, "{}", read.stderr);
    assert!(read.stdout.len() > 30_000);
    assert!(read.stdout.ends_with("-- end slice 2/2 --\n"));
}

#[test]
fn parts_print_every_line_once() {
    let lab = Lab::new("lib-read-parts");
    lab.put("go/book.md", &source(ID_A, &[], &filler("B", 40, 30)));
    let id = plan_id(&lab.plan(&["go/book"]));
    let whole = lab.read_slices(&id, &["1"]);
    let one = lab.read_slices(&id, &["1", "--part", "1/2"]);
    let two = lab.read_slices(&id, &["1", "--part=2/2"]);
    assert_eq!((one.code, two.code), (0, 0), "{}{}", one.stderr, two.stderr);
    assert!(
        one.stdout
            .starts_with(&format!("-- slice 1/1 part 1/2: go/book {ID_A} lines 7-"))
    );
    assert!(one.stdout.ends_with("-- end slice 1/1 part 1/2 --\n"));
    assert!(two.stdout.ends_with("-- end slice 1/1 part 2/2 --\n"));
    let mut together = numbered_lines(&one.stdout);
    together.extend(numbered_lines(&two.stdout));
    assert_eq!(together, numbered_lines(&whole.stdout));
    let (a, b) = (numbered_lines(&one.stdout), numbered_lines(&two.stdout));
    assert!(!a.is_empty() && !b.is_empty());
    assert_eq!(lab.log(&id).len(), 3);
    assert!(lab.log(&id)[1].ends_with(&format!("{}-{}", a[0].0, a.last().unwrap().0)));
}

#[test]
fn a_bad_part_is_a_usage_error_that_logs_nothing() {
    let lab = plain_lab("lib-read-badpart");
    lab.put("go/tiny.md", &source(ID_A, &[], "# T\nx\nyy\n"));
    let id = plan_id(&lab.plan(&["go/errors", "go/tiny"]));
    for part in ["3/2", "0/2", "1/1", "1/9", "2", "a/b", "1/2/3", "-1/2"] {
        let run = lab.read_slices(&id, &["1", "--part", part]);
        assert_eq!(run.code, 2, "{part}");
        assert!(run.stdout.is_empty());
        assert!(run.stderr.contains("--part"), "{}", run.stderr);
    }
    let run = lab.read_slices(&id, &["2", "--part", "1/4"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("--part"));
    assert!(lab.coverage(&id).contains("read 0 of 2 slices"));
    assert_eq!(lab.read_slices(&id, &["2", "--part", "3/3"]).code, 0);
}

#[test]
fn six_readers_at_once_log_all_six() {
    let lab = Lab::new("lib-read-six");
    lab.put("go/guide.md", &guide("Go", "Lead.", &[]));
    for i in 1..=6 {
        lab.put(
            &format!("go/s{i}.md"),
            &source(
                &format!("01M3EZ8NVEC2KJQNGK5DTK35{i}A"),
                &[],
                &sentence_body(i),
            ),
        );
    }
    let refs: Vec<String> = (1..=6).map(|i| format!("go/s{i}")).collect();
    let id = plan_id(&lab.plan(&refs.iter().map(String::as_str).collect::<Vec<_>>()));
    let children: Vec<Child> = (1..=6)
        .map(|i| lab.spawn(&["library", "read", &id, &i.to_string()]))
        .collect();
    for child in children {
        let run = finish(child);
        assert_eq!(run.code, 0, "{}", run.stderr);
    }
    let draft: String = (1..=6)
        .map(|i| {
            format!(
                "bilbo:01M3EZ8NVEC2KJQNGK5DTK35{i}A \"{}\"\n",
                sentence(i, 100)
            )
        })
        .collect();
    let run = lab.cite(&["--plan", &id], &draft);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(run.stdout.matches("\tok\t").count(), 6, "{}", run.stdout);
    assert!(run.stdout.contains("citations: 6 checked, 6 ok"));
    assert!(run.stdout.contains("read 6 of 6 slices"), "{}", run.stdout);
    assert!(run.stdout.contains("not read: none"));
}

/// The sentence on line `n` of source `i`.
fn sentence(i: usize, n: usize) -> String {
    format!("Reader {i} holds the line {n} with a quick brown fox")
}

/// A title and 200 sentences of about 90 bytes.
fn sentence_body(i: usize) -> String {
    let mut body = String::from("# S\n");
    for n in 1..=200 {
        body.push_str(&format!("{} and some padding.\n", sentence(i, n)));
    }
    body
}

fn book_lab(name: &str) -> Lab {
    let lab = Lab::new(name);
    lab.put("go/guide.md", &guide("Go", "Lead.", &[]));
    let mut body = String::from("# Book\n");
    for n in 1..=60 {
        body.push_str(&format!("Line {n} says the quick brown fox jumps over.\n"));
    }
    lab.put("go/book.md", &source(ID_A, &[], &body));
    lab
}

fn quote_of(n: usize) -> String {
    format!("bilbo:{ID_A} \"Line {n} says the quick brown fox jumps over\"\n")
}

#[test]
fn a_read_is_logged_and_counts_for_cite() {
    let lab = book_lab("lib-read-logged");
    let planned = lab.plan(&["go/book", "--slice-lines", "20"]);
    let id = plan_id(&planned);
    let slices = rows(&planned.stdout).len();
    assert!(slices >= 3, "{}", planned.stdout);
    let read = lab.read_slices(&id, &["3"]);
    assert_eq!(read.code, 0, "{}", read.stderr);
    let (last, text) = *numbered_lines(&read.stdout).last().unwrap();
    assert!(last > 7, "{last}");
    let n: usize = text.split(' ').nth(1).unwrap().parse().unwrap();
    let run = lab.cite(&["--plan", &id], &quote_of(n));
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("\tok\t"), "{}", run.stdout);
    assert!(
        run.stdout.contains(&format!("read 1 of {slices} slices")),
        "{}",
        run.stdout
    );
    let run = lab.cite(&["--plan", &id], &quote_of(1));
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stdout.contains("\tunread\t"), "{}", run.stdout);
    assert!(run.stdout.contains("(slice 1)"), "{}", run.stdout);
}

#[test]
fn parts_add_up_to_lines_not_to_a_slice() {
    let lab = book_lab("lib-read-partial");
    let id = plan_id(&lab.plan(&["go/book"]));
    let read = lab.read_slices(&id, &["1", "--part", "1/2"]);
    assert_eq!(read.code, 0, "{}", read.stderr);
    assert!(lab.coverage(&id).contains("read 0 of 1 slices"));
    let run = lab.cite(&["--plan", &id], &quote_of(1));
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("\tok\t"), "{}", run.stdout);
    let run = lab.cite(&["--plan", &id], &quote_of(60));
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stdout.contains("\tunread\t"), "{}", run.stdout);
    let rest = lab.read_slices(&id, &["1", "--part", "2/2"]);
    assert_eq!(rest.code, 0, "{}", rest.stderr);
    assert!(lab.coverage(&id).contains("read 1 of 1 slices"));
    assert_eq!(lab.cite(&["--plan", &id], &quote_of(60)).code, 0);
}

#[test]
fn a_frontmatter_of_another_length_is_a_changed_source() {
    let lab = book_lab("lib-read-frontlen");
    let id = plan_id(&lab.plan(&["go/book"]));
    let text = lab.read("go/book.md");
    let moved = text.replacen("digest: ", "kept: 1-61\ndigest: ", 1);
    lab.put("go/book.md", &moved);
    let run = lab.read_slices(&id, &["1"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("changed since plan"), "{}", run.stderr);
    assert!(lab.coverage(&id).contains("read 0 of 1 slices"));
    let run = lab.cite(&["--plan", &id], &quote_of(3));
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stdout.contains("\tunread\t"), "{}", run.stdout);
    assert!(
        run.stdout.contains("changed since its plan"),
        "{}",
        run.stdout
    );
}

#[test]
fn a_root_with_a_trailing_slash_is_the_same_root() {
    let lab = book_lab("lib-read-slash");
    let id = plan_id(&lab.plan(&["go/book"]));
    let slashed = format!("{}/", lab.root.display());
    let mut env = lab.env();
    env.retain(|(name, _)| *name != "BILBO_HOME");
    env.push(("BILBO_HOME", slashed.as_str()));
    let run = bilbo(lab.dir.path(), &env, &["library", "read", &id, "1"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let run = bilbo_input(lab.dir.path(), &env, &["cite", "--plan", &id], &quote_of(3));
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
}

#[test]
fn a_reingested_source_is_refused_and_nothing_is_logged() {
    let lab = Lab::new("lib-read-reingest");
    lab.land_text(CAPTURE, "go/errors", "1-3", &[]);
    let id = plan_id(&lab.plan(&["go/errors"]));
    lab.land_text(
        "# Errors\n\nWrap them more.\n",
        "go/errors",
        "1-3",
        &["--replace"],
    );
    let run = lab.read_slices(&id, &["1"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("go/errors"), "{}", run.stderr);
    assert!(run.stderr.contains("new plan"), "{}", run.stderr);
    assert!(lab.coverage(&id).contains("read 0 of 1 slices"));
}

#[test]
fn a_deleted_source_is_refused_too() {
    let lab = plain_lab("lib-read-deleted");
    let id = plan_id(&lab.plan(&["go/errors"]));
    std::fs::remove_file(lab.root.join("library/go/errors.md")).unwrap();
    let run = lab.read_slices(&id, &["1"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("go/errors") && run.stderr.contains("new plan"));
    assert!(lab.log(&id).is_empty());
}

#[test]
fn a_moved_source_is_still_read_under_its_new_name() {
    let lab = plain_lab("lib-read-moved");
    let id = plan_id(&lab.plan(&["go/errors"]));
    std::fs::rename(
        lab.root.join("library/go/errors.md"),
        lab.root.join("library/go/errors-2009.md"),
    )
    .unwrap();
    let run = lab.read_slices(&id, &["1"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.starts_with(&format!(
        "-- slice 1/1: go/errors-2009 {ID_B} lines 7-20 --\n"
    )));
    assert_eq!(lab.log(&id).len(), 1);
}

#[test]
fn read_usage_errors() {
    let lab = nine_lab("lib-read-usage");
    let refs = nine_refs();
    let id = plan_id(&lab.plan(&refs.iter().map(String::as_str).collect::<Vec<_>>()));
    for args in [
        vec!["read", &id, "10"],
        vec!["read", &id, "0"],
        vec!["read", &id, "x"],
        vec!["read", &id, "-1"],
        vec!["read", &id],
        vec!["read", "not-a-plan", "1"],
        vec!["read", &id, "1", "--depth", "1"],
        vec!["read", &id, "1", "--part"],
    ] {
        let run = lab.library(&args);
        assert_eq!(run.code, 2, "{args:?}: {}", run.stderr);
        assert!(run.stdout.is_empty());
    }
    let run = lab.read_slices(&id, &["10"]);
    assert!(
        run.stderr.contains("slice 10") && run.stderr.contains("9 slices"),
        "{}",
        run.stderr
    );
    assert!(lab.log(&id).is_empty());
}

#[test]
fn an_unknown_plan_exits_1() {
    let lab = plain_lab("lib-read-unknown");
    let run = lab.read_slices(ID_C, &["1"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains(ID_C), "{}", run.stderr);
    assert!(!lab.plans().exists());
}

#[test]
fn a_plan_is_read_only_under_the_store_that_made_it() {
    let lab = plain_lab("lib-read-other-store");
    let id = plan_id(&lab.plan(&["go/errors"]));
    let other = lab.dir.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    let mut env = lab.env();
    env.retain(|(name, _)| *name != "BILBO_HOME");
    env.push(("BILBO_HOME", other.to_str().unwrap()));
    let run = bilbo(lab.dir.path(), &env, &["library", "read", &id, "1"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.contains(lab.root.to_str().unwrap()),
        "{}",
        run.stderr
    );
    assert!(lab.coverage(&id).contains("read 0 of 1 slices"));
}

// The citation pre-check

const WRAPPED: &str = "Always wrap errors with context before returning them";
const OLD_ERRORS: &str = "# Errors\n\n## Wrapping\n\nAlways wrap errors with context before returning them.\n\n## Is\n\nUse errors.Is to compare against sentinel values reliably.\n";

/// `go/errors` landed from `OLD_ERRORS`, and its id.
fn cited_lab(name: &str) -> (Lab, String) {
    let lab = Lab::new(name);
    let run = lab.land_text(OLD_ERRORS, "go/errors", "3-9", &[]);
    let id = field(&run.stdout, "id").to_string();
    (lab, id)
}

/// A note whose line 8 is `citation`.
fn cite_in_note(lab: &Lab, id: &str, citation: &str) {
    let text = format!("{}\n{citation}\n", note_text(ID_C, "Errors"));
    let notes = lab.root.join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(notes.join("gotcha-errors.md"), text).unwrap();
    assert!(!id.is_empty());
}

fn wrapping(id: &str) -> String {
    format!("bilbo:{id}#Wrapping \"{WRAPPED}\"")
}

/// Stages `text` and runs `land --replace` over `go/errors`.
fn replace_errors(lab: &Lab, text: &str, extra: &[&str]) -> (Run, String) {
    let stage = lab.stage(text, ORIGIN, &[]);
    let keep = format!("3-{}", text.lines().count());
    let mut args = vec!["--keep", keep.as_str(), "--replace"];
    args.extend(extra);
    (lab.land(&stage, "go/errors", &args), stage)
}

#[test]
fn a_replace_with_no_citing_note_lands() {
    let (lab, _) = cited_lab("pre-none");
    let (run, _) = replace_errors(&lab, "# Errors\n\nWrap them.\n", &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty());
}

#[test]
fn a_citation_that_keeps_resolving_does_not_block() {
    let (lab, id) = cited_lab("pre-keeps");
    cite_in_note(&lab, &id, &wrapping(&id));
    let text = OLD_ERRORS.replace("Use errors.Is", "Call errors.Is");
    let (run, _) = replace_errors(&lab, &text, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
}

#[test]
fn a_citation_that_already_failed_does_not_block() {
    let (lab, id) = cited_lab("pre-failing");
    cite_in_note(
        &lab,
        &id,
        &format!("bilbo:{id}#Wrapping \"this quote was never in the source\""),
    );
    let (run, _) = replace_errors(&lab, "# Errors\n\n## Wrapping\n\nWrap them.\n", &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
}

#[test]
fn a_citation_in_a_guide_is_not_checked() {
    let (lab, id) = cited_lab("pre-guide");
    let guide = format!("{}\n{}\n", lab.read("go/guide.md"), wrapping(&id));
    lab.put("go/guide.md", &guide);
    let (run, _) = replace_errors(&lab, "# Errors\n\nWrap them.\n", &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
}

#[test]
fn a_quote_the_new_text_dropped_blocks_the_replace() {
    let (lab, id) = cited_lab("pre-dropped");
    cite_in_note(&lab, &id, &wrapping(&id));
    let before = snapshot(&lab.root);
    let (run, stage) = replace_errors(
        &lab,
        "# Errors\n\n## Wrapping\n\nWrap them.\n\n## Is\n\nUse errors.Is to compare against sentinel values reliably.\n",
        &[],
    );
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("1 citation"), "{}", run.stderr);
    assert!(run.stderr.contains("--force"), "{}", run.stderr);
    assert!(
        run.stderr
            .contains("bilbo: notes/gotcha-errors.md:8: ok -> quote_missing"),
        "{}",
        run.stderr
    );
    assert_eq!(snapshot(&lab.root), before);
    assert!(lab.staged().contains(&stage));
    assert!(lab.captures().len() == 1, "{:?}", lab.captures());
}

#[test]
fn a_renamed_heading_blocks_the_replace() {
    let (lab, id) = cited_lab("pre-renamed");
    cite_in_note(&lab, &id, &wrapping(&id));
    let before = snapshot(&lab.root);
    let (run, _) = replace_errors(&lab, &OLD_ERRORS.replace("## Wrapping", "## Wrap"), &[]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr
            .contains("notes/gotcha-errors.md:8: ok -> anchor_missing"),
        "{}",
        run.stderr
    );
    assert_eq!(snapshot(&lab.root), before);
}

#[test]
fn force_replaces_and_keeps_the_lines_as_warnings() {
    let (lab, id) = cited_lab("pre-force");
    cite_in_note(&lab, &id, &wrapping(&id));
    let (run, stage) = replace_errors(
        &lab,
        &OLD_ERRORS.replace("## Wrapping", "## Wrap"),
        &["--force"],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stderr
            .contains("bilbo: notes/gotcha-errors.md:8: ok -> anchor_missing"),
        "{}",
        run.stderr
    );
    assert_eq!(run.stdout.lines().count(), 4, "{}", run.stdout);
    assert_eq!(field(&run.stdout, "id"), id);
    assert!(lab.read("go/errors.md").contains("## Wrap\n"));
    assert!(!lab.staged().contains(&stage));
    assert_eq!(lab.captures().len(), 2);
}

#[test]
fn force_without_replace_is_a_usage_error() {
    let lab = Lab::new("pre-force-alone");
    let stage = lab.stage(CAPTURE, ORIGIN, &[]);
    let before = snapshot(&lab.root);
    let run = lab.land(&stage, "go/effective-go", &["--keep", "1-3", "--force"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.contains("--force") && run.stderr.contains("--replace"),
        "{}",
        run.stderr
    );
    assert_eq!(snapshot(&lab.root), before);
    assert!(lab.staged().contains(&stage));
}

// --- stage <url> and --html ---

const PAGES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pages");

fn fixture(name: &str, ext: &str) -> Vec<u8> {
    std::fs::read(format!("{PAGES}/{name}.{ext}")).unwrap()
}

fn html_route(name: &str) -> Route {
    Route::ok("text/html; charset=utf-8", fixture(name, "html"))
}

fn capture_of(run: &Run) -> Vec<u8> {
    std::fs::read(field(&run.stdout, "capture")).unwrap()
}

fn stage_folder(run: &Run) -> PathBuf {
    PathBuf::from(field(&run.stdout, "capture"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn fetch_json(run: &Run) -> serde_json::Value {
    let text = std::fs::read_to_string(stage_folder(run).join("fetch.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// Stages `url` and expects a refusal that names it, leaves no stage folder and changes nothing.
fn refusal(lab: &Lab, url: &str) -> Run {
    let before = snapshot(&lab.root);
    let run = lab.library(&["stage", url]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(run.stderr.contains(url), "{}", run.stderr);
    assert!(lab.staged().is_empty());
    assert_eq!(snapshot(&lab.root), before);
    run
}

fn served(route: Route) -> (Pages, String) {
    let pages = Pages::start(vec![("/p", route)]);
    let url = pages.url("/p");
    (pages, url)
}

#[test]
fn each_fixture_page_stages_to_its_markdown() {
    for name in ["headings", "code", "tables", "main", "escapes"] {
        let lab = Lab::new(&format!("fetch-{name}"));
        let pages = Pages::start(vec![("/p", html_route(name))]);
        let before = snapshot(&lab.root);
        let run = lab.library(&["stage", &pages.url("/p")]);
        assert_eq!(run.code, 0, "{name}: {}", run.stderr);
        assert_eq!(
            String::from_utf8(capture_of(&run)).unwrap(),
            String::from_utf8(fixture(name, "md")).unwrap(),
            "{name}"
        );
        assert_eq!(snapshot(&lab.root), before);
    }
}

#[test]
fn raw_is_the_served_bytes_and_fetch_json_records_a_200() {
    let lab = Lab::new("fetch-200");
    let pages = Pages::start(vec![("/p", html_route("main"))]);
    let url = pages.url("/p");
    let run = lab.library(&["stage", &url]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        std::fs::read(field(&run.stdout, "raw")).unwrap(),
        fixture("main", "html")
    );
    let fetch = fetch_json(&run);
    assert_eq!(fetch["url"], url);
    assert_eq!(fetch["final_url"], url);
    assert_eq!(fetch["status"], 200);
    assert_eq!(fetch["media_type"], "text/html");
    assert_eq!(
        fetch["converter"],
        format!("bilbo {}", env!("CARGO_PKG_VERSION"))
    );
    let at = fetch["fetched_at"].as_str().unwrap();
    assert!(at.ends_with("-03:00"), "{at}");
    assert!(at.parse::<jiff::Timestamp>().is_ok(), "{at}");
    assert_eq!(fetch.as_object().unwrap().len(), 6);
    assert!(!run.stdout.contains("final url:"));
}

#[test]
fn a_redirect_is_recorded_and_the_origin_stays_as_given() {
    let lab = Lab::new("fetch-301");
    let pages = Pages::start(vec![
        ("/old", Route::redirect("/new")),
        ("/new", html_route("main")),
    ]);
    let (old, new) = (pages.url("/old"), pages.url("/new"));
    let run = lab.library(&["stage", &old]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let fetch = fetch_json(&run);
    assert_eq!(fetch["url"], old);
    assert_eq!(fetch["final_url"], new);
    assert_eq!(fetch["status"], 200);
    assert_eq!(fetch["media_type"], "text/html");
    assert_eq!(
        fetch["converter"],
        format!("bilbo {}", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(
        std::fs::read(field(&run.stdout, "raw")).unwrap(),
        fixture("main", "html")
    );
    assert_eq!(field(&run.stdout, "final url"), new);

    let land = lab.land(field(&run.stdout, "stage"), "go/main", &["--keep", "1-3"]);
    assert_eq!(land.code, 0, "{}", land.stderr);
    let source = lab.read("go/main.md");
    assert!(
        source.contains(&format!("origin: \"url: {old}\"")),
        "{source}"
    );
    assert!(
        source.contains(&format!("fetched: {}", today())),
        "{source}"
    );
}

#[test]
fn stage_prints_the_fetch_lines_for_a_page_with_a_main() {
    let lab = Lab::new("fetch-lines");
    let pages = Pages::start(vec![("/p", html_route("main"))]);
    let run = lab.library(&["stage", &pages.url("/p")]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let md = String::from_utf8(fixture("main", "md")).unwrap();
    let lines: Vec<&str> = md.lines().collect();
    let manifest = lines.iter().position(|l| *l == "# The Manifest").unwrap() + 1;
    let out: Vec<&str> = run.stdout.lines().collect();
    assert!(out[0].starts_with("stage: "));
    assert!(out[1].starts_with("capture: "));
    assert!(out[2].starts_with("raw: "));
    assert_eq!(out[3], "media type: text/html");
    assert_eq!(out[4], "content: 13-19");
    assert_eq!(manifest, 13);
    assert_eq!(out[5], format!("lines: {}", lines.len()));
    assert!(out[6].starts_with("tokens: "));
    assert_eq!(out[7], "title: The Manifest");
    assert_eq!(out[8], "keep: 14-19");
    assert!(out[9].is_empty());
}

fn source_with(origin: &str) -> String {
    format!(
        "---\nid: {ID_B}\nfetched: 2026-01-01\norigin: \"{origin}\"\ndigest: {ZERO}\n---\n# T\n\nText.\n"
    )
}

#[test]
fn stage_reports_an_existing_source_with_the_same_origin() {
    let lab = Lab::new("fetch-existing");
    let pages = Pages::start(vec![("/p", html_route("main"))]);
    let url = pages.url("/p");
    let first = lab.library(&["stage", &url]);
    assert!(!first.stdout.contains("existing:"));
    let land = lab.land(field(&first.stdout, "stage"), "go/page", &["--keep", "1-3"]);
    assert_eq!(land.code, 0, "{}", land.stderr);
    let origin = format!("url: {url}");
    lab.put("aa/z.md", &source_with(&origin));
    lab.put("go/b.md", &source_with(&origin));
    lab.put("zz/prefix.md", &source_with(&format!("{origin}/more")));
    let again = lab.library(&["stage", &url]);
    assert_eq!(again.code, 0, "{}", again.stderr);
    let existing: Vec<&str> = again
        .stdout
        .lines()
        .filter(|l| l.starts_with("existing:"))
        .collect();
    assert_eq!(
        existing,
        ["existing: aa/z", "existing: go/b", "existing: go/page"],
        "{}",
        again.stdout
    );
}

#[test]
fn a_landed_url_source_has_no_capture_key_and_keeps_the_record() {
    let lab = Lab::new("fetch-land");
    let pages = Pages::start(vec![("/p", html_route("main"))]);
    let run = lab.library(&["stage", &pages.url("/p")]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let land = lab.land(field(&run.stdout, "stage"), "go/page", &["--keep", "1-3"]);
    assert_eq!(land.code, 0, "{}", land.stderr);
    assert!(!lab.read("go/page.md").contains("capture:"));
    let facts = lab.library(&["go"]);
    assert!(!facts.stdout.contains(" · capture "), "{}", facts.stdout);
    let folder = PathBuf::from(field(&land.stdout, "capture folder"));
    let mut names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["capture.md", "fetch.json", "landed", "raw"]);
    let guide = lab
        .read("go/guide.md")
        .replace("TODO: describe this source.", "The example manifest page.")
        .replace("TODO: describe this corpus.", "Example pages.");
    lab.put("go/guide.md", &guide);
    let check = lab.run(&["check"]);
    assert_eq!(check.code, 0, "{}{}", check.stdout, check.stderr);
}

#[test]
fn text_markdown_keeps_crlf_as_text_and_plain_text_has_no_converter() {
    let lab = Lab::new("fetch-text");
    let pages = Pages::start(vec![
        (
            "/md",
            Route::ok(
                "text/markdown; charset=utf-8",
                "# Errors\r\n\r\nWrap them.\r\n",
            ),
        ),
        ("/txt", Route::ok("text/plain", "# Errors\n\nWrap them.\n")),
    ]);
    let md = lab.library(&["stage", &pages.url("/md")]);
    assert_eq!(md.code, 0, "{}", md.stderr);
    assert_eq!(capture_of(&md), b"# Errors\n\nWrap them.\n");
    assert_eq!(fetch_json(&md)["converter"], serde_json::Value::Null);
    assert_eq!(fetch_json(&md)["media_type"], "text/markdown");
    assert!(!md.stdout.lines().any(|l| l.starts_with("content:")));
    let txt = lab.library(&["stage", &pages.url("/txt")]);
    assert_eq!(txt.code, 0, "{}", txt.stderr);
    assert_eq!(fetch_json(&txt)["converter"], serde_json::Value::Null);
    assert_eq!(
        std::fs::read(field(&txt.stdout, "raw")).unwrap(),
        capture_of(&txt)
    );
}

#[test]
fn not_found_and_server_errors_are_refused_with_the_status() {
    let lab = Lab::new("fetch-status");
    let pages = Pages::start(vec![("/boom", Route::status(500))]);
    let missing = refusal(&lab, &pages.url("/missing"));
    assert!(missing.stderr.contains("404"), "{}", missing.stderr);
    let boom = refusal(&lab, &pages.url("/boom"));
    assert!(boom.stderr.contains("500"), "{}", boom.stderr);
}

#[test]
fn a_pdf_is_refused_with_the_route() {
    let lab = Lab::new("fetch-pdf");
    let (_pages, url) = served(Route::ok("application/pdf", "%PDF-1.7 x"));
    let run = refusal(&lab, &url);
    assert!(run.stderr.contains("PDF"), "{}", run.stderr);
    assert!(
        run.stderr.contains(&format!("--origin \"url: {url}\"")),
        "{}",
        run.stderr
    );
}

#[test]
fn an_image_is_refused_naming_its_media_type() {
    let lab = Lab::new("fetch-png");
    let (_pages, url) = served(Route::ok("image/png", vec![0x89, b'P', b'N', b'G']));
    let run = refusal(&lab, &url);
    assert!(run.stderr.contains("image/png"), "{}", run.stderr);
}

#[test]
fn a_body_that_is_not_utf8_is_refused() {
    let lab = Lab::new("fetch-latin1");
    let (_pages, url) = served(Route::ok(
        "text/html; charset=iso-8859-1",
        b"<p>caf\xE9</p>".to_vec(),
    ));
    let run = refusal(&lab, &url);
    assert!(run.stderr.contains("UTF-8"), "{}", run.stderr);
}

#[test]
fn a_page_of_scripts_only_is_refused() {
    let lab = Lab::new("fetch-empty");
    let (_pages, url) = served(Route::ok(
        "text/html",
        "<html><body><script>var a = 1;</script><div id=\"root\"></div></body></html>",
    ));
    let run = refusal(&lab, &url);
    assert!(run.stderr.contains("no text"), "{}", run.stderr);
}

#[test]
fn a_body_over_16_mib_is_refused() {
    let lab = Lab::new("fetch-big");
    let (_pages, url) = served(Route::ok("text/plain", vec![b'a'; 16 * 1024 * 1024 + 1]));
    let run = refusal(&lab, &url);
    assert!(run.stderr.contains("16 MiB"), "{}", run.stderr);
}

#[test]
fn eleven_redirects_are_refused() {
    let lab = Lab::new("fetch-redirects");
    let paths: Vec<String> = (0..=11).map(|i| format!("/r{i}")).collect();
    let mut routes: Vec<(&str, Route)> = (0..11)
        .map(|i| (paths[i].as_str(), Route::redirect(&paths[i + 1])))
        .collect();
    routes.push((paths[11].as_str(), Route::ok("text/plain", "end\n")));
    let pages = Pages::start(routes);
    let run = refusal(&lab, &pages.url("/r0"));
    assert!(run.stderr.contains("10 redirects"), "{}", run.stderr);
}

#[test]
fn a_dead_port_is_unreachable() {
    let lab = Lab::new("fetch-dead");
    let url = format!("{}/p", dead_url());
    let run = refusal(&lab, &url);
    assert!(run.stderr.contains("unreachable"), "{}", run.stderr);
}

#[test]
fn url_arguments_that_are_usage_errors_send_no_request() {
    let lab = Lab::new("fetch-usage");
    let pages = Pages::start(vec![("/p", html_route("main"))]);
    let url = pages.url("/p");
    let cases: Vec<(Vec<String>, &str)> = vec![
        (
            vec![url.clone(), "--origin".into(), "url: https://go.dev".into()],
            "--origin",
        ),
        (
            vec![url.clone(), "--fetched".into(), "2026-01-02".into()],
            "--fetched",
        ),
        (vec![url.clone(), "--html".into()], "--html"),
        (vec![format!("{url}#names")], "#names"),
        (vec![format!("{url}\"x")], "\""),
        (vec![format!("{url}/a b")], "whitespace"),
        (vec![format!("{url}/a\\b")], "\\"),
        (vec!["ftp://example.org/spec.txt".into()], "https"),
    ];
    for (extra, needle) in cases {
        let mut args = vec!["stage"];
        args.extend(extra.iter().map(String::as_str));
        let run = lab.library(&args);
        assert_eq!(run.code, 2, "{extra:?}: {}", run.stderr);
        assert!(run.stderr.contains(needle), "{extra:?}: {}", run.stderr);
        assert!(lab.staged().is_empty());
    }
    assert!(pages.requests().is_empty());
}

#[test]
fn a_url_without_a_scheme_is_a_missing_file() {
    let lab = Lab::new("fetch-noscheme");
    let run = lab.library(&[
        "stage",
        "go.dev/doc/effective_go",
        "--origin",
        "url: https://go.dev/doc/effective_go",
    ]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(
        run.stderr.contains("go.dev/doc/effective_go"),
        "{}",
        run.stderr
    );
    assert!(lab.staged().is_empty());
}

#[test]
fn an_unclosed_fence_warns_and_still_stages() {
    let lab = Lab::new("fetch-fence");
    let file = lab.input("fence.md", b"# Title\n\nText.\n\n```go\nfunc main() {}\n");
    let run = lab.library(&["stage", file.to_str().unwrap(), "--origin", ORIGIN]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stderr
            .lines()
            .any(|l| l == "bilbo: unclosed fence: the code fence on line 5 is never closed"),
        "{}",
        run.stderr
    );
    assert_eq!(lab.staged().len(), 1);
}

#[test]
fn a_blockquoted_heading_is_a_lost_heading() {
    let lab = Lab::new("fetch-lost");
    let pages = Pages::start(vec![("/p", html_route("headings"))]);
    let run = lab.library(&["stage", &pages.url("/p")]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let lost: Vec<&str> = run
        .stderr
        .lines()
        .filter(|l| l.contains("heading lost:"))
        .collect();
    assert_eq!(
        lost,
        ["bilbo: heading lost: <h2> 'Documentation Index' is not a heading in the capture"],
        "{}",
        run.stderr
    );
}

#[test]
fn fourteen_lost_headings_print_ten_lines_and_a_count() {
    let lab = Lab::new("fetch-many-lost");
    let body: String = (1..=14)
        .map(|i| format!("<blockquote><h2>Lost heading {i}</h2></blockquote>"))
        .collect();
    let (_pages, url) = served(Route::ok("text/html", format!("<body>{body}</body>")));
    let run = lab.library(&["stage", &url]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let lost: Vec<&str> = run
        .stderr
        .lines()
        .filter(|l| l.contains("heading lost:"))
        .collect();
    assert_eq!(lost.len(), 11, "{}", run.stderr);
    assert_eq!(
        lost.iter()
            .filter(|l| l.starts_with("bilbo: heading lost: <h2> "))
            .count(),
        10
    );
    assert_eq!(lost[10], "bilbo: heading lost: 4 more");
}

#[test]
fn a_menu_of_links_is_a_navigation_suspect() {
    let lab = Lab::new("fetch-nav");
    let pages = Pages::start(vec![("/p", html_route("main"))]);
    let run = lab.library(&["stage", &pages.url("/p")]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let md = String::from_utf8(fixture("main", "md")).unwrap();
    let at = |text: &str| md.lines().position(|l| l == text).unwrap() + 1;
    let (a, b) = (at("- [Home](/)"), at("- [Contact](/contact)"));
    assert!(
        run.stderr.lines().any(
            |l| l == format!("bilbo: navigation suspect: lines {a}-{b}, 9 lines of links only")
        ),
        "{}",
        run.stderr
    );
}

#[test]
fn a_saved_page_is_converted_with_html_and_kept_raw() {
    let lab = Lab::new("fetch-html-flag");
    let file = lab.input("page.html", &fixture("main", "html"));
    let before = snapshot(&lab.root);
    let run = lab.library(&[
        "stage",
        file.to_str().unwrap(),
        "--html",
        "--origin",
        "url: https://platform.example.com/docs/agents",
    ]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(snapshot(&lab.root), before);
    assert_eq!(capture_of(&run), fixture("main", "md"));
    assert_eq!(
        std::fs::read(field(&run.stdout, "raw")).unwrap(),
        fixture("main", "html")
    );
    assert!(!stage_folder(&run).join("fetch.json").exists());
    assert!(run.stdout.lines().any(|l| l.starts_with("content: ")));
    assert!(!run.stdout.lines().any(|l| l.starts_with("media type:")));
    let land = lab.land(field(&run.stdout, "stage"), "go/saved", &["--keep", "1-3"]);
    assert_eq!(land.code, 0, "{}", land.stderr);
    assert!(lab.read("go/saved.md").contains("capture: external"));
}

#[test]
fn the_same_file_without_html_stays_html_with_no_raw() {
    let lab = Lab::new("fetch-no-html-flag");
    let file = lab.input("page.html", &fixture("main", "html"));
    let run = lab.library(&[
        "stage",
        file.to_str().unwrap(),
        "--origin",
        "url: https://platform.example.com/docs/agents",
    ]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(capture_of(&run), fixture("main", "html"));
    assert!(!stage_folder(&run).join("raw").exists());
    assert!(
        !run.stdout
            .lines()
            .any(|l| l.starts_with("raw:") || l.starts_with("content:"))
    );
}

#[test]
fn a_redirect_to_the_same_path_with_a_slash_prints_the_final_url() {
    let lab = Lab::new("fetch-slash");
    let pages = Pages::start(vec![
        ("/dir", Route::redirect("/dir/")),
        ("/dir/", html_route("main")),
    ]);
    let run = lab.library(&["stage", &pages.url("/dir")]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(field(&run.stdout, "final url"), pages.url("/dir/"));
}

#[test]
fn a_bare_host_prints_no_final_url() {
    let lab = Lab::new("fetch-bare");
    let pages = Pages::start(vec![("/", Route::ok("text/plain", "# T\n\ntext\n"))]);
    let run = lab.library(&["stage", &pages.url.clone()]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(!run.stdout.contains("final url:"), "{}", run.stdout);
}

#[test]
fn a_page_without_a_main_has_no_content_lines() {
    let lab = Lab::new("fetch-nomain");
    let (_pages, url) = served(Route::ok(
        "text/html",
        "<h1>Site</h1><article><h2>One</h2><p>a</p></article><article><h2>Two</h2><p>b</p></article>",
    ));
    let run = lab.library(&["stage", &url]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(field(&run.stdout, "content"), "-");
    assert_eq!(field(&run.stdout, "title"), "Site");
}

#[test]
fn an_unreadable_library_leaves_no_stage_folder() {
    use std::os::unix::fs::PermissionsExt;
    let lab = Lab::new("fetch-library-locked");
    let library = lab.root.join("library");
    std::fs::create_dir_all(library.join("go")).unwrap();
    std::fs::set_permissions(&library, std::fs::Permissions::from_mode(0o000)).unwrap();
    let (_pages, url) = served(Route::ok("text/plain", "# T\n\ntext\n"));
    let run = lab.library(&["stage", &url]);
    std::fs::set_permissions(&library, std::fs::Permissions::from_mode(0o755)).unwrap();
    let readable = run.code == 0;
    if !readable {
        assert_eq!(run.code, 1, "{}", run.stderr);
        assert!(
            run.stderr.contains(&library.display().to_string()),
            "{}",
            run.stderr
        );
    }
    assert_eq!(lab.staged().len(), usize::from(readable));
}
