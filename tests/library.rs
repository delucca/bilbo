mod common;

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use common::{Run, TempDir, bilbo, note_text, snapshot};

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
    format!("# T\n{}\n", "a".repeat(bytes - 5))
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
        &source(ID_A, &["capture: legacy"], &sized(1000)),
    );
    let run = lab.library(&["go"]);
    assert_eq!(run.code, 0);
    let facts = format!(
        "`effective-go.md` · {ID_A} · 1 KB · 400 tokens · fetched 2026-08-23 · 0 headings · capture legacy"
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

    for reserved in ["plan", "read"] {
        let run = lab.library(&[reserved]);
        assert_eq!(run.code, 2);
        assert!(
            run.stderr
                .contains(&format!("bilbo: '{reserved}' is reserved")),
            "{}",
            run.stderr
        );
    }
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
fn a_legacy_source_has_no_capture_folder() {
    let lab = Lab::new("lib-show-legacy");
    lab.put(
        "go/old.md",
        &source(ID_A, &["capture: legacy"], "# Old\n\ntext\n"),
    );
    let run = lab.library(&["show", "go/old"]);
    assert!(run.stdout.contains("\ncapture: legacy\n"));
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
    let config = lab.input("config", b"embeder.url = http://bagend:8081\n");
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
