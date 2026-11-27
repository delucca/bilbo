use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};

use bilbo_cli::console::env::{Env, Who};
use bilbo_cli::console::message::Message;
use bilbo_cli::console::outcome::{Failure, Outcome};
use bilbo_cli::console::settings;
use bilbo_cli::console::view::View;
use bilbo_entries::forget::{Request, forget};
use bilbo_entries::read::refs::Target;
use bilbo_store::entries::{Capture, Entries, Generation, Head, StoreError};
use bilbo_store::id::Id;
use bilbo_store::labels::{CaptureLabel, Tags};
use bilbo_store::segments::{Options, Step, Store};

mod note_line;
mod notes;
mod runner;
mod sandbox;
mod scratch;

use runner::{Run, Sandbox};

const PLAIN: &[u8] =
    include_bytes!("../../../libs/entries/rust/fixtures/markdown/no-frontmatter.md");
const NOTE: &[u8] = include_bytes!("../../../libs/entries/rust/fixtures/markdown/note.md");
const SECTIONS: &[u8] = include_bytes!("../../../libs/entries/rust/fixtures/markdown/sections.md");
const SESSION: &[u8] = include_bytes!(
    "../../../libs/entries/rust/fixtures/session/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl"
);
const UUID: &str = "8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b";

fn tune(
    mut indexing: bilbo_cli::console::indexing::Indexing<'_>,
) -> bilbo_cli::console::indexing::Indexing<'_> {
    indexing.tokenizer = &runner::shared_tokenizer;
    indexing
}

impl Sandbox {
    fn new(test: &str) -> Sandbox {
        let dir = scratch::scratch_dir(test);
        for folder in ["home", "config", "garden notes"] {
            fs::create_dir_all(dir.join(folder)).expect("create scratch");
        }
        let sandbox = Sandbox { dir };
        sandbox.follow(&[("garden notes", "markdown")]);
        sandbox
    }

    fn follow(&self, follows: &[(&str, &str)]) {
        let text: String = follows
            .iter()
            .map(|(folder, kind)| {
                format!(
                    "[[follow]]\npath = \"{}\"\nkind = \"{kind}\"\n\n",
                    self.dir.join(folder).display()
                )
            })
            .collect();
        fs::write(self.dir.join("config/config.toml"), text).expect("a config file");
    }

    fn notes(&self) -> PathBuf {
        self.dir.join("garden notes")
    }

    fn session(&self) -> PathBuf {
        self.dir
            .join("home/.claude/projects/-home-me-src-garden")
            .join(format!("{UUID}.jsonl"))
    }

    fn vars(&self) -> HashMap<String, OsString> {
        let mut vars = scratch::vars(&self.dir);
        vars.insert("BILBO_SEARCH_WORKERS".to_owned(), OsString::from("2"));
        vars
    }

    fn run(&self, args: &[&str]) -> Run {
        let vars = self.vars();
        runner::bilbo_with(Rc::new(move |name| vars.get(name).cloned()), args, &tune)
    }

    fn refresh(&self) -> Run {
        let run = self.run(&["refresh"]);
        assert!(run.code <= 1, "{:?}", run.stderr);
        run
    }

    fn layout(&self) -> bilbo_store::layout::Layout {
        let vars = self.vars();
        settings::load(
            &|name| vars.get(name).cloned(),
            &|path| fs::read(path),
            &bilbo_cli::kind_keys(),
            &bilbo_cli::provider_keys(),
        )
        .expect("settings")
        .layout()
    }

    fn store(&self) -> Store {
        Store::open(&self.layout(), Options::default(), &mut |_| {}).expect("open the store")
    }

    fn head(&self, file: &Path) -> Head {
        self.store()
            .by_capture(&CaptureLabel::file(file).expect("a label"))
            .expect("head")
            .expect("an entry")
    }

    fn forget(&self, id: Id) {
        let mut store = self.store();
        forget_in(&mut store, id).expect("forgotten");
    }

    fn head_of(&self, id: Id) -> Head {
        let store = self.store();
        let label = store
            .meta(id)
            .expect("meta")
            .and_then(|meta| meta.labels.capture)
            .expect("a label");
        store.by_capture(&label).expect("a lookup").expect("a head")
    }

    fn searched(&self, words: &[&str]) -> (Run, u64) {
        let mut args = vec!["recall"];
        args.extend(words);
        let run = self.run(&args);
        let searched = run
            .stderr
            .iter()
            .find_map(|line| {
                line.strip_prefix("bilbo: searched ")?
                    .split(' ')
                    .next()?
                    .parse()
                    .ok()
            })
            .unwrap_or(0);
        (run, searched)
    }
}

fn write(path: &Path, bytes: &[u8]) {
    sandbox::write(path, bytes);
    fs::File::options()
        .write(true)
        .open(path)
        .expect("open")
        .set_modified(SystemTime::now() - Duration::from_secs(3600))
        .expect("backdate");
}

fn forget_in(
    store: &mut Store,
    id: Id,
) -> Result<bilbo_entries::forget::Output, bilbo_entries::forget::Error> {
    let request = Request {
        targets: vec![Target::Entry(id)],
        listed: 5,
    };
    forget(&request, &|_| Vec::new(), store)
}

fn nothing_found(run: &Run, searched: u64) {
    let noun = if searched == 1 { "entry" } else { "entries" };
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        [
            format!("bilbo: searched {searched} {noun}"),
            "bilbo: nothing matched".to_owned()
        ]
    );
}

