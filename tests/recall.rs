mod common;

use std::path::{Path, PathBuf};

use common::{Fake, IDS, Run, TempDir, bilbo, config, dead_url, note_text, snapshot, store, write};

const USAGE: &str = "\
usage: bilbo new <kind> <topic> [--title <text>]
       bilbo check
       bilbo recall <query>... [--kind <kind>]... [--limit <n>]
       bilbo index
       bilbo --help
new creates <root>/notes/<kind>-<topic>.md and prints its path.
check prints every problem in the store and changes nothing.
recall prints the notes that best match the query, best first, 10 unless --limit says otherwise.
index embeds the passages the vector cache lacks and drops the ones no note holds any more.
kinds: plan, spec, design, decision, gotcha, research, review, report, reference
root: $BILBO_HOME, else $XDG_DATA_HOME/bilbo, else $HOME/.local/share/bilbo
config: $BILBO_CONFIG, else $XDG_CONFIG_HOME/bilbo/config, else $HOME/.config/bilbo/config
cache: $XDG_CACHE_HOME/bilbo, else $HOME/.cache/bilbo
";

const CREATED: &str = "2026-10-02T14:23-03:00";

fn recall(dir: &TempDir, root: &Path, args: &[&str]) -> Run {
    let mut full = vec!["recall"];
    full.extend(args);
    bilbo(dir.path(), &[("BILBO_HOME", root.to_str().unwrap())], &full)
}

/// The title is line 6, line 7 is blank and `body` starts on line 8.
fn note(title: &str, body: &str) -> String {
    format!("{}\n{body}", note_text(IDS[0], title))
}

fn blocks(run: &Run) -> usize {
    if run.stdout.is_empty() {
        0
    } else {
        run.stdout.trim_end().split("\n\n").count()
    }
}

fn failed(run: &Run, code: i32, stderr_start: &str) {
    assert_eq!(run.code, code, "{}", run.stderr);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.starts_with(stderr_start), "{}", run.stderr);
}

#[test]
fn one_matching_note() {
    let dir = TempDir::new("recall-one");
    let root = store(&dir);
    let body = "Where notes live.\n## Layout\n\nOne flat folder.\n";
    write(&root, "decision-note-store.md", &note("Note store", body));
    let run = recall(&dir, &root, &["flat", "layout"]);
    assert_eq!(run.code, 0);
    assert!(run.stderr.is_empty());
    assert_eq!(
        run.stdout,
        format!(
            "{}:9\tdecision\t{CREATED}\nNote store > Layout\nOne flat folder.\n",
            root.join("notes/decision-note-store.md").display()
        )
    );
}

#[test]
fn unquoted_words_form_one_query() {
    let dir = TempDir::new("recall-unquoted");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "## S\n\nnote store\n"));
    let split = recall(&dir, &root, &["note", "store"]);
    let joined = recall(&dir, &root, &["note store"]);
    assert_eq!(split.code, 0);
    assert_eq!(split.stdout, joined.stdout);
}

#[test]
fn note_without_the_words_is_not_printed() {
    let dir = TempDir::new("recall-without");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "## S\n\nrollback\n"));
    write(&root, "plan-b.md", &note("B", "## S\n\nunrelated\n"));
    let run = recall(&dir, &root, &["rollback"]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("plan-a.md") && !run.stdout.contains("plan-b.md"));
}

#[test]
fn blocks_are_separated_by_one_blank_line() {
    let dir = TempDir::new("recall-blocks");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "## S\n\nrollback one\n"));
    write(&root, "plan-b.md", &note("B", "## S\n\nrollback two\n"));
    let run = recall(&dir, &root, &["rollback"]);
    let notes = root.join("notes");
    assert_eq!(
        run.stdout,
        format!(
            "{a}:8\tplan\t{CREATED}\nA > S\nrollback one\n\n{b}:8\tplan\t{CREATED}\nB > S\nrollback two\n",
            a = notes.join("plan-a.md").display(),
            b = notes.join("plan-b.md").display()
        )
    );
}

#[test]
fn nested_heading_path_is_printed() {
    let dir = TempDir::new("recall-nested");
    let root = store(&dir);
    let body = "## Gotchas\n\n### Two slots\n\nslotsxyz here\n";
    write(&root, "plan-a.md", &note("Embedder", body));
    let run = recall(&dir, &root, &["slotsxyz"]);
    assert_eq!(
        run.stdout.lines().nth(1),
        Some("Embedder > Gotchas > Two slots")
    );
}

#[test]
fn fenced_heading_opens_no_passage() {
    let dir = TempDir::new("recall-fence");
    let root = store(&dir);
    let body = "## Setup\n\n```\n# install deps\n```\n";
    write(&root, "plan-a.md", &note("Guide", body));
    let run = recall(&dir, &root, &["install"]);
    assert_eq!(run.code, 0);
    let mut lines = run.stdout.lines();
    assert!(lines.next().unwrap().contains(":8\tplan\t"));
    assert_eq!(lines.next(), Some("Guide > Setup"));
}

#[test]
fn no_title_uses_the_file_name() {
    let dir = TempDir::new("recall-no-title");
    let root = store(&dir);
    let text = format!(
        "---\nid: {}\ncreated: {CREATED}\n---\n\n## Part\n\nrollback here\n",
        IDS[0]
    );
    write(&root, "plan-untitled.md", &text);
    let run = recall(&dir, &root, &["rollback"]);
    assert_eq!(run.stdout.lines().nth(1), Some("plan-untitled > Part"));
}

