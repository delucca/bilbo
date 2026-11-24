use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::Run;
use super::recall::hit_refs;
use super::scratch::Scratch;
use super::sessions::{counts, first};

fn hits(run: &Run) -> Vec<String> {
    hit_refs(&run.stdout)
        .into_iter()
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
        let run = if row == 1 {
            scratch.run_with_input_in(args, b"draft\n", &work)
        } else {
            scratch.run_leaving_input_unread_in(args, b"draft\n", &work)
        };
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

fn shell_word(path: &Path) -> String {
    format!("'{}'", path.to_str().expect("UTF-8").replace('\'', "'\\''"))
}

struct Editor {
    script: PathBuf,
    tmp: PathBuf,
    noted: PathBuf,
}

impl Editor {
    fn new(scratch: &Scratch, body: &str) -> Editor {
        let tmp = scratch.dir.join("tmp");
        let noted = scratch.dir.join("noted");
        fs::create_dir_all(&tmp).expect("a temp folder");
        fs::create_dir_all(&noted).expect("a folder for what the script notes");
        let folder = scratch.dir.join("script folder");
        fs::create_dir_all(&folder).expect("a folder with a space");
        let script = folder.join("editor.sh");
        fs::write(&script, format!("#!/bin/sh\n{body}\n")).expect("a script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("a mode");
        Editor { script, tmp, noted }
    }

    fn visual(&self) -> String {
        shell_word(&self.script)
    }

    fn drafts(&self) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = fs::read_dir(&self.tmp)
            .expect("the temp folder")
            .map(|entry| entry.expect("an entry").path())
            .collect();
        found.sort();
        found
    }

    fn noted(&self, name: &str) -> String {
        fs::read_to_string(self.noted.join(name)).expect("what the script noted")
    }

    fn run(&self, scratch: &Scratch, visual: &str, args: &[&str]) -> Run {
        let binary = fs::canonicalize(super::binary()).expect("the binary");
        scratch.run_with(
            &[&["remember", "--edit"][..], args].concat(),
            &[
                ("VISUAL", visual),
                ("TMPDIR", self.tmp.to_str().expect("UTF-8")),
                ("NOTED", self.noted.to_str().expect("UTF-8")),
                ("BILBO_BIN", binary.to_str().expect("UTF-8")),
                ("PATH", "/usr/bin:/bin"),
            ],
        )
    }
}

fn the_record_is_the_sentence(scratch: &Scratch, id: &str) {
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
        ["The quince blooms early."],
        "{}",
        record.stdout
    );
}