fn counts(run: &Run) -> &str {
    run.stdout.lines().next().expect("counts")
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

fn passed_line(path: &Path, id: Id) -> String {
    let raw = path.display().to_string();
    let said = format!(
        "bilbo: passed over {}: its entry {id} was forgotten",
        bilbo_cli::console::message::sanitize(&raw)
    );
    if bilbo_cli::console::message::renders_as_written(&raw) {
        format!("{said}; bilbo remember {} keeps it again", quoted(&raw))
    } else {
        said
    }
}

#[test]
fn a_forgotten_note_under_a_follow_is_not_kept_again_by_the_next_pass() {
    let sandbox = Sandbox::new("forget_note_pass");
    let plain = sandbox.notes().join("no-frontmatter.md");
    let other = sandbox.notes().join("note.md");
    write(&plain, PLAIN);
    write(&other, NOTE);
    let first = sandbox.refresh();
    assert_eq!(
        counts(&first),
        "new 2, appended 0, new generation 0, unchanged 0, refused 0"
    );
    let (hit, before) = sandbox.searched(&["frontmatter"]);
    assert_eq!(hit.code, 0, "{:?}", hit.stderr);
    assert!(hit.stdout.contains("No frontmatter"), "{}", hit.stdout);
    let id = sandbox.head(&plain).id;
    sandbox.forget(id);

    let next = sandbox.refresh();

    assert_eq!(next.code, 0, "{:?}", next.stderr);
    assert_eq!(
        counts(&next),
        "new 0, appended 0, new generation 0, unchanged 2, refused 0"
    );
    assert_eq!(next.stderr, [passed_line(&plain, id)]);
    let again = sandbox.refresh();
    assert_eq!(again.stderr, Vec::<String>::new());

    let (found, _) = sandbox.searched(&["frontmatter"]);
    nothing_found(&found, before - 1);
    let entries = sandbox.store().captures_with("file:").expect("labels");
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries
            .iter()
            .filter(|(_, label)| label.path().is_some())
            .count(),
        1
    );
}

#[test]
fn a_forgotten_session_that_grows_stays_out() {
    let sandbox = Sandbox::new("forget_session_grows");
    let file = sandbox.session();
    sandbox.follow(&[("home/.claude/projects", "session")]);
    write(&sandbox.notes().join("note.md"), NOTE);
    write(&file, SESSION);
    let first = sandbox.refresh();
    assert_eq!(first.code, 0, "{:?} {}", first.stderr, first.stdout);
    let (hit, before) = sandbox.searched(&["purslane"]);
    assert_eq!(hit.code, 0, "{:?}", hit.stderr);
    assert!(hit.stdout.contains("purslane"), "{}", hit.stdout);
    let id = sandbox.head(&file).id;
    sandbox.forget(id);
    let record = std::str::from_utf8(SESSION)
        .expect("utf-8")
        .lines()
        .find(|line| line.contains("purslane"))
        .expect("the purslane record");
    write(&file, &[SESSION, record.as_bytes(), b"\n"].concat());

    let run = sandbox.refresh();

    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.stderr, [passed_line(&file, id)]);
    let read = sandbox.run(&["read", &id.to_string()]);
    assert!(
        read.stdout.lines().any(|line| line == "records: 0"),
        "{}",
        read.stdout
    );
    let (found, _) = sandbox.searched(&["purslane"]);
    nothing_found(&found, before - 1);
}

#[test]
fn a_note_renamed_to_a_forgotten_notes_name_is_kept_as_itself() {
    let sandbox = Sandbox::new("forget_rename");
    let first = sandbox.notes().join("note.md");
    let second = sandbox.notes().join("sections.md");
    write(&first, NOTE);
    write(&second, SECTIONS);
    sandbox.refresh();
    let first_id = sandbox.head(&first).id;
    let second_id = sandbox.head(&second).id;
    fs::remove_file(&first).expect("remove");
    let withdrawn = sandbox.refresh();
    assert_eq!(withdrawn.code, 0, "{:?}", withdrawn.stderr);
    let emptied = sandbox.head(&first);
    assert_eq!(
        (emptied.id, emptied.generation, emptied.records),
        (first_id, Generation(2), 0)
    );
    fs::rename(&second, &first).expect("rename");
    let refused = sandbox.refresh();
    assert_eq!(refused.code, 1, "{:?}", refused.stderr);
    assert!(
        refused
            .stderr
            .iter()
            .any(|line| line.contains("but this path was kept as entry")),
        "{:?}",
        refused.stderr
    );

    sandbox.forget(first_id);
    let moved = sandbox.refresh();

    assert_eq!(moved.code, 0, "{:?}", moved.stderr);
    assert!(
        moved
            .stderr
            .iter()
            .any(|line| line.starts_with("bilbo: moved ")),
        "{:?}",
        moved.stderr
    );
    let read = sandbox.run(&["read", &second_id.to_string()]);
    assert!(
        read.stdout
            .lines()
            .any(|line| line == format!("label: file:{} (capture)", first.display())),
        "{}",
        read.stdout
    );
}

#[test]
fn a_forget_stopped_between_its_two_writes_is_finished_by_the_next() {
    let sandbox = Sandbox::new("forget_stopped");
    let file = sandbox.notes().join("no-frontmatter.md");
    write(&file, PLAIN);
    sandbox.refresh();
    let id = sandbox.head(&file).id;
    let (hit, searched) = sandbox.searched(&["frontmatter"]);
    assert_eq!(hit.code, 0, "{:?}", hit.stderr);
    assert!(hit.stdout.contains("No frontmatter"), "{}", hit.stdout);
    assert_eq!(searched, 1);
    let frames = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&frames);
    let options = Options {
        hook: Some(Box::new(move |step| {
            if step == Step::FrameWritten && counted.fetch_add(1, Ordering::SeqCst) == 1 {
                Err(std::io::Error::other("disk full"))
            } else {
                Ok(())
            }
        })),
        ..Options::default()
    };
    let mut store = Store::open(&sandbox.layout(), options, &mut |_| {}).expect("open");
    let error = forget_in(&mut store, id).expect_err("stopped");
    assert!(
        matches!(
            error,
            bilbo_entries::forget::Error::Store {
                error: StoreError::Io { .. },
                ..
            }
        ),
        "{error:?}"
    );
    drop(store);

    let held = sandbox.store().meta(id).expect("meta").expect("a meta");
    let label = held.labels.capture.expect("a label").to_string();
    assert!(label.starts_with("forgotten:"), "{label}");
    let next = sandbox.refresh();
    assert_eq!(next.stderr, [passed_line(&file, id)]);
    let (hit, searched) = sandbox.searched(&["frontmatter"]);
    assert_eq!(hit.code, 0, "{:?}", hit.stderr);
    assert!(hit.stdout.contains("No frontmatter"), "{}", hit.stdout);
    assert_eq!(searched, 1);

    sandbox.forget(id);
    sandbox.refresh();
    let (found, _) = sandbox.searched(&["frontmatter"]);
    nothing_found(&found, 0);
}

