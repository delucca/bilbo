use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, SystemTime};

use bilbo_cli::console::env::{Env, Who};
use bilbo_cli::console::message::Message;
use bilbo_cli::console::outcome::{Failure, Outcome};
use bilbo_cli::console::settings;
use bilbo_cli::console::view::View;
use bilbo_entries::tag::{self, Change};
use bilbo_store::entries::{Capture, Entries};
use bilbo_store::id::Id;
use bilbo_store::labels::{CaptureLabel, Tag, Tags};
use bilbo_store::segments::{Options, Store};

mod runner;
mod scratch;

use runner::{Run, Sandbox};

const ID_A: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";
const ID_B: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DF";
const ID_TWIN: &str = "01JC6Q3V7XKM4P8R2T6W9YB0DE";
const ID_GONE: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DG";
const SHORT_A: &str = "2T6W9YB0DE";
const BODIES: [&str; 3] = [
    "Seville marmalade ripens beside the tomato vines, forty jars by October.",
    "Quince marmalade keeps on the top rack behind the flour bins, away from damp.",
    "Rhubarb marmalade cools on the windowsill while the kettle is put back on.",
];
const WAIT: Duration = Duration::from_secs(10);
const WAITING: &str = "bilbo: waiting for another bilbo to finish writing the store";
const USAGE: [&str; 5] = [
    "bilbo: usage: bilbo tag",
    "bilbo:        bilbo tag tag:<name>",
    "bilbo:        bilbo tag <ref>... [--add <tag>]... [--remove <tag>]...",
    "bilbo: see 'bilbo tag --help'",
    "",
];
const GRAMMAR: &str = "parts of a to z and digits joined by single hyphens, at most 64 bytes";

fn tune(
    mut indexing: bilbo_cli::console::indexing::Indexing<'_>,
) -> bilbo_cli::console::indexing::Indexing<'_> {
    indexing.tokenizer = &runner::shared_tokenizer;
    indexing
}

impl Sandbox {
    fn new(test: &str) -> Sandbox {
        let dir = scratch::scratch_dir(test);
        for folder in ["home", "config", "notes"] {
            fs::create_dir_all(dir.join(folder)).expect("create scratch");
        }
        fs::write(dir.join("config/config.toml"), "").expect("an empty config file");
        Sandbox { dir }
    }

    fn with_notes(test: &str, notes: &[(&str, &str)]) -> Sandbox {
        let sandbox = Sandbox::new(test);
        for ((name, id), body) in notes.iter().zip(BODIES) {
            sandbox.write(name, &format!("---\nid: {id}\n---\n# {name}\n\n{body}\n"));
        }
        let run = sandbox.run(&["remember", &sandbox.path("notes")]);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
        sandbox
    }

    fn path(&self, name: &str) -> String {
        self.dir.join(name).to_str().expect("UTF-8").to_owned()
    }

    fn write(&self, name: &str, text: &str) {
        let path = self.dir.join("notes").join(name);
        fs::write(&path, text).expect("write");
        let file = fs::File::options().write(true).open(&path).expect("open");
        file.set_modified(SystemTime::now() - Duration::from_secs(3600))
            .expect("backdate");
    }

    fn vars(&self, extra: &[(&str, &str)]) -> HashMap<String, OsString> {
        let mut vars = scratch::vars(&self.dir);
        vars.insert("BILBO_SEARCH_WORKERS".to_owned(), OsString::from("2"));
        for (name, value) in extra {
            vars.insert((*name).to_owned(), OsString::from(value));
        }
        vars
    }

    fn run(&self, args: &[&str]) -> Run {
        self.run_with(args, &[])
    }

    fn run_with(&self, args: &[&str], extra: &[(&str, &str)]) -> Run {
        let vars = self.vars(extra);
        runner::bilbo_with(Rc::new(move |name| vars.get(name).cloned()), args, &tune)
    }

    fn layout(&self) -> bilbo_store::layout::Layout {
        let vars = self.vars(&[]);
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

    fn declare(&self, id: &str, change: &Change<'_>) {
        let mut store = self.store();
        tag::declare(Id::parse(id).expect("an id"), change, 32, &mut store).expect("declare");
    }

    fn tagged(&self) -> Run {
        self.run(&["tag"])
    }
}

fn name(text: &str) -> Tag {
    Tag::new(text).expect("a tag")
}

fn usage_after(reason: &str) -> Vec<String> {
    std::iter::once(reason)
        .chain(USAGE[..4].iter().copied())
        .map(str::to_owned)
        .collect()
}

fn none_carried(run: &Run) {
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.stdout, "");
    assert_eq!(run.stderr, ["bilbo: no entry carries a tag"]);
}

