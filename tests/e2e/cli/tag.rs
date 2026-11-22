use super::scratch::Scratch;

const ID_A: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";
const ID_B: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DF";
const SHORT_A: &str = "2T6W9YB0DE";
const SHORT_B: &str = "2T6W9YB0DF";
const PAGE: &str = r#"bilbo tag: list the tags, list the entries of one, or change entries' tags.

Usage:
  bilbo tag
  bilbo tag tag:<name>
  bilbo tag <ref>... [--add <tag>]... [--remove <tag>]...

Options:
      --add <tag>     Declare this tag for each entry named
      --remove <tag>  Take this tag off each entry named

Output:
  tag:<name>  <n> entries  <kind> <n>, ...     one row a tag
  <id>  <kind>  <capture label>  person|model  one row an entry of a tag
  changed <n>, unchanged <n>                   after --add or --remove

Exit:
  0  success, also when nothing changed and when no entry carries a tag
  1  a ref that names no entry, a tag no entry carries, an entry that would
     pass tag.cap, a store failure
  2  usage or config error, or a ref that names several entries

Examples:
  bilbo tag
  bilbo tag tag:rust
  bilbo tag 01M6ZGE4KTBYYH07AS2KVMSCAF --add garden --remove shed

Docs: https://github.com/delucca/bilbo/wiki/Commands#tag
"#;

const BODIES: [&str; 3] = [
    "Seville marmalade ripens beside the tomato vines, forty jars by October.",
    "Quince marmalade keeps on the top rack behind the flour bins, away from damp.",
    "Rhubarb marmalade cools on the windowsill while the kettle is put back on.",
];

fn notes(name: &str) -> Scratch {
    let scratch = Scratch::new(name);
    for ((file, id), body) in [("a.md", ID_A), ("b.md", ID_B)].into_iter().zip(BODIES) {
        scratch.write(file, &format!("---\nid: {id}\n---\n# {file}\n\n{body}\n"));
    }
    scratch
}

#[test]
fn tags_declared_at_capture_and_by_the_verb_scope_a_recall() {
    let scratch = notes("tag_scope");
    let kept = scratch.run(&["remember", &scratch.path("notes"), "--tag", "docs"]);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    assert_eq!(
        kept.stderr,
        "bilbo: tagged 2 entries\nbilbo: tag:docs is new\n"
    );
    let added = scratch.run(&["tag", ID_A, "--add", "garden"]);
    assert_eq!(added.code, 0, "{}", added.stderr);
    assert_eq!(added.stdout, "changed 1, unchanged 0\n");

    let listed = scratch.run(&["tag"]);
    assert_eq!(
        listed.stdout,
        "tag:docs\t2 entries\tmarkdown 2\ntag:garden\t1 entries\tmarkdown 1\n"
    );
    let garden = scratch.run(&["recall", "--in", "tag:garden", "marmalade"]);
    assert_eq!(garden.code, 0, "{}", garden.stderr);
    assert!(garden.stdout.contains(SHORT_A), "{}", garden.stdout);
    assert!(!garden.stdout.contains(SHORT_B), "{}", garden.stdout);
    let docs = scratch.run(&["recall", "--in", "tag:docs", "marmalade"]);
    assert_eq!(docs.code, 0, "{}", docs.stderr);
    assert!(docs.stdout.contains(SHORT_A), "{}", docs.stdout);
    assert!(docs.stdout.contains(SHORT_B), "{}", docs.stdout);
}

#[test]
fn the_tag_page_is_printed_by_help_and_by_dash_dash_help() {
    let scratch = Scratch::new("tag_help");
    for form in [&["help", "tag"][..], &["tag", "--help"]] {
        let run = scratch.run(form);
        assert_eq!((run.code, run.stderr.as_str()), (0, ""), "{form:?}");
        assert_eq!(run.stdout, PAGE, "{form:?}");
    }
}

#[test]
fn a_tag_is_kept_across_a_reindex() {
    let scratch = notes("tag_reindex");
    scratch.run(&["remember", &scratch.path("notes"), "--tag", "docs"]);
    scratch.run(&["tag", ID_A, "--add", "garden"]);
    let listed = scratch.run(&["tag"]);
    let garden = scratch.run(&["tag", "tag:garden"]);

    let again = scratch.run(&["reindex"]);
    assert_eq!(again.code, 0, "{}", again.stderr);

    assert_eq!(scratch.run(&["tag"]).stdout, listed.stdout);
    assert_eq!(scratch.run(&["tag", "tag:garden"]).stdout, garden.stdout);
    let recalled = scratch.run(&["recall", "--in", "tag:garden", "marmalade"]);
    assert_eq!(recalled.code, 0, "{}", recalled.stderr);
    assert!(recalled.stdout.contains(SHORT_A), "{}", recalled.stdout);
    assert!(!recalled.stdout.contains(SHORT_B), "{}", recalled.stdout);
}