#[test]
fn a_forgotten_note_named_to_remember_is_in_recall_again_as_the_same_entry() {
    let sandbox = Sandbox::new("forget_by_hand");
    let file = sandbox.notes().join("no-frontmatter.md");
    write(&file, PLAIN);
    sandbox.refresh();
    let id = sandbox.head(&file).id;
    sandbox.forget(id);
    sandbox.refresh();
    let (gone, _) = sandbox.searched(&["frontmatter"]);
    assert_eq!(gone.code, 1, "{:?}", gone.stderr);

    let run = sandbox.run(&["remember", &file.display().to_string()]);

    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        counts(&run),
        "new 0, appended 1, new generation 0, unchanged 0, refused 0"
    );
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: kept {} again: its entry {id} was forgotten",
            file.display()
        )]
    );
    let (found, _) = sandbox.searched(&["frontmatter"]);
    assert_eq!(found.code, 0, "{:?}", found.stderr);
    assert!(
        found.stdout.contains(&id.short().to_string()),
        "{}",
        found.stdout
    );
    assert!(found.stdout.contains("No frontmatter"), "{}", found.stdout);
    let old = sandbox.run(&["read", &format!("{id}#g1")]);
    assert_eq!(old.code, 0, "{:?}", old.stderr);
    assert!(old.stdout.contains("generation: 1 of 2"), "{}", old.stdout);
    assert!(old.stdout.contains("Just a title"), "{}", old.stdout);
}

#[test]
fn a_folder_given_to_remember_passes_a_forgotten_file_over_and_says_so() {
    let sandbox = Sandbox::new("forget_folder_by_hand");
    let file = sandbox.notes().join("no-frontmatter.md");
    write(&file, PLAIN);
    sandbox.refresh();
    let id = sandbox.head(&file).id;
    sandbox.forget(id);

    let run = sandbox.run(&["remember", &sandbox.notes().display().to_string()]);

    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        counts(&run),
        "new 0, appended 0, new generation 0, unchanged 1, refused 0"
    );
    assert_eq!(run.stderr, [passed_line(&file, id)]);
}

#[test]
fn the_passed_over_line_is_held_back_when_the_store_fails_at_the_close() {
    for (name, folder) in [("refresh", false), ("remember_folder", true)] {
        let sandbox = Sandbox::new(&format!("forget_failed_close_{name}"));
        let file = sandbox.notes().join("no-frontmatter.md");
        write(&file, PLAIN);
        sandbox.refresh();
        let id = sandbox.head(&file).id;
        sandbox.forget(id);
        let flag = sandbox.layout().state.join("store.again");
        let _ = fs::remove_file(&flag);
        fs::create_dir(&flag).expect("a folder in its place");

        let run = if folder {
            sandbox.run(&["remember", &sandbox.notes().display().to_string()])
        } else {
            sandbox.run(&["refresh"])
        };

        assert_eq!(run.code, 1, "{name}: {:?}", run.stderr);
        assert_eq!(
            counts(&run),
            "new 0, appended 0, new generation 0, unchanged 1, refused 0",
            "{name}"
        );
        assert!(
            run.stderr.iter().all(|line| !line.contains("passed over")),
            "{name}: {:?}",
            run.stderr
        );
        assert!(
            run.stderr
                .iter()
                .any(|line| line.starts_with("bilbo: cannot ")),
            "{name}: {:?}",
            run.stderr
        );
    }
}

#[test]
fn a_pass_served_for_another_bilbo_says_the_passed_over_line_before_its_counts() {
    let sandbox = Sandbox::new("forget_served");
    let file = sandbox.notes().join("no-frontmatter.md");
    let other = sandbox.dir.join("x.md");
    write(&file, PLAIN);
    write(&other, NOTE);
    sandbox.refresh();
    let id = sandbox.head(&file).id;
    sandbox.forget(id);
    fs::write(sandbox.layout().state.join("store.again"), b"1").expect("the flag");

    let run = sandbox.run(&["remember", &other.display().to_string()]);

    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let at = run
        .stderr
        .iter()
        .position(|line| *line == passed_line(&file, id))
        .unwrap_or_else(|| panic!("{:?}", run.stderr));
    assert!(
        run.stderr[at + 1].starts_with("bilbo: ran the follows again for another bilbo: "),
        "{:?}",
        run.stderr
    );
}

#[test]
fn a_served_pass_says_its_passed_over_line_unless_the_store_fails_at_the_close() {
    for store_fails in [true, false] {
        let sandbox = Sandbox::new(if store_fails {
            "forget_served_failed"
        } else {
            "forget_served_closed"
        });
        let file = sandbox.notes().join("no-frontmatter.md");
        write(&file, PLAIN);
        sandbox.refresh();
        let id = sandbox.head(&file).id;
        sandbox.forget(id);

        let vars = sandbox.vars();
        let settings = settings::load(
            &|name| vars.get(name).cloned(),
            &|path| fs::read(path),
            &bilbo_cli::kind_keys(),
            &bilbo_cli::provider_keys(),
        )
        .expect("settings");
        let readers = bilbo_cli::readers(&settings).expect("the readers");
        let rendering = bilbo_cli::rendering(&settings).expect("the rendering");
        let disk = |cache: &Path| bilbo_cli::index(cache, &settings.search);
        let indexing = bilbo_cli::indexing(
            &settings,
            &readers,
            &rendering,
            &runner::shared_tokenizer,
            &disk,
        );
        let options = Options {
            hook: store_fails.then(|| -> bilbo_store::segments::Hook {
                Box::new(|step| {
                    if step == Step::Releasing {
                        Err(std::io::Error::other("released"))
                    } else {
                        Ok(())
                    }
                })
            }),
            ..settings.store.options()
        };
        let store = Store::open(&settings.layout(), options, &mut |_| {}).expect("open the store");
        fs::write(settings.layout().state.join("store.again"), b"1").expect("the flag");
        let kinds = bilbo_cli::kinds(&settings).expect("the kinds");

        let closing = bilbo_cli::remember::follows::finish(
            store,
            &settings.follow.value,
            &kinds,
            &indexing,
            &mut |_: &bilbo_cli::console::message::Message| {},
            0,
        );

        let lines: Vec<String> = closing
            .messages
            .iter()
            .flat_map(bilbo_cli::console::message::Message::plain_lines)
            .collect();
        let passed = passed_line(&file, id);
        assert_eq!(closing.failed, store_fails, "{lines:?}");
        if store_fails {
            assert!(
                lines.iter().all(|line| !line.contains("passed over")),
                "{lines:?}"
            );
        } else {
            let at = lines
                .iter()
                .position(|line| *line == passed)
                .expect("the line");
            assert!(
                lines[at + 1].starts_with("bilbo: ran the follows again for another bilbo: "),
                "{lines:?}"
            );
        }
    }
}