#[test]
fn split_part_prints_its_own_line() {
    let dir = TempDir::new("recall-split");
    let root = store(&dir);
    let mut body = String::from("## Big section\n");
    for i in 0..10 {
        let lead = if i == 8 { "zebrafish " } else { "" };
        body.push_str(&format!("\n{lead}{}\n", "lorem ".repeat(150)));
    }
    write(&root, "plan-a.md", &note("Big", &body));
    let run = recall(&dir, &root, &["zebrafish"]);
    assert_eq!(run.code, 0);
    // The heading is line 8 and paragraph i is line 10 + 2i; parts of four paragraphs start at 0, 4 and 8.
    assert!(
        run.stdout.lines().next().unwrap().contains(":26\tplan\t"),
        "{}",
        run.stdout
    );
}

#[test]
fn case_and_accents_are_ignored() {
    let dir = TempDir::new("recall-accents");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "## S\n\nDecisão tomada\n"));
    let run = recall(&dir, &root, &["DECISAO"]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout.lines().nth(2), Some("Decisão tomada"));
}

#[test]
fn words_are_whole_words() {
    let dir = TempDir::new("recall-whole");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "## S\n\nrollbacks\n"));
    let run = recall(&dir, &root, &["rollback"]);
    failed(&run, 1, "bilbo: no notes match\n");
}

#[test]
fn query_without_words_is_usage_error() {
    let dir = TempDir::new("recall-no-words");
    let root = store(&dir);
    for (args, query) in [
        (vec!["- ?"], "- ?"),
        (vec!["--", "-h"], "-h"),
        (vec!["a", "b"], "a b"),
    ] {
        let run = recall(&dir, &root, &args);
        failed(
            &run,
            2,
            &format!("bilbo: query '{query}' has no words of 2 or more letters or digits\n"),
        );
    }
}

#[test]
fn missing_query_is_usage_error() {
    let dir = TempDir::new("recall-missing");
    let root = store(&dir);
    for args in [&[][..], &["--kind", "plan"][..], &["--"][..]] {
        failed(&recall(&dir, &root, args), 2, "bilbo: missing <query>\n");
    }
}

#[test]
fn more_query_words_rank_higher() {
    let dir = TempDir::new("recall-more-words");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "## S\n\nrollback only\n"));
    write(&root, "plan-b.md", &note("B", "## S\n\nrollback timeout\n"));
    let run = recall(&dir, &root, &["rollback", "timeout"]);
    assert!(run.stdout.lines().next().unwrap().contains("plan-b.md"));
}

#[test]
fn one_block_per_note() {
    let dir = TempDir::new("recall-one-block");
    let root = store(&dir);
    let body = "## One\n\nrollback\n\n## Two\n\nrollback\n\n## Three\n\nrollback\n";
    write(&root, "plan-a.md", &note("A", body));
    assert_eq!(blocks(&recall(&dir, &root, &["rollback"])), 1);
}

#[test]
fn ties_fall_back_to_path_order() {
    let dir = TempDir::new("recall-ties");
    let root = store(&dir);
    write(&root, "plan-b.md", &note("Same", "## S\n\nrollback\n"));
    write(&root, "plan-a.md", &note("Same", "## S\n\nrollback\n"));
    let run = recall(&dir, &root, &["rollback"]);
    let first = run.stdout.lines().next().unwrap();
    assert!(first.contains("plan-a.md"), "{first}");
    assert_eq!(blocks(&run), 2);
}

#[test]
fn frontmatter_is_not_searched() {
    let dir = TempDir::new("recall-frontmatter");
    let root = store(&dir);
    let text = format!(
        "---\nid: {}\ncreated: {CREATED}\nsources:\n  - \"url: https://github.com/x\"\n---\n\n# T\n\ntext\n",
        IDS[0]
    );
    write(&root, "plan-a.md", &text);
    failed(
        &recall(&dir, &root, &["github"]),
        1,
        "bilbo: no notes match\n",
    );
}

#[test]
fn bad_timestamp_shows_a_dash() {
    let dir = TempDir::new("recall-bad-created");
    let root = store(&dir);
    let text = note("A", "## S\n\nrollback\n").replace(CREATED, "2026-10-02");
    write(&root, "plan-a.md", &text);
    let run = recall(&dir, &root, &["rollback"]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.lines().next().unwrap().ends_with("\tplan\t-"));
}

#[test]
fn badly_named_file_is_skipped() {
    let dir = TempDir::new("recall-bad-name");
    let root = store(&dir);
    write(&root, "idea-foo.md", &note("A", "## S\n\nrollback\n"));
    let run = recall(&dir, &root, &["rollback"]);
    failed(&run, 1, "bilbo: no notes match\n");
    assert_eq!(run.stderr, "bilbo: no notes match\n");
}

#[test]
fn other_entries_are_skipped() {
    let dir = TempDir::new("recall-other");
    let root = store(&dir);
    let text = note("A", "## S\n\nrollback\n");
    write(&root, ".plan-hidden.md", &text);
    write(&root, "plan-a.md.bak", &text);
    write(&root, "plan-real.md", &text);
    let notes = root.join("notes");
    std::fs::create_dir(notes.join("plan-dir.md")).unwrap();
    std::os::unix::fs::symlink(notes.join("missing-rollback"), notes.join("plan-gone.md")).unwrap();
    let run = recall(&dir, &root, &["rollback"]);
    assert_eq!(run.code, 0);
    assert_eq!(blocks(&run), 1);
    assert!(run.stdout.contains("plan-real.md"));
}

