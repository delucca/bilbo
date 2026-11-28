use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use bilbo_settings::file::File;
use bilbo_settings::key::Source;
use bilbo_setup::keys::section;
use bilbo_setup::plugin::{
    Act, Agent, Held, Hook, Hooks, Installed, Marketplace, Seen, Stale, Standing, Trusting,
};
use bilbo_setup::report::{Line, Status};
use bilbo_setup::steps::{Notes, Ports, Request, Taking, Timing, Tool, Way, remove, run};
use bilbo_setup::timer::{Files, Job, Left, Loaded, LogPath, Manager, Scheduler};

type Log = Rc<RefCell<Vec<String>>>;

struct Fake {
    step: &'static str,
    log: Log,
    manifest: Result<(), String>,
    state: RefCell<Option<bool>>,
    gives_hooks: bool,
    keys: Rc<RefCell<Vec<String>>>,
    opts: Opts,
}

#[derive(Clone, Default)]
struct Opts {
    other: Option<bilbo_setup::plugin::Other>,
    held_fails: Option<String>,
    run_fails: Option<(Act, String)>,
    forget_fails: bool,
}

struct Trusted {
    step: &'static str,
    log: Log,
    keys: Rc<RefCell<Vec<String>>>,
    drops: bool,
    fails: bool,
}

impl Hooks for Trusted {
    fn list(&mut self) -> Result<Vec<Hook>, String> {
        self.log.borrow_mut().push(format!("{} list", self.step));
        Ok(vec![Hook {
            key: "k".to_owned(),
            hash: "h".to_owned(),
            standing: Standing::Trusted,
        }])
    }

    fn trust(&mut self, _hooks: &[Hook]) -> Result<(), String> {
        self.log.borrow_mut().push(format!("{} trust", self.step));
        Ok(())
    }

    fn kept(&mut self) -> Result<Vec<String>, String> {
        self.log.borrow_mut().push(format!("{} kept", self.step));
        Ok(self.keys.borrow().clone())
    }

    fn forget(&mut self, keys: &[String]) -> Result<(), String> {
        self.log.borrow_mut().push(format!("{} forget", self.step));
        if self.fails {
            return Err("config/batchWrite failed".to_owned());
        }
        if self.drops {
            self.keys.borrow_mut().retain(|key| !keys.contains(key));
        }
        Ok(())
    }

    fn close(&mut self) {}
}

impl Agent for Fake {
    fn step(&self) -> &'static str {
        self.step
    }

    fn name(&self) -> &'static str {
        self.step
    }

    fn id(&self) -> &'static str {
        "bilbo@bilbo"
    }

    fn manifest(&self, _folder: &Path) -> Result<(), String> {
        self.log
            .borrow_mut()
            .push(format!("{} manifest", self.step));
        self.manifest.clone()
    }

    fn look(&self, _folder: &Path) -> Result<Seen, String> {
        self.log.borrow_mut().push(format!("{} look", self.step));
        Ok(Seen {
            marketplace: Marketplace::Here,
            plugin: self.state.borrow().map(|stale| Installed {
                on: true,
                stale: stale.then(|| Stale::Version("0.1".to_owned())),
            }),
            other: None,
        })
    }

    fn run(&self, act: Act, _folder: &Path) -> Result<(), String> {
        self.log.borrow_mut().push(format!("{} {act:?}", self.step));
        if let Some((failing, said)) = &self.opts.run_fails
            && *failing == act
        {
            return Err(said.clone());
        }
        *self.state.borrow_mut() = match act {
            Act::Install => Some(false),
            _ => None,
        };
        Ok(())
    }

    fn words(&self, _act: Act, _folder: &Path) -> String {
        "words".to_owned()
    }

    fn hooks(&self, _folder: &Path) -> Option<Trusting> {
        self.log.borrow_mut().push(format!("{} hooks", self.step));
        self.gives_hooks.then(|| Trusting {
            step: "codex-hooks",
            hooks: Box::new(Trusted {
                step: self.step,
                log: self.log.clone(),
                keys: self.keys.clone(),
                drops: true,
                fails: self.opts.forget_fails,
            }),
        })
    }

    fn held(&self) -> Result<Held, String> {
        self.log.borrow_mut().push(format!("{} held", self.step));
        if let Some(said) = &self.opts.held_fails {
            return Err(said.clone());
        }
        let there = self.state.borrow().is_some();
        Ok(Held {
            plugin: there,
            marketplace: there,
            other: self.opts.other.clone(),
        })
    }
}