const ID_A: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";
const ID_TWIN: &str = "01JC6Q3V7XKM4P8R2T6W9YB0DE";
const MAIN: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";
const CLAUDE: &[u8] = include_bytes!(
    "../../../libs/entries/rust/fixtures/session/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl"
);
const SUBAGENT: &[u8] =
    include_bytes!("../../../libs/entries/rust/fixtures/session/agent-ad63c2b3c9e7a4b3d.jsonl");
const META: &[u8] =
    include_bytes!("../../../libs/entries/rust/fixtures/session/agent-ad63c2b3c9e7a4b3d.meta.json");
const WAIT: Duration = Duration::from_secs(10);
const WAITING: &str = "bilbo: waiting for another bilbo to finish writing the store";
const FORGOTTEN: &str = "forgotten 1, belonging 0, unchanged 0\n";

impl Sandbox {
    fn forgetting(&self, refs: &[&str]) -> Run {
        let mut args = vec!["forget"];
        args.extend(refs);
        self.run(&args)
    }

    fn open_len(&self) -> u64 {
        fs::metadata(self.layout().state.join("open.seg"))
            .expect("open.seg")
            .len()
    }

    fn hit_ref(run: &Run) -> String {
        run.stdout
            .lines()
            .find_map(|line| line.strip_prefix("=== "))
            .and_then(|header| header.split('\t').next())
            .and_then(|reference| reference.split('#').next())
            .expect("a hit")
            .to_owned()
    }

    fn keep_note(&self, text: &str) -> Id {
        let kept = sandbox::run(self.vars(), &["remember", "-"], text.as_bytes(), None);
        assert_eq!(kept.code, 0, "{:?}", kept.stderr);
        let id = kept
            .stderr
            .iter()
            .find_map(|line| line.strip_prefix("bilbo: kept the note as "))
            .expect("the kept line");
        Id::parse(id).expect("an id")
    }
}

#[test]
fn a_forgotten_file_is_gone_from_recall_right_after() {
    let sandbox = Sandbox::new("forget_verb_file");
    let plain = sandbox.notes().join("no-frontmatter.md");
    write(&plain, PLAIN);
    write(&sandbox.notes().join("note.md"), NOTE);
    sandbox.refresh();
    let (hit, searched) = sandbox.searched(&["frontmatter"]);
    assert_eq!((hit.code, searched), (0, 2), "{:?}", hit.stderr);
    let short = Sandbox::hit_ref(&hit);
    let id = sandbox.head(&plain).id;

    let run = sandbox.forgetting(&[&short]);

    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.stdout, FORGOTTEN);
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: forgot {id}: its label was {}; bilbo read {id}#g1 prints the outline of what it held",
            bilbo_cli::console::message::sanitize(&format!("file:{}", plain.display()))
        )]
    );
    let (again, _) = sandbox.searched(&["frontmatter"]);
    nothing_found(&again, 1);
    let read = sandbox.run(&["read", &id.to_string()]);
    assert!(
        read.stdout
            .lines()
            .any(|line| line.starts_with("label: forgotten:")),
        "{}",
        read.stdout
    );
    assert!(read.stdout.lines().any(|line| line == "records: 0"));
    let old = sandbox.run(&["read", &format!("{id}#g1")]);
    assert!(old.stdout.contains("Just a title"), "{}", old.stdout);
}

#[test]
fn a_forgotten_note_with_no_file_is_gone_from_recall_right_after() {
    let sandbox = Sandbox::new("forget_verb_note");
    write(&sandbox.notes().join("note.md"), NOTE);
    let id = sandbox.keep_note("Beet greens keep well in a cold frame.");
    sandbox.refresh();
    let hit = sandbox.run(&["recall", "beet", "greens", "--kind", "note"]);
    assert_eq!(hit.code, 0, "{:?} {}", hit.stderr, hit.stdout);
    assert!(hit.stdout.contains("cold frame"), "{}", hit.stdout);
    let searched = sandbox.searched(&["synthetic"]).1;

    let run = sandbox.forgetting(&[&id.to_string()]);

    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.stdout, FORGOTTEN);
    let again = sandbox.run(&["recall", "beet", "greens", "--kind", "note"]);
    nothing_found(&again, searched - 1);
}