#[test]
fn a_tag_added_is_listed_with_its_entries_and_read_shows_it() {
    let sandbox = Sandbox::with_notes("tag_added", &[("a.md", ID_A)]);

    let added = sandbox.run(&["tag", ID_A, "--add", "garden"]);
    assert_eq!(added.code, 0, "{:?}", added.stderr);
    assert_eq!(added.stdout, "changed 1, unchanged 0\n");
    assert_eq!(added.stderr, ["bilbo: tag:garden is new"]);

    let tags = sandbox.tagged();
    assert_eq!(tags.stdout, "tag:garden\t1 entries\tmarkdown 1\n");
    let entries = sandbox.run(&["tag", "tag:garden"]);
    assert_eq!(entries.code, 0, "{:?}", entries.stderr);
    assert_eq!(
        entries.stdout,
        format!(
            "{ID_A}\tmarkdown\tfile:{}\tperson\n",
            sandbox.path("notes/a.md")
        )
    );
    let read = sandbox.run(&["read", ID_A]);
    assert!(
        read.stdout.lines().any(|line| line == "label: tag:garden"),
        "{}",
        read.stdout
    );
}

#[test]
fn a_tag_removed_is_kept_as_removed_and_read_shows_it() {
    let sandbox = Sandbox::with_notes("tag_removed", &[("a.md", ID_A)]);
    sandbox.run(&["tag", ID_A, "--add", "garden"]);

    let removed = sandbox.run(&["tag", ID_A, "--remove", "garden"]);

    assert_eq!(removed.code, 0, "{:?}", removed.stderr);
    assert_eq!(removed.stdout, "changed 1, unchanged 0\n");
    none_carried(&sandbox.tagged());
    let read = sandbox.run(&["read", ID_A]);
    assert!(
        read.stdout
            .lines()
            .any(|line| line == "removed: tag:garden")
    );
    assert!(!read.stdout.lines().any(|line| line == "label: tag:garden"));
}

#[test]
fn adding_a_models_tag_makes_it_yours_and_says_changed() {
    let sandbox = Sandbox::with_notes("tag_models", &[("a.md", ID_A)]);
    let picks = [name("a")];
    sandbox.declare(
        ID_A,
        &Change::Model {
            picks: &picks,
            takes_back: false,
        },
    );
    assert!(sandbox.run(&["tag", "tag:a"]).stdout.ends_with("\tmodel\n"));

    let added = sandbox.run(&["tag", ID_A, "--add", "a"]);

    assert_eq!(added.code, 0, "{:?}", added.stderr);
    assert_eq!(added.stdout, "changed 1, unchanged 0\n");
    assert!(
        sandbox
            .run(&["tag", "tag:a"])
            .stdout
            .ends_with("\tperson\n")
    );
}

#[test]
fn a_tag_no_entry_carries_exits_1_and_a_store_with_no_tag_exits_0() {
    let sandbox = Sandbox::with_notes("tag_none", &[("a.md", ID_A)]);

    let missing = sandbox.run(&["tag", "tag:shed"]);
    assert_eq!(missing.code, 1);
    assert_eq!(missing.stdout, "");
    assert_eq!(missing.stderr, ["bilbo: tag:shed names no entry"]);

    none_carried(&sandbox.tagged());
}

#[test]
fn a_ref_that_names_no_entry_changes_nothing_and_exits_1() {
    let sandbox = Sandbox::with_notes("tag_no_entry", &[("a.md", ID_A)]);
    for missing in [
        ID_GONE,
        "ZZZZZZZZZZ",
        "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03",
    ] {
        let run = sandbox.run(&["tag", ID_A, missing, "--add", "garden"]);
        let read = sandbox.run(&["read", missing]);
        assert_eq!(run.code, 1, "{missing}: {:?}", run.stderr);
        assert_eq!(read.code, 1, "{missing}");
        assert_eq!(run.stdout, "");
        assert_eq!(run.stderr, read.stderr, "{missing}");
    }
    none_carried(&sandbox.tagged());
}