#[test]
fn unreadable_note_is_skipped() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("recall-unreadable");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "## S\n\nrollback\n"));
    write(&root, "plan-b.md", &note("B", "## S\n\nrollback\n"));
    let locked = root.join("notes/plan-a.md");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&locked).is_ok() {
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
        eprintln!("skipped: the file is readable despite 0o000 (running as root?)");
        return;
    }
    let run = recall(&dir, &root, &["rollback"]);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("plan-b.md") && !run.stdout.contains("plan-a.md"));
}

#[test]
fn invalid_utf8_is_read_lossily() {
    let dir = TempDir::new("recall-lossy");
    let root = store(&dir);
    let mut bytes = note("A", "## S\n\n").into_bytes();
    bytes.extend_from_slice(b"rollback \xff here\n");
    std::fs::write(root.join("notes/plan-a.md"), bytes).unwrap();
    let run = recall(&dir, &root, &["rollback"]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("plan-a.md"));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn non_utf8_name_is_skipped() {
    use std::os::unix::ffi::OsStrExt;
    let dir = TempDir::new("recall-non-utf8-name");
    let root = store(&dir);
    let name = std::ffi::OsStr::from_bytes(b"plan-\xff.md");
    std::fs::write(
        root.join("notes").join(name),
        note("A", "## S\n\nrollback\n"),
    )
    .unwrap();
    failed(
        &recall(&dir, &root, &["rollback"]),
        1,
        "bilbo: no notes match\n",
    );
}

#[test]
fn kind_filter_keeps_only_that_kind() {
    let dir = TempDir::new("recall-kind");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "## S\n\nrollback\n"));
    write(&root, "decision-b.md", &note("B", "## S\n\nrollback\n"));
    let run = recall(&dir, &root, &["rollback", "--kind", "decision"]);
    assert_eq!(blocks(&run), 1);
    assert!(run.stdout.contains("decision-b.md"));
}

#[test]
fn kind_filter_takes_several_kinds() {
    let dir = TempDir::new("recall-kinds");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "## S\n\nrollback\n"));
    write(&root, "decision-b.md", &note("B", "## S\n\nrollback\n"));
    write(&root, "spec-c.md", &note("C", "## S\n\nrollback\n"));
    let args = [
        "rollback", "--kind", "plan", "--kind", "decision", "--kind", "plan",
    ];
    let run = recall(&dir, &root, &args);
    assert_eq!(blocks(&run), 2);
    assert!(!run.stdout.contains("spec-c.md"));
}

#[test]
fn unknown_kind_lists_the_kinds() {
    let dir = TempDir::new("recall-unknown-kind");
    let root = store(&dir);
    let run = recall(&dir, &root, &["rollback", "--kind", "idea"]);
    failed(
        &run,
        2,
        "bilbo: unknown kind 'idea'; kinds: plan, spec, design, decision, gotcha, research, review, report, reference\n",
    );
}

#[test]
fn kind_needs_a_value() {
    let dir = TempDir::new("recall-kind-value");
    let root = store(&dir);
    for args in [
        &["rollback", "--kind"][..],
        &["rollback", "--kind="][..],
        &["rollback", "--kind", ""][..],
    ] {
        failed(
            &recall(&dir, &root, args),
            2,
            "bilbo: --kind needs a value\n",
        );
    }
}

fn many(dir: &TempDir, count: usize) -> std::path::PathBuf {
    let root = store(dir);
    for i in 0..count {
        write(
            &root,
            &format!("plan-n{i:02}.md"),
            &note("N", "## S\n\nrollback\n"),
        );
    }
    root
}

#[test]
fn default_limit_is_10() {
    let dir = TempDir::new("recall-default-limit");
    let root = many(&dir, 15);
    assert_eq!(blocks(&recall(&dir, &root, &["rollback"])), 10);
}

#[test]
fn limit_caps_the_blocks() {
    let dir = TempDir::new("recall-limit");
    let root = many(&dir, 15);
    assert_eq!(
        blocks(&recall(&dir, &root, &["rollback", "--limit", "3"])),
        3
    );
    assert_eq!(
        blocks(&recall(&dir, &root, &["rollback", "--limit", "05"])),
        5
    );
}

#[test]
fn zero_limit_is_refused() {
    let dir = TempDir::new("recall-zero-limit");
    let root = many(&dir, 2);
    let run = recall(&dir, &root, &["rollback", "--limit", "0"]);
    failed(
        &run,
        2,
        "bilbo: --limit must be a whole number of 1 or more, got '0'\n",
    );
}

#[test]
fn bad_limits_are_refused() {
    let dir = TempDir::new("recall-bad-limits");
    let root = many(&dir, 2);
    for bad in ["-1", "abc", "1.5", "+3", "000"] {
        let run = recall(&dir, &root, &["rollback", "--limit", bad]);
        failed(
            &run,
            2,
            &format!("bilbo: --limit must be a whole number of 1 or more, got '{bad}'\n"),
        );
    }
    for args in [
        &["rollback", "--limit"][..],
        &["rollback", "--limit="][..],
        &["rollback", "--limit", ""][..],
    ] {
        failed(
            &recall(&dir, &root, args),
            2,
            "bilbo: --limit needs a value\n",
        );
    }
    for args in [
        &["rollback", "--limit", "2", "--limit", "3"][..],
        &["rollback", "--limit=2", "--limit", "3"][..],
    ] {
        failed(
            &recall(&dir, &root, args),
            2,
            "bilbo: --limit given more than once\n",
        );
    }
}

#[test]
fn options_before_or_after_the_query() {
    let dir = TempDir::new("recall-option-order");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "## S\n\nrollback\n"));
    write(&root, "decision-b.md", &note("B", "## S\n\nrollback\n"));
    let before = recall(&dir, &root, &["--kind", "decision", "rollback"]);
    let after = recall(&dir, &root, &["rollback", "--kind", "decision"]);
    assert_eq!(before.code, 0);
    assert_eq!(before.stdout, after.stdout);
}