#[test]
fn a_session_goes_with_its_subagent_its_notes_and_its_summary() {
    let Family {
        sandbox,
        session,
        subagent,
        ..
    } = session_with_a_subagent("forget_verb_session");
    let line = note_line::line(session, "The garden fence was asked about.", None, &[]);
    let notes = notes::noted(&sandbox.layout(), session, &[line]);
    let summary_label = bilbo_derive::summary::label(session);
    let summary_line =
        bilbo_derive::summary::record::write(&bilbo_derive::summary::record::Summary {
            text: "A garden log.".to_owned(),
            instruction: "instruction".to_owned(),
            provider: "anthropic".to_owned(),
            model: "model-a".to_owned(),
            answered: None,
            made: 1,
            entry: session,
            generation: Generation(1),
            records: 2,
            content: 1,
            notes: None,
            sent: 1,
            tokens_in: None,
            tokens_out: None,
        });
    {
        let mut store = sandbox.store();
        store
            .capture(&Capture {
                label: &summary_label,
                kind: bilbo_derive::summary::KIND,
                id: None,
                id_label: None,
                stamp: None,
                bytes: summary_line.as_bytes(),
                ends: &[summary_line.len()],
                tags: &Tags::default(),
            })
            .expect("a summary");
        store.close().expect("close");
    }
    let summary = sandbox
        .store()
        .by_capture(&summary_label)
        .expect("a lookup")
        .expect("a summary")
        .id;
    sandbox.refresh();
    let queries: [(&[&str], Id); 3] = [
        (&["recall", "tomatoes"], session),
        (&["recall", "packets"], subagent),
        (&["recall", "fence", "--kind", "note"], notes),
    ];
    for (query, id) in queries {
        let before = sandbox.run(query);
        assert_eq!(before.code, 0, "{query:?}: {:?}", before.stderr);
        assert!(
            before.stdout.contains(&id.short().to_string()),
            "{query:?}: {}",
            before.stdout
        );
    }

    let run = sandbox.forgetting(&[&session.to_string()]);

    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.stdout, "forgotten 1, belonging 3, unchanged 0\n");
    assert_eq!(run.stderr.len(), 4, "{:?}", run.stderr);
    assert!(run.stderr[0].starts_with(&format!("bilbo: forgot {session}: ")));
    let mut members: Vec<Id> = vec![subagent, notes, summary];
    members.sort();
    for (line, member) in run.stderr[1..].iter().zip(&members) {
        assert!(
            line.starts_with(&format!("bilbo: forgot {member} with {session}: ")),
            "{line}"
        );
        assert_eq!(
            line.contains("prints the outline of what it held"),
            *member != summary,
            "{line}"
        );
    }
    for (query, _) in queries {
        let after = sandbox.run(query);
        assert_eq!(after.code, 1, "{query:?}: {:?}", after.stderr);
        assert_eq!(after.stdout, "", "{query:?}");
        assert_eq!(
            after.stderr.last().map(String::as_str),
            Some("bilbo: nothing matched"),
            "{query:?}"
        );
    }
}

#[test]
fn forgetting_twice_is_unchanged_and_exits_0() {
    let sandbox = Sandbox::new("forget_verb_twice");
    let file = sandbox.notes().join("no-frontmatter.md");
    write(&file, PLAIN);
    sandbox.refresh();
    let id = sandbox.head(&file).id.to_string();
    assert_eq!(sandbox.forgetting(&[&id]).code, 0);
    let size = sandbox.open_len();

    let again = sandbox.forgetting(&[&id]);

    assert_eq!(again.code, 0, "{:?}", again.stderr);
    assert_eq!(again.stdout, "forgotten 0, belonging 0, unchanged 1\n");
    assert_eq!(again.stderr, Vec::<String>::new());
    assert_eq!(sandbox.open_len(), size);
}

#[test]
fn a_ref_that_names_none_or_several_writes_nothing() {
    let sandbox = Sandbox::new("forget_verb_refs");
    for (name, id) in [("a.md", ID_A), ("twin.md", ID_TWIN)] {
        write(
            &sandbox.notes().join(name),
            format!("---\nid: {id}\n---\n# {name}\n\nbody of {name}\n").as_bytes(),
        );
    }
    sandbox.refresh();
    let size = sandbox.open_len();
    let missing = Id::from_bytes([9; 16]).to_string();

    let none = sandbox.forgetting(&[ID_A, &missing]);
    assert_eq!(none.code, 1, "{:?}", none.stderr);
    assert_eq!(none.stdout, "");
    assert_eq!(none.stderr, [format!("bilbo: no entry {missing}")]);
    assert_eq!(sandbox.open_len(), size);

    let short = Id::parse(ID_A).expect("an id").short().to_string();
    let several = sandbox.forgetting(&[ID_A, &short]);
    assert_eq!(several.code, 2, "{:?}", several.stderr);
    assert_eq!(several.stdout, "");
    let file = |name: &str| {
        sandbox
            .notes()
            .join(name)
            .canonicalize()
            .expect("canonical")
    };
    let ambiguity = format!(
        "bilbo: {short} names 2 entries: {ID_A} (file:{}), {ID_TWIN} (file:{}); give the id of one",
        file("a.md").display(),
        file("twin.md").display()
    );
    assert_eq!(
        several.stderr,
        [
            bilbo_cli::console::message::sanitize(&ambiguity),
            "bilbo: usage: bilbo forget <ref>...".to_owned(),
            "bilbo: see 'bilbo forget --help'".to_owned(),
        ]
    );
    assert_eq!(sandbox.open_len(), size);
}

#[test]
fn forget_waits_for_another_writer_whatever_store_wait_says() {
    waits_while_held(0, &["store.lock", "store.writer"], false);
    waits_while_held(1, &["store.lock"], true);
}

#[test]
fn the_writer_waits_for_a_writer_and_never_asks_for_a_pass() {
    let sandbox = Sandbox::new("forget_policy");
    let vars = sandbox.vars();
    let settings = settings::load(
        &|name| vars.get(name).cloned(),
        &|path| fs::read(path),
        &bilbo_cli::kind_keys(),
        &bilbo_cli::provider_keys(),
    )
    .expect("settings");
    assert_eq!(
        bilbo_cli::forget::writer_options(&settings).held,
        bilbo_store::segments::Held::Wait
    );
}