#[test]
fn an_entry_past_the_cap_changes_nothing_and_exits_1() {
    let sandbox = Sandbox::with_notes("tag_cap", &[("a.md", ID_A), ("b.md", ID_B)]);
    sandbox.run(&["tag", ID_A, "--add", "x"]);

    let run = sandbox.run_with(
        &["tag", ID_B, ID_A, "--add", "y"],
        &[("BILBO_TAG_CAP", "1")],
    );

    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: {ID_A} would carry more than tag.cap tags (1); nothing was changed"
        )]
    );
    assert_eq!(sandbox.tagged().stdout, "tag:x\t1 entries\tmarkdown 1\n");
}

#[test]
fn each_usage_error_exits_2_with_its_sentence_and_the_usage() {
    let sandbox = Sandbox::with_notes("tag_usage", &[("a.md", ID_A), ("twin.md", ID_TWIN)]);
    let not_a_tag = format!("bilbo: --add 'Rust' is not a tag: {GRAMMAR}");
    let cases: [(Vec<&str>, String); 4] = [
        (
            vec!["tag", ID_A],
            format!("bilbo: give --add or --remove to change the tags of '{ID_A}'"),
        ),
        (vec!["tag", ID_A, "--add", "Rust"], not_a_tag),
        (
            vec!["tag", ID_A, "--add", "a", "--remove", "a"],
            "bilbo: tag:a is both added and removed".to_owned(),
        ),
        (vec!["tag", "bilbo:{ID_A}#r1", "--add", "a"], String::new()),
    ];
    for (args, reason) in cases {
        let args: Vec<String> = args.iter().map(|arg| arg.replace("{ID_A}", ID_A)).collect();
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let reason = if reason.is_empty() {
            format!("bilbo: 'bilbo:{ID_A}#r1' names records; a tag is on an entry")
        } else {
            reason
        };
        let run = sandbox.run(&args);
        assert_eq!(run.code, 2, "{args:?}: {:?}", run.stderr);
        assert_eq!(run.stdout, "", "{args:?}");
        assert_eq!(run.stderr, usage_after(&reason), "{args:?}");
    }

    for refs in [vec!["zzz"], vec![ID_A, SHORT_A]] {
        let mut args = vec!["tag"];
        args.extend(&refs);
        args.extend(["--add", "a"]);
        let run = sandbox.run(&args);
        let read = sandbox.run(&["read", refs[refs.len() - 1]]);
        assert_eq!(run.code, 2, "{refs:?}: {:?}", run.stderr);
        assert_eq!(read.code, 2, "{refs:?}");
        assert_eq!(run.stderr[0], read.stderr[0], "{refs:?}");
        assert_eq!(run.stderr[1..], usage_after("")[1..], "{refs:?}");
    }
    none_carried(&sandbox.tagged());
}

fn run_noticed(vars: HashMap<String, OsString>, args: &[String], notices: Sender<String>) -> Run {
    let argv: Vec<OsString> = std::iter::once("bilbo".to_owned())
        .chain(args.iter().cloned())
        .map(OsString::from)
        .collect();
    let mut env = Env {
        var: Rc::new(move |name| vars.get(name).cloned()),
        stdin: Box::new(io::empty()),
        who: Who::Nobody,
        notice: Box::new(move |message| {
            notices.send(message.plain_lines().join("\n")).ok();
        }),
    };
    let settings = settings::load(
        &*env.var,
        &|path| fs::read(path),
        &bilbo_cli::kind_keys(),
        &bilbo_cli::provider_keys(),
    )
    .map_err(Failure::from);
    let (code, stdout, stderr) = match bilbo_cli::run(&argv, &mut env, settings) {
        Ok(outcome) => (
            outcome.exit_code(),
            outcome.stdout(&View::PLAIN),
            outcome.stderr_lines(&View::PLAIN),
        ),
        Err(failure) => (
            failure.exit_code(),
            String::new(),
            failure.stderr_lines(&View::PLAIN),
        ),
    };
    Run {
        code,
        stdout,
        stderr,
    }
}

