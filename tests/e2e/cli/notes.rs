use std::fs;
use std::path::Path;
use std::process::Command;

use super::Run;
use super::scratch::Scratch;

fn counts(new: u64, appended: u64, generation: u64, unchanged: u64, refused: u64) -> String {
    format!(
        "new {new}, appended {appended}, new generation {generation}, unchanged {unchanged}, refused {refused}"
    )
}

fn first(run: &Run) -> &str {
    run.stdout.lines().next().unwrap_or_default()
}

fn hits(run: &Run) -> Vec<String> {
    run.stdout
        .lines()
        .filter_map(|line| line.strip_prefix("=== "))
        .filter_map(|header| header.split('\t').next())
        .map(|reference| reference.split('#').next().unwrap_or_default().to_owned())
        .collect()
}

fn entry_of(scratch: &Scratch, word: &str, file: &Path) -> String {
    let found = scratch.run(&["recall", "--", word]);
    assert_eq!(found.code, 0, "{}", found.stderr);
    for reference in hits(&found) {
        let read = scratch.run(&["read", &reference]);
        if read
            .stdout
            .contains(&format!("label: file:{} (capture)", file.display()))
        {
            let id = read.stdout.lines().next().expect("a first line");
            return id.strip_prefix("bilbo:").expect("an id line").to_owned();
        }
    }
    panic!("no hit for {}: {}", file.display(), found.stdout);
}

fn label_of(scratch: &Scratch, id: &str) -> String {
    let read = scratch.run(&["read", id]);
    assert_eq!(read.code, 0, "{}", read.stderr);
    read.stdout
        .lines()
        .find(|line| line.starts_with("label: "))
        .expect("a label line")
        .to_owned()
}

fn capture(file: &Path) -> String {
    format!("label: file:{} (capture)", file.display())
}

fn keeps(scratch: &Scratch, file: &Path) -> Run {
    scratch.run(&["remember", file.to_str().expect("UTF-8")])
}

fn moved_line(new: &Path, id: &str, old: &Path) -> String {
    format!(
        "bilbo: moved {}: its id {id} was held by file:{}",
        new.display(),
        old.display()
    )
}