fn waits_while_held(number: usize, locks: &[&str], shared: bool) {
    let sandbox = Sandbox::new(&format!("forget_waits_{number}"));
    let file = sandbox.notes().join("no-frontmatter.md");
    write(&file, PLAIN);
    sandbox.refresh();
    let id = sandbox.head(&file).id.to_string();
    let layout = sandbox.layout();
    let held: Vec<fs::File> = locks
        .iter()
        .map(|name| {
            let file = fs::File::create(layout.state.join(name)).expect("a lock file");
            if shared {
                file.lock_shared().expect("take the lock");
            } else {
                file.lock().expect("take the lock");
            }
            file
        })
        .collect();
    let (sender, receiver) = mpsc::channel();
    let mut vars = sandbox.vars();
    vars.insert("BILBO_STORE_WAIT".to_owned(), OsString::from("0s"));
    let args = ["forget".to_owned(), id];
    let runner = thread::spawn(move || {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        sandbox::run(vars, &refs, b"", Some(sender))
    });

    assert_eq!(
        receiver.recv_timeout(WAIT).expect("a waiting notice"),
        WAITING
    );
    assert!(!runner.is_finished());
    let again = || fs::read(layout.state.join("store.again")).ok();
    let before_again = again();
    for file in &held {
        file.unlock().expect("release the lock");
    }
    let finished = runner.join().expect("the run finishes");
    assert_eq!(finished.code, 0, "{:?}", finished.stderr);
    assert_eq!(finished.stdout, FORGOTTEN);
    assert!(finished.notices.contains(&WAITING.to_owned()));
    assert!(
        finished
            .stderr
            .iter()
            .all(|line| !line.contains("requested another pass")),
        "{:?}",
        finished.stderr
    );
    assert!(
        finished
            .stderr
            .iter()
            .any(|line| line.starts_with("bilbo: forgot ")),
        "{:?}",
        finished.stderr
    );
    assert_eq!(again(), before_again);
}

fn outcome_of(
    vars: HashMap<String, OsString>,
    args: &[&str],
    notice: Box<dyn FnMut(&Message)>,
) -> Result<Outcome, Failure> {
    let argv: Vec<OsString> = std::iter::once("bilbo")
        .chain(args.iter().copied())
        .map(OsString::from)
        .collect();
    let mut env = Env {
        var: Rc::new(move |name| vars.get(name).cloned()),
        stdin: Box::new(io::empty()),
        who: Who::Nobody,
        notice,
    };
    let settings = settings::load(
        &*env.var,
        &|path| fs::read(path),
        &bilbo_cli::kind_keys(),
        &bilbo_cli::provider_keys(),
    )
    .map_err(Failure::from);
    bilbo_cli::run_tuned(&bilbo_cli::bilbo(), &argv, &mut env, settings, &tune)
}

#[test]
fn forget_serves_a_refresh_that_found_it_holding_the_store() {
    let sandbox = Sandbox::new("forget_serves");
    sandbox.follow(&[("garden notes", "markdown"), ("follow", "markdown")]);
    let file = sandbox.notes().join("no-frontmatter.md");
    write(&file, PLAIN);
    sandbox.refresh();
    let id = sandbox.head(&file).id.to_string();
    let kept = sandbox.dir.join("follow/new.md");
    write(
        &kept,
        b"# New\n\nA note kept by the pass that forget serves.\n",
    );
    let layout = sandbox.layout();
    let mut open_seg = fs::OpenOptions::new()
        .append(true)
        .open(layout.state.join("open.seg"))
        .expect("open.seg");
    io::Write::write_all(&mut open_seg, &[1, 0, 0]).expect("a torn tail");
    drop(open_seg);
    let bare = Rc::new(std::cell::RefCell::new(None));
    let vars = sandbox.vars();
    let notice = {
        let (bare, vars) = (Rc::clone(&bare), vars.clone());
        Box::new(move |message: &Message| {
            let said = message.plain_lines().join("\n");
            if said.contains("torn frame") && bare.borrow().is_none() {
                let vars = vars.clone();
                *bare.borrow_mut() = Some(runner::bilbo_with(
                    Rc::new(move |name| vars.get(name).cloned()),
                    &["refresh"],
                    &tune,
                ));
            }
        })
    };

    let outcome = outcome_of(vars, &["forget", &id], notice).expect("the forget run");

    let bare = bare.borrow();
    let bare = bare.as_ref().expect("the bare run");
    assert_eq!(bare.code, 0, "{:?}", bare.stderr);
    assert_eq!(
        bare.stderr,
        ["bilbo: another bilbo is writing the store; requested another pass over the follows"]
    );
    assert_eq!(outcome.exit_code(), 0);
    assert_eq!(outcome.stdout(&View::PLAIN), FORGOTTEN);
    let said = outcome.stderr_lines(&View::PLAIN);
    assert!(
        said.iter()
            .any(|line| line.starts_with("bilbo: ran the follows again for another bilbo: new 1, ")),
        "{said:?}"
    );
    let label = CaptureLabel::file(&kept.canonicalize().expect("canonical")).expect("a label");
    assert!(
        sandbox
            .store()
            .by_capture(&label)
            .expect("a lookup")
            .is_some()
    );
}

fn no_tokenizer() -> Result<bilbo_search::tokenizer::Tokenizer, Failure> {
    Err(Failure::Refused("the tokenizer is gone".to_owned()))
}

fn without_a_tokenizer(
    mut indexing: bilbo_cli::console::indexing::Indexing<'_>,
) -> bilbo_cli::console::indexing::Indexing<'_> {
    indexing.tokenizer = &no_tokenizer;
    indexing
}

#[test]
fn an_index_that_is_not_updated_is_said_and_the_entry_is_forgotten_all_the_same() {
    let sandbox = Sandbox::new("forget_no_index");
    let file = sandbox.notes().join("no-frontmatter.md");
    write(&file, PLAIN);
    write(&sandbox.notes().join("note.md"), NOTE);
    sandbox.refresh();
    let id = sandbox.head(&file).id;
    let vars = sandbox.vars();

    let run = runner::bilbo_with(
        Rc::new(move |name| vars.get(name).cloned()),
        &["forget", &id.to_string()],
        &without_a_tokenizer,
    );

    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(run.stdout, FORGOTTEN);
    assert!(
        run.stderr
            .iter()
            .any(|line| line.starts_with("bilbo: forgot ")),
        "{:?}",
        run.stderr
    );
    assert!(
        run.stderr
            .iter()
            .any(|line| line.starts_with("bilbo: the search index was not updated: ")),
        "{:?}",
        run.stderr
    );
    assert!(
        run.stderr
            .iter()
            .all(|line| !line.starts_with("bilbo: stopped part way")),
        "{:?}",
        run.stderr
    );
    let (found, _) = sandbox.searched(&["frontmatter"]);
    assert_eq!(found.code, 1, "{:?}", found.stderr);
    assert_eq!(found.stdout, "");
    assert!(
        found.stderr.iter().any(|line| line.contains("behind")),
        "{:?}",
        found.stderr
    );
    assert!(
        found
            .stderr
            .iter()
            .any(|line| line.starts_with("bilbo: left out 1 hit ")),
        "{:?}",
        found.stderr
    );
    assert_eq!(
        found.stderr.last().map(String::as_str),
        Some("bilbo: nothing matched")
    );
}