fn a_draft_folders_name(folder: &Path) {
    let name = folder
        .file_name()
        .expect("a name")
        .to_string_lossy()
        .into_owned();
    let id = name
        .strip_prefix(".bilbo-note-")
        .expect("the folder's name");
    assert_eq!(id.len(), 26, "{name}");
    assert!(
        id.chars()
            .all(|c| "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(c)),
        "{name}"
    );
    assert!("01234567".contains(&id[..1]), "{name}");
}

fn no_root(scratch: &Scratch) {
    for folder in ["store", "state", "cache"] {
        assert!(!scratch.dir.join(folder).exists(), "{folder}");
    }
}

const NOTHING: &str = "new 0, appended 0, new generation 0, unchanged 0, refused 0\nrecords 0, raw bytes 0, stored bytes 0\n";

fn last_line(run: &Run) -> &str {
    run.stderr.lines().last().unwrap_or_default()
}

fn follow_the_temp_folder(scratch: &Scratch, editor: &Editor) {
    let set_up = scratch.run(&[
        "remember",
        editor.tmp.to_str().expect("UTF-8"),
        "--kind",
        "markdown",
        "--follow",
    ]);
    assert!(
        set_up.stderr.contains("saved the follow in "),
        "{}",
        set_up.stderr
    );
}

#[test]
fn a_note_written_in_the_editor_is_kept_and_its_file_is_removed() {
    let scratch = Scratch::new("edit_kept");
    let editor = Editor::new(
        &scratch,
        r#"printf '%s\n' "$1" > "$NOTED/argument"
/bin/ls -ld "$1" > "$NOTED/file"
/bin/ls -ld "$(dirname "$1")" > "$NOTED/folder"
printf 'The quince blooms early.\n' > "$1""#,
    );
    let run = editor.run(&scratch, &editor.visual(), &["--tag", "garden"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(first(&run), counts(1, 0, 0, 0, 0));
    let id = kept_note(&run);
    assert!(
        run.stderr.contains("bilbo: tagged 1 entries\n"),
        "{}",
        run.stderr
    );
    assert!(
        run.stderr.contains("bilbo: tag:garden is new\n"),
        "{}",
        run.stderr
    );

    let argument = editor.noted("argument");
    assert_eq!(
        argument.trim_end(),
        editor
            .tmp
            .join(format!(".bilbo-note-{id}"))
            .join("note.md")
            .to_str()
            .expect("UTF-8")
    );
    let file = editor.noted("file");
    let fields: Vec<&str> = file.split_whitespace().collect();
    assert!(fields[0].starts_with("-rw-------"), "{file}");
    assert_eq!(fields[4], "0", "{file}");
    assert!(editor.noted("folder").starts_with("drwx------"));
    assert!(editor.drafts().is_empty(), "{:?}", editor.drafts());

    let found = scratch.run(&["recall", "--", "quince"]);
    assert_eq!(found.code, 0, "{}", found.stderr);
    assert_eq!(hits(&found), [id[id.len() - 10..].to_owned()]);
    the_record_is_the_sentence(&scratch, &id);
}

#[test]
fn an_editor_that_saves_nothing_keeps_nothing() {
    for body in ["exit 0", r#"printf '   \n\n  \t\n' > "$1""#] {
        let scratch = Scratch::new("edit_empty");
        let editor = Editor::new(&scratch, body);
        let run = editor.run(&scratch, &editor.visual(), &[]);
        assert_eq!(run.code, 1, "{body}: {}", run.stderr);
        assert_eq!(run.stdout, NOTHING, "{body}");
        assert_eq!(
            run.stderr, "bilbo: nothing to keep: the note is empty\n",
            "{body}"
        );
        assert!(editor.drafts().is_empty(), "{body}");
        no_root(&scratch);
    }
}

#[test]
fn an_editor_that_fails_keeps_nothing_whatever_it_saved() {
    let scratch = Scratch::new("edit_failed");
    let editor = Editor::new(&scratch, "printf 'A sentence.\\n' > \"$1\"\nexit 3");
    let run = editor.run(&scratch, &editor.visual(), &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let drafts = editor.drafts();
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    let file = drafts[0].join("note.md");
    assert_eq!(
        last_line(&run),
        format!(
            "bilbo: the editor exited with 3; nothing was kept; what was saved stays in {}",
            file.display()
        )
    );
    assert_eq!(
        fs::read_to_string(&file).expect("the draft"),
        "A sentence.\n"
    );
    no_root(&scratch);

    let scratch = Scratch::new("edit_missing");
    let editor = Editor::new(&scratch, "exit 0");
    let run = editor.run(&scratch, "bilbo-no-such-editor", &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        last_line(&run),
        "bilbo: the editor exited with 127; nothing was kept"
    );
    assert!(editor.drafts().is_empty());
    no_root(&scratch);
}

#[test]
fn an_editors_value_with_arguments_runs_as_a_command_line() {
    let scratch = Scratch::new("edit_arguments");
    let editor = Editor::new(
        &scratch,
        r#"printf '%s\n' "$#" "$1" "$2" > "$NOTED/arguments"
printf 'The shed door sticks.\n' > "$2""#,
    );
    let visual = format!("{} word", editor.visual());
    let run = scratch.run_with(
        &["remember", "--edit"],
        &[
            ("VISUAL", &visual),
            ("EDITOR", "false"),
            ("TMPDIR", editor.tmp.to_str().expect("UTF-8")),
            ("NOTED", editor.noted.to_str().expect("UTF-8")),
            ("PATH", "/usr/bin:/bin"),
        ],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let arguments = editor.noted("arguments");
    let lines: Vec<&str> = arguments.lines().collect();
    assert_eq!(lines[0], "2");
    assert_eq!(lines[1], "word");
    assert!(lines[2].ends_with("/note.md"), "{arguments}");
}

#[test]
fn no_bilbo_waits_while_the_editor_is_open() {
    let scratch = Scratch::new("edit_no_wait");
    let editor = Editor::new(
        &scratch,
        r#"printf 'The quince blooms early.\n' > "$1"
"$BILBO_BIN" refresh > "$NOTED/out" 2> "$NOTED/err"
printf '%s\n' "$?" > "$NOTED/code""#,
    );
    follow_the_temp_folder(&scratch, &editor);
    let run = editor.run(&scratch, &editor.visual(), &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(editor.noted("code").trim(), "0", "{}", editor.noted("err"));
    let err = editor.noted("err");
    assert!(!err.contains("waiting"), "{err}");
    assert!(!err.contains("another bilbo is writing"), "{err}");
    assert_eq!(editor.noted("out"), NOTHING);
    let id = kept_note(&run);
    let found = scratch.run(&["recall", "--", "quince"]);
    assert_eq!(hits(&found), [id[id.len() - 10..].to_owned()]);
    assert_eq!(found.code, 0, "{}", found.stderr);
    assert!(
        has_line(&found.stderr, "bilbo: searched 1 entry"),
        "{}",
        found.stderr
    );
    let read = scratch.run(&["read", &id]);
    assert!(read.stdout.contains("kind: note\n"), "{}", read.stdout);
    let reindexed = scratch.run(&["reindex"]);
    assert_eq!(reindexed.code, 0, "{}", reindexed.stderr);
    let counted: Vec<&str> = first(&reindexed).split(", ").collect();
    assert_eq!(
        (counted[0], counted[2], counted[3]),
        ("entries 1", "unread 0", "failed 0"),
        "{}",
        reindexed.stdout
    );
}

#[test]
fn an_editor_that_saves_by_replacing_the_file_is_read() {
    let scratch = Scratch::new("edit_replaced");
    let editor = Editor::new(
        &scratch,
        r#"D="$(dirname "$1")"
: > "$D/note.md~"
printf 'The quince blooms early.\n' > "$D/new"
chmod 644 "$D/new"
mv "$D/new" "$1""#,
    );
    let run = editor.run(&scratch, &editor.visual(), &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let id = kept_note(&run);
    let found = scratch.run(&["recall", "--", "quince"]);
    assert_eq!(hits(&found), [id[id.len() - 10..].to_owned()]);
    the_record_is_the_sentence(&scratch, &id);
    assert!(editor.drafts().is_empty(), "{:?}", editor.drafts());
}

#[test]
fn a_scratch_file_swapped_for_a_link_is_no_note() {
    let scratch = Scratch::new("edit_link");
    let editor = Editor::new(
        &scratch,
        r#"printf 'The quince blooms early.\n' > "$NOTED/other.txt"
rm "$1"
ln -s "$NOTED/other.txt" "$1""#,
    );
    let run = editor.run(&scratch, &editor.visual(), &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let drafts = editor.drafts();
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    let file = drafts[0].join("note.md");
    assert!(
        last_line(&run).starts_with(&format!(
            "bilbo: cannot read the note's file {}: ",
            file.display()
        )),
        "{}",
        run.stderr
    );
    assert!(file.is_symlink());
    no_root(&scratch);
}

#[test]
fn text_the_run_does_not_keep_stays_in_its_file() {
    let scratch = Scratch::new("edit_not_utf8");
    let editor = Editor::new(&scratch, r#"printf '\377\376' > "$1""#);
    let run = editor.run(&scratch, &editor.visual(), &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let drafts = editor.drafts();
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    let file = drafts[0].join("note.md");
    assert_eq!(
        last_line(&run),
        format!(
            "bilbo: refused the note: it is not UTF-8; what was saved stays in {}",
            file.display()
        )
    );
    assert_eq!(fs::read(&file).expect("the draft"), [0xff, 0xfe]);

    let scratch = Scratch::new("edit_opaque");
    let body = format!("printf '%s' '{}' > \"$1\"", "z".repeat(256));
    let editor = Editor::new(&scratch, &body);
    let run = editor.run(&scratch, &editor.visual(), &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(first(&run), counts(0, 0, 0, 0, 1));
    let drafts = editor.drafts();
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    let file = drafts[0].join("note.md");
    assert_eq!(
        last_line(&run),
        format!(
            "bilbo: refused the note: the store would keep only a placeholder for its text, which it takes for encoded data; what was saved stays in {}",
            file.display()
        )
    );
    assert_eq!(
        fs::read_to_string(&file).expect("the draft"),
        "z".repeat(256)
    );
}

#[test]
fn a_path_with_a_space_and_a_quoted_editor_are_one_argument_and_one_program() {
    let scratch = Scratch::new("edit_space");
    let editor = Editor::new(&scratch, "exit 0");
    let tmp = scratch.dir.join("temp folder");
    let tools = scratch.dir.join("tool box");
    fs::create_dir_all(&tmp).expect("a temp folder with a space");
    fs::create_dir_all(&tools).expect("a folder with a space");
    let script = tools.join("editor.sh");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$#\" \"$1\" \"$2\" > \"$NOTED/arguments\"\nprintf 'The quince blooms early.\\n' > \"$2\"\n",
    )
    .expect("a script");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("a mode");
    let visual = format!("{} word", shell_word(&script));
    let run = scratch.run_with(
        &["remember", "--edit"],
        &[
            ("VISUAL", &visual),
            ("TMPDIR", tmp.to_str().expect("UTF-8")),
            ("NOTED", editor.noted.to_str().expect("UTF-8")),
            ("PATH", "/usr/bin:/bin"),
        ],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let id = kept_note(&run);
    let arguments = editor.noted("arguments");
    let lines: Vec<&str> = arguments.lines().collect();
    assert_eq!(lines[0], "2");
    assert_eq!(lines[1], "word");
    assert_eq!(
        lines[2],
        tmp.join(format!(".bilbo-note-{id}"))
            .join("note.md")
            .to_str()
            .expect("UTF-8")
    );
}

#[test]
fn an_editor_ended_by_a_signal_keeps_nothing() {
    let scratch = Scratch::new("edit_signal");
    let editor = Editor::new(&scratch, "printf 'A sentence.\\n' > \"$1\"\nkill -9 $$");
    let run = editor.run(&scratch, &editor.visual(), &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let drafts = editor.drafts();
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    let file = drafts[0].join("note.md");
    let ending = format!(
        "; nothing was kept; what was saved stays in {}",
        file.display()
    );
    let by_signal = format!("bilbo: the editor was stopped by a signal{ending}");
    let by_code = format!("bilbo: the editor exited with 137{ending}");
    assert!(
        last_line(&run) == by_signal || last_line(&run) == by_code,
        "{}",
        run.stderr
    );
    assert_eq!(
        fs::read_to_string(&file).expect("the draft"),
        "A sentence.\n"
    );
    no_root(&scratch);
}

#[test]
fn a_draft_an_editor_failed_on_is_never_an_entry() {
    let scratch = Scratch::new("edit_follow");
    let editor = Editor::new(
        &scratch,
        "printf 'The quince blooms early.\\n' > \"$1\"\nexit 3",
    );
    follow_the_temp_folder(&scratch, &editor);
    let run = editor.run(&scratch, &editor.visual(), &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let drafts = editor.drafts();
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    assert_eq!(
        last_line(&run),
        format!(
            "bilbo: the editor exited with 3; nothing was kept; what was saved stays in {}",
            drafts[0].join("note.md").display()
        )
    );
    let pass = scratch.run(&["refresh"]);
    assert_eq!(pass.code, 0, "{}", pass.stderr);
    assert_eq!(pass.stdout, NOTHING);
    let found = scratch.run(&["recall", "--", "quince"]);
    assert_eq!(found.code, 1, "{}", found.stderr);
    assert!(hits(&found).is_empty());
}

#[test]
fn a_draft_folder_made_with_another_mode_is_removed_again() {
    let scratch = Scratch::new("edit_umask");
    let editor = Editor::new(&scratch, "printf 'ran\\n' > \"$NOTED/ran\"");
    let binary = fs::canonicalize(super::binary()).expect("the binary");
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "umask 0200; exec \"$BILBO_BIN\" remember --edit"])
        .env_clear();
    for (name, value) in scratch.env() {
        command.env(name, value);
    }
    let output = command
        .env("EDITOR", editor.visual())
        .env("TMPDIR", &editor.tmp)
        .env("NOTED", &editor.noted)
        .env("BILBO_BIN", &binary)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("bilbo runs");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8");
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    let line = stderr.lines().last().expect("a line");
    let folder = line
        .strip_prefix("bilbo: cannot make the note's folder ")
        .and_then(|rest| rest.strip_suffix(" is not a private folder of this user"))
        .and_then(|rest| rest.split_once(": "))
        .map(|(folder, same)| {
            assert_eq!(folder, same);
            folder.to_owned()
        })
        .expect("the line of 5.1");
    assert_eq!(Path::new(&folder).parent(), Some(editor.tmp.as_path()));
    a_draft_folders_name(Path::new(&folder));
    assert!(!Path::new(&folder).exists());
    assert!(editor.drafts().is_empty());
    assert!(!editor.noted.join("ran").exists());
    no_root(&scratch);
}

fn run_within(
    scratch: &Scratch,
    editor: &Editor,
    temp: &str,
    folder: &Path,
    deadline: std::time::Duration,
) -> Run {
    scratch.run_within(
        &["remember", "--edit"],
        &[
            ("EDITOR", &editor.visual()),
            ("TMPDIR", temp),
            ("NOTED", editor.noted.to_str().expect("UTF-8")),
            ("PATH", "/usr/bin:/bin"),
        ],
        Some(folder),
        deadline,
    )
}

fn the_folder_of(run: &Run, under: &Path, error: i32) -> String {
    let line = run.stderr.strip_suffix('\n').expect("one line");
    assert_eq!(line.lines().count(), 1, "{}", run.stderr);
    let rest = line
        .strip_prefix("bilbo: cannot make the note's folder ")
        .expect("row 1's line");
    let ending = format!(": {}", std::io::Error::from_raw_os_error(error));
    let folder = rest.strip_suffix(&ending).expect("the system's error");
    let path = Path::new(folder);
    assert_eq!(path.parent(), Some(under), "{folder}");
    a_draft_folders_name(path);
    folder.to_owned()
}

#[test]
fn a_temp_folder_that_is_no_folder_or_cannot_be_found_is_said_at_once_by_its_whole_path() {
    let deadline = std::time::Duration::from_secs(20);
    let scratch = Scratch::new("edit_temp_fifo");
    let editor = Editor::new(
        &scratch,
        "printf '%s\\n' \"$1\" > \"$NOTED/argument\"\nprintf 'The quince blooms early.\\n' > \"$1\"",
    );
    let pipe = scratch.dir.join("pipe");
    let made = Command::new("/usr/bin/mkfifo")
        .arg(&pipe)
        .status()
        .expect("mkfifo runs");
    assert!(made.success());
    let run = run_within(
        &scratch,
        &editor,
        pipe.to_str().expect("UTF-8"),
        &scratch.dir,
        deadline,
    );
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(run.stdout, "");
    the_folder_of(&run, &pipe, 20);
    assert!(!editor.noted.join("argument").exists());
    assert!(std::os::unix::fs::FileTypeExt::is_fifo(
        &fs::symlink_metadata(&pipe)
            .expect("still there")
            .file_type()
    ));
    no_root(&scratch);

    let scratch = Scratch::new("edit_temp_missing");
    let editor = Editor::new(
        &scratch,
        "printf '%s\\n' \"$1\" > \"$NOTED/argument\"\nprintf 'The quince blooms early.\\n' > \"$1\"",
    );
    let work = scratch.dir.join("work");
    fs::create_dir_all(&work).expect("a working folder");
    let run = run_within(&scratch, &editor, "missing", &work, deadline);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(run.stdout, "");
    the_folder_of(&run, &work.join("missing"), 2);
    assert!(!editor.noted.join("argument").exists());
    assert_eq!(fs::read_dir(&work).expect("work").count(), 0);
    no_root(&scratch);

    let scratch = Scratch::new("edit_temp_link");
    let editor = Editor::new(
        &scratch,
        "printf '%s\\n' \"$1\" > \"$NOTED/argument\"\nprintf 'The quince blooms early.\\n' > \"$1\"",
    );
    let work = scratch.dir.join("work");
    fs::create_dir_all(work.join("tmp")).expect("a temp folder");
    std::os::unix::fs::symlink("tmp", work.join("link")).expect("a link");
    let run = run_within(&scratch, &editor, "link", &work, deadline);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let id = kept_note(&run);
    assert_eq!(
        editor.noted("argument").trim_end(),
        work.join("tmp")
            .join(format!(".bilbo-note-{id}"))
            .join("note.md")
            .to_str()
            .expect("UTF-8")
    );
    assert_eq!(fs::read_dir(work.join("tmp")).expect("tmp").count(), 0);
}

#[test]
fn a_store_that_fails_before_the_note_is_kept_says_where_the_draft_is() {
    for (row, code, expected) in [
        (1, 2, "belongs to bilbo 0.20"),
        (2, 1, "can be written by others"),
    ] {
        let scratch = Scratch::new("edit_store_fails");
        fs::write(scratch.dir.join("notes/a.md"), "# Garden\n").expect("a note");
        if row == 1 {
            fs::create_dir_all(scratch.dir.join("store/notes")).expect("a 0.20 folder");
        } else {
            let state = scratch.dir.join("state/bilbo");
            fs::create_dir_all(&state).expect("a state folder");
            fs::set_permissions(&state, fs::Permissions::from_mode(0o777)).expect("a mode");
        }
        let target = scratch.run(&["remember", &scratch.path("notes")]);
        assert_eq!(target.code, code, "row {row}: {}", target.stderr);
        let line = target.stderr.strip_suffix('\n').expect("one line");
        assert_eq!(line.lines().count(), 1, "row {row}: {}", target.stderr);
        assert!(line.contains(expected), "row {row}: {line}");

        let editor = Editor::new(&scratch, r#"printf 'The quince blooms early.\n' > "$1""#);
        let run = editor.run(&scratch, &editor.visual(), &[]);
        assert_eq!(run.code, code, "row {row}: {}", run.stderr);
        let drafts = editor.drafts();
        assert_eq!(drafts.len(), 1, "row {row}: {drafts:?}");
        let file = drafts[0].join("note.md");
        assert_eq!(
            last_line(&run),
            format!("{line}; what was saved stays in {}", file.display()),
            "row {row}"
        );
        assert_eq!(
            fs::read_to_string(&file).expect("the draft"),
            "The quince blooms early.\n",
            "row {row}"
        );
    }
}