#[test]
fn equals_forms_match_two_argument_forms() {
    let dir = TempDir::new("recall-equals");
    let root = store(&dir);
    write(&root, "decision-a.md", &note("A", "## S\n\nrollback\n"));
    write(&root, "decision-b.md", &note("B", "## S\n\nrollback\n"));
    write(&root, "plan-c.md", &note("C", "## S\n\nrollback\n"));
    let equals = recall(&dir, &root, &["rollback", "--kind=decision", "--limit=1"]);
    let split = recall(
        &dir,
        &root,
        &["rollback", "--kind", "decision", "--limit", "1"],
    );
    assert_eq!(equals.code, 0);
    assert_eq!(blocks(&equals), 1);
    assert_eq!(equals.stdout, split.stdout);
}

#[test]
fn double_dash_ends_options() {
    let dir = TempDir::new("recall-double-dash");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "## S\n\ntitle flag\n"));
    let run = recall(&dir, &root, &["--", "--title", "flag"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(blocks(&run), 1);
}

#[test]
fn unknown_option_is_usage_error() {
    let dir = TempDir::new("recall-unknown-option");
    let root = store(&dir);
    for option in ["--json", "-x", "--kinds", "--limit5"] {
        let run = recall(&dir, &root, &["rollback", option]);
        failed(&run, 2, &format!("bilbo: unknown option '{option}'\n"));
    }
}

#[test]
fn help_flags_print_help() {
    let dir = TempDir::new("recall-help");
    for args in [
        &["--help"][..],
        &["rollback", "-h"][..],
        &["--kind", "--help"][..],
    ] {
        let run = recall(&dir, &dir.path().join("store"), args);
        assert_eq!(run.code, 0);
        assert_eq!(run.stdout, USAGE);
        assert!(run.stderr.is_empty());
    }
}

#[test]
fn help_after_double_dash_is_a_query() {
    let dir = TempDir::new("recall-help-query");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "## S\n\nhelp\n"));
    let run = recall(&dir, &root, &["--", "--help"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(blocks(&run), 1);
}

#[test]
fn snippet_collapses_and_cuts_at_300_chars() {
    let dir = TempDir::new("recall-snippet");
    let root = store(&dir);
    let half = "ação ".repeat(50);
    let body = format!("## S\n\n{half}\n\n{half}\n");
    write(&root, "plan-a.md", &note("A", &body));
    let run = recall(&dir, &root, &["acao"]);
    let snippet = run.stdout.lines().nth(2).unwrap();
    let expected: String = vec!["ação"; 100].join(" ").chars().take(300).collect();
    assert_eq!(snippet.chars().count(), 300);
    assert_eq!(snippet, expected);
    assert!(!snippet.contains(['\t', '\r']));
}

#[test]
fn short_snippet_is_not_padded() {
    let dir = TempDir::new("recall-short");
    let root = store(&dir);
    write(
        &root,
        "plan-a.md",
        &note("A", "## Slots\n\nUse two slots.\n"),
    );
    let run = recall(&dir, &root, &["slots"]);
    assert_eq!(run.stdout.lines().nth(2), Some("Use two slots."));
}

#[test]
fn empty_snippet_prints_a_dash() {
    let dir = TempDir::new("recall-empty-snippet");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("Rollback plan", ""));
    write(&root, "plan-b.md", &note("B", "## S\n\nrollback\n"));
    let run = recall(&dir, &root, &["rollback"]);
    let notes = root.join("notes");
    assert_eq!(
        run.stdout,
        format!(
            "{b}:8\tplan\t{CREATED}\nB > S\nrollback\n\n{a}:6\tplan\t{CREATED}\nRollback plan\n-\n",
            a = notes.join("plan-a.md").display(),
            b = notes.join("plan-b.md").display()
        )
    );
}

#[test]
fn no_match_exits_1() {
    let dir = TempDir::new("recall-no-match");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "## S\n\nrollback\n"));
    let run = recall(&dir, &root, &["wumpus"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(run.stderr, "bilbo: no notes match\n");
}

#[test]
fn missing_store_is_refused() {
    let dir = TempDir::new("recall-missing-store");
    let root = dir.path().join("nowhere");
    let run = recall(&dir, &root, &["rollback"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!("bilbo: no store at {}\n", root.display())
    );
    assert!(!root.exists());
}

#[test]
fn recall_leaves_store_as_found() {
    let dir = TempDir::new("recall-readonly");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "## S\n\nrollback\n"));
    write(&root, ".new-X.tmp", "hidden");
    write(&root, "idea-bad.md", "# bad name\n");
    std::fs::create_dir(root.join("notes/archive")).unwrap();
    let before = snapshot(&root);
    let run = recall(&dir, &root, &["rollback"]);
    assert_eq!(run.code, 0);
    assert!(!run.stdout.is_empty());
    assert_eq!(snapshot(&root), before);
}

const BENCH_SECTIONS: usize = 6;