#[test]
fn tag_waits_for_another_writer_and_changes_after_it() {
    let sandbox = Sandbox::with_notes("tag_waits", &[("a.md", ID_A)]);
    let layout = sandbox.layout();
    let held: Vec<fs::File> = ["store.lock", "store.writer"]
        .into_iter()
        .map(|name| {
            let file = fs::File::create(layout.state.join(name)).expect("a lock file");
            file.lock().expect("take the lock");
            file
        })
        .collect();

    let (sender, receiver) = mpsc::channel();
    let vars = sandbox.vars(&[]);
    let args = ["tag", ID_A, "--add", "garden"].map(str::to_owned);
    let runner = thread::spawn(move || run_noticed(vars, &args, sender));

    assert_eq!(
        receiver.recv_timeout(WAIT).expect("a waiting notice"),
        WAITING
    );
    for file in &held {
        file.unlock().expect("release the lock");
    }
    let finished = runner.join().expect("the run finishes");
    assert_eq!(finished.code, 0, "{:?}", finished.stderr);
    assert_eq!(finished.stdout, "changed 1, unchanged 0\n");
}

#[test]
fn a_change_waits_for_another_writer_whatever_store_wait_says() {
    let sandbox = Sandbox::with_notes("tag_waits_without_bound", &[("a.md", ID_A)]);
    let layout = sandbox.layout();
    let held: Vec<fs::File> = ["store.lock", "store.writer"]
        .into_iter()
        .map(|name| {
            let file = fs::File::create(layout.state.join(name)).expect("a lock file");
            file.lock().expect("take the lock");
            file
        })
        .collect();

    let (sender, receiver) = mpsc::channel();
    let vars = sandbox.vars(&[("BILBO_STORE_WAIT", "0s")]);
    let args = ["tag", ID_A, "--add", "garden"].map(str::to_owned);
    let runner = thread::spawn(move || run_noticed(vars, &args, sender));

    assert_eq!(
        receiver.recv_timeout(WAIT).expect("a waiting notice"),
        WAITING
    );
    for file in &held {
        file.unlock().expect("release the lock");
    }
    let finished = runner.join().expect("the run finishes");
    assert_eq!(finished.code, 0, "{:?}", finished.stderr);
    assert_eq!(finished.stdout, "changed 1, unchanged 0\n");
}

fn files_under(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("list") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push((path.clone(), fs::read(&path).expect("read")));
            }
        }
    }
    found.sort();
    found
}

