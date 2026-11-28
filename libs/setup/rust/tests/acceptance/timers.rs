use std::cell::RefCell;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::time::Duration;

use bilbo_setup::report::{Line, Status};
use bilbo_setup::timer::{Files, Job, Left, Loaded, LogPath, Manager, Scheduler, removal, step};

type Site = (&'static str, Box<dyn Fn(&mut Job, OsString)>);

fn job() -> Job {
    Job {
        step: "refresh-timer",
        verb: "refresh",
        interval: Duration::from_secs(600),
        program: Some(PathBuf::from("/opt/bilbo/bin/bilbo")),
        log: PathBuf::from("/l/refresh.log"),
        variables: vec![("HOME".to_owned(), OsString::from("/home/me"))],
        path: vec![PathBuf::from("/usr/bin")],
        clause: None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Written {
    None,
    Old,
    New,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stuck {
    No,
    Files,
    Loaded,
    Both,
}

struct Mem {
    files: RefCell<Written>,
    loaded: RefCell<bool>,
    something: RefCell<bool>,
    folder: RefCell<bool>,
    log: LogPath,
    manager: Manager,
    acts: RefCell<Vec<&'static str>>,
    fails: Option<&'static str>,
    never_loads: bool,
    stuck: Stuck,
    ran: bool,
    looked: RefCell<Vec<Left>>,
    handed: RefCell<Vec<Left>>,
    looks: RefCell<Vec<&'static str>>,
    broken: Option<&'static str>,
}

impl Mem {
    fn new(files: Written, loaded: bool, folder: bool) -> Mem {
        Mem {
            files: RefCell::new(files),
            loaded: RefCell::new(loaded),
            something: RefCell::new(loaded),
            folder: RefCell::new(folder),
            log: LogPath::Regular,
            manager: Manager::Ready,
            acts: RefCell::new(Vec::new()),
            fails: None,
            never_loads: false,
            stuck: Stuck::No,
            ran: true,
            looked: RefCell::new(Vec::new()),
            handed: RefCell::new(Vec::new()),
            looks: RefCell::new(Vec::new()),
            broken: None,
        }
    }

    fn acts(&self) -> Vec<&'static str> {
        self.acts.borrow().clone()
    }

    fn act(&self, name: &'static str) -> Result<(), String> {
        self.acts.borrow_mut().push(name);
        if self.fails == Some(name) {
            Err(format!("fake {name} failed: boom"))
        } else {
            Ok(())
        }
    }
}

impl Scheduler for Mem {
    fn manager(&self) -> Manager {
        self.manager.clone()
    }

    fn label(&self, _job: &Job) -> String {
        "the.label".to_owned()
    }

    fn files(&self, _job: &Job) -> Result<Files, String> {
        self.looks.borrow_mut().push("files");
        if self.broken == Some("files") {
            return Err("cannot read /u/x: denied".to_owned());
        }
        Ok(match *self.files.borrow() {
            Written::None => Files::Absent,
            Written::Old => Files::Differ,
            Written::New => Files::Wanted,
        })
    }

    fn loaded(&self, _job: &Job) -> Result<Loaded, String> {
        self.looks.borrow_mut().push("loaded");
        Ok(Loaded {
            loaded: *self.loaded.borrow(),
            something: *self.something.borrow(),
            running: *self.loaded.borrow(),
        })
    }

    fn log_path(&self, _job: &Job) -> Result<LogPath, String> {
        self.looks.borrow_mut().push("log_path");
        if self.broken == Some("log_path") {
            return Err("cannot read /l/refresh.log: denied".to_owned());
        }
        Ok(self.log)
    }

    fn log_folder(&self, _job: &Job) -> Result<bool, String> {
        self.looks.borrow_mut().push("log_folder");
        if self.broken == Some("log_folder") {
            return Err("cannot read /l: denied".to_owned());
        }
        Ok(*self.folder.borrow())
    }

    fn make_log_folder(&self, _job: &Job) -> Result<(), String> {
        self.act("folder")?;
        *self.folder.borrow_mut() = true;
        Ok(())
    }

    fn unload(&self, _job: &Job, _seen: Loaded) -> Result<(), String> {
        self.act("unload")?;
        *self.loaded.borrow_mut() = false;
        *self.something.borrow_mut() = false;
        Ok(())
    }

    fn write(&self, _job: &Job) -> Result<(), String> {
        self.act("write")?;
        *self.files.borrow_mut() = Written::New;
        Ok(())
    }

    fn left(&self, _job: &Job) -> Result<Left, String> {
        self.looks.borrow_mut().push("left");
        if self.broken == Some("left") {
            return Err("cannot read /u/x: denied".to_owned());
        }
        let found = Left {
            there: if *self.files.borrow() == Written::None {
                Vec::new()
            } else {
                vec![PathBuf::from("/u/the.label.plist")]
            },
            loaded: if *self.loaded.borrow() {
                vec!["the.label".to_owned()]
            } else {
                Vec::new()
            },
            ..Left::default()
        };
        self.looked.borrow_mut().push(found.clone());
        Ok(found)
    }

    fn take_out(&self, _job: &Job, found: &Left) -> Result<Option<String>, String> {
        self.act("take_out")?;
        self.handed.borrow_mut().push(found.clone());
        if self.stuck != Stuck::Files && self.stuck != Stuck::Both {
            *self.files.borrow_mut() = Written::None;
        }
        if self.stuck != Stuck::Loaded && self.stuck != Stuck::Both {
            *self.loaded.borrow_mut() = false;
            *self.something.borrow_mut() = false;
        }
        Ok(self.ran.then(|| "do unload".to_owned()))
    }

    fn load(&self, _job: &Job) -> Result<String, String> {
        self.act("load")?;
        if !self.never_loads {
            *self.loaded.borrow_mut() = true;
            *self.something.borrow_mut() = true;
        }
        Ok("do load".to_owned())
    }
}

fn run(mem: &Mem) -> Line {
    step(mem, &job())
}

const HEAD: &str = "every 10m, log /l/refresh.log";

#[test]
fn a_timer_is_kept_only_when_its_files_are_the_texts_it_is_loaded_and_its_log_has_a_folder() {
    let rows: Vec<(Mem, Status, Vec<&str>, &str)> = vec![
        (
            Mem::new(Written::New, true, true),
            Status::Kept,
            vec![],
            HEAD,
        ),
        (
            Mem::new(Written::New, true, false),
            Status::Updated,
            vec!["folder"],
            "every 10m, log /l/refresh.log; its log's folder was gone",
        ),
        (
            Mem::new(Written::New, false, true),
            Status::Updated,
            vec!["load"],
            "every 10m, log /l/refresh.log; it was not loaded",
        ),
        (
            Mem::new(Written::New, false, false),
            Status::Updated,
            vec!["folder", "load"],
            "every 10m, log /l/refresh.log; it was not loaded",
        ),
        (
            Mem::new(Written::None, false, true),
            Status::Installed,
            vec!["folder", "write", "load"],
            HEAD,
        ),
        (
            Mem::new(Written::Old, true, true),
            Status::Updated,
            vec!["unload", "folder", "write", "load"],
            "every 10m, log /l/refresh.log; its unit differed",
        ),
        (
            Mem::new(Written::None, true, true),
            Status::Updated,
            vec!["unload", "folder", "write", "load"],
            "every 10m, log /l/refresh.log; its unit differed",
        ),
    ];
    for (at, (mem, status, acts, detail)) in rows.into_iter().enumerate() {
        let line = run(&mem);
        assert_eq!(line.status, status, "row {at}: {}", line.detail);
        assert_eq!(mem.acts(), acts, "row {at}");
        assert_eq!(line.detail, detail, "row {at}");
    }
}

#[test]
fn a_timer_not_loaded_after_the_load_is_failed() {
    let mut mem = Mem::new(Written::None, false, true);
    mem.never_loads = true;
    let line = run(&mem);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (Status::Failed, "the.label is not loaded after do load")
    );
}

#[test]
fn a_failed_change_is_finished_by_the_next_run() {
    let mut first = Mem::new(Written::Old, true, true);
    first.fails = Some("unload");
    let line = run(&first);
    assert_eq!(line.status, Status::Failed);
    assert_eq!(*first.files.borrow(), Written::Old, "no file was written");
    first.fails = None;
    let line = run(&first);
    assert_eq!(line.status, Status::Updated, "{}", line.detail);
    assert!(line.detail.ends_with("its unit differed"));
    assert_eq!(first.acts()[1..], ["unload", "folder", "write", "load"]);

    let mut second = Mem::new(Written::Old, true, true);
    second.fails = Some("load");
    let line = run(&second);
    assert_eq!(line.status, Status::Failed);
    assert_eq!(*second.files.borrow(), Written::New);
    assert!(!*second.loaded.borrow());
    second.fails = None;
    let line = run(&second);
    assert_eq!(line.status, Status::Updated, "{}", line.detail);
    assert!(
        line.detail.ends_with("; it was not loaded"),
        "{}",
        line.detail
    );
}

#[test]
fn a_scheduler_that_does_not_answer_gives_its_own_row() {
    let ready = Mem::new(Written::None, false, false);
    assert_eq!(run(&ready).status, Status::Installed);
    for (manager, status, detail) in [
        (
            Manager::Not(
                Status::Skipped,
                "no systemd user session answers".to_owned(),
            ),
            Status::Skipped,
            "no systemd user session answers",
        ),
        (
            Manager::Not(
                Status::Failed,
                "systemctl --user is-system-running answered 'stopping'".to_owned(),
            ),
            Status::Failed,
            "systemctl --user is-system-running answered 'stopping'",
        ),
        (
            Manager::Not(
                Status::Failed,
                "systemctl --user is-system-running answered nothing".to_owned(),
            ),
            Status::Failed,
            "systemctl --user is-system-running answered nothing",
        ),
        (
            Manager::Not(
                Status::Failed,
                "cannot start /bin/systemctl: no such file".to_owned(),
            ),
            Status::Failed,
            "cannot start /bin/systemctl: no such file",
        ),
        (
            Manager::Not(
                Status::Failed,
                "systemctl --user is-system-running: it did not end within 15s".to_owned(),
            ),
            Status::Failed,
            "systemctl --user is-system-running: it did not end within 15s",
        ),
        (
            Manager::Not(Status::Skipped, "systemctl is not on PATH".to_owned()),
            Status::Skipped,
            "systemctl is not on PATH",
        ),
    ] {
        let mut mem = Mem::new(Written::None, false, false);
        mem.manager = manager;
        let line = run(&mem);
        assert_eq!((line.status, line.detail.as_str()), (status, detail));
        assert!(mem.acts().is_empty(), "nothing was written");
        assert!(
            mem.looks.borrow().is_empty(),
            "nothing was asked of the double"
        );
    }
}

#[test]
fn a_look_that_could_not_be_made_ends_the_step_before_any_change() {
    for (which, sentence) in [
        ("files", "cannot read /u/x: denied"),
        ("log_path", "cannot read /l/refresh.log: denied"),
        ("log_folder", "cannot read /l: denied"),
    ] {
        let mut mem = Mem::new(Written::New, true, true);
        mem.broken = Some(which);
        let line = run(&mem);
        assert_eq!(
            (line.status, line.detail.as_str()),
            (Status::Failed, sentence),
            "{which}"
        );
        assert!(
            mem.acts().is_empty(),
            "{which}: no unload, no write, no load"
        );
    }
}

#[test]
fn a_value_a_unit_cannot_carry_is_refused() {
    let sites: [Site; 4] = [
        (
            "this bilbo's path",
            Box::new(|job, value| job.program = Some(PathBuf::from(value))),
        ),
        (
            "the log's path",
            Box::new(|job, value| job.log = PathBuf::from(value)),
        ),
        (
            "HOME",
            Box::new(|job, value| job.variables = vec![("HOME".to_owned(), value)]),
        ),
        (
            "a folder of PATH",
            Box::new(|job, value| job.path = vec![PathBuf::from(value)]),
        ),
    ];
    let characters = [
        ('\n', "U+000A"),
        ('\u{1}', "U+0001"),
        ('\u{7f}', "U+007F"),
        ('\u{fffe}', "U+FFFE"),
        ('\u{ffff}', "U+FFFF"),
    ];
    for (what, set) in &sites {
        let mut inputs: Vec<(OsString, String)> = characters
            .iter()
            .map(|(c, code)| {
                (
                    OsString::from(format!("/v{c}x")),
                    format!("{what} holds {code}, which a unit cannot carry"),
                )
            })
            .collect();
        inputs.push((
            OsString::from_vec(vec![b'/', 0xff]),
            format!("{what} is not UTF-8, which a unit cannot carry"),
        ));
        inputs.push((
            OsString::from("/v\u{1}\n"),
            format!("{what} holds U+0001, which a unit cannot carry"),
        ));
        for (value, sentence) in inputs {
            let mem = Mem::new(Written::Old, true, true);
            let mut changed = job();
            set(&mut changed, value.clone());
            let line = step(&mem, &changed);
            assert_eq!(
                (line.status, line.detail),
                (Status::Failed, sentence),
                "{what}: {value:?}"
            );
            assert!(
                mem.acts().is_empty(),
                "{what}: nothing was unloaded or written"
            );
        }
        for carried in ["/v\u{fffd}", "/v\u{80}"] {
            let mem = Mem::new(Written::New, true, true);
            let mut fine = job();
            set(&mut fine, OsString::from(carried));
            assert_eq!(step(&mem, &fine).status, Status::Kept, "{what}: {carried}");
        }
    }
    let mem = Mem::new(Written::Old, true, true);
    let mut colon = job();
    colon.path = vec![PathBuf::from("/a:b")];
    let line = step(&mem, &colon);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (
            Status::Failed,
            "the folder /a:b holds a colon, which PATH cannot carry"
        )
    );
    assert!(mem.acts().is_empty());
    let mem = Mem::new(Written::New, true, true);
    let mut none = job();
    none.program = None;
    let line = step(&mem, &none);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (
            Status::Failed,
            "cannot find this bilbo's own path, which a timer runs"
        )
    );
}