const BENCH_WORDS: &[&str] = &[
    "embedder",
    "timeout",
    "decisão",
    "ação",
    "configuração",
    "memória",
    "código",
    "índice",
    "sessão",
    "versão",
    "também",
    "não",
    "função",
    "próximo",
    "rollback",
    "cache",
    "store",
    "note",
    "index",
    "query",
    "passage",
    "heading",
    "ranking",
    "score",
    "token",
    "buffer",
    "thread",
    "queue",
    "retry",
    "backoff",
    "latency",
    "deploy",
    "release",
    "branch",
    "commit",
    "review",
    "agent",
    "prompt",
    "model",
    "context",
    "window",
    "limit",
    "batch",
    "worker",
    "stream",
    "socket",
    "client",
    "server",
    "request",
    "response",
    "schema",
    "migration",
    "column",
    "table",
    "record",
    "field",
    "value",
    "string",
    "number",
    "array",
    "object",
    "module",
    "package",
    "crate",
    "library",
    "compiler",
    "runtime",
    "memory",
    "storage",
    "decisão",
    "solução",
    "organização",
    "informação",
    "atenção",
    "relação",
    "operação",
    "documentação",
    "integração",
    "execução",
    "validação",
    "descrição",
    "condição",
    "posição",
    "variável",
    "método",
    "análise",
    "técnica",
    "prática",
    "histórico",
    "automático",
    "dinâmico",
    "estático",
    "através",
    "além",
    "então",
    "porém",
    "já",
    "até",
    "você",
    "são",
    "está",
    "será",
    "podem",
    "devem",
    "quando",
    "depois",
    "antes",
    "sempre",
    "nunca",
    "porque",
    "enquanto",
    "durante",
    "entre",
    "sobre",
    "sem",
    "com",
    "para",
    "the",
    "and",
    "with",
    "from",
    "into",
    "over",
    "under",
    "while",
    "after",
    "before",
    "because",
    "should",
    "would",
    "could",
    "might",
    "every",
    "other",
    "which",
    "their",
    "about",
    "first",
    "last",
    "next",
    "same",
    "each",
    "only",
];