#[test]
fn tag_and_in_still_reach_a_forgotten_entry() {
    let sandbox = Sandbox::new("forget_tag_in");
    let file = sandbox.notes().join("no-frontmatter.md");
    write(&file, PLAIN);
    write(&sandbox.notes().join("note.md"), NOTE);
    sandbox.refresh();
    let id = sandbox.head(&file).id.to_string();
    assert_eq!(sandbox.forgetting(&[&id]).code, 0);

    let added = sandbox.run(&["tag", &id, "--add", "bed"]);
    assert_eq!(
        added.stdout, "changed 1, unchanged 0\n",
        "{:?}",
        added.stderr
    );
    for scope in [id.as_str(), "tag:bed", "frontmatter"] {
        let run = sandbox.run(&["recall", "--in", scope, "frontmatter"]);
        assert_eq!(run.code, 1, "{scope}: {:?}", run.stderr);
        assert!(
            run.stderr
                .iter()
                .any(|line| line.starts_with("bilbo: --in kept 1 entr")),
            "{scope}: {:?}",
            run.stderr
        );
        assert_eq!(
            run.stderr.last().map(String::as_str),
            Some("bilbo: nothing matched"),
            "{scope}"
        );
    }
}

#[test]
fn a_session_id_no_longer_names_a_forgotten_session() {
    let sandbox = Sandbox::new("forget_uuid");
    sandbox.follow(&[
        ("garden notes", "markdown"),
        ("home/.claude/projects", "session"),
    ]);
    write(&sandbox.notes().join("note.md"), NOTE);
    let file = sandbox.session();
    write(&file, SESSION);
    sandbox.refresh();

    let first = sandbox.forgetting(&[UUID]);
    let second = sandbox.forgetting(&[UUID]);

    assert_eq!(first.code, 0, "{:?}", first.stderr);
    assert_eq!(second.code, 1, "{:?}", second.stderr);
    assert_eq!(
        second.stderr,
        [format!("bilbo: no entry for the session {UUID}")]
    );
}

#[test]
fn the_other_entries_are_found_as_before() {
    let sandbox = Sandbox::new("forget_others");
    let file = sandbox.notes().join("no-frontmatter.md");
    write(&file, PLAIN);
    write(&sandbox.notes().join("note.md"), NOTE);
    sandbox.refresh();
    let id = sandbox.head(&file).id.to_string();
    let before = sandbox.run(&["recall", "synthetic"]);
    assert_eq!(before.code, 0, "{:?}", before.stderr);

    assert_eq!(sandbox.forgetting(&[&id]).code, 0);

    let after = sandbox.run(&["recall", "synthetic"]);
    assert_eq!(after.stdout, before.stdout);
}

fn hook_failing_at(step: Step, number: usize) -> bilbo_store::segments::Hook {
    let seen = Arc::new(AtomicUsize::new(0));
    Box::new(move |reached| {
        if reached == step && seen.fetch_add(1, Ordering::SeqCst) + 1 == number {
            Err(io::Error::other("disk full"))
        } else {
            Ok(())
        }
    })
}

fn open_with(sandbox: &Sandbox, hook: Option<bilbo_store::segments::Hook>) -> Store {
    let options = Options {
        hook,
        ..Options::default()
    };
    Store::open(&sandbox.layout(), options, &mut |_| {}).expect("open the store")
}

struct Family {
    sandbox: Sandbox,
    main: PathBuf,
    session: Id,
    subagent: Id,
}

fn session_with_a_subagent(test: &str) -> Family {
    let sandbox = Sandbox::new(test);
    sandbox.follow(&[
        ("garden notes", "markdown"),
        ("home/.claude/projects", "session"),
    ]);
    write(&sandbox.notes().join("note.md"), NOTE);
    let project = sandbox
        .dir
        .join("home/.claude/projects/-home-me-src-garden");
    let main = project.join(format!("{MAIN}.jsonl"));
    let child = project.join(format!("{MAIN}/subagents/agent-ad63c2b3c9e7a4b3d.jsonl"));
    write(&main, CLAUDE);
    write(&child, SUBAGENT);
    write(&child.with_extension("meta.json"), META);
    sandbox.refresh();
    let session = sandbox.head(&main).id;
    let subagent = sandbox.head(&child).id;
    Family {
        sandbox,
        main,
        session,
        subagent,
    }
}

fn stopped_by_uuid(family: &Family) -> bilbo_entries::forget::Error {
    let mut store = open_with(
        &family.sandbox,
        Some(hook_failing_at(Step::FrameWritten, 2)),
    );
    let request = Request {
        targets: vec![
            bilbo_entries::read::refs::Ref::parse(MAIN)
                .expect("a ref")
                .target,
        ],
        listed: 5,
    };
    forget(&request, &bilbo_cli::labelled, &mut store).expect_err("stopped")
}