#[test]
fn a_tag_change_makes_no_index_run_and_recall_sees_it_at_once() {
    let sandbox = Sandbox::with_notes("tag_no_index", &[("a.md", ID_A), ("b.md", ID_B)]);
    let before = files_under(&sandbox.dir.join("cache/bilbo/index"));

    let added = sandbox.run(&["tag", ID_A, "--add", "x"]);
    assert_eq!(added.code, 0, "{:?}", added.stderr);

    assert!(files_under(&sandbox.dir.join("cache/bilbo/index")) == before);
    let run = sandbox.run(&["recall", "--in", "tag:x", "marmalade"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let short = |id: &str| Id::parse(id).expect("an id").short().to_string();
    assert!(run.stdout.contains(&short(ID_A)), "{}", run.stdout);
    assert!(!run.stdout.contains(&short(ID_B)), "{}", run.stdout);
    assert!(
        run.stderr.iter().all(|line| {
            !line.starts_with("bilbo: the index is behind")
                && !line.starts_with("bilbo: --in may name fewer")
        }),
        "{:?}",
        run.stderr
    );
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
fn tag_serves_a_refresh_that_found_it_holding_the_store() {
    let sandbox = Sandbox::with_notes("tag_serves", &[("a.md", ID_A)]);
    let followed = sandbox.dir.join("follow");
    fs::create_dir_all(&followed).expect("a folder");
    let file = followed.join("new.md");
    fs::write(&file, "# New\n\nA note kept by the pass that tag serves.\n").expect("write");
    fs::File::options()
        .write(true)
        .open(&file)
        .expect("open")
        .set_modified(SystemTime::now() - Duration::from_secs(3600))
        .expect("backdate");
    fs::write(
        sandbox.dir.join("config/config.toml"),
        format!("[[follow]]\npath = {:?}\nkind = \"markdown\"\n", followed),
    )
    .expect("a config that follows a folder");
    let layout = sandbox.layout();
    let mut open_seg = fs::OpenOptions::new()
        .append(true)
        .open(layout.state.join("open.seg"))
        .expect("open.seg");
    io::Write::write_all(&mut open_seg, &[1, 0, 0]).expect("a torn tail");
    drop(open_seg);

    let bare = Rc::new(std::cell::RefCell::new(None));
    let vars = sandbox.vars(&[]);
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
    let outcome = outcome_of(vars, &["tag", ID_A, "--add", "garden"], notice).expect("the tag run");

    let bare = bare.borrow();
    let bare = bare.as_ref().expect("the bare run");
    assert_eq!(bare.code, 0, "{:?}", bare.stderr);
    assert_eq!(bare.stdout, "");
    assert_eq!(
        bare.stderr,
        ["bilbo: another bilbo is writing the store; requested another pass over the follows"]
    );
    assert_eq!(outcome.exit_code(), 0);
    assert_eq!(outcome.stdout(&View::PLAIN), "changed 1, unchanged 0\n");
    let said = outcome.stderr_lines(&View::PLAIN);
    assert_eq!(said.len(), 2, "{said:?}");
    assert_eq!(said[0], "bilbo: tag:garden is new");
    assert!(
        said[1].starts_with("bilbo: ran the follows again for another bilbo: new 1, "),
        "{said:?}"
    );
    assert_eq!(
        fs::metadata(layout.state.join("store.again"))
            .expect("the flag")
            .len(),
        0
    );
    let store = Store::open_read_only(&layout, Options::default(), &mut |_| {}).expect("open");
    let label = CaptureLabel::file(&file.canonicalize().expect("canonical")).expect("a label");
    assert!(store.by_capture(&label).expect("a lookup").is_some());
}

#[test]
fn a_change_opens_nothing_for_writing_until_every_ref_is_known_good() {
    let sandbox = Sandbox::new("tag_known_good");
    let layout = sandbox.layout();
    let before = files_under(&sandbox.dir);
    let run = sandbox.run(&["tag", ID_A, "--add", "garden"]);
    let read = sandbox.run(&["read", ID_A]);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(run.code, read.code);
    assert_eq!(run.stderr, read.stderr);
    assert!(
        run.stderr[0].starts_with("bilbo: no store at"),
        "{:?}",
        run.stderr
    );
    assert!(files_under(&sandbox.dir) == before);
    assert!(!layout.root.exists(), "no root was made");
    assert!(!layout.state.join("open.seg").exists());

    let sandbox = Sandbox::with_notes("tag_known_good_catalog", &[("a.md", ID_A)]);
    let catalog = sandbox.dir.join("cache/bilbo/catalog.sqlite");
    fs::remove_file(&catalog).expect("remove the catalog");
    let run = sandbox.run(&["tag", ID_GONE, "--add", "garden"]);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(run.stderr, [format!("bilbo: no entry {ID_GONE}")]);
    assert!(!catalog.exists(), "the catalog was not made again");
}

#[test]
fn the_listings_escape_for_the_terminal_they_are_shown_on() {
    let sandbox = Sandbox::with_notes("tag_terminal", &[("a.md", ID_A)]);
    let label = CaptureLabel::new("file:/n/\u{202e}a.md".to_owned()).expect("a label");
    let tags = Tags::default();
    let mut store = sandbox.store();
    let kept = store
        .capture(&Capture {
            label: &label,
            kind: "markdown",
            id: None,
            id_label: None,
            tags: &tags,
            stamp: None,
            bytes: b"one\n",
            ends: &[4],
        })
        .expect("kept");
    let bilbo_store::entries::Captured::New(kept) = kept else {
        panic!("a new entry");
    };
    let add = std::collections::BTreeSet::from([name("garden")]);
    let none = std::collections::BTreeSet::new();
    tag::declare(
        kept.id,
        &Change::Person {
            add: &add,
            remove: &none,
        },
        32,
        &mut store,
    )
    .expect("declare");
    drop(store);

    let outcome = outcome_of(sandbox.vars(&[]), &["tag", "tag:garden"], Box::new(|_| {}))
        .expect("the listing");
    let terminal = View {
        terminal: true,
        human: false,
        paint: false,
        unicode: false,
    };
    let shown = outcome.stdout(&terminal);
    assert!(!shown.contains('\u{202e}'), "{shown:?}");
    assert!(shown.contains("\\u{202e}"), "{shown:?}");
    assert!(outcome.stdout(&View::PLAIN).contains('\u{202e}'));
}
