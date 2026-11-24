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

fn has_line(text: &str, line: &str) -> bool {
    text.lines().any(|found| found == line)
}

fn entry_of(scratch: &Scratch, word: &str, file: &Path) -> String {
    let found = scratch.run(&["recall", "--", word]);
    assert_eq!(found.code, 0, "{}", found.stderr);
    for reference in hits(&found) {
        let read = scratch.run(&["read", &reference]);
        assert_eq!(read.code, 0, "{}", read.stderr);
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
    assert_eq!(read.code, 0, "{}", read.stderr);
    assert!(has_line(&read.stdout, "generation: 1"), "{}", read.stdout);
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
    assert_eq!(after.code, 0, "{}", after.stderr);
    assert!(has_line(&after.stdout, "generation: 2"), "{}", after.stdout);
    assert_eq!(label_of(&scratch, &id), capture(&file));

    let new_word = scratch.run(&["recall", "--", "giraffes"]);
    assert_eq!(new_word.code, 0, "{}", new_word.stderr);
    let reference = &hits(&new_word)[0];
    let same = scratch.run(&["read", reference]);
    assert_eq!(same.code, 0, "{}", same.stderr);
    assert!(
        same.stdout.starts_with(&format!("bilbo:{id}\n")),
        "the same entry"
    );
    let old_word = scratch.run(&["recall", "--", "zebras"]);
    assert_eq!(old_word.code, 1);
    assert!(hits(&old_word).is_empty());
    assert_eq!(
        old_word.stderr.lines().last(),
        Some("bilbo: nothing matched"),
        "{}",
        old_word.stderr
    );
    let outline = scratch.run(&["read", &format!("{id}#g1")]);
    assert_eq!(outline.code, 0, "{}", outline.stderr);
    assert!(
        has_line(&outline.stdout, "generation: 1 of 2"),
        "{}",
        outline.stdout
    );
    assert!(
        outline
            .stdout
            .lines()
            .any(|line| line.starts_with("#g1:r") && line.ends_with(" Garden")),
        "{}",
        outline.stdout
    );
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
    let moved_read = scratch.run(&["read", &id]);
    assert_eq!(moved_read.code, 0, "{}", moved_read.stderr);
    assert!(
        has_line(&moved_read.stdout, &capture(&new)),
        "{}",
        moved_read.stdout
    );
    assert!(
        has_line(&moved_read.stdout, "generation: 2"),
        "{}",
        moved_read.stdout
    );
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
    let withdrew = format!(
        "bilbo: withdrew {}: its file is gone; bilbo read {id}#g2 prints the outline of what it held",
        new.display()
    );
    assert_eq!(
        pass.stderr,
        format!(
            "bilbo: skipped {home}/.claude/projects: no such file or folder\nbilbo: skipped {home}/.codex/sessions: no such file or folder\n{withdrew}\n",
            home = scratch.path("home")
        )
    );
    let read = scratch.run(&["read", &id]);
    assert_eq!(read.code, 0, "{}", read.stderr);
    assert!(has_line(&read.stdout, "records: 0"), "{}", read.stdout);
    assert!(has_line(&read.stdout, "generation: 3"), "{}", read.stdout);
    let gone = scratch.run(&["recall", "--", "quince"]);
    assert_eq!(gone.code, 1);
    assert!(
        gone.stderr.ends_with("bilbo: nothing matched\n"),
        "{}",
        gone.stderr
    );
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
    assert_eq!(first(&kept), counts(0, 1, 0, 0, 0));
    assert_eq!(entry_of(&scratch, "radish", &gone), id);
    let back = scratch.run(&["read", &id]);
    assert_eq!(back.code, 0, "{}", back.stderr);
    assert!(has_line(&back.stdout, "generation: 2"), "{}", back.stdout);
    assert!(has_line(&back.stdout, "records: 3"), "{}", back.stdout);
    let old = scratch.run(&["read", &format!("{id}#g1:r1-3")]);
    assert_eq!(old.code, 0, "{}", old.stderr);
    assert!(
        old.stdout.contains("The beet seeds are in the drawer."),
        "{}",
        old.stdout
    );
}

const TWO_LINES: &[u8] =
    b"\n\nThe beet seeds are in the drawer.\nThe radishes go beside the fence.\n\n";

fn kept_note(run: &Run) -> String {
    run.stderr
        .lines()
        .find_map(|line| line.strip_prefix("bilbo: kept the note as "))
        .expect("the kept line")
        .to_owned()
}

#[test]
fn a_note_piped_to_remember_is_found_by_recall_and_read_by_its_id() {
    let scratch = Scratch::new("piped_note");
    let kept = scratch.run_with_input(&["remember", "-"], TWO_LINES);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    assert_eq!(first(&kept), counts(1, 0, 0, 0, 0));
    let id = kept_note(&kept);

    let found = scratch.run(&["recall", "--", "radishes"]);
    assert_eq!(found.code, 0, "{}", found.stderr);
    assert_eq!(hits(&found).len(), 1, "{}", found.stdout);
    assert_eq!(hits(&found)[0], id[id.len() - 10..], "{}", found.stdout);
    let read = scratch.run(&["read", &id]);
    assert_eq!(read.code, 0, "{}", read.stderr);
    assert!(read.stdout.contains("kind: note\n"), "{}", read.stdout);
    assert!(
        read.stdout
            .contains(&format!("label: note:{id} (capture)\n")),
        "{}",
        read.stdout
    );
    let record = scratch.run(&["read", &format!("{id}#r1")]);
    assert_eq!(record.code, 0, "{}", record.stderr);
    let lines: Vec<&str> = record.stdout.lines().collect();
    assert!(
        lines.first().is_some_and(|line| line.starts_with("#r1 ")),
        "{}",
        record.stdout
    );
    assert_eq!(
        lines.last(),
        Some(&"end: #r1 of 1 record"),
        "{}",
        record.stdout
    );
    assert_eq!(
        lines[1..lines.len() - 1],
        [
            "The beet seeds are in the drawer.",
            "The radishes go beside the fence."
        ],
        "{}",
        record.stdout
    );
}

#[test]
fn derive_leaves_a_written_note_alone() {
    let scratch = Scratch::new("piped_derive");
    scratch.config(&super::scratch::closed(
        "derive.enabled = true\nderive.in = [\"projects\"]\nllm.provider = \"anthropic\"\nllm.anthropic.model = \"claude-test-1\"\n",
    ));
    let kept = scratch.run_with_input(&["remember", "-"], TWO_LINES);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    let id = kept_note(&kept);
    let dry = scratch.run_with(
        &["derive", "--dry-run", &id],
        &[("ANTHROPIC_API_KEY", "sk-test-PLUM-7391")],
    );
    assert_eq!(dry.code, 0, "{}{}", dry.stdout, dry.stderr);
    let entries: Vec<Vec<&str>> = dry
        .stdout
        .lines()
        .filter(|line| line.starts_with("entry\t"))
        .map(|line| line.split('\t').collect())
        .collect();
    assert_eq!(
        entries,
        [vec![
            "entry",
            id.as_str(),
            "left out: no instruction",
            "-",
            "0"
        ]],
        "{}",
        dry.stdout
    );
}

#[test]
fn a_note_the_store_would_change_is_refused_through_the_binary() {
    let scratch = Scratch::new("piped_opaque");
    let refused = scratch.run_with_input(&["remember", "-"], "z".repeat(256).as_bytes());
    assert_eq!(refused.code, 1, "{}", refused.stderr);
    assert_eq!(
        refused.stdout,
        format!(
            "{}\nrecords 0, raw bytes 0, stored bytes 0\n",
            counts(0, 0, 0, 0, 1)
        )
    );
    assert_eq!(
        refused.stderr,
        "bilbo: refused the note: the store would keep only a placeholder for its text, which it takes for encoded data\n"
    );
    let kept = scratch.run_with_input(&["remember", "-"], b"The beet seeds are in the drawer.\n");
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    let id = kept_note(&kept);
    let read = scratch.run(&["read", &format!("{id}#r1")]);
    assert_eq!(read.code, 0, "{}", read.stderr);
    assert!(
        read.stdout.contains("The beet seeds are in the drawer.\n"),
        "{}",
        read.stdout
    );
}

#[test]
fn a_lone_dash_is_standard_input_only_as_it_was_typed() {
    for (row, args) in [
        (1, &["remember", "--", "-"][..]),
        (2, &["remember", "--", "-/"]),
        (3, &["remember", "--", "-/."]),
        (4, &["remember", "./-"]),
    ] {
        let scratch = Scratch::new("lone_dash");
        let work = scratch.dir.join("work");
        fs::create_dir_all(work.join("-")).expect("a folder named -");
        fs::write(
            work.join("-/garden.md"),
            "# Garden\n\nThe beds face south.\n",
        )
        .expect("a note");
        let run = scratch.run_with_input_in(args, b"draft\n", &work);
        assert_eq!(run.code, 0, "row {row}: {}", run.stderr);
        assert_eq!(first(&run), counts(1, 0, 0, 0, 0), "row {row}");
        let beds = scratch.run(&["recall", "--", "beds"]);
        let draft = scratch.run(&["recall", "--", "draft"]);
        if row == 1 {
            let id = kept_note(&run);
            let read = scratch.run(&["read", &id]);
            assert!(read.stdout.contains("kind: note\n"), "{}", read.stdout);
            assert_eq!(beds.code, 1, "{}", beds.stdout);
            assert!(
                beds.stderr.ends_with("bilbo: nothing matched\n"),
                "{}",
                beds.stderr
            );
        } else {
            assert_eq!(run.stderr, "", "row {row}");
            assert_eq!(beds.code, 0, "row {row}: {}", beds.stderr);
            assert_eq!(hits(&beds).len(), 1, "row {row}: {}", beds.stdout);
            let read = scratch.run(&["read", &hits(&beds)[0]]);
            assert!(
                read.stdout.contains("kind: markdown\n"),
                "row {row}: {}",
                read.stdout
            );
            assert!(
                read.stdout.contains(&format!(
                    "label: file:{}/work/-/garden.md (capture)\n",
                    scratch.dir.display()
                )),
                "row {row}: {}",
                read.stdout
            );
            assert_eq!(draft.code, 1, "row {row}: {}", draft.stdout);
            assert!(
                draft.stderr.ends_with("bilbo: nothing matched\n"),
                "row {row}: {}",
                draft.stderr
            );
        }
    }
}