struct Timers {
    jobs: Rc<RefCell<Vec<Job>>>,
    log: Log,
    gone: RefCell<Vec<String>>,
}

impl Scheduler for Timers {
    fn manager(&self) -> Manager {
        Manager::Ready
    }

    fn label(&self, _job: &Job) -> String {
        "label".to_owned()
    }

    fn files(&self, job: &Job) -> Result<Files, String> {
        self.jobs.borrow_mut().push(job.clone());
        self.log.borrow_mut().push(job.verb.to_owned());
        Ok(Files::Wanted)
    }

    fn loaded(&self, _job: &Job) -> Result<Loaded, String> {
        Ok(Loaded {
            loaded: true,
            something: true,
            running: true,
        })
    }

    fn log_path(&self, _job: &Job) -> Result<LogPath, String> {
        Ok(LogPath::Regular)
    }

    fn log_folder(&self, _job: &Job) -> Result<bool, String> {
        Ok(true)
    }

    fn make_log_folder(&self, _job: &Job) -> Result<(), String> {
        Ok(())
    }

    fn unload(&self, _job: &Job, _seen: Loaded) -> Result<(), String> {
        Ok(())
    }

    fn write(&self, _job: &Job) -> Result<(), String> {
        Ok(())
    }

    fn load(&self, _job: &Job) -> Result<String, String> {
        Ok("loaded".to_owned())
    }

    fn left(&self, job: &Job) -> Result<Left, String> {
        self.log.borrow_mut().push(format!("{} left", job.verb));
        let gone = self.gone.borrow().iter().any(|verb| verb == job.verb);
        Ok(Left {
            loaded: if gone {
                Vec::new()
            } else {
                vec!["label".to_owned()]
            },
            ..Left::default()
        })
    }

    fn take_out(&self, job: &Job, _found: &Left) -> Result<Option<String>, String> {
        self.log.borrow_mut().push(format!("{} take_out", job.verb));
        self.gone.borrow_mut().push(job.verb.to_owned());
        Ok(Some("bootout".to_owned()))
    }
}

fn request(config: &str, clause: Option<&str>) -> Request {
    let file = File::parse(Path::new("/c/config.toml"), config.as_bytes()).expect("toml");
    let var = |_: &str| None::<OsString>;
    Request {
        plugin: Some(PathBuf::from("/pkg")),
        program: Some(PathBuf::from("/opt/bilbo/bin/bilbo")),
        logs: PathBuf::from("/l"),
        variables: vec![("HOME".to_owned(), OsString::from("/home/me"))],
        clause: clause.map(str::to_owned),
        section: section(&Source {
            var: &var,
            file: &file,
        })
        .expect("section"),
    }
}

fn found(
    step: &'static str,
    hooks: Option<&'static str>,
    state: Option<bool>,
    manifest: Result<(), String>,
    folder: &str,
    log: &Log,
) -> Tool {
    Tool {
        step,
        hooks,
        way: Way::Found {
            agent: Box::new(Fake {
                step,
                log: log.clone(),
                manifest,
                state: RefCell::new(state),
                gives_hooks: hooks.is_some(),
                opts: Opts::default(),
                keys: Rc::new(RefCell::new(if hooks.is_some() {
                    vec!["bilbo@bilbo:a".to_owned(), "bilbo@bilbo:b".to_owned()]
                } else {
                    Vec::new()
                })),
            }),
            folder: PathBuf::from(folder),
        },
    }
}

fn tool(step: &'static str, way: Way) -> Tool {
    Tool {
        step,
        hooks: (step == "codex").then_some("codex-hooks"),
        way,
    }
}

fn kept(step: &'static str, log: &Log, folder: &str) -> Tool {
    let hooks = (step == "codex").then_some("codex-hooks");
    found(step, hooks, Some(false), Ok(()), folder, log)
}

fn broken(step: &'static str, log: &Log) -> Tool {
    let hooks = (step == "codex").then_some("codex-hooks");
    found(
        step,
        hooks,
        None,
        Err("it is broken".to_owned()),
        "/bin",
        log,
    )
}

