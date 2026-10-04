mod common;

use std::path::{Path, PathBuf};

use common::{
    Fake, IDS, Locked, Run, TempDir, bench_library, bench_store, bilbo, config, dead_url, guide,
    library, note_text, snapshot, store, write,
};

const USAGE: &str = "\
usage: bilbo new <kind> <topic> [--title <text>]
       bilbo check
       bilbo recall <query>... [--kind <kind>]... [--limit <n>]
       bilbo recall <query>... --library [--corpus <corpus>]... [--limit <n>]
       bilbo index
       bilbo setup [--yes | --interactive] [--remove] [<setup option>]...
       bilbo digest
       bilbo library [<corpus>]
       bilbo library show <corpus>/<name>|<id>[#<anchor>] [--depth <n>]
       bilbo library stage <url> | <file> --origin \"<url|doc>: <value>\" [--fetched <YYYY-MM-DD>] [--html]
       bilbo library land <stage> <corpus>/<name> --keep <a>-<b>[,<c>-<d>]... [--title <text>] [--replace [--force]]
       bilbo library plan <ref>... [--budget-tokens <n>] [--slice-bytes <n>] [--slice-lines <n>]
       bilbo library read <plan> <slice>... [--part <k>/<n>]
       bilbo cite [--plan <plan>]... [<file> | -]
       bilbo --help
       bilbo --version
new creates <root>/notes/<kind>-<topic>.md and prints its path.
check prints every problem in the store and changes nothing.
recall prints the notes that best match the query, best first, 10 unless --limit says otherwise.
recall --library searches the sources and guides of the library by keyword instead of the notes; --corpus narrows it.
index embeds the passages the vector cache lacks and drops the ones no note holds any more.
digest reads a prompt hook's JSON on stdin and prints the notes that bear on the prompt; it always exits 0.
library lists the corpora, prints a corpus's guide with the facts of each source, or a source's outline; stage and land add a source; plan cuts picks into slices and partitions; read prints slices and logs them.
cite checks every bilbo: citation in a draft, and with --plan prints the coverage of the plans' reads.
setup creates the store and the config and installs the agent plugin, the index timer, the note watcher and, when asked, the local embedder; in a terminal it asks first.
setup options: --embedder-url <url>, --embedder-model <name>, --embedder-token-env <var>, --embedder-token-file <path>, --embedder-query-prefix <text>, --embedder-local, --embedder-port <port>, --llama-server <path>, --no-plugin, --claude <path>, --codex <path>, --plugin-source <folder|owner/repo#ref>, --no-timer, --index-every <minutes>, --no-watch
kinds: plan, spec, design, decision, gotcha, research, review, report, reference
root: $BILBO_HOME, else $XDG_DATA_HOME/bilbo, else $HOME/.local/share/bilbo
config: $BILBO_CONFIG, else $XDG_CONFIG_HOME/bilbo/config, else $HOME/.config/bilbo/config
cache: $XDG_CACHE_HOME/bilbo, else $HOME/.cache/bilbo
state: $XDG_STATE_HOME/bilbo, else $HOME/.local/state/bilbo
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

#[test]
#[ignore = "timing; run with --release -- --ignored"]
fn recall_over_a_6_mib_store_is_fast() {
    let dir = TempDir::new("recall-speed");
    let (root, mib) = bench_store(&dir);
    let library_mib = bench_library(&root);

    let args = ["embedder", "timeout", "decisao"];
    assert_eq!(recall(&dir, &root, &args).code, 0);
    let start = std::time::Instant::now();
    let run = recall(&dir, &root, &args);
    let elapsed = start.elapsed();
    eprintln!(
        "store {mib:.1} MiB beside a {library_mib:.1} MiB library, recall took {} ms",
        elapsed.as_millis()
    );
    assert_eq!(run.code, 0);
    assert!(elapsed.as_millis() < 250, "{elapsed:?}");
}

#[test]
#[ignore = "timing; run with --release -- --ignored"]
fn recall_library_over_14_mib_is_fast() {
    let dir = TempDir::new("recall-library-speed");
    let root = store(&dir);
    let mib = bench_library(&root);

    let args = ["embedder", "timeout", "decisao", "--library"];
    assert_eq!(recall(&dir, &root, &args).code, 0);
    let start = std::time::Instant::now();
    let run = recall(&dir, &root, &args);
    let elapsed = start.elapsed();
    eprintln!(
        "library {mib:.1} MiB, recall --library took {} ms",
        elapsed.as_millis()
    );
    assert_eq!(run.code, 0);
    assert!(elapsed.as_millis() < 500, "{elapsed:?}");
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
    let keys = "keys: embedder.url, embedder.model, embedder.token_file, embedder.token_env, embedder.query_prefix, embedder.min_similarity, digest.enable, digest.min_similarity, digest.log, history.keep_days";
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

#[test]
fn digest_log_must_be_on_or_off() {
    let dir = TempDir::new("recall-digest-log");
    let root = store(&dir);
    let config_path = config(&dir, &["digest.log = yes"]);
    let run = recall_with(&dir, &root, &config_path, &[], &["rollback"]);
    failed(&run, 2, "bilbo: ");
    assert!(
        run.stderr
            .contains(&format!("{}:1: digest.log", config_path.display())),
        "{}",
        run.stderr
    );
}

#[test]
fn digest_enable_must_be_on_or_off() {
    let dir = TempDir::new("recall-digest-enable");
    let root = store(&dir);
    let config_path = config(&dir, &["digest.enable = no"]);
    let run = recall_with(&dir, &root, &config_path, &[], &["rollback"]);
    failed(&run, 2, "bilbo: ");
    assert!(
        run.stderr
            .contains(&format!("{}:1: digest.enable", config_path.display())),
        "{}",
        run.stderr
    );
}

#[test]
fn digest_keys_alone_are_keyword_only() {
    let dir = TempDir::new("recall-digest-alone");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("Plan a", "rollback steps\n"));
    let config_path = config(&dir, &["digest.log = on"]);
    let run = recall_with(&dir, &root, &config_path, &[], &["rollback"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("plan-a.md"), "{}", run.stdout);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
}

// Library recall.

/// The title is line 7. Headings: Concurrency 11, Goroutines 15, Channels 19, Errors 23.
const EFFECTIVE_GO: &str = "# Effective Go\n\nIntro text.\n\n## Concurrency\n\nShare memory by communicating.\n\n### Goroutines\n\nA goroutine is cheap.\n\n### Channels\n\nChannels sync.\n\n## Errors\n\nReturn errors.\n";

fn go_root(dir: &TempDir) -> PathBuf {
    let root = store(dir);
    library(&root, "go", "effective-go", EFFECTIVE_GO);
    root
}

fn path_of(root: &Path, file: &str) -> String {
    root.join("library").join(file).display().to_string()
}

fn first_lines(run: &Run) -> Vec<&str> {
    run.stdout
        .trim_end()
        .split("\n\n")
        .filter(|b| !b.is_empty())
        .map(|b| b.lines().next().unwrap())
        .collect()
}

fn recall_lib(dir: &TempDir, root: &Path, args: &[&str]) -> Run {
    let mut full = args.to_vec();
    full.push("--library");
    recall(dir, root, &full)
}

#[test]
fn a_source_block() {
    let dir = TempDir::new("recall-lib-source");
    let root = go_root(&dir);
    let run = recall_lib(&dir, &root, &["goroutine"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty());
    assert_eq!(
        run.stdout,
        format!(
            "{}:15\tsource\tgo/effective-go\t15-18\nConcurrency > Goroutines\nA goroutine is cheap.\n",
            path_of(&root, "go/effective-go.md")
        )
    );
}

#[test]
fn a_guide_block() {
    let dir = TempDir::new("recall-lib-guide");
    let root = store(&dir);
    library(&root, "go", "effective-go", EFFECTIVE_GO);
    guide(
        &root,
        "go",
        "About go.",
        &[
            ("effective-go", "Idioms for writing clear Go."),
            ("errors", "Wrap them."),
        ],
    );
    let run = recall_lib(&dir, &root, &["idioms"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        format!(
            "{}:10\tguide\tgo\t10-13\neffective-go\nIdioms for writing clear Go.\n",
            path_of(&root, "go/guide.md")
        )
    );
}

#[test]
fn a_hit_under_the_title() {
    let dir = TempDir::new("recall-lib-title");
    let root = go_root(&dir);
    let run = recall_lib(&dir, &root, &["intro"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let mut lines = run.stdout.lines();
    assert_eq!(
        lines.next().unwrap(),
        format!(
            "{}:7\tsource\tgo/effective-go\t7-10",
            path_of(&root, "go/effective-go.md")
        )
    );
    assert_eq!(lines.next(), Some("-"));
}

#[test]
fn a_source_with_no_heading_below_its_title() {
    let dir = TempDir::new("recall-lib-flat");
    let root = store(&dir);
    library(&root, "go", "flat", "# Flat\n\nOne\nTwo\nThree wumpus\n");
    let run = recall_lib(&dir, &root, &["wumpus"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout.lines().next().unwrap().ends_with("\t7-11"),
        "{}",
        run.stdout
    );
    assert_eq!(run.stdout.lines().nth(1), Some("-"));
}

#[test]
fn a_section_holds_its_subsections() {
    let dir = TempDir::new("recall-lib-subsections");
    let root = go_root(&dir);
    let run = recall_lib(&dir, &root, &["memory"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let mut lines = run.stdout.lines();
    assert_eq!(
        lines.next().unwrap(),
        format!(
            "{}:11\tsource\tgo/effective-go\t11-22",
            path_of(&root, "go/effective-go.md")
        )
    );
    assert_eq!(lines.next(), Some("Concurrency"));
}

#[test]
fn a_later_part_has_its_own_line_and_the_same_section() {
    let dir = TempDir::new("recall-lib-part");
    let root = store(&dir);
    let first = "alpha ".repeat(600);
    let second = format!("zebra {}", "beta ".repeat(600));
    let body = format!("# T\n\n## Big\n\n{first}\n\n{second}\n\n## Next\n\nx\n");
    library(&root, "go", "big", &body);
    let run = recall_lib(&dir, &root, &["zebra"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        first_lines(&run),
        [format!(
            "{}:13\tsource\tgo/big\t9-14",
            path_of(&root, "go/big.md")
        )]
    );
    assert_eq!(run.stdout.lines().nth(1), Some("Big"));
}

#[test]
fn a_fenced_heading_does_not_end_a_section() {
    let dir = TempDir::new("recall-lib-fenced");
    let root = store(&dir);
    let body = "# T\n\n### Goroutines\n\nA goroutine.\n\n```\n## not a heading\n```\n\nmore\n\n## Next\n\nx\n";
    library(&root, "go", "fence", body);
    let run = recall_lib(&dir, &root, &["goroutine"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout.lines().next().unwrap().ends_with("\t9-18"),
        "{}",
        run.stdout
    );
}

#[test]
fn the_section_is_what_library_show_prints() {
    let dir = TempDir::new("recall-lib-show");
    let root = go_root(&dir);
    let run = recall_lib(&dir, &root, &["goroutine"]);
    let first = run.stdout.lines().next().unwrap();
    let range = first.rsplit('\t').next().unwrap();
    let anchor = run.stdout.lines().nth(1).unwrap();
    let show = bilbo(
        dir.path(),
        &[("BILBO_HOME", root.to_str().unwrap())],
        &["library", "show", &format!("go/effective-go#{anchor}")],
    );
    assert_eq!(show.code, 0, "{}", show.stderr);
    let row = show.stdout.lines().find(|l| l.contains(anchor)).unwrap();
    assert!(row.starts_with(&format!("{range}\t")), "{row} vs {range}");
}

#[test]
fn notes_are_not_searched() {
    let dir = TempDir::new("recall-lib-notes");
    let root = go_root(&dir);
    write(
        &root,
        "gotcha-goroutines.md",
        &note("Goroutines", "wumpus goroutine\n"),
    );
    let run = recall_lib(&dir, &root, &["wumpus"]);
    failed(&run, 1, "bilbo: no sources match\n");
    assert_eq!(run.stderr, "bilbo: no sources match\n");
}

#[test]
fn a_note_block_is_not_printed() {
    let dir = TempDir::new("recall-lib-no-note-block");
    let root = go_root(&dir);
    write(
        &root,
        "gotcha-goroutines.md",
        &note("Goroutines", "A goroutine.\n"),
    );
    let run = recall_lib(&dir, &root, &["goroutine"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(!run.stdout.contains("/notes/"), "{}", run.stdout);
    assert!(!run.stdout.contains("2026-10-02"), "{}", run.stdout);
    assert_eq!(blocks(&run), 1);
}

#[test]
fn the_embedder_is_never_asked() {
    let dir = TempDir::new("recall-lib-embedder");
    let fake = Fake::start(4);
    let (root, config) = embedded(&dir, &fake, &[], &rollback_notes());
    write(&root, "plan-fresh.md", &note("Fresh", "fresh text\n"));
    library(&root, "go", "effective-go", EFFECTIVE_GO);
    let sent = fake.requests().len();
    let before = snapshot(&dir.path().join("cache"));
    for args in [
        vec!["goroutine", "--library"],
        vec!["goroutine", "--corpus", "go"],
    ] {
        let run = recall_with(&dir, &root, &config, &[], &args);
        assert_eq!(run.code, 0, "{}", run.stderr);
        assert!(run.stderr.is_empty(), "{}", run.stderr);
    }
    assert_eq!(fake.requests().len(), sent);
    assert_eq!(snapshot(&dir.path().join("cache")), before);
}

#[test]
fn a_paraphrase_is_not_a_hit() {
    let dir = TempDir::new("recall-lib-paraphrase");
    let fake = Fake::start(4);
    fake.vector("thread", &[1.0, 0.0, 0.0, 0.0]);
    fake.vector("goroutine", &[1.0, 0.0, 0.0, 0.0]);
    let (root, config) = embedded(&dir, &fake, &[], &rollback_notes());
    library(&root, "go", "effective-go", EFFECTIVE_GO);
    let run = recall_with(&dir, &root, &config, &[], &["thread", "--library"]);
    failed(&run, 1, "bilbo: no sources match\n");
    assert_eq!(run.stderr, "bilbo: no sources match\n");
}

#[test]
fn library_frontmatter_is_not_searched() {
    let dir = TempDir::new("recall-lib-front");
    let root = store(&dir);
    library(&root, "go", "a", "# A\n\ntext\n");
    let path = root.join("library/go/a.md");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        text.replace("example.com/doc", "golang.org/doc/effective_go"),
    )
    .unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains("golang"));
    let run = recall_lib(&dir, &root, &["golang"]);
    failed(&run, 1, "bilbo: no sources match\n");
}

#[test]
fn an_edited_source_is_still_found() {
    let dir = TempDir::new("recall-lib-edited");
    let root = go_root(&dir);
    let path = root.join("library/go/effective-go.md");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("\nA goroutine leak, added by hand.\n");
    std::fs::write(&path, text).unwrap();
    let run = recall_lib(&dir, &root, &["leak"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("effective-go.md"), "{}", run.stdout);
}

#[test]
fn invalid_entries_are_skipped() {
    let dir = TempDir::new("recall-lib-invalid");
    let root = store(&dir);
    library(&root, "go", "ok", "# Ok\n\nnothing here\n");
    for file in [
        "go/Effective_Go.md",
        "Go-Old/errors.md",
        "go/.draft.md",
        "go/sub/deep.md",
        "plan/errors.md",
    ] {
        let path = root.join("library").join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "---\n---\n# T\n\nA goroutine.\n").unwrap();
    }
    let run = recall_lib(&dir, &root, &["goroutine"]);
    failed(&run, 1, "bilbo: no sources match\n");
    assert_eq!(run.stderr, "bilbo: no sources match\n");
}

#[test]
fn captures_are_not_searched() {
    let dir = TempDir::new("recall-lib-captures");
    let root = store(&dir);
    library(&root, "go", "ok", "# Ok\n\nnothing here\n");
    let captures = root.join(".bilbo/captures/01M3YJ7R6HK6NQ30DCDB1P4DYB");
    std::fs::create_dir_all(&captures).unwrap();
    std::fs::write(captures.join("capture.md"), "A goroutine.\n").unwrap();
    let run = recall_lib(&dir, &root, &["goroutine"]);
    failed(&run, 1, "bilbo: no sources match\n");
}

#[test]
fn library_more_query_words_rank_higher() {
    let dir = TempDir::new("recall-lib-words");
    let root = store(&dir);
    library(&root, "go", "effective-go", "# A\n\ngoroutine leak\n");
    library(&root, "go", "errors", "# B\n\nleak only\n");
    let run = recall_lib(&dir, &root, &["goroutine", "leak"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let firsts = first_lines(&run);
    assert_eq!(firsts.len(), 2);
    assert!(firsts[0].contains("effective-go.md"), "{firsts:?}");
    assert!(firsts[1].contains("errors.md"), "{firsts:?}");
}

#[test]
fn one_block_per_source() {
    let dir = TempDir::new("recall-lib-one-block");
    let root = store(&dir);
    let mut body = String::from("# Clippy lints\n");
    for n in 0..30 {
        body.push_str(&format!("\n## lint_{n}\n\nreturn value {n}\n"));
    }
    library(&root, "rust", "clippy-lints", &body);
    library(&root, "rust", "book", "# Book\n\nreturn early\n");
    library(&root, "go", "errors", "# Errors\n\nreturn an error\n");
    let run = recall_lib(&dir, &root, &["return"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(blocks(&run), 3);
    let catalog = first_lines(&run)
        .iter()
        .filter(|l| l.contains("clippy-lints.md"))
        .count();
    assert_eq!(catalog, 1);
}

#[test]
fn sources_and_guides_share_one_order() {
    let dir = TempDir::new("recall-lib-shared-order");
    let root = store(&dir);
    library(&root, "go", "effective-go", "# A\n\nidioms and errors\n");
    guide(
        &root,
        "go",
        "About go.",
        &[("effective-go", "idioms for go")],
    );
    let run = recall_lib(&dir, &root, &["idioms", "errors"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let firsts = first_lines(&run);
    assert_eq!(firsts.len(), 2);
    assert!(firsts[0].contains("\tsource\t"), "{firsts:?}");
    assert!(firsts[1].contains("\tguide\tgo\t"), "{firsts:?}");
}

#[test]
fn the_default_limit_is_10() {
    let dir = TempDir::new("recall-lib-limit");
    let root = store(&dir);
    for n in 0..15 {
        library(
            &root,
            "go",
            &format!("source-{n:02}"),
            "# S\n\nA goroutine.\n",
        );
    }
    assert_eq!(blocks(&recall_lib(&dir, &root, &["goroutine"])), 10);
    assert_eq!(
        blocks(&recall_lib(&dir, &root, &["goroutine", "--limit", "3"])),
        3
    );
}

#[test]
fn library_ties_fall_back_to_path_order() {
    let dir = TempDir::new("recall-lib-ties");
    let root = store(&dir);
    library(&root, "rust", "same", "# S\n\nA goroutine.\n");
    library(&root, "go", "same", "# S\n\nA goroutine.\n");
    let run = recall_lib(&dir, &root, &["goroutine"]);
    let firsts = first_lines(&run);
    assert!(firsts[0].contains("/go/same.md"), "{firsts:?}");
    assert!(firsts[1].contains("/rust/same.md"), "{firsts:?}");
}

fn errors_library(dir: &TempDir) -> PathBuf {
    let root = store(dir);
    for corpus in ["go", "rust", "haskell"] {
        library(&root, corpus, "errors", "# Errors\n\nWrapping errors.\n");
    }
    root
}

#[test]
fn one_corpus() {
    let dir = TempDir::new("recall-lib-one-corpus");
    let root = errors_library(&dir);
    let run = recall(&dir, &root, &["wrapping", "--corpus", "go"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(blocks(&run), 1);
    assert!(run.stdout.contains("/go/errors.md"), "{}", run.stdout);
}

#[test]
fn two_corpora() {
    let dir = TempDir::new("recall-lib-two-corpora");
    let root = errors_library(&dir);
    let run = recall(
        &dir,
        &root,
        &["errors", "--corpus", "go", "--corpus", "rust"],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(blocks(&run), 2);
    assert!(!run.stdout.contains("haskell"), "{}", run.stdout);
}

#[test]
fn a_corpus_named_with_no_library() {
    let dir = TempDir::new("recall-lib-corpus-no-library");
    let root = store(&dir);
    let run = recall(&dir, &root, &["errors", "--corpus", "lisp"]);
    failed(&run, 1, "bilbo: no library at ");
    assert_eq!(
        run.stderr,
        format!("bilbo: no library at {}\n", root.display())
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_corpus_or_source_is_skipped() {
    let dir = TempDir::new("recall-lib-unreadable");
    let root = store(&dir);
    library(&root, "go", "ok", "# Ok\n\nA goroutine.\n");
    let locked_file = library(&root, "go", "locked", "# Locked\n\nA goroutine.\n");
    library(&root, "rust", "a", "# A\n\nA goroutine.\n");
    let _file = Locked::new(&locked_file, 0o644);
    let _folder = Locked::new(&root.join("library/rust"), 0o755);
    if std::fs::read(&locked_file).is_ok() || std::fs::read_dir(root.join("library/rust")).is_ok() {
        eprintln!("skipped: the entries are readable despite mode 000 (running as root?)");
        return;
    }
    let run = recall_lib(&dir, &root, &["goroutine"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert_eq!(blocks(&run), 1);
    assert!(run.stdout.contains("/go/ok.md"), "{}", run.stdout);
}

#[test]
fn an_unknown_corpus() {
    let dir = TempDir::new("recall-lib-unknown-corpus");
    let root = errors_library(&dir);
    let run = recall(&dir, &root, &["errors", "--corpus", "lisp"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: no corpus 'lisp' in {}\n",
            root.join("library").display()
        )
    );
}

#[test]
fn a_bad_or_reserved_corpus_is_a_usage_error() {
    let dir = TempDir::new("recall-lib-bad-corpus");
    let root = errors_library(&dir);
    for name in ["Go", "plan", "go--x"] {
        let run = recall(&dir, &root, &["errors", "--corpus", name]);
        failed(&run, 2, "bilbo: ");
        assert!(run.stderr.contains(&format!("'{name}'")), "{}", run.stderr);
    }
    let run = recall(&dir, &root, &["errors", "--corpus"]);
    failed(&run, 2, "bilbo: --corpus needs a value");
}

#[test]
fn kind_with_the_library_is_a_usage_error() {
    let dir = TempDir::new("recall-lib-kind");
    let root = errors_library(&dir);
    for args in [
        &["goroutine", "--library", "--kind", "reference"][..],
        &["goroutine", "--corpus", "go", "--kind", "reference"][..],
    ] {
        let run = recall(&dir, &root, args);
        failed(&run, 2, "bilbo: ");
        assert!(
            run.stderr.contains("--kind") && run.stderr.contains("--library"),
            "{}",
            run.stderr
        );
    }
}

#[test]
fn library_takes_no_value() {
    let dir = TempDir::new("recall-lib-value");
    let root = errors_library(&dir);
    let run = recall(&dir, &root, &["wrapping", "--library=go"]);
    failed(&run, 2, "bilbo: ");
    assert!(run.stderr.contains("--library"), "{}", run.stderr);
}

#[test]
fn library_options_go_anywhere() {
    let dir = TempDir::new("recall-lib-anywhere");
    let root = errors_library(&dir);
    let a = recall(
        &dir,
        &root,
        &["--library", "--corpus=go", "wrapping", "errors"],
    );
    let b = recall(&dir, &root, &["wrapping", "errors", "--corpus", "go"]);
    assert_eq!(a.code, 0, "{}", a.stderr);
    assert_eq!(a.stdout, b.stdout);
    assert_eq!(blocks(&a), 1);
}

#[test]
fn double_dash_makes_library_a_word() {
    let dir = TempDir::new("recall-lib-dashdash");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "the --library flag\n"));
    let run = recall(&dir, &root, &["--", "--library", "flag"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("plan-a.md"), "{}", run.stdout);
}

#[test]
fn nothing_matches() {
    let dir = TempDir::new("recall-lib-nothing");
    let root = go_root(&dir);
    let run = recall_lib(&dir, &root, &["wumpus"]);
    failed(&run, 1, "bilbo: no sources match\n");
    assert_eq!(run.stderr, "bilbo: no sources match\n");
}

#[test]
fn no_library() {
    let dir = TempDir::new("recall-lib-none");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "goroutine\n"));
    let run = recall_lib(&dir, &root, &["goroutine"]);
    failed(&run, 1, "bilbo: no library at ");
    assert_eq!(
        run.stderr,
        format!("bilbo: no library at {}\n", root.display())
    );

    std::fs::create_dir_all(root.join("library/Not-Valid")).unwrap();
    std::fs::create_dir_all(root.join("library/plan")).unwrap();
    std::fs::write(root.join("library/loose.md"), "x").unwrap();
    let run = recall_lib(&dir, &root, &["goroutine"]);
    assert_eq!(
        run.stderr,
        format!("bilbo: no library at {}\n", root.display())
    );
    assert_eq!(run.code, 1);
}

#[test]
fn a_library_without_notes() {
    let dir = TempDir::new("recall-lib-no-notes");
    let root = dir.path().join("store");
    library(&root, "go", "effective-go", EFFECTIVE_GO);
    assert!(!root.join("notes").exists());
    let run = recall_lib(&dir, &root, &["goroutine"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert!(run.stdout.contains("effective-go.md"), "{}", run.stdout);

    let none = dir.path().join("empty");
    let run = recall_lib(&dir, &none, &["goroutine"]);
    failed(&run, 1, "bilbo: no library at ");
    assert!(!run.stderr.contains("no store"), "{}", run.stderr);
}

#[test]
fn plain_recall_without_notes_still_needs_a_store() {
    let dir = TempDir::new("recall-lib-plain-no-notes");
    let root = dir.path().join("store");
    library(&root, "go", "effective-go", EFFECTIVE_GO);
    let run = recall(&dir, &root, &["goroutine"]);
    failed(&run, 1, &format!("bilbo: no store at {}\n", root.display()));
}

#[test]
fn library_recall_still_loads_the_settings() {
    let dir = TempDir::new("recall-lib-config");
    let root = go_root(&dir);
    let config_path = config(&dir, &["digest.log = yes"]);
    let run = recall_with(&dir, &root, &config_path, &[], &["goroutine", "--library"]);
    failed(&run, 2, "bilbo: ");
    assert!(run.stderr.contains("digest.log"), "{}", run.stderr);
}

#[test]
fn library_recall_leaves_the_store_as_found() {
    let dir = TempDir::new("recall-lib-readonly");
    let root = go_root(&dir);
    write(&root, "plan-a.md", &note("A", "goroutine\n"));
    let before = snapshot(&root);
    let run = recall_lib(&dir, &root, &["goroutine"]);
    assert_eq!(run.code, 0);
    assert_eq!(snapshot(&root), before);
}

// Plain search leaves the library alone.

fn library_with_goroutines(root: &Path) {
    for n in 0..20 {
        library(
            root,
            "go",
            &format!("source-{n:02}"),
            "# S\n\nA goroutine leak.\n",
        );
    }
    guide(root, "go", "About go.", &[("source-00", "goroutine notes")]);
}

#[test]
fn plain_recall_ignores_sources() {
    let dir = TempDir::new("recall-plain-ignores");
    let root = store(&dir);
    write(
        &root,
        "gotcha-goroutines.md",
        &note("Goroutines", "A goroutine leak.\n"),
    );
    library_with_goroutines(&root);
    let library_before = snapshot(&root.join("library"));
    let run = recall(&dir, &root, &["goroutine"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert_eq!(blocks(&run), 1);
    assert!(
        run.stdout.contains("gotcha-goroutines.md"),
        "{}",
        run.stdout
    );
    assert!(!run.stdout.contains("library"), "{}", run.stdout);
    assert_eq!(snapshot(&root.join("library")), library_before);

    let locked = Locked::new(&root.join("library"), 0o755);
    let again = recall(&dir, &root, &["goroutine"]);
    drop(locked);
    assert_eq!(again.code, 0, "{}", again.stderr);
    assert!(again.stderr.is_empty(), "{}", again.stderr);
    assert_eq!(again.stdout, run.stdout);
}

#[test]
fn plain_recall_gives_no_hint_when_only_the_library_matches() {
    let dir = TempDir::new("recall-plain-no-hint");
    let root = store(&dir);
    write(&root, "plan-a.md", &note("A", "rollback\n"));
    library_with_goroutines(&root);
    let library_before = snapshot(&root.join("library"));
    let run = recall(&dir, &root, &["goroutine"]);
    failed(&run, 1, "bilbo: no notes match\n");
    assert_eq!(run.stderr, "bilbo: no notes match\n");
    assert_eq!(snapshot(&root.join("library")), library_before);

    let locked = Locked::new(&root.join("library"), 0o755);
    let again = recall(&dir, &root, &["goroutine"]);
    drop(locked);
    failed(&again, 1, "bilbo: no notes match\n");
    assert_eq!(again.stderr, "bilbo: no notes match\n");
}