#[test]
fn the_use_case_fails_a_log_that_is_a_link_or_no_regular_file_before_anything_is_asked() {
    for (log, sentence) in [
        (LogPath::Link, "the log /l/refresh.log is a symbolic link"),
        (
            LogPath::Other,
            "the log /l/refresh.log is not a regular file",
        ),
    ] {
        for mem in [
            Mem::new(Written::None, false, true),
            Mem::new(Written::New, true, true),
        ] {
            let mem = Mem { log, ..mem };
            let line = run(&mem);
            assert_eq!(
                (line.status, line.detail.as_str()),
                (Status::Failed, sentence)
            );
            assert!(mem.acts().is_empty());
        }
    }
}

#[test]
fn a_timers_removal_unloads_then_deletes() {
    let mem = Mem::new(Written::New, true, true);
    let line = removal(&mem, &job());
    assert_eq!(
        (line.step, line.status, line.detail.as_str()),
        ("refresh-timer", Status::Removed, "the.label")
    );
    assert_eq!(mem.acts(), ["take_out"]);
    assert_eq!(*mem.looks.borrow(), ["left", "left"]);
    assert_eq!(
        *mem.handed.borrow(),
        [mem.looked.borrow()[0].clone()],
        "take_out goes by exactly what the first look gave"
    );

    let again = removal(&mem, &job());
    assert_eq!(
        (again.status, again.detail.as_str()),
        (Status::Absent, "no timer the.label")
    );
    assert_eq!(mem.acts(), ["take_out"], "a second removal changes nothing");

    for (stuck, ran, said) in [
        (
            Stuck::Files,
            true,
            "cannot delete /u/the.label.plist: it is still there",
        ),
        (
            Stuck::Files,
            false,
            "cannot delete /u/the.label.plist: it is still there",
        ),
        (
            Stuck::Both,
            true,
            "cannot delete /u/the.label.plist: it is still there",
        ),
        (
            Stuck::Loaded,
            true,
            "the.label is still loaded after do unload",
        ),
        (Stuck::Loaded, false, "the.label is still loaded"),
    ] {
        let mut mem = Mem::new(Written::New, true, true);
        mem.stuck = stuck;
        mem.ran = ran;
        let line = removal(&mem, &job());
        assert_eq!(
            (line.status, line.detail.as_str()),
            (Status::Failed, said),
            "{stuck:?} {ran}"
        );
    }

    let mut broken = Mem::new(Written::New, true, true);
    broken.broken = Some("left");
    let line = removal(&broken, &job());
    assert_eq!(line.status, Status::Failed);
    assert_eq!(line.detail, "cannot read /u/x: denied");
    assert!(
        broken.acts().is_empty(),
        "a question that failed is not read as nothing there"
    );

    let mut failing = Mem::new(Written::New, true, true);
    failing.fails = Some("take_out");
    let line = removal(&failing, &job());
    assert_eq!(
        (line.status, line.detail.as_str()),
        (Status::Failed, "fake take_out failed: boom")
    );
    assert_eq!(*failing.looks.borrow(), ["left"]);

    let mut away = Mem::new(Written::None, false, false);
    away.manager = Manager::Not(
        Status::Skipped,
        "no systemd user session answers".to_owned(),
    );
    let line = removal(&away, &job());
    assert_eq!(line.status, Status::Skipped);
    assert!(away.acts().is_empty());
}