fn timers_in(log: &Log) -> (Timing, Rc<RefCell<Vec<Job>>>) {
    let jobs = Rc::new(RefCell::new(Vec::new()));
    let timers = Timers {
        jobs: jobs.clone(),
        log: log.clone(),
        gone: RefCell::new(Vec::new()),
    };
    (Timing::Has(Box::new(timers)), jobs)
}

fn timers() -> (Timing, Rc<RefCell<Vec<Job>>>) {
    timers_in(&Log::default())
}

fn bare(step: &'static str, hooks: &'static str, log: &Log) -> Tool {
    Tool {
        step,
        hooks: Some(hooks),
        way: Way::Found {
            agent: Box::new(Fake {
                step,
                log: log.clone(),
                manifest: Ok(()),
                state: RefCell::new(Some(false)),
                gives_hooks: false,
                keys: Rc::default(),
                opts: Opts::default(),
            }),
            folder: PathBuf::from("/bin"),
        },
    }
}

struct Folder {
    log: Option<Log>,
}

impl Notes for Folder {
    fn see_to(&self) -> Line {
        if let Some(log) = &self.log {
            log.borrow_mut().push("notes".to_owned());
        }
        Line {
            step: "notes",
            status: Status::Kept,
            detail: "/notes is followed as markdown".to_owned(),
        }
    }
}

fn ports(tools: [Tool; 3], scheduler: Timing) -> Ports<'static> {
    Ports {
        notes: Box::new(Folder { log: None }),
        tools,
        scheduler,
    }
}

fn ports_logged(tools: [Tool; 3], scheduler: Timing, log: &Log) -> Ports<'static> {
    Ports {
        notes: Box::new(Folder {
            log: Some(log.clone()),
        }),
        tools,
        scheduler,
    }
}

fn shown(lines: &[Line]) -> Vec<(&'static str, Status, String)> {
    lines
        .iter()
        .map(|line| (line.step, line.status, line.detail.clone()))
        .collect()
}

fn missing(value: &str, default: bool) -> Way {
    Way::Missing {
        value: value.to_owned(),
        default,
    }
}

#[test]
fn the_steps_run_in_their_order_and_none_stops_the_next() {
    let log = Log::default();
    let (scheduler, jobs) = timers_in(&log);
    let report = run(
        &request("", None),
        &ports_logged(
            [
                broken("claude", &log),
                broken("codex", &log),
                broken("agy", &log),
            ],
            scheduler,
            &log,
        ),
    );
    assert_eq!(
        *log.borrow(),
        [
            "notes",
            "claude manifest",
            "codex manifest",
            "agy manifest",
            "refresh",
            "derive"
        ]
    );
    let steps: Vec<&str> = report.lines.iter().map(|line| line.step).collect();
    assert_eq!(
        steps,
        [
            "notes",
            "claude",
            "codex",
            "codex-hooks",
            "agy",
            "refresh-timer",
            "derive-timer"
        ]
    );
    let statuses: Vec<Status> = report.lines.iter().map(|line| line.status).collect();
    assert_eq!(
        statuses,
        [
            Status::Kept,
            Status::Failed,
            Status::Failed,
            Status::Skipped,
            Status::Failed,
            Status::Kept,
            Status::Kept
        ]
    );
    assert_eq!(jobs.borrow().len(), 2, "both timers were asked");
}

