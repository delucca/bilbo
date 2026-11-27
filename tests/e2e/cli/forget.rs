use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use super::recall::hit_refs;
use super::scratch::Scratch;
use super::sessions::{counts, first};

const FORGOTTEN: &str = "forgotten 1, belonging 0, unchanged 0\n";
const NOTHING: &str = "bilbo: nothing matched\n";
const NOTE: &str = include_str!("../../../libs/entries/rust/fixtures/markdown/note.md");
const SECTIONS: &str = include_str!("../../../libs/entries/rust/fixtures/markdown/sections.md");
const PLAIN: &str = include_str!("../../../libs/entries/rust/fixtures/markdown/no-frontmatter.md");
const ID_A: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";
const ID_B: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DG";

fn shown(path: &Path) -> String {
    path.display()
        .to_string()
        .chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

fn quoted(path: &str) -> String {
    let plain = !path.is_empty()
        && path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/._-+:@%,=".contains(&byte));
    if plain {
        path.to_owned()
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

fn passed_line(path: &Path, id: &str) -> String {
    let raw = path.display().to_string();
    let said = format!(
        "bilbo: passed over {}: its entry {id} was forgotten",
        shown(path)
    );
    if raw.chars().all(|c| !c.is_control()) {
        format!("{said}; bilbo remember {} keeps it again", quoted(&raw))
    } else {
        said
    }
}

fn old(path: &Path) {
    fs::File::options()
        .write(true)
        .open(path)
        .expect("open")
        .set_modified(SystemTime::now() - Duration::from_secs(3600))
        .expect("an old time");
}

fn write(scratch: &Scratch, name: &str, text: &str) -> std::path::PathBuf {
    scratch.write(name, text);
    let path = scratch.dir.join("notes").join(name);
    old(&path);
    path
}

fn entry_part(scratch: &Scratch, word: &str) -> String {
    let found = scratch.run(&["recall", "--", word]);
    assert_eq!(found.code, 0, "{}", found.stderr);
    hit_refs(&found.stdout)
        .first()
        .and_then(|reference| reference.split('#').next())
        .expect("a hit")
        .to_owned()
}

fn id_of(scratch: &Scratch, reference: &str) -> String {
    let read = scratch.run(&["read", reference]);
    assert_eq!(read.code, 0, "{}", read.stderr);
    read.stdout
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("bilbo:"))
        .expect("an id line")
        .to_owned()
}

fn gone(scratch: &Scratch, word: &str, searched: &str) {
    let after = scratch.run(&["recall", "--", word]);
    assert_eq!(after.code, 1, "{}", after.stderr);
    assert_eq!(after.stdout, "");
    assert_eq!(
        after.stderr,
        format!("bilbo: searched {searched}\n{NOTHING}")
    );
}

#[test]
fn a_forgotten_file_is_gone_from_recall_right_after() {
    let scratch = Scratch::new("forget_file");
    write(
        &scratch,
        "beets.md",
        "# Beets\n\nThe beet seeds are in the drawer.\n",
    );
    write(
        &scratch,
        "shed.md",
        "# Shed\n\nThe trowel hangs by the door.\n",
    );
    let kept = scratch.run(&["remember", &scratch.path("notes")]);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    let found = scratch.run(&["recall", "--", "beet"]);
    assert_eq!(found.code, 0, "{}", found.stderr);
    assert!(
        found.stderr.contains("bilbo: searched 2 entries\n"),
        "{}",
        found.stderr
    );
    let reference = entry_part(&scratch, "beet");

    let run = scratch.run(&["forget", &reference]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, FORGOTTEN);
    gone(&scratch, "beet", "1 entry");
}

#[test]
fn a_forgotten_note_from_standard_input_is_gone_from_recall_right_after() {
    let scratch = Scratch::new("forget_piped");
    write(
        &scratch,
        "shed.md",
        "# Shed\n\nThe trowel hangs by the door.\n",
    );
    let kept = scratch.run(&["remember", &scratch.path("notes")]);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    let note = scratch.run_with_input(&["remember", "-"], b"The radishes go beside the fence.\n");
    assert_eq!(note.code, 0, "{}", note.stderr);
    let id = note
        .stderr
        .lines()
        .find_map(|line| line.strip_prefix("bilbo: kept the note as "))
        .expect("the kept line")
        .to_owned();
    let found = scratch.run(&["recall", "--", "radishes"]);
    assert_eq!(found.code, 0, "{}", found.stderr);
    assert!(
        found.stderr.contains("bilbo: searched 2 entries\n"),
        "{}",
        found.stderr
    );

    let run = scratch.run(&["forget", &id]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, FORGOTTEN);
    gone(&scratch, "radishes", "1 entry");
}

#[test]
fn a_note_renamed_to_a_forgotten_notes_name_is_kept_as_itself() {
    let scratch = Scratch::new("forget_name");
    let first_file = write(&scratch, "decision-beets.md", NOTE);
    let second_file = write(&scratch, "report-shed.md", SECTIONS);
    let set_up = scratch.run(&[
        "remember",
        &scratch.path("notes"),
        "--kind",
        "markdown",
        "--follow",
    ]);
    assert_eq!(set_up.code, 0, "{}", set_up.stderr);
    fs::remove_file(&first_file).expect("removed");
    let withdrawn = scratch.run(&["refresh"]);
    assert_eq!(withdrawn.code, 0, "{}", withdrawn.stderr);
    fs::rename(&second_file, &first_file).expect("renamed");
    old(&first_file);
    let refused = scratch.run(&["refresh"]);
    assert_eq!(refused.code, 1, "{}", refused.stderr);
    assert!(
        refused
            .stderr
            .contains(&format!("bilbo: refused {}", shown(&first_file))),
        "{}",
        refused.stderr
    );

    let run = scratch.run(&["forget", ID_A]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, FORGOTTEN);
    let moved = scratch.run(&["refresh"]);

    assert_eq!(moved.code, 0, "{}", moved.stderr);
    assert!(
        moved.stderr.contains(&format!(
            "bilbo: moved {}: its id {ID_B} was held by file:{}",
            shown(&first_file),
            shown(&second_file)
        )),
        "{}",
        moved.stderr
    );
    let read = scratch.run(&["read", ID_B]);
    assert!(
        read.stdout
            .lines()
            .any(|line| line == format!("label: file:{} (capture)", shown(&first_file))),
        "{}",
        read.stdout
    );
}

#[test]
fn a_forgotten_file_under_a_follow_is_not_kept_again_by_the_next_pass() {
    let scratch = Scratch::new("forget_pass");
    let plain = write(&scratch, "no-frontmatter.md", PLAIN);
    write(&scratch, "sections.md", SECTIONS);
    let set_up = scratch.run(&[
        "remember",
        &scratch.path("notes"),
        "--kind",
        "markdown",
        "--follow",
    ]);
    assert_eq!(set_up.code, 0, "{}", set_up.stderr);
    let reference = entry_part(&scratch, "frontmatter");
    let id = id_of(&scratch, &reference);
    assert_eq!(scratch.run(&["forget", &id]).code, 0);

    let pass = scratch.run(&["refresh"]);

    assert_eq!(pass.code, 0, "{}", pass.stderr);
    assert!(first(&pass).starts_with("new 0, "), "{}", pass.stdout);
    let said: Vec<&str> = pass
        .stderr
        .lines()
        .filter(|line| line.starts_with("bilbo: passed over "))
        .collect();
    assert_eq!(said, [passed_line(&plain, &id)]);
    let again = scratch.run(&["refresh"]);
    assert_eq!(again.code, 0, "{}", again.stderr);
    assert!(
        again
            .stderr
            .lines()
            .all(|line| !line.starts_with("bilbo: passed over ")),
        "{}",
        again.stderr
    );
    gone(&scratch, "frontmatter", "1 entry");

    let back = scratch.run(&["remember", plain.to_str().expect("UTF-8")]);
    assert_eq!(back.code, 0, "{}", back.stderr);
    assert_eq!(first(&back), counts(0, 1, 0, 0, 0));
    assert!(
        back.stderr.contains(&format!(
            "bilbo: kept {} again: its entry {id} was forgotten",
            shown(&plain)
        )),
        "{}",
        back.stderr
    );
    let found = scratch.run(&["recall", "--", "frontmatter"]);
    assert_eq!(found.code, 0, "{}", found.stderr);
}

#[test]
fn forget_with_no_ref_or_with_a_locator_is_a_usage_error() {
    let scratch = Scratch::new("forget_usage");
    let none = scratch.run(&["forget"]);
    assert_eq!(none.code, 2);
    assert_eq!(none.stdout, "");
    assert_eq!(
        none.stderr,
        "bilbo: missing <ref>\nbilbo: usage: bilbo forget <ref>...\nbilbo: see 'bilbo forget --help'\n"
    );
    let locator = scratch.run(&["forget", "2T6W9YB0DE#r3"]);
    assert_eq!(locator.code, 2);
    assert_eq!(locator.stdout, "");
    assert_eq!(
        locator.stderr,
        "bilbo: '2T6W9YB0DE#r3' names records; an entry is forgotten whole\nbilbo: usage: bilbo forget <ref>...\nbilbo: see 'bilbo forget --help'\n"
    );
}