#[test]
fn a_run_stopped_part_way_names_its_entries_by_id_and_the_retry_finishes_them() {
    let family = session_with_a_subagent("forget_stopped_part_way");
    let ids = stopped_ids(&family);

    let by_uuid = family.sandbox.forgetting(&[MAIN]);
    assert_eq!(by_uuid.code, 1, "{:?}", by_uuid.stderr);
    assert_eq!(
        by_uuid.stderr,
        [format!("bilbo: no entry for the session {MAIN}")]
    );
    let by_id = family
        .sandbox
        .forgetting(&ids.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(by_id.code, 0, "{:?}", by_id.stderr);
    assert_eq!(by_id.stdout, "forgotten 1, belonging 1, unchanged 0\n");
    assert!(
        by_id
            .stderr
            .iter()
            .any(|line| line.starts_with(&format!("bilbo: forgot {} with ", family.subagent)))
    );

    let other = session_with_a_subagent("forget_stopped_replaced");
    let ids = stopped_ids(&other);
    write(&other.main, SESSION);
    other.sandbox.refresh();
    let finished = other
        .sandbox
        .forgetting(&ids.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(finished.stdout, "forgotten 1, belonging 1, unchanged 0\n");
    let found = other.sandbox.run(&["recall", "purslane"]);
    assert_eq!(found.code, 0, "{:?}", found.stderr);
    assert!(found.stdout.contains("purslane"), "{}", found.stdout);
}

fn stopped_ids(family: &Family) -> Vec<String> {
    let error = stopped_by_uuid(family);
    let cause = match &error {
        bilbo_entries::forget::Error::Store { error, named } => {
            assert_eq!(named, &vec![family.session]);
            bilbo_cli::console::message::sanitize(&error.to_string())
        }
        other => panic!("{other:?}"),
    };
    let outcome =
        bilbo_cli::forget::ended(&bilbo_cli::bilbo(), error).expect("an outcome, not a failure");
    assert_eq!(outcome.exit_code(), 1);
    assert_eq!(outcome.stdout(&View::PLAIN), "");
    let said = outcome.stderr_lines(&View::PLAIN);
    assert_eq!(
        said,
        [
            format!(
                "bilbo: stopped part way: bilbo forget {} finishes it",
                family.session
            ),
            format!("bilbo: {cause}"),
        ]
    );
    said[0]
        .strip_prefix("bilbo: stopped part way: bilbo forget ")
        .and_then(|rest| rest.strip_suffix(" finishes it"))
        .expect("the line's form")
        .split(' ')
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_write_that_failed_after_it_was_durable_is_found_made() {
    let sandbox = Sandbox::new("forget_durable");
    let file = sandbox.notes().join("no-frontmatter.md");
    write(&file, PLAIN);
    write(&sandbox.notes().join("note.md"), NOTE);
    sandbox.refresh();
    let id = sandbox.head(&file).id;
    let searched = sandbox.searched(&["synthetic"]).1;
    let mut store = open_with(&sandbox, Some(hook_failing_at(Step::Appended, 2)));
    let request = Request {
        targets: vec![Target::Entry(id)],
        listed: 5,
    };
    let error = forget(&request, &bilbo_cli::labelled, &mut store).expect_err("failed");
    assert!(matches!(error, bilbo_entries::forget::Error::Store { .. }));
    drop(store);

    let meta = sandbox.store().meta(id).expect("meta").expect("a meta");
    assert!(
        meta.labels
            .capture
            .expect("a label")
            .as_str()
            .starts_with("forgotten:")
    );
    assert_eq!(sandbox.head_of(id).records, 0);
    let again = sandbox.forgetting(&[&id.to_string()]);

    assert_eq!(again.code, 0, "{:?}", again.stderr);
    assert_eq!(again.stdout, "forgotten 0, belonging 0, unchanged 1\n");
    let (found, _) = sandbox.searched(&["frontmatter"]);
    nothing_found(&found, searched - 1);
}

#[test]
fn a_close_that_fails_says_the_counts_and_keeps_the_other_lines_back() {
    let sandbox = Sandbox::new("forget_failed_closing");
    let file = sandbox.notes().join("no-frontmatter.md");
    write(&file, PLAIN);
    sandbox.refresh();
    let id = sandbox.head(&file).id;
    let vars = sandbox.vars();
    let settings = settings::load(
        &|name| vars.get(name).cloned(),
        &|path| fs::read(path),
        &bilbo_cli::kind_keys(),
        &bilbo_cli::provider_keys(),
    )
    .expect("settings");
    let readers = bilbo_cli::readers(&settings).expect("the readers");
    let mut env = Env::quiet();
    let args = bilbo_cli::forget::Args {
        refs: vec![id.to_string()],
    };

    let outcome = bilbo_cli::forget::run(
        &bilbo_cli::bilbo(),
        &args,
        &mut env,
        &settings,
        &readers,
        &|store, _| {
            store.release();
            bilbo_cli::console::closing::Closing {
                messages: vec![Message::error("the close failed")],
                failed: true,
                refused: false,
            }
        },
        &bilbo_cli::labelled,
    )
    .expect("an outcome");

    assert_eq!(outcome.exit_code(), 1);
    assert_eq!(outcome.stdout(&View::PLAIN), FORGOTTEN);
    assert_eq!(
        outcome.stderr_lines(&View::PLAIN),
        ["bilbo: the close failed"]
    );
}

#[test]
fn a_run_stopped_by_a_store_answer_it_does_not_know_says_the_same_command() {
    let id = Id::mint().expect("an id");
    let error = bilbo_entries::forget::Error::NotEmptied {
        id,
        named: vec![id],
        cause: None,
    };
    let outcome =
        bilbo_cli::forget::ended(&bilbo_cli::bilbo(), error).expect("an outcome, not a failure");
    assert_eq!(outcome.exit_code(), 1);
    assert_eq!(outcome.stdout(&View::PLAIN), "");
    let said = outcome.stderr_lines(&View::PLAIN);
    assert_eq!(said.len(), 2, "{said:?}");
    assert_eq!(
        said[0],
        format!("bilbo: stopped part way: bilbo forget {id} finishes it")
    );
}

#[test]
fn the_kept_again_line_is_held_back_when_the_store_fails_at_the_close() {
    let sandbox = Sandbox::new("forget_failed_close_again");
    let file = sandbox.notes().join("no-frontmatter.md");
    write(&file, PLAIN);
    sandbox.refresh();
    let id = sandbox.head(&file).id;
    sandbox.forget(id);
    let flag = sandbox.layout().state.join("store.again");
    let _ = fs::remove_file(&flag);
    fs::create_dir(&flag).expect("a folder in its place");

    let run = sandbox.run(&["remember", &file.display().to_string()]);

    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert!(
        run.stderr.iter().all(|line| !line.contains(" again: ")),
        "{:?}",
        run.stderr
    );
    assert!(
        run.stderr
            .iter()
            .any(|line| line.starts_with("bilbo: cannot ")),
        "{:?}",
        run.stderr
    );
}