#[test]
fn the_hooks_step_runs_only_when_codex_left_the_plugin_installed() {
    let cases: Vec<(&str, Option<bool>, Status)> = vec![
        ("kept", Some(false), Status::Kept),
        ("installed", None, Status::Installed),
        ("updated", Some(true), Status::Updated),
    ];
    for (name, state, status) in cases {
        let log = Log::default();
        let report = run(
            &request("", None),
            &ports(
                [
                    kept("claude", &log, "/bin"),
                    found("codex", Some("codex-hooks"), state, Ok(()), "/bin", &log),
                    kept("agy", &log, "/bin"),
                ],
                Timing::Absent,
            ),
        );
        let codex = &report.lines[2];
        assert_eq!(codex.status, status, "{name}");
        let hooks = &report.lines[3];
        assert_eq!(hooks.step, "codex-hooks", "{name}");
        assert_eq!(hooks.status, Status::Kept, "{name}: {}", hooks.detail);
        assert!(log.borrow().contains(&"codex list".to_owned()), "{name}");
        assert_eq!(
            report.lines.len(),
            7,
            "{name}: claude and agy have no hooks row"
        );
    }

    let failed_log = Log::default();
    let report = run(
        &request("", None),
        &ports(
            [
                kept("claude", &failed_log, "/bin"),
                broken("codex", &failed_log),
                kept("agy", &failed_log, "/bin"),
            ],
            Timing::Absent,
        ),
    );
    assert_eq!(report.lines[3].status, Status::Skipped);
    assert!(
        failed_log
            .borrow()
            .iter()
            .all(|entry| !entry.starts_with("codex hooks") && !entry.starts_with("codex list")),
        "{:?}",
        failed_log.borrow()
    );

    let missing_log = Log::default();
    let report = run(
        &request("", None),
        &ports(
            [
                kept("claude", &missing_log, "/bin"),
                tool("codex", missing("codex", true)),
                kept("agy", &missing_log, "/bin"),
            ],
            Timing::Absent,
        ),
    );
    assert_eq!(report.lines[3].status, Status::Skipped);
    assert!(
        missing_log
            .borrow()
            .iter()
            .all(|entry| !entry.contains("hooks") && !entry.contains("list")),
        "{:?}",
        missing_log.borrow()
    );

    let bare_log = Log::default();
    let report = run(
        &request("", None),
        &ports(
            [
                kept("claude", &bare_log, "/bin"),
                bare("codex", "codex-hooks", &bare_log),
                kept("agy", &bare_log, "/bin"),
            ],
            Timing::Absent,
        ),
    );
    let steps: Vec<(&str, Status)> = report
        .lines
        .iter()
        .map(|line| (line.step, line.status))
        .collect();
    assert_eq!(
        steps,
        [
            ("notes", Status::Kept),
            ("claude", Status::Kept),
            ("codex", Status::Kept),
            ("agy", Status::Kept),
            ("refresh-timer", Status::Skipped),
            ("derive-timer", Status::Skipped),
        ]
    );
}

#[test]
fn a_tool_that_is_absent_is_skipped_and_one_named_and_absent_fails() {
    for step in ["claude", "codex", "agy"] {
        let row = |way: Way| {
            let log = Log::default();
            let mut tools = [
                kept("claude", &log, "/bin"),
                kept("codex", &log, "/bin"),
                kept("agy", &log, "/bin"),
            ];
            let at = ["claude", "codex", "agy"]
                .iter()
                .position(|name| *name == step)
                .expect("a tool");
            tools[at] = tool(step, way);
            let report = run(&request("", None), &ports(tools, Timing::Absent));
            report
                .lines
                .iter()
                .find(|line| line.step == step)
                .cloned()
                .expect("row")
        };
        let default = row(missing(step, true));
        assert_eq!(
            (default.status, default.detail),
            (Status::Skipped, format!("{step} is not on PATH"))
        );
        let named = row(missing("mine", false));
        assert_eq!(
            (named.status, named.detail),
            (
                Status::Failed,
                format!("setup.{step}.program: mine is not on PATH")
            )
        );
        let path = row(missing("/x/mine", false));
        assert_eq!(
            (path.status, path.detail),
            (
                Status::Failed,
                format!("setup.{step}.program: /x/mine is not a program this user can run")
            )
        );
    }
}

#[test]
fn a_tool_that_is_kept_out_is_skipped_and_never_asked() {
    let report = run(
        &request("", None),
        &ports(
            [
                tool("claude", Way::Off),
                tool("codex", Way::Off),
                tool("agy", Way::Off),
            ],
            Timing::Absent,
        ),
    );
    assert_eq!(
        shown(&report.lines[1..5]),
        [
            (
                "claude",
                Status::Skipped,
                "setup.claude.enabled is false".to_owned()
            ),
            (
                "codex",
                Status::Skipped,
                "setup.codex.enabled is false".to_owned()
            ),
            (
                "codex-hooks",
                Status::Skipped,
                "the codex step was skipped".to_owned()
            ),
            (
                "agy",
                Status::Skipped,
                "setup.agy.enabled is false".to_owned()
            ),
        ]
    );
}