#[test]
fn a_note_written_and_updated_by_the_skills_commands_is_found_at_once_and_keeps_its_older_text() {
    let scratch = Scratch::new("skill_write");
    let folder = scratch.path("notes");
    let set_up = scratch.run(&["remember", &folder, "--kind", "markdown", "--follow"]);
    assert_eq!(set_up.code, 1);
    assert!(
        set_up.stderr.contains(&format!(
            "nothing to keep: markdown claims no file under {folder}"
        )),
        "{}",
        set_up.stderr
    );
    assert!(
        set_up.stderr.contains("saved the follow in "),
        "{}",
        set_up.stderr
    );
    let follows = scratch.run(&["config", "follow"]);
    assert_eq!(follows.code, 0);
    assert!(
        follows
            .stdout
            .contains(&format!("{{ path = \"{folder}\", kind = \"markdown\" }}")),
        "{}",
        follows.stdout
    );
    let nothing = scratch.run(&["recall", "--", "zebras"]);
    assert_eq!(nothing.code, 1);
    assert!(
        nothing
            .stderr
            .starts_with(&format!("bilbo: no store at {}", scratch.path("store"))),
        "{}",
        nothing.stderr
    );

    let file = scratch.dir.join("notes/decision-zebras.md");
    assert!(!file.exists());
    fs::write(&file, "# Garden\n\nThe zebras graze at dawn.\n").expect("a new note");
    let kept = keeps(&scratch, &file);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    assert_eq!(first(&kept), counts(1, 0, 0, 0, 0));
    let found = scratch.run(&["recall", "--", "zebras"]);
    assert_eq!(found.code, 0, "{}", found.stderr);
    assert_eq!(hits(&found).len(), 1, "{}", found.stdout);
    let read = scratch.run(&["read", &hits(&found)[0]]);
    let mut lines = read.stdout.lines();
    let id = lines
        .next()
        .and_then(|line| line.strip_prefix("bilbo:"))
        .expect("an id line")
        .to_owned();
    assert_eq!(lines.next(), Some("kind: markdown"));
    assert_eq!(lines.next(), Some(capture(&file).as_str()));

    let check = keeps(&scratch, &file);
    assert_eq!(check.code, 0, "{}", check.stderr);
    assert_eq!(first(&check), counts(0, 0, 0, 1, 0));
    assert!(first(&check).ends_with("refused 0"));
    assert_eq!(label_of(&scratch, &id), capture(&file));
    fs::write(&file, "# Garden\n\nThe giraffes graze at dawn.\n").expect("an edit");
    let updated = keeps(&scratch, &file);
    assert_eq!(updated.code, 0, "{}", updated.stderr);
    assert_eq!(first(&updated), counts(0, 0, 1, 0, 0));
    let after = scratch.run(&["read", &id]);
    assert!(after.stdout.contains("generation: 2"), "{}", after.stdout);
    assert_eq!(label_of(&scratch, &id), capture(&file));

    let new_word = scratch.run(&["recall", "--", "giraffes"]);
    assert_eq!(new_word.code, 0, "{}", new_word.stderr);
    let reference = &hits(&new_word)[0];
    assert!(
        scratch
            .run(&["read", reference])
            .stdout
            .starts_with(&format!("bilbo:{id}\n")),
        "the same entry"
    );
    let old_word = scratch.run(&["recall", "--", "zebras"]);
    assert_eq!(old_word.code, 1);
    assert!(hits(&old_word).is_empty());
    let outline = scratch.run(&["read", &format!("{id}#g1")]);
    assert_eq!(outline.code, 0, "{}", outline.stderr);
    let records = scratch.run(&["read", &format!("{id}#g1:r1-3")]);
    assert_eq!(records.code, 0, "{}", records.stderr);
    assert!(
        records.stdout.contains("The zebras graze at dawn."),
        "{}",
        records.stdout
    );
}