fn xorshift(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn bench_text(seed: &mut u64, words: usize) -> String {
    let picked: Vec<&str> = (0..words)
        .map(|_| BENCH_WORDS[(xorshift(seed) % BENCH_WORDS.len() as u64) as usize])
        .collect();
    picked.join(" ")
}

/// A paragraph of 40 to 120 words.
fn paragraph(seed: &mut u64) -> String {
    let words = 40 + (xorshift(seed) % 81) as usize;
    bench_text(seed, words)
}

#[test]
#[ignore = "timing; run with --release -- --ignored"]
fn recall_over_a_6_mib_store_is_fast() {
    const KINDS: [&str; 9] = [
        "plan",
        "spec",
        "design",
        "decision",
        "gotcha",
        "research",
        "review",
        "report",
        "reference",
    ];
    let dir = TempDir::new("recall-speed");
    let root = store(&dir);
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut total = 0usize;
    for i in 0..450 {
        let mut text = format!(
            "---\nid: {}\ncreated: {CREATED}\n---\n\n# Bench note {i}\n",
            IDS[0]
        );
        for section in 0..BENCH_SECTIONS {
            text.push_str(&format!(
                "\n## Section {section}\n\n{}\n",
                paragraph(&mut seed)
            ));
            for sub in 0..2 + (section + i) % 3 {
                text.push_str(&format!("\n### Part {sub}\n\n{}\n", paragraph(&mut seed)));
            }
            if section % 3 == 0 {
                text.push_str("\n```\nfn main() {}\n# not a heading\n```\n");
            }
        }
        total += text.len();
        write(&root, &format!("{}-bench-{i}.md", KINDS[i % 9]), &text);
    }
    let mib = total as f64 / (1024.0 * 1024.0);
    assert!((5.5..=6.5).contains(&mib), "{mib} MiB");

    let args = ["embedder", "timeout", "decisao"];
    assert_eq!(recall(&dir, &root, &args).code, 0);
    let start = std::time::Instant::now();
    let run = recall(&dir, &root, &args);
    let elapsed = start.elapsed();
    eprintln!("store {mib:.1} MiB, recall took {} ms", elapsed.as_millis());
    assert_eq!(run.code, 0);
    assert!(elapsed.as_millis() < 250, "{elapsed:?}");
}

/// A store and a config for the embedder at `url`, with `extra` config lines after the standard two.
fn prepare(dir: &TempDir, url: &str, extra: &[&str]) -> (PathBuf, PathBuf) {
    let root = store(dir);
    let url = format!("embedder.url = {url}");
    let mut lines = vec![url.as_str(), "embedder.model = test-model"];
    lines.extend(extra);
    (root, config(dir, &lines))
}

fn recall_with(
    dir: &TempDir,
    root: &Path,
    config: &Path,
    extra_env: &[(&str, &str)],
    args: &[&str],
) -> Run {
    let mut vars = vec![
        ("BILBO_HOME", root.to_str().unwrap()),
        ("BILBO_CONFIG", config.to_str().unwrap()),
    ];
    let cache = dir.path().join("cache");
    vars.push(("XDG_CACHE_HOME", cache.to_str().unwrap()));
    vars.extend_from_slice(extra_env);
    let mut full = vec!["recall"];
    full.extend(args);
    bilbo(dir.path(), &vars, &full)
}

fn index_now(dir: &TempDir, root: &Path, config: &Path, extra_env: &[(&str, &str)]) {
    let cache = dir.path().join("cache");
    let mut vars = vec![
        ("BILBO_HOME", root.to_str().unwrap()),
        ("BILBO_CONFIG", config.to_str().unwrap()),
        ("XDG_CACHE_HOME", cache.to_str().unwrap()),
    ];
    vars.extend_from_slice(extra_env);
    let run = bilbo(dir.path(), &vars, &["index"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
}

/// A store holding `notes` (file name, title, body), indexed through the fake; returns the root and the config.
fn embedded(
    dir: &TempDir,
    fake: &Fake,
    extra: &[&str],
    notes: &[(&str, &str, &str)],
) -> (PathBuf, PathBuf) {
    let (root, config) = prepare(dir, &fake.url, extra);
    for (name, title, body) in notes {
        write(&root, name, &note(title, body));
    }
    index_now(dir, &root, &config, &[]);
    (root, config)
}

fn note_store() -> (&'static str, &'static str, &'static str) {
    (
        "decision-note-store.md",
        "Note store",
        "## Layout\n\nOne flat folder.\n",
    )
}

fn rollback_notes() -> [(&'static str, &'static str, &'static str); 2] {
    [
        ("plan-a.md", "Plan a", "rollback steps\n"),
        ("plan-b.md", "Plan b", "undo the release\n"),
    ]
}

fn down_config(dir: &TempDir) -> PathBuf {
    config(
        dir,
        &[
            &format!("embedder.url = {}", dead_url()),
            "embedder.model = test-model",
        ],
    )
}

#[test]
fn paraphrase_is_found() {
    let dir = TempDir::new("recall-paraphrase");
    let fake = Fake::start(4);
    fake.vector("where do notes live", &[1.0, 0.0, 0.0, 0.0]);
    fake.vector("flat folder", &[0.6, 0.8, 0.0, 0.0]);
    let (root, config) = embedded(&dir, &fake, &[], &[note_store()]);
    let run = recall_with(&dir, &root, &config, &[], &["where", "do", "notes", "live"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert_eq!(blocks(&run), 1, "{}", run.stdout);
    assert!(
        run.stdout.contains("decision-note-store.md"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("One flat folder."), "{}", run.stdout);
}

#[test]
fn weak_similarity_is_not_a_hit() {
    let dir = TempDir::new("recall-weak");
    let fake = Fake::start(4);
    fake.vector("where do notes live", &[1.0, 0.0, 0.0, 0.0]);
    fake.vector("flat folder", &[0.2, 0.9798, 0.0, 0.0]);
    let (root, config) = embedded(&dir, &fake, &[], &[note_store()]);
    let run = recall_with(&dir, &root, &config, &[], &["where do notes live"]);
    failed(&run, 1, "bilbo: no notes match\n");
    assert_eq!(run.stderr, "bilbo: no notes match\n");
}

#[test]
fn agreement_beats_one_signal() {
    let dir = TempDir::new("recall-agreement");
    let fake = Fake::start(4);
    fake.vector("rollback steps", &[0.8, 0.6, 0.0, 0.0]);
    fake.vector("undo the release", &[1.0, 0.0, 0.0, 0.0]);
    fake.vector("rollback", &[1.0, 0.0, 0.0, 0.0]);
    let (root, config) = embedded(&dir, &fake, &[], &rollback_notes());
    let run = recall_with(&dir, &root, &config, &[], &["rollback"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert_eq!(blocks(&run), 2, "{}", run.stdout);
    let a = run.stdout.find("plan-a.md").unwrap();
    let b = run.stdout.find("plan-b.md").unwrap();
    assert!(a < b, "{}", run.stdout);
}

#[test]
fn embedder_down_falls_back() {
    let dir = TempDir::new("recall-down");
    let fake = Fake::start(4);
    let (root, _) = embedded(&dir, &fake, &[], &rollback_notes());
    let dead = down_config(&dir);
    let run = recall_with(&dir, &root, &dead, &[], &["rollback"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("plan-a.md"), "{}", run.stdout);
    assert_eq!(run.stderr.lines().count(), 1, "{}", run.stderr);
    let start = format!(
        "bilbo: embedder unavailable (embedder {} unreachable: ",
        dead_url_of(&dead)
    );
    assert!(run.stderr.starts_with(&start), "{}", run.stderr);
    assert!(
        run.stderr.ends_with("); keyword results only\n"),
        "{}",
        run.stderr
    );
}

/// The URL a config written by `down_config` names.
fn dead_url_of(config: &Path) -> String {
    let text = std::fs::read_to_string(config).unwrap();
    text.lines()
        .next()
        .unwrap()
        .strip_prefix("embedder.url = ")
        .unwrap()
        .to_string()
}

#[test]
fn stalled_embedder_falls_back_after_5_s() {
    let dir = TempDir::new("recall-stall");
    let fake = Fake::start(4);
    let (root, config) = embedded(&dir, &fake, &[], &rollback_notes());
    fake.stall();
    let start = std::time::Instant::now();
    let run = recall_with(&dir, &root, &config, &[], &["rollback"]);
    let elapsed = start.elapsed();
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: embedder unavailable (embedder {} did not answer within 5 s); keyword results only\n",
            fake.url
        )
    );
    assert!(run.stdout.contains("plan-a.md"), "{}", run.stdout);
    assert!(elapsed.as_secs_f64() >= 4.5, "{elapsed:?}");
    assert!(elapsed.as_secs_f64() < 10.0, "{elapsed:?}");
}

#[test]
fn rejected_query_falls_back() {
    let dir = TempDir::new("recall-rejected");
    let fake = Fake::start(4);
    let (root, config) = embedded(&dir, &fake, &[], &rollback_notes());
    for code in [401, 500] {
        fake.status(code);
        let run = recall_with(&dir, &root, &config, &[], &["rollback"]);
        assert_eq!(run.code, 0, "{}", run.stderr);
        assert_eq!(
            run.stderr,
            format!(
                "bilbo: embedder unavailable (embedder {} answered {code}); keyword results only\n",
                fake.url
            )
        );
        assert!(run.stdout.contains("plan-a.md"), "{}", run.stdout);
    }
}

#[test]
fn note_written_after_the_index() {
    let dir = TempDir::new("recall-after-index");
    let fake = Fake::start(4);
    fake.vector("rollback", &[1.0, 0.0, 0.0, 0.0]);
    let (root, config) = embedded(&dir, &fake, &[], &[note_store()]);
    let cache = dir.path().join("cache");
    let made = bilbo(
        dir.path(),
        &[
            ("BILBO_HOME", root.to_str().unwrap()),
            ("XDG_CACHE_HOME", cache.to_str().unwrap()),
        ],
        &["new", "plan", "rollback-plan", "--title", "Rollback plan"],
    );
    assert_eq!(made.code, 0, "{}", made.stderr);
    let path = root.join("notes/plan-rollback-plan.md");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("\nrollback steps\n");
    std::fs::write(&path, text).unwrap();
    let run = recall_with(&dir, &root, &config, &[], &["rollback"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout.contains("plan-rollback-plan.md"),
        "{}",
        run.stdout
    );
    assert_eq!(
        run.stderr,
        "bilbo: 1 passages not indexed; run bilbo index\n"
    );
}

#[test]
fn no_embedder_no_warnings() {
    let dir = TempDir::new("recall-no-embedder");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("Plan a", "rollback steps\n"));
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let run = bilbo(
        dir.path(),
        &[
            ("BILBO_HOME", root.to_str().unwrap()),
            ("HOME", home.to_str().unwrap()),
        ],
        &["recall", "rollback"],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert_eq!(blocks(&run), 1);
}

#[test]
fn query_prefix_and_cut() {
    let dir = TempDir::new("recall-prefix");
    let fake = Fake::start(4);
    let extra = ["embedder.query_prefix = \"Instruct: find notes\\nQuery: \""];
    let (root, config) = embedded(&dir, &fake, &extra, &[note_store()]);
    recall_with(&dir, &root, &config, &[], &["where do notes live"]);
    let last = fake.requests().pop().unwrap();
    assert_eq!(
        last.inputs,
        ["Instruct: find notes\nQuery: where do notes live"]
    );

    let long = "ação ".repeat(600);
    let long = long.trim_end();
    recall_with(&dir, &root, &config, &[], &[long]);
    let sent = fake.requests().pop().unwrap().inputs;
    assert_eq!(sent.len(), 1);
    let full = format!("Instruct: find notes\nQuery: {long}");
    assert!(sent[0].len() <= 2000, "{}", sent[0].len());
    assert!(sent[0].len() >= 1996, "{}", sent[0].len());
    assert!(full.starts_with(&sent[0]));
}

#[test]
fn unindexed_store_sends_no_query() {
    let dir = TempDir::new("recall-unindexed");
    let fake = Fake::start(4);
    let (root, config) = prepare(&dir, &fake.url, &[]);
    for (name, title, body) in rollback_notes() {
        write(&root, name, &note(title, body));
    }
    let run = recall_with(&dir, &root, &config, &[], &["rollback"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stderr,
        "bilbo: 2 passages not indexed; run bilbo index\n"
    );
    assert!(run.stdout.contains("plan-a.md"), "{}", run.stdout);
    assert!(fake.requests().is_empty());
}

#[test]
fn model_mismatch_counts_every_passage() {
    let dir = TempDir::new("recall-model");
    let fake = Fake::start(4);
    let (root, _) = embedded(&dir, &fake, &[], &rollback_notes());
    let sent = fake.requests().len();
    let other = config(
        &dir,
        &[
            &format!("embedder.url = {}", fake.url),
            "embedder.model = model-b",
        ],
    );
    let run = recall_with(&dir, &root, &other, &[], &["rollback"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stderr,
        "bilbo: 2 passages not indexed; run bilbo index\n"
    );
    assert_eq!(fake.requests().len(), sent);
}

#[test]
fn kind_filter_applies_to_meaning() {
    let dir = TempDir::new("recall-kind-meaning");
    let fake = Fake::start(4);
    fake.vector("where do notes live", &[1.0, 0.0, 0.0, 0.0]);
    fake.vector("flat folder", &[0.6, 0.8, 0.0, 0.0]);
    let (root, config) = embedded(
        &dir,
        &fake,
        &[],
        &[note_store(), ("plan-a.md", "Plan a", "other text\n")],
    );
    let all = recall_with(&dir, &root, &config, &[], &["where do notes live"]);
    assert_eq!(all.code, 0, "{}", all.stderr);
    assert!(
        all.stdout.contains("decision-note-store.md"),
        "{}",
        all.stdout
    );
    let plans = recall_with(
        &dir,
        &root,
        &config,
        &[],
        &["where do notes live", "--kind", "plan"],
    );
    failed(&plans, 1, "bilbo: no notes match\n");
}

#[test]
fn keyword_matches_past_50_still_print() {
    let dir = TempDir::new("recall-past-50");
    let fake = Fake::start(4);
    let (root, config) = prepare(&dir, &fake.url, &[]);
    for i in 0..60 {
        write(
            &root,
            &format!("plan-n{i:02}.md"),
            &note(&format!("N{i}"), "rollback\n"),
        );
    }
    index_now(&dir, &root, &config, &[]);
    let run = recall_with(&dir, &root, &config, &[], &["rollback", "--limit", "60"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert_eq!(blocks(&run), 60);
}

#[test]
fn no_match_keeps_the_warning() {
    let dir = TempDir::new("recall-no-match-warning");
    let fake = Fake::start(4);
    let (root, _) = embedded(&dir, &fake, &[], &rollback_notes());
    let dead = down_config(&dir);
    let run = recall_with(&dir, &root, &dead, &[], &["zzz", "unmatched"]);
    failed(&run, 1, "bilbo: embedder unavailable (");
    assert!(
        run.stderr
            .ends_with("); keyword results only\nbilbo: no notes match\n"),
        "{}",
        run.stderr
    );
    assert_eq!(run.stderr.lines().count(), 2, "{}", run.stderr);
}

#[test]
fn empty_token_variable_falls_back() {
    let dir = TempDir::new("recall-empty-token");
    let fake = Fake::start(4);
    let (root, config) = prepare(&dir, &fake.url, &["embedder.token_env = EMBED_TOKEN"]);
    for (name, title, body) in rollback_notes() {
        write(&root, name, &note(title, body));
    }
    index_now(&dir, &root, &config, &[("EMBED_TOKEN", "tok")]);
    let run = recall_with(&dir, &root, &config, &[("EMBED_TOKEN", "")], &["rollback"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stderr,
        "bilbo: embedder unavailable (embedder token variable EMBED_TOKEN is empty); keyword results only\n"
    );
    assert!(run.stdout.contains("plan-a.md"), "{}", run.stdout);
}

#[test]
fn recall_leaves_store_and_cache_as_found() {
    let dir = TempDir::new("recall-as-found");
    let fake = Fake::start(4);
    fake.vector("where do notes live", &[1.0, 0.0, 0.0, 0.0]);
    fake.vector("flat folder", &[0.6, 0.8, 0.0, 0.0]);
    let (root, config) = embedded(&dir, &fake, &[], &[note_store()]);
    let cache = dir.path().join("cache");
    let (store_before, cache_before) = (snapshot(&root), snapshot(&cache));
    let run = recall_with(&dir, &root, &config, &[], &["where do notes live"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(blocks(&run), 1);
    assert_eq!(snapshot(&root), store_before);
    assert_eq!(snapshot(&cache), cache_before);
}

#[test]
fn config_errors_exit_2() {
    let dir = TempDir::new("recall-config");
    let fake = Fake::start(4);
    let (root, config_path) = embedded(&dir, &fake, &[], &rollback_notes());
    let path = config_path.display().to_string();
    let url = format!("embedder.url = {}", fake.url);
    let keys = "keys: embedder.url, embedder.model, embedder.token_file, embedder.token_env, embedder.query_prefix, embedder.min_similarity";
    let cases: Vec<(Vec<&str>, String)> = vec![
        (
            vec!["# c", "embedder.model = x", "embeder.url = http://h"],
            format!("bilbo: {path}:3: unknown key 'embeder.url'; {keys}\n"),
        ),
        (
            vec![&url],
            format!("bilbo: {path}: embedder.url is set but embedder.model is not\n"),
        ),
        (
            vec![
                &url,
                "embedder.model = m",
                "embedder.token_file = /x",
                "embedder.token_env = Y",
            ],
            format!("bilbo: {path}: set embedder.token_file or embedder.token_env, not both\n"),
        ),
        (
            vec![&url, "embedder.model = m", "embedder.min_similarity = 1.5"],
            format!(
                "bilbo: {path}:3: embedder.min_similarity must be a number from 0 to 1, got '1.5'\n"
            ),
        ),
        (
            vec![&url, "embedder.model ="],
            format!("bilbo: {path}:2: embedder.model needs a value\n"),
        ),
    ];
    let sent = fake.requests().len();
    for (lines, stderr) in cases {
        config(&dir, &lines);
        let run = recall_with(&dir, &root, &config_path, &[], &["rollback"]);
        failed(&run, 2, "bilbo: ");
        assert_eq!(run.stderr, stderr);

        let usage = recall_with(&dir, &root, &config_path, &[], &["- ?"]);
        assert_eq!(usage.code, 2);
        assert!(
            usage
                .stderr
                .starts_with("bilbo: query '- ?' has no words of 2 or more letters or digits\n"),
            "{}",
            usage.stderr
        );
    }
    assert_eq!(fake.requests().len(), sent);

    let missing = dir.path().join("no-such-config");
    let run = recall_with(&dir, &root, &missing, &[], &["rollback"]);
    failed(
        &run,
        2,
        &format!("bilbo: cannot read {}: ", missing.display()),
    );
}

#[test]
fn textless_title_is_not_a_meaning_hit() {
    let dir = TempDir::new("recall-textless-title");
    let fake = Fake::start(4);
    fake.vector("quux", &[1.0, 0.0, 0.0, 0.0]);
    fake.vector("Zork\n", &[1.0, 0.0, 0.0, 0.0]);
    let notes = [("plan-a.md", "Zork", "## Sec\n\nalpha beta\n")];
    let (root, config) = embedded(&dir, &fake, &[], &notes);
    assert!(
        fake.inputs().iter().all(|input| input != "Zork\n"),
        "{:?}",
        fake.inputs()
    );
    let run = recall_with(&dir, &root, &config, &[], &["quux"]);
    failed(&run, 1, "bilbo: no notes match\n");
    assert_eq!(run.stderr, "bilbo: no notes match\n");
}

#[test]
fn not_indexed_count_ignores_textless_passages() {
    let dir = TempDir::new("recall-textless-count");
    let fake = Fake::start(4);
    let (root, config) = embedded(&dir, &fake, &[], &rollback_notes());
    write(&root, "plan-empty.md", &note("Empty", ""));
    write(&root, "plan-fresh.md", &note("Fresh", "fresh text\n"));
    let run = recall_with(&dir, &root, &config, &[], &["rollback"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stderr,
        "bilbo: 1 passages not indexed; run bilbo index\n"
    );
}