#[test]
fn the_hooks_row_says_what_setup_knows_of_its_tool() {
    let log = Log::default();
    let hooks_of = |codex: Tool, plugin: Option<&str>| {
        let log = Log::default();
        let mut asked = request("", None);
        asked.plugin = plugin.map(PathBuf::from);
        let report = run(
            &asked,
            &ports(
                [
                    kept("claude", &log, "/bin"),
                    codex,
                    kept("agy", &log, "/bin"),
                ],
                Timing::Absent,
            ),
        );
        report.lines[3].clone()
    };
    for (codex, plugin, sentence) in [
        (
            tool("codex", Way::Off),
            Some("/pkg"),
            "the codex step was skipped",
        ),
        (
            tool("codex", missing("codex", true)),
            Some("/pkg"),
            "the codex step was skipped",
        ),
        (
            tool("codex", missing("mine", false)),
            Some("/pkg"),
            "the codex step failed",
        ),
        (broken("codex", &log), Some("/pkg"), "the codex step failed"),
        (kept("codex", &log, "/bin"), None, "the codex step failed"),
    ] {
        let line = hooks_of(codex, plugin);
        assert_eq!(line.status, Status::Skipped);
        assert_eq!(line.detail, sentence);
    }
}

#[test]
fn no_scheduler_skips_both_timers() {
    let off = [
        tool("claude", Way::Off),
        tool("codex", Way::Off),
        tool("agy", Way::Off),
    ];
    let report = run(&request("", None), &ports(off, Timing::Absent));
    assert_eq!(
        shown(&report.lines[5..]),
        [
            (
                "refresh-timer",
                Status::Skipped,
                "this platform has neither launchd nor systemd".to_owned()
            ),
            (
                "derive-timer",
                Status::Skipped,
                "this platform has neither launchd nor systemd".to_owned()
            ),
        ]
    );
    for (program, status) in [
        ("launchctl", Status::Failed),
        ("systemctl", Status::Skipped),
    ] {
        let off = [
            tool("claude", Way::Off),
            tool("codex", Way::Off),
            tool("agy", Way::Off),
        ];
        let report = run(
            &request("", None),
            &ports(off, Timing::Missing { program, status }),
        );
        let sentence = format!("{program} is not on PATH");
        assert_eq!(
            shown(&report.lines[5..]),
            [
                ("refresh-timer", status, sentence.clone()),
                ("derive-timer", status, sentence),
            ],
            "{program}"
        );
    }
}

#[test]
fn the_derive_timers_detail_names_the_key_that_leaves_derive_off() {
    let clauses = [
        "derive.enabled is false in the config and the unit",
        "llm.provider is empty in the config and the unit",
        "derive.in is empty in the config and the unit",
    ];
    for clause in clauses.map(Some).into_iter().chain([None]) {
        let off = [
            tool("claude", Way::Off),
            tool("codex", Way::Off),
            tool("agy", Way::Off),
        ];
        let (scheduler, _) = timers();
        let report = run(&request("", clause), &ports(off, scheduler));
        assert_eq!(
            shown(&report.lines[1..5]),
            [
                (
                    "claude",
                    Status::Skipped,
                    "setup.claude.enabled is false".to_owned()
                ),
                (
                    "codex",
                    Status::Skipped,
                    "setup.codex.enabled is false".to_owned()
                ),
                (
                    "codex-hooks",
                    Status::Skipped,
                    "the codex step was skipped".to_owned()
                ),
                (
                    "agy",
                    Status::Skipped,
                    "setup.agy.enabled is false".to_owned()
                ),
            ]
        );
        assert_eq!(
            report.lines[5].detail, "every 10m, log /l/refresh.log",
            "the refresh timer carries no clause"
        );
        let expected = match clause {
            Some(clause) => format!("every 30m, log /l/derive.log; {clause}"),
            None => "every 30m, log /l/derive.log".to_owned(),
        };
        assert_eq!(report.lines[6].detail, expected);
    }
}