#[test]
fn a_note_renamed_and_deleted_by_the_skills_commands_stays_one_entry_and_leaves_recall() {
    let scratch = Scratch::new("skill_rename_delete");
    let old = scratch.dir.join("notes/decision-beds.md");
    let other = scratch.dir.join("notes/report-shed.md");
    fs::write(&old, "# Beds\n\nThe quince tree shades the beds.\n").expect("a note");
    fs::write(&other, "# Shed\n\nThe trowel hangs by the door.\n").expect("a note");
    let folder = scratch.path("notes");
    let set_up = scratch.run(&["remember", &folder, "--kind", "markdown", "--follow"]);
    assert_eq!(set_up.code, 0, "{}", set_up.stderr);
    assert_eq!(first(&set_up), counts(2, 0, 0, 0, 0));

    let id = entry_of(&scratch, "quince", &old);
    let check = keeps(&scratch, &old);
    assert_eq!(check.code, 0, "{}", check.stderr);
    assert_eq!(first(&check), counts(0, 0, 0, 1, 0));
    assert_eq!(label_of(&scratch, &id), capture(&old));
    let new = scratch.dir.join("notes/decision-beds-and-trees.md");
    assert!(!new.exists());
    let body = fs::read_to_string(&old).expect("the note");
    fs::write(&old, format!("---\nid: {id}\n---\n\n{body}")).expect("the id written");
    let mv = Command::new("mv")
        .args(["-n"])
        .arg(&old)
        .arg(&new)
        .status()
        .expect("mv runs");
    assert!(mv.success());
    assert!(!old.exists() && new.is_file());

    let kept = keeps(&scratch, &new);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    assert_eq!(kept.stderr, format!("{}\n", moved_line(&new, &id, &old)));
    assert_eq!(first(&kept), counts(0, 0, 1, 0, 0));
    assert_eq!(label_of(&scratch, &id), capture(&new));
    let found = scratch.run(&["recall", "--", "quince"]);
    assert_eq!(found.code, 0, "{}", found.stderr);
    assert!(!found.stderr.contains("passed over"), "{}", found.stderr);
    assert_eq!(hits(&found).len(), 1);
    assert!(
        scratch
            .run(&["read", &hits(&found)[0]])
            .stdout
            .starts_with(&format!("bilbo:{id}\n"))
    );
    let records: u64 = kept
        .stdout
        .lines()
        .nth(1)
        .and_then(|line| line.strip_prefix("records "))
        .and_then(|rest| rest.split(',').next())
        .and_then(|count| count.parse().ok())
        .expect("a records count");

    let check = keeps(&scratch, &new);
    assert_eq!(check.code, 0, "{}", check.stderr);
    assert_eq!(first(&check), counts(0, 0, 0, 1, 0));
    assert_eq!(label_of(&scratch, &id), capture(&new));
    fs::remove_file(&new).expect("the note is removed");
    let pass = scratch.run(&["refresh"]);
    assert_eq!(pass.code, 0, "{}", pass.stderr);
    assert_eq!(first(&pass), counts(0, 0, 1, 1, 0));
    assert!(
        pass.stderr.contains(&format!(
            "bilbo: withdrew {}: its file is gone; bilbo read {id}#g2 prints the outline of what it held",
            new.display()
        )),
        "{}",
        pass.stderr
    );
    let gone = scratch.run(&["recall", "--", "quince"]);
    assert_eq!(gone.code, 1);
    assert!(
        gone.stderr.ends_with("bilbo: nothing matched\n"),
        "{}",
        gone.stderr
    );
    let read = scratch.run(&["read", &id]);
    assert!(read.stdout.contains("records: 0"), "{}", read.stdout);
    assert!(read.stdout.contains("generation: 3"), "{}", read.stdout);
    let old_text = scratch.run(&["read", &format!("{id}#g2:r1-{records}")]);
    assert_eq!(old_text.code, 0, "{}", old_text.stderr);
    assert!(
        old_text.stdout.contains("The quince tree shades the beds."),
        "{}",
        old_text.stdout
    );
}

#[test]
fn a_note_written_on_the_name_of_a_deleted_note_is_that_note_again() {
    let scratch = Scratch::new("skill_name_again");
    let gone = scratch.dir.join("notes/decision-beets.md");
    let other = scratch.dir.join("notes/report-shed.md");
    fs::write(&gone, "# Beets\n\nThe beet seeds are in the drawer.\n").expect("a note");
    fs::write(&other, "# Shed\n\nThe trowel hangs by the door.\n").expect("a note");
    let folder = scratch.path("notes");
    let set_up = scratch.run(&["remember", &folder, "--kind", "markdown", "--follow"]);
    assert_eq!(set_up.code, 0, "{}", set_up.stderr);
    let id = entry_of(&scratch, "beet", &gone);
    fs::remove_file(&gone).expect("the note is removed");
    let pass = scratch.run(&["refresh"]);
    assert_eq!(pass.code, 0, "{}", pass.stderr);
    assert_eq!(first(&pass), counts(0, 0, 1, 1, 0));

    assert!(!gone.exists());
    fs::write(&gone, "# Beets\n\nThe radish rows run along the fence.\n").expect("a new note");
    let kept = keeps(&scratch, &gone);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    assert!(
        first(&kept).starts_with("new 0, appended 1, "),
        "{}",
        first(&kept)
    );
    assert_eq!(entry_of(&scratch, "radish", &gone), id);
    let old = scratch.run(&["read", &format!("{id}#g1:r1-3")]);
    assert_eq!(old.code, 0, "{}", old.stderr);
    assert!(
        old.stdout.contains("The beet seeds are in the drawer."),
        "{}",
        old.stdout
    );
}