#[test]
fn a_units_path_is_the_tools_folders_then_the_keys() {
    let log = Log::default();
    let (scheduler, jobs) = timers();
    run(
        &request(r#"setup.timer.path = "/b:/c""#, None),
        &ports(
            [
                kept("claude", &log, "/a"),
                kept("codex", &log, "/a"),
                kept("agy", &log, "/b"),
            ],
            scheduler,
        ),
    );
    let paths: Vec<Option<String>> = jobs
        .borrow()
        .iter()
        .map(|job| {
            job.environment()
                .into_iter()
                .find(|(name, _)| name == "PATH")
                .map(|(_, value)| value)
        })
        .collect();
    assert_eq!(
        paths,
        [Some("/a:/b:/c".to_owned()), Some("/a:/b:/c".to_owned())]
    );

    let (scheduler, jobs) = timers();
    run(
        &request(r#"setup.timer.path = "/k""#, None),
        &ports(
            [
                tool("claude", missing("claude", true)),
                tool("codex", Way::Off),
                kept("agy", &log, "/x"),
            ],
            scheduler,
        ),
    );
    assert_eq!(
        jobs.borrow()[0].path,
        [PathBuf::from("/x"), PathBuf::from("/k")]
    );

    let (scheduler, jobs) = timers();
    run(
        &request(r#"setup.timer.path = """#, None),
        &ports(
            [
                tool("claude", Way::Off),
                tool("codex", missing("codex", true)),
                tool("agy", Way::Off),
            ],
            scheduler,
        ),
    );
    assert!(jobs.borrow()[0].path.is_empty());
    assert!(
        jobs.borrow()[0]
            .environment()
            .iter()
            .all(|(name, _)| name != "PATH")
    );
}

fn taking_of(tools: [Tool; 3], scheduler: Timing) -> Taking {
    Taking { tools, scheduler }
}

fn names(lines: &[Line]) -> Vec<(&'static str, Status)> {
    lines.iter().map(|line| (line.step, line.status)).collect()
}

#[test]
fn a_removal_runs_the_hooks_before_their_tool_and_no_notes_step() {
    let log = Log::default();
    let (timing, _) = timers_in(&log);
    let report = remove(&taking_of(
        [
            kept("claude", &log, "/bin"),
            kept("codex", &log, "/bin"),
            kept("agy", &log, "/bin"),
        ],
        timing,
    ));
    assert_eq!(
        names(&report.lines),
        [
            ("claude", Status::Removed),
            ("codex-hooks", Status::Removed),
            ("codex", Status::Removed),
            ("agy", Status::Removed),
            ("refresh-timer", Status::Removed),
            ("derive-timer", Status::Removed),
        ]
    );
    let log = log.borrow().clone();
    let at = |word: &str| log.iter().position(|entry| entry == word).expect(word);
    assert!(at("codex kept") < at("codex forget"));
    assert!(
        at("codex forget") < at("codex held")
            || log.iter().filter(|e| *e == "codex held").count() == 2
    );
    assert!(!log.iter().any(|entry| entry == "notes"));
    assert_eq!(report.lines[1].detail, "2 trust entries of bilbo@bilbo");

    let log = Log::default();
    let (timing, _) = timers_in(&log);
    let claude = Tool {
        step: "claude",
        hooks: None,
        way: Way::Found {
            agent: Box::new(Fake {
                step: "claude",
                log: log.clone(),
                manifest: Ok(()),
                state: RefCell::new(Some(false)),
                gives_hooks: false,
                keys: Rc::default(),
                opts: Opts {
                    held_fails: Some("claude plugin list --json: it broke".to_owned()),
                    ..Opts::default()
                },
            }),
            folder: PathBuf::from("/bin"),
        },
    };
    let report = remove(&taking_of(
        [
            claude,
            kept("codex", &log, "/bin"),
            kept("agy", &log, "/bin"),
        ],
        timing,
    ));
    assert_eq!(
        report
            .lines
            .iter()
            .map(|line| (line.step, line.status, line.detail.as_str()))
            .collect::<Vec<_>>(),
        [
            (
                "claude",
                Status::Failed,
                "claude plugin list --json: it broke"
            ),
            (
                "codex-hooks",
                Status::Removed,
                "2 trust entries of bilbo@bilbo"
            ),
            (
                "codex",
                Status::Removed,
                "bilbo@bilbo and the marketplace bilbo"
            ),
            (
                "agy",
                Status::Removed,
                "bilbo@bilbo and the marketplace bilbo"
            ),
            ("refresh-timer", Status::Removed, "label"),
            ("derive-timer", Status::Removed, "label"),
        ]
    );
    let log = log.borrow();
    let at = |word: &str| log.iter().position(|entry| entry == word).expect(word);
    assert!(at("codex forget") < at("codex Uninstall"));
    assert!(log.iter().any(|entry| entry == "agy held"));
    assert!(log.iter().any(|entry| entry == "refresh take_out"));
    assert!(log.iter().any(|entry| entry == "derive take_out"));
}

fn codex_with(log: &Log, plugin: bool, keys: &[&str], opts: Opts) -> Tool {
    Tool {
        step: "codex",
        hooks: Some("codex-hooks"),
        way: Way::Found {
            agent: Box::new(Fake {
                step: "codex",
                log: log.clone(),
                manifest: Ok(()),
                state: RefCell::new(plugin.then_some(false)),
                gives_hooks: true,
                keys: Rc::new(RefCell::new(
                    keys.iter().map(|key| (*key).to_owned()).collect(),
                )),
                opts,
            }),
            folder: PathBuf::from("/bin"),
        },
    }
}

type Two = [(&'static str, Status, String); 2];

fn two_rows(codex: Tool, log: &Log) -> (Two, Vec<String>) {
    let (timing, _) = timers_in(log);
    let report = remove(&taking_of(
        [tool("claude", Way::Off), codex, tool("agy", Way::Off)],
        timing,
    ));
    let rows = [1, 2].map(|at| {
        let line = &report.lines[at];
        (line.step, line.status, line.detail.clone())
    });
    let changes = log
        .borrow()
        .iter()
        .filter(|entry| {
            ["forget", "Uninstall", "Remove", "trust"]
                .iter()
                .any(|word| entry.starts_with("codex ") && entry.ends_with(word))
        })
        .cloned()
        .collect();
    (rows, changes)
}

fn row(step: &'static str, status: Status, detail: &str) -> (&'static str, Status, String) {
    (step, status, detail.to_owned())
}

#[test]
fn codexs_two_rows_go_by_one_look() {
    let skipped_hooks = |said: &str| row("codex-hooks", Status::Skipped, said);
    let none: Vec<String> = Vec::new();
    for (name, way, hooks, codex) in [
        (
            "kept out",
            Way::Off,
            skipped_hooks("the codex step was skipped"),
            row("codex", Status::Skipped, "setup.codex.enabled is false"),
        ),
        (
            "not found at its default",
            Way::Missing {
                value: "codex".to_owned(),
                default: true,
            },
            skipped_hooks("the codex step was skipped"),
            row("codex", Status::Skipped, "codex is not on PATH"),
        ),
        (
            "named and not found",
            Way::Missing {
                value: "mycodex".to_owned(),
                default: false,
            },
            skipped_hooks("the codex step failed"),
            row(
                "codex",
                Status::Failed,
                "setup.codex.program: mycodex is not on PATH",
            ),
        ),
    ] {
        let log = Log::default();
        let (rows, changes) = two_rows(tool("codex", way), &log);
        assert_eq!(rows, [hooks, codex], "{name}");
        assert_eq!(changes, none, "{name}");
        assert!(
            !log.borrow().iter().any(|entry| entry.starts_with("codex")),
            "{name}: Codex is not asked anything"
        );
    }

    let log = Log::default();
    let broken = Opts {
        held_fails: Some("codex plugin list --json: it broke".to_owned()),
        ..Opts::default()
    };
    let (rows, changes) = two_rows(codex_with(&log, true, &["bilbo@bilbo:a"], broken), &log);
    assert_eq!(
        rows,
        [
            skipped_hooks("the codex step failed"),
            row(
                "codex",
                Status::Failed,
                "codex plugin list --json: it broke"
            ),
        ]
    );
    assert_eq!(changes, none);
    assert!(!log.borrow().iter().any(|entry| entry == "codex hooks"));

    let log = Log::default();
    let other = Opts {
        other: Some(bilbo_setup::plugin::Other {
            id: "x@bilbo".to_owned(),
            scope: None,
        }),
        ..Opts::default()
    };
    let (rows, changes) = two_rows(codex_with(&log, true, &["bilbo@bilbo:a"], other), &log);
    assert_eq!(rows[0], skipped_hooks("the codex step failed"));
    assert_eq!(rows[1].1, Status::Failed);
    assert_eq!(
        rows[1].2,
        "codex has x@bilbo installed from the marketplace bilbo; bilbo setup changes no marketplace another installation uses"
    );
    assert_eq!(changes, none);
    assert!(!log.borrow().iter().any(|entry| entry == "codex hooks"));

    let log = Log::default();
    let (rows, changes) = two_rows(
        codex_with(
            &log,
            true,
            &["bilbo@bilbo:a", "bilbo@bilbo:b"],
            Opts::default(),
        ),
        &log,
    );
    assert_eq!(
        rows,
        [
            row(
                "codex-hooks",
                Status::Removed,
                "2 trust entries of bilbo@bilbo"
            ),
            row(
                "codex",
                Status::Removed,
                "bilbo@bilbo and the marketplace bilbo"
            ),
        ]
    );
    assert_eq!(changes, ["codex forget", "codex Uninstall", "codex Remove"]);
    let looks = log
        .borrow()
        .iter()
        .take_while(|entry| *entry != "codex forget")
        .filter(|entry| *entry == "codex held")
        .count();
    assert_eq!(looks, 1, "one look before anything changes");

    let log = Log::default();
    let (rows, changes) = two_rows(
        codex_with(&log, false, &["bilbo@bilbo:a"], Opts::default()),
        &log,
    );
    assert_eq!(
        rows,
        [
            row(
                "codex-hooks",
                Status::Removed,
                "1 trust entry of bilbo@bilbo"
            ),
            row("codex", Status::Absent, "no plugin bilbo@bilbo"),
        ]
    );
    assert_eq!(
        changes,
        ["codex forget"],
        "orphaned trust is taken out alone"
    );

    let log = Log::default();
    let write_fails = Opts {
        forget_fails: true,
        ..Opts::default()
    };
    let (rows, changes) = two_rows(
        codex_with(&log, true, &["bilbo@bilbo:a"], write_fails),
        &log,
    );
    assert_eq!(
        rows,
        [
            row("codex-hooks", Status::Failed, "config/batchWrite failed"),
            row(
                "codex",
                Status::Removed,
                "bilbo@bilbo and the marketplace bilbo"
            ),
        ]
    );
    assert_eq!(changes, ["codex forget", "codex Uninstall", "codex Remove"]);

    let log = Log::default();
    let uninstall_fails = Opts {
        run_fails: Some((Act::Uninstall, "codex words: it broke".to_owned())),
        ..Opts::default()
    };
    let (rows, changes) = two_rows(
        codex_with(&log, true, &["bilbo@bilbo:a"], uninstall_fails),
        &log,
    );
    assert_eq!(
        rows,
        [
            row(
                "codex-hooks",
                Status::Removed,
                "1 trust entry of bilbo@bilbo"
            ),
            row("codex", Status::Failed, "codex words: it broke"),
        ]
    );
    assert_eq!(
        changes,
        ["codex forget", "codex Uninstall"],
        "no trust is put back after the uninstall"
    );
}

#[test]
fn a_removal_stops_at_another_installation() {
    let log = Log::default();
    let (timing, _) = timers_in(&log);
    let other = Tool {
        step: "codex",
        hooks: Some("codex-hooks"),
        way: Way::Found {
            agent: Box::new(Fake {
                step: "codex",
                log: log.clone(),
                manifest: Ok(()),
                state: RefCell::new(Some(false)),
                gives_hooks: true,
                keys: Rc::new(RefCell::new(vec!["bilbo@bilbo:a".to_owned()])),
                opts: Opts {
                    other: Some(bilbo_setup::plugin::Other {
                        id: "x@bilbo".to_owned(),
                        scope: None,
                    }),
                    ..Opts::default()
                },
            }),
            folder: PathBuf::from("/bin"),
        },
    };
    let report = remove(&taking_of(
        [
            kept("claude", &log, "/bin"),
            other,
            kept("agy", &log, "/bin"),
        ],
        timing,
    ));
    let rows: Vec<(&str, Status, &str)> = report
        .lines
        .iter()
        .map(|line| (line.step, line.status, line.detail.as_str()))
        .collect();
    assert_eq!(
        rows[1],
        ("codex-hooks", Status::Skipped, "the codex step failed")
    );
    assert_eq!(rows[2].0, "codex");
    assert_eq!(rows[2].1, Status::Failed);
    assert!(
        rows[2]
            .2
            .starts_with("codex has x@bilbo installed from the marketplace bilbo;"),
        "{}",
        rows[2].2
    );
    let log = log.borrow();
    assert!(
        !log.iter()
            .any(|entry| entry.starts_with("codex kept") || entry.starts_with("codex forget"))
    );
    assert!(
        !log.iter()
            .any(|entry| entry == "codex Uninstall" || entry == "codex Remove")
    );
    assert_eq!(rows[0].1, Status::Removed, "the other tools still run");
}
