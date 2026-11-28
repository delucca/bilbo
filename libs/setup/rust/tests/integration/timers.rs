use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use bilbo_programs::{Fail, Programs};
use bilbo_setup::launchd::Launchd;
use bilbo_setup::program::Bounds;
use bilbo_setup::report::{Line, Status};
use bilbo_setup::systemd::Systemd;
use bilbo_setup::timer::{Job, Scheduler, removal, step};

use super::{Folder, scratch, serial};

fn bounds() -> Bounds {
    Bounds {
        wait: Duration::from_secs(15),
        poll: Duration::from_millis(10),
    }
}

struct World {
    programs: Programs,
    home: PathBuf,
    units: PathBuf,
    state: PathBuf,
    _keep: Vec<Folder>,
}

fn world() -> World {
    let (bin, state, home) = (scratch(), scratch(), scratch());
    let programs = Programs::write(&bin, &state).expect("stand-ins");
    let units = home.join(".config/systemd/user");
    programs.units(&units).expect("the units' folder");
    World {
        programs,
        home: home.to_path_buf(),
        units,
        state: state.to_path_buf(),
        _keep: vec![bin, state, home],
    }
}

impl World {
    fn job(&self, verb: &'static str) -> Job {
        Job {
            step: if verb == "refresh" {
                "refresh-timer"
            } else {
                "derive-timer"
            },
            verb,
            interval: Duration::from_secs(if verb == "refresh" { 600 } else { 1800 }),
            program: Some(PathBuf::from("/opt/bilbo/bin/bilbo")),
            log: self.home.join(format!("state/timer/{verb}.log")),
            variables: vec![("HOME".to_owned(), OsString::from(&self.home))],
            path: vec![PathBuf::from("/usr/bin")],
            clause: None,
        }
    }

    fn systemd(&self) -> Systemd {
        Systemd::new(
            Some(self.programs.path("systemctl")),
            self.units.clone(),
            bounds(),
        )
    }

    fn launchd(&self) -> Launchd {
        Launchd::new(
            Some(self.programs.path("launchctl")),
            self.home.join("Library/LaunchAgents"),
            501,
            bounds(),
        )
    }

    fn calls(&self, name: &str) -> Vec<String> {
        self.programs.log(name)
    }

    fn changes(&self, name: &str) -> Vec<String> {
        self.calls(name)
            .into_iter()
            .filter(|line| !line.starts_with("print") && !line.contains("is-"))
            .collect()
    }
}

fn run(scheduler: &dyn Scheduler, job: &Job) -> Line {
    step(scheduler, job)
}

fn plist(world: &World, job: &Job) -> PathBuf {
    world.home.join(format!(
        "Library/LaunchAgents/io.github.delucca.bilbo.{}.plist",
        job.verb
    ))
}

#[test]
fn launchd_boots_the_job_out_then_in() {
    let _serial = serial();
    let world = world();
    let job = world.job("refresh");
    let first = run(&world.launchd(), &job);
    assert_eq!(first.status, Status::Installed, "{}", first.detail);
    let target = "gui/501/io.github.delucca.bilbo.refresh";
    let file = plist(&world, &job);
    assert_eq!(
        world.calls("launchctl"),
        [
            format!("print {target}"),
            format!("bootstrap gui/501 {}", file.display()),
            format!("print {target}"),
        ]
    );
    assert_eq!(
        world.programs.loaded_labels(),
        ["gui/501/io.github.delucca.bilbo.refresh"]
    );

    let before = world.calls("launchctl").len();
    assert_eq!(run(&world.launchd(), &job).status, Status::Kept);
    assert_eq!(world.calls("launchctl").len(), before + 1);

    fs::write(&file, "changed\n").expect("a changed unit");
    let before = world.calls("launchctl").len();
    let updated = run(&world.launchd(), &job);
    assert_eq!(updated.status, Status::Updated, "{}", updated.detail);
    assert!(updated.detail.ends_with("its unit differed"));
    assert_eq!(
        world.calls("launchctl")[before..],
        [
            format!("print {target}"),
            format!("bootout --wait {target}"),
            format!("bootstrap gui/501 {}", file.display()),
            format!("print {target}"),
        ]
    );
    assert_ne!(fs::read_to_string(&file).expect("the file"), "changed\n");

    fs::write(&file, "changed again\n").expect("a changed unit");
    world
        .programs
        .leave_before_bootout(target)
        .expect("it leaves");
    let before = world.calls("launchctl").len();
    let left = run(&world.launchd(), &job);
    assert_eq!(left.status, Status::Updated, "{}", left.detail);
    assert!(
        left.detail.ends_with("; its unit differed"),
        "{}",
        left.detail
    );
    assert_eq!(
        world.calls("launchctl")[before..],
        [
            format!("print {target}"),
            format!("bootout --wait {target}"),
            format!("bootstrap gui/501 {}", file.display()),
            format!("print {target}"),
        ]
    );
    assert_eq!(world.programs.loaded_labels(), [target]);

    let other = Launchd::new(
        Some(world.programs.path("launchctl")),
        world.home.join("Library/LaunchAgents"),
        502,
        bounds(),
    );
    let before = world.calls("launchctl").len();
    let elsewhere = run(&other, &job);
    assert_eq!(elsewhere.status, Status::Updated, "{}", elsewhere.detail);
    assert!(
        elsewhere.detail.ends_with("; it was not loaded"),
        "{}",
        elsewhere.detail
    );
    assert_eq!(
        world.calls("launchctl")[before],
        "print gui/502/io.github.delucca.bilbo.refresh"
    );
    assert_eq!(world.programs.loaded_labels().len(), 2);
}

#[test]
fn launchd_reads_its_exit_codes_as_documented() {
    let _serial = serial();
    let world = world();
    let job = world.job("refresh");
    run(&world.launchd(), &job);
    fs::write(plist(&world, &job), "changed\n").expect("a changed unit");
    world
        .programs
        .set("launchctl", "code.bootout", "5")
        .expect("exit 5");
    let line = run(&world.launchd(), &job);
    assert_eq!(line.status, Status::Failed);
    assert_eq!(
        line.detail,
        "launchctl bootout --wait gui/501/io.github.delucca.bilbo.refresh failed: exit 5"
    );
    assert_eq!(
        fs::read_to_string(plist(&world, &job)).expect("file"),
        "changed\n"
    );
    world
        .programs
        .unfail("launchctl", "bootout")
        .expect("no fault");
    fs::remove_file(world.state.join("launchctl.code.bootout")).expect("no code");
    world
        .programs
        .leave_before_bootout("gui/501/io.github.delucca.bilbo.refresh")
        .expect("it leaves");
    let line = run(&world.launchd(), &job);
    assert_eq!(
        line.status,
        Status::Updated,
        "an unload that exits 3 is fine: {}",
        line.detail
    );
    world
        .programs
        .set("launchctl", "code.print", "1")
        .expect("exit 1");
    let line = run(&world.launchd(), &job);
    assert_eq!(line.status, Status::Failed);
    assert!(
        line.detail
            .contains("print gui/501/io.github.delucca.bilbo.refresh failed"),
        "{}",
        line.detail
    );
    let unloaded = world.job("derive");
    let line = run(&world.launchd(), &unloaded);
    assert_eq!(
        line.status,
        Status::Failed,
        "an exit of 1 is no `not loaded`"
    );
}

fn enabled(world: &World, job: &Job) -> bool {
    world
        .units
        .join(format!("timers.target.wants/bilbo-{}.timer", job.verb))
        .is_symlink()
}

#[test]
fn systemd_stops_writes_reloads_enables_and_restarts_the_timer() {
    let _serial = serial();
    let world = world();
    let job = world.job("refresh");
    let timer = "bilbo-refresh.timer";
    let manager = run(&world.systemd(), &job);
    assert_eq!(manager.status, Status::Installed, "{}", manager.detail);
    assert_eq!(
        world.calls("systemctl"),
        [
            "--user is-system-running".to_owned(),
            format!("--user is-active {timer}"),
            "--user daemon-reload".to_owned(),
            format!("--user enable {timer}"),
            format!("--user restart {timer}"),
            format!("--user is-active {timer}"),
            format!("--user is-enabled {timer}"),
        ]
    );
    assert!(
        enabled(&world, &job),
        "the stand-in made the link on the disk"
    );
    assert!(
        !world
            .units
            .join("bilbo-refresh.service")
            .to_string_lossy()
            .is_empty()
    );
    assert!(world.units.join("bilbo-refresh.service").is_file());

    fs::write(world.units.join(timer), "changed\n").expect("a changed unit");
    let before = world.calls("systemctl").len();
    let line = run(&world.systemd(), &job);
    assert_eq!(line.status, Status::Updated, "{}", line.detail);
    let rest: Vec<String> = world.calls("systemctl")[before..].to_vec();
    assert_eq!(
        rest,
        [
            "--user is-system-running".to_owned(),
            format!("--user is-active {timer}"),
            format!("--user is-enabled {timer}"),
            format!("--user stop {timer}"),
            format!("--user disable {timer}"),
            "--user daemon-reload".to_owned(),
            format!("--user enable {timer}"),
            format!("--user restart {timer}"),
            format!("--user is-active {timer}"),
            format!("--user is-enabled {timer}"),
        ]
    );
    assert!(enabled(&world, &job));

    fs::write(world.units.join(timer), "changed\n").expect("a changed unit");
    world
        .programs
        .fail_for(
            "systemctl",
            "daemon-reload",
            None,
            true,
            Fail::Exit("reload broke"),
            false,
        )
        .expect("one call fails");
    let line = run(&world.systemd(), &job);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (
            Status::Failed,
            "systemctl --user daemon-reload failed: reload broke"
        )
    );
    assert!(!enabled(&world, &job), "disable took the link away");
    let line = run(&world.systemd(), &job);
    assert_eq!(line.status, Status::Updated, "{}", line.detail);
    assert!(enabled(&world, &job), "enable made it again");

    world
        .programs
        .set(
            "systemctl",
            &format!("value.is-active.{timer}"),
            "deactivating",
        )
        .expect("deactivating");
    fs::write(world.units.join(timer), "changed\n").expect("a changed unit");
    let before = world.calls("systemctl").len();
    run(&world.systemd(), &job);
    assert!(
        world.calls("systemctl")[before..].contains(&format!("--user stop {timer}")),
        "a timer that is deactivating is not at rest"
    );

    let fresh = self::world();
    fresh
        .programs
        .flag("systemctl", "silent.is-active")
        .expect("no line");
    let line = run(&fresh.systemd(), &job);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (
            Status::Failed,
            "systemctl --user is-active bilbo-refresh.timer answered nothing"
        )
    );
    assert!(!fresh.units.join(timer).exists(), "nothing was written");
}

#[test]
fn the_systemd_manager_answers_each_outcome_with_its_row() {
    let _serial = serial();
    let world = world();
    let job = world.job("refresh");
    for (answer, status, detail) in [
        (
            "offline",
            Status::Skipped,
            "no systemd user session answers",
        ),
        (
            "stopping",
            Status::Failed,
            "systemctl --user is-system-running answered 'stopping'",
        ),
    ] {
        world
            .programs
            .set("systemctl", "value.is-system-running", answer)
            .expect("state");
        let line = run(&world.systemd(), &job);
        assert_eq!((line.status, line.detail.as_str()), (status, detail));
    }
    world
        .programs
        .flag("systemctl", "silent.is-system-running")
        .expect("no line");
    let line = run(&world.systemd(), &job);
    assert_eq!(
        line.detail,
        "systemctl --user is-system-running answered nothing"
    );
    assert!(!world.units.exists(), "nothing was written");
    let missing = Systemd::new(None, world.units.clone(), bounds());
    let line = run(&missing, &job);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (Status::Skipped, "systemctl is not on PATH")
    );
    let script = world.home.join("systemctl");
    fs::write(&script, "#!/nonexistent/interpreter\n").expect("a program");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("a mode");
    let broken = Systemd::new(Some(script.clone()), world.units.clone(), bounds());
    let line = run(&broken, &job);
    let error = Command::new(&script)
        .output()
        .expect_err("the program cannot start");
    assert_eq!(
        (line.status, line.detail.as_str()),
        (
            Status::Failed,
            format!("cannot start {}: {error}", script.display()).as_str()
        )
    );
    assert!(!world.units.exists(), "no units' folder");
    assert!(
        !job.log.parent().expect("a folder").exists(),
        "no log's folder"
    );
    let hanging = self::world();
    hanging
        .programs
        .fail("systemctl", "is-system-running", Fail::Hang, false)
        .expect("a hang");
    let quick = Systemd::new(
        Some(hanging.programs.path("systemctl")),
        hanging.units.clone(),
        Bounds {
            wait: Duration::from_secs(1),
            poll: Duration::from_millis(10),
        },
    );
    let line = run(&quick, &job);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (
            Status::Failed,
            "systemctl --user is-system-running: it did not end within 1s"
        )
    );
    assert!(!hanging.units.exists(), "no units' folder");
    assert!(
        !job.log.parent().expect("a folder").exists(),
        "no log's folder"
    );
    let none = Launchd::new(None, world.home.join("Library/LaunchAgents"), 501, bounds());
    let line = run(&none, &job);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (Status::Failed, "launchctl is not on PATH")
    );
}

#[test]
fn a_systemd_change_that_stops_part_way_says_what_it_did_and_the_next_run_finishes_it() {
    let _serial = serial();
    let job_of = |world: &World| world.job("refresh");
    let timer = "bilbo-refresh.timer";
    let loaded_from_other_files = |world: &World| {
        let job = job_of(world);
        assert_eq!(run(&world.systemd(), &job).status, Status::Installed);
        fs::write(world.units.join(timer), "other files\n").expect("other files");
        job
    };

    let world = world();
    let job = loaded_from_other_files(&world);
    world
        .programs
        .fail("systemctl", "disable", Fail::Exit("disable broke"), false)
        .expect("marked");
    let line = run(&world.systemd(), &job);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (
            Status::Failed,
            "systemctl --user disable bilbo-refresh.timer failed: disable broke"
        )
    );
    assert_eq!(
        fs::read_to_string(world.units.join(timer)).expect("file"),
        "other files\n"
    );
    assert!(enabled(&world, &job), "stopped and still enabled");
    assert_eq!(
        world
            .programs
            .get("systemctl", &format!("active.{timer}"))
            .as_deref(),
        Some("inactive")
    );
    assert!(!line.detail.contains("unloaded"));
    fs::remove_file(world.state.join("systemctl.fail.disable")).expect("fault gone");
    let line = run(&world.systemd(), &job);
    assert_eq!(line.status, Status::Updated, "{}", line.detail);

    let world = self::world();
    let job = loaded_from_other_files(&world);
    world
        .programs
        .fail("systemctl", "restart", Fail::Exit("restart broke"), false)
        .expect("marked");
    let line = run(&world.systemd(), &job);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (
            Status::Failed,
            "systemctl --user restart bilbo-refresh.timer failed: restart broke"
        )
    );
    assert_ne!(
        fs::read_to_string(world.units.join(timer)).expect("file"),
        "other files\n"
    );
    assert!(enabled(&world, &job));
    assert_eq!(
        world
            .programs
            .get("systemctl", &format!("active.{timer}"))
            .as_deref(),
        Some("inactive")
    );
    fs::remove_file(world.state.join("systemctl.fail.restart")).expect("fault gone");
    let line = run(&world.systemd(), &job);
    assert_eq!(line.status, Status::Updated, "{}", line.detail);
    assert!(
        line.detail.ends_with("it was not loaded"),
        "{}",
        line.detail
    );

    let world = self::world();
    let job = job_of(&world);
    let line = run(&world.systemd(), &job);
    assert_eq!(line.status, Status::Installed);
    fs::write(world.units.join(timer), "other files\n").expect("other files");
    world
        .programs
        .set("systemctl", "skip.is-active", "1")
        .expect("one answer");
    let line = run(&world.systemd(), &job);
    world
        .programs
        .set("systemctl", "skip.is-active", "99")
        .expect("answers again");
    assert_eq!(line.status, Status::Failed);
    assert!(line.detail.ends_with("answered nothing"), "{}", line.detail);
    let line = run(&world.systemd(), &job);
    assert_eq!(line.status, Status::Kept, "{}", line.detail);
}

#[test]
fn a_unit_someone_changed_is_written_again() {
    let _serial = serial();
    let world = world();
    let job = world.job("derive");
    run(&world.systemd(), &job);
    let service = world.units.join("bilbo-derive.service");
    let wanted = fs::read_to_string(&service).expect("the service");
    fs::write(&service, format!("{wanted}Environment=\"X=1\"\n")).expect("one more line");
    let line = run(&world.systemd(), &job);
    assert_eq!(line.status, Status::Updated, "{}", line.detail);
    assert_eq!(fs::read_to_string(&service).expect("the service"), wanted);
    assert!(
        world
            .calls("systemctl")
            .iter()
            .any(|call| call.contains("restart"))
    );
}

#[test]
fn a_missing_log_folder_is_made_again() {
    let _serial = serial();
    for scheduler in ["launchd", "systemd"] {
        let world = world();
        let job = world.job("refresh");
        let line = if scheduler == "launchd" {
            run(&world.launchd(), &job)
        } else {
            run(&world.systemd(), &job)
        };
        assert_eq!(line.status, Status::Installed);
        let folder = job.log.parent().expect("a folder").to_owned();
        assert!(folder.is_dir());
        assert_eq!(
            fs::metadata(&folder).expect("meta").permissions().mode() & 0o777,
            0o700
        );
        fs::remove_dir_all(&folder).expect("deleted");
        let line = if scheduler == "launchd" {
            run(&world.launchd(), &job)
        } else {
            run(&world.systemd(), &job)
        };
        assert_eq!(line.status, Status::Updated, "{scheduler}: {}", line.detail);
        assert!(
            line.detail.ends_with("its log's folder was gone"),
            "{}",
            line.detail
        );
        assert!(folder.is_dir());
        assert_eq!(
            fs::metadata(&folder).expect("meta").permissions().mode() & 0o777,
            0o700
        );
    }
}

#[test]
fn a_log_that_is_a_link_or_no_regular_file_fails_before_anything_is_loaded() {
    let _serial = serial();
    for scheduler in ["launchd", "systemd"] {
        for installed in [false, true] {
            for (verb, link) in [("refresh", true), ("derive", false)] {
                let world = world();
                let job = world.job(verb);
                let run_it = |world: &World| {
                    if scheduler == "launchd" {
                        run(&world.launchd(), &job)
                    } else {
                        run(&world.systemd(), &job)
                    }
                };
                if installed {
                    assert_eq!(run_it(&world).status, Status::Installed);
                }
                let log = job.log.clone();
                fs::create_dir_all(log.parent().expect("a folder")).expect("folder");
                let target = world.home.join("other");
                fs::write(&target, "mine\n").expect("a file");
                if link {
                    symlink(&target, &log).expect("a link");
                } else {
                    fs::create_dir(&log).expect("a folder as log");
                }
                let name = if scheduler == "launchd" {
                    "launchctl"
                } else {
                    "systemctl"
                };
                let before = world.changes(name).len();
                let line = run_it(&world);
                let sentence = if link {
                    format!("the log {} is a symbolic link", log.display())
                } else {
                    format!("the log {} is not a regular file", log.display())
                };
                assert_eq!((line.status, line.detail), (Status::Failed, sentence));
                assert_eq!(
                    world.changes(name).len(),
                    before,
                    "no command that changes anything"
                );
                assert_eq!(fs::read_to_string(&target).expect("file"), "mine\n");
            }
        }
    }
    let world = world();
    let job = world.job("refresh");
    fs::create_dir_all(job.log.parent().expect("folder")).expect("folder");
    fs::write(&job.log, "old output\n").expect("a regular log");
    assert_eq!(run(&world.systemd(), &job).status, Status::Installed);

    for scheduler in ["launchd", "systemd"] {
        let world = self::world();
        let job = world.job("refresh");
        let run_it = |world: &World| {
            if scheduler == "launchd" {
                run(&world.launchd(), &job)
            } else {
                run(&world.systemd(), &job)
            }
        };
        assert_eq!(run_it(&world).status, Status::Installed);
        let folder = job.log.parent().expect("a folder").to_owned();
        fs::write(&job.log, "old output\n").expect("a regular log");
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o600)).expect("a mode");
        let probe = fs::symlink_metadata(&job.log);
        let name = if scheduler == "launchd" {
            "launchctl"
        } else {
            "systemctl"
        };
        let before = world.changes(name).len();
        let line = run_it(&world);
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).expect("a mode");
        let Err(error) = probe else {
            continue;
        };
        assert_eq!(
            (line.status, line.detail.as_str()),
            (
                Status::Failed,
                format!("cannot read {}: {error}", job.log.display()).as_str()
            ),
            "{scheduler}"
        );
        assert_eq!(
            world.changes(name).len(),
            before,
            "no command that changes anything"
        );
    }
}

struct Umask(rustix::fs::Mode);

impl Umask {
    fn set(mask: u32) -> Umask {
        let mode = rustix::fs::Mode::from_bits_truncate(mask.try_into().expect("a mask"));
        Umask(rustix::process::umask(mode))
    }
}

impl Drop for Umask {
    fn drop(&mut self) {
        rustix::process::umask(self.0);
    }
}

fn mode_of(path: &std::path::Path) -> u32 {
    fs::metadata(path).expect("meta").permissions().mode() & 0o777
}

#[test]
fn a_file_at_the_temporary_name_is_neither_followed_nor_removed() {
    let _serial = serial();
    let world = world();
    let job = world.job("refresh");
    let file = plist(&world, &job);
    let folder = file.parent().expect("a folder").to_owned();
    fs::create_dir_all(&folder).expect("the LaunchAgents folder");
    let theirs = world.home.join("theirs");
    fs::write(&theirs, "theirs").expect("a file");
    let temporary = folder.join(format!(
        ".io.github.delucca.bilbo.refresh.plist.{}.tmp",
        std::process::id()
    ));
    symlink(&theirs, &temporary).expect("a link at the temporary name");
    let line = run(&world.launchd(), &job);
    assert_eq!(line.status, Status::Failed, "{}", line.detail);
    assert!(
        line.detail
            .starts_with(&format!("cannot write {}: ", file.display())),
        "{}",
        line.detail
    );
    assert_eq!(fs::read_to_string(&theirs).expect("theirs"), "theirs");
    assert!(temporary.is_symlink(), "the link was not removed");
    assert!(
        !world
            .calls("launchctl")
            .iter()
            .any(|call| call.starts_with("bootstrap")),
        "nothing was loaded"
    );
}

#[test]
fn a_timers_files_and_folders_get_their_modes_whatever_the_umask() {
    let _serial = serial();
    {
        let _umask = Umask::set(0o002);
        let world = world();
        let job = world.job("refresh");
        assert_eq!(run(&world.launchd(), &job).status, Status::Installed);
        let file = plist(&world, &job);
        assert_eq!(mode_of(&file), 0o644);
        let systemd = self::world();
        assert_eq!(run(&systemd.systemd(), &job).status, Status::Installed);
        for unit in ["bilbo-refresh.service", "bilbo-refresh.timer"] {
            assert_eq!(mode_of(&systemd.units.join(unit)), 0o644, "{unit}");
        }
        fs::set_permissions(&file, fs::Permissions::from_mode(0o664)).expect("a mode");
        let before = world.calls("launchctl").len();
        let line = run(&world.launchd(), &job);
        assert_eq!(line.status, Status::Updated, "{}", line.detail);
        assert!(
            line.detail.ends_with("; its unit differed"),
            "{}",
            line.detail
        );
        let changes: Vec<String> = world.calls("launchctl")[before..]
            .iter()
            .filter(|call| call.starts_with("boot"))
            .cloned()
            .collect();
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert!(changes[0].starts_with("bootout") && changes[1].starts_with("bootstrap"));
        assert_eq!(mode_of(&file), 0o644);
    }
    let world = world();
    let _umask = Umask::set(0o200);
    let mut job = world.job("refresh");
    let kept = mode_of(&world.home);
    job.log = world.home.join("one/two/refresh.log");
    world.launchd().make_log_folder(&job).expect("made");
    assert_eq!(mode_of(&world.home.join("one")), 0o700);
    assert_eq!(mode_of(&world.home.join("one/two")), 0o700);
    assert_eq!(mode_of(&world.home), kept);
}

#[test]
fn a_unit_file_that_cannot_be_read_ends_the_step_before_any_unload() {
    let _serial = serial();
    for scheduler in ["launchd", "systemd"] {
        let world = world();
        let job = world.job("refresh");
        let run_it = |world: &World| {
            if scheduler == "launchd" {
                run(&world.launchd(), &job)
            } else {
                run(&world.systemd(), &job)
            }
        };
        assert_eq!(run_it(&world).status, Status::Installed);
        let file = if scheduler == "launchd" {
            plist(&world, &job)
        } else {
            world.units.join("bilbo-refresh.timer")
        };
        fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).expect("a mode");
        let probe = fs::read(&file);
        let name = if scheduler == "launchd" {
            "launchctl"
        } else {
            "systemctl"
        };
        let before = world.changes(name).len();
        let line = run_it(&world);
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).expect("a mode");
        if let Err(error) = probe {
            assert_eq!(
                (line.status, line.detail.as_str()),
                (
                    Status::Failed,
                    format!("cannot read {}: {error}", file.display()).as_str()
                ),
                "{scheduler}"
            );
            assert_eq!(
                world.changes(name).len(),
                before,
                "no command that changes anything"
            );
            assert_eq!(
                run_it(&world).status,
                Status::Kept,
                "{scheduler}: still loaded"
            );
        }
        let file = if scheduler == "launchd" {
            file
        } else {
            world.systemd().files_of(&job)[0].0.clone()
        };
        let text = fs::read_to_string(&file).expect("the unit");
        let aside = world.home.join("aside");
        fs::rename(&file, &aside).expect("the unit is moved");
        let copy = world.home.join("copy");
        fs::write(&copy, &text).expect("a copy of the unit");
        for (case, target) in [
            ("a dangling link", world.home.join("nothing")),
            ("a link to a regular file", copy.clone()),
        ] {
            symlink(&target, &file).expect("a link at the unit's name");
            let before = world.changes(name).len();
            let line = run_it(&world);
            fs::remove_file(&file).expect("the link is taken away");
            assert_eq!(
                (line.status, line.detail.as_str()),
                (
                    Status::Failed,
                    format!("cannot read {}: not a regular file", file.display()).as_str()
                ),
                "{scheduler}: {case}"
            );
            assert_eq!(
                world.changes(name).len(),
                before,
                "{scheduler}: {case}: no command that changes anything"
            );
        }
        fs::rename(&aside, &file).expect("the unit is back");
        assert_eq!(
            run_it(&world).status,
            Status::Kept,
            "{scheduler}: still loaded"
        );
    }
}

fn installed(world: &World, job: &Job) {
    let line = run(&world.systemd(), job);
    assert_eq!(line.status, Status::Installed, "{}", line.detail);
}

fn fresh_log(world: &World) -> usize {
    world.calls("systemctl").len()
}

fn seen(world: &World, name: &str) -> Vec<String> {
    fs::read_to_string(world.state.join(format!("{name}.seen")))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_systemd_removal_takes_out_the_run_the_link_and_what_the_manager_holds() {
    let _serial = serial();
    let timer = "bilbo-refresh.timer";
    let service = "bilbo-refresh.service";
    let both = format!("{service} {timer}");
    let user = |words: &str| format!("--user {words}");
    let show = |unit: &str| format!("--user show --property=LoadState --value {unit}");
    let look = || {
        vec![
            user(&format!("is-active {timer}")),
            user(&format!("is-active {service}")),
            show(timer),
            show(service),
        ]
    };

    let world = world();
    let job = world.job("refresh");
    installed(&world, &job);
    let from = fresh_log(&world);
    let line = removal(&world.systemd(), &job);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (Status::Removed, timer),
        "{}",
        line.detail
    );
    let mut wanted = vec![user("is-system-running")];
    wanted.extend(look());
    wanted.extend([
        user(&format!("stop {timer}")),
        user(&format!("disable {timer}")),
        user(&format!("is-active {service}")),
        user("daemon-reload"),
    ]);
    wanted.extend(look());
    assert_eq!(world.calls("systemctl")[from..], wanted);
    assert_eq!(
        seen(&world, "systemctl"),
        [
            format!("{}\t{both}", user(&format!("stop {timer}"))),
            format!("{}\t{both}", user(&format!("disable {timer}"))),
        ]
    );
    assert!(!world.units.join(timer).exists() && !world.units.join(service).exists());
    assert!(
        !world
            .units
            .join("timers.target.wants")
            .join(timer)
            .is_symlink()
    );

    let from = fresh_log(&world);
    let again = removal(&world.systemd(), &job);
    assert_eq!(
        (again.status, again.detail.as_str()),
        (Status::Absent, "no timer bilbo-refresh.timer")
    );
    assert!(
        world.calls("systemctl")[from..]
            .iter()
            .all(|call| !call.contains("stop")
                && !call.contains("disable")
                && !call.contains("reload")),
        "a second removal only asks"
    );

    let world = self::world();
    let job = world.job("refresh");
    installed(&world, &job);
    world
        .programs
        .set("systemctl", &format!("active.{service}"), "activating")
        .expect("a run");
    let line = removal(&world.systemd(), &job);
    assert_eq!(line.status, Status::Removed, "{}", line.detail);
    assert_eq!(
        seen(&world, "systemctl"),
        [
            format!("{}\t{both}", user(&format!("stop {timer}"))),
            format!("{}\t{both}", user(&format!("disable {timer}"))),
            format!("{}\t{both}", user(&format!("stop {service}"))),
        ],
        "both files are there at each of the three changes, in this order"
    );

    let world = self::world();
    let job = world.job("refresh");
    installed(&world, &job);
    world
        .programs
        .flag("systemctl", &format!("starts.{service}"))
        .expect("a run that begins");
    let line = removal(&world.systemd(), &job);
    assert_eq!(line.status, Status::Removed, "{}", line.detail);
    assert!(
        world
            .calls("systemctl")
            .iter()
            .any(|call| call == &user(&format!("stop {service}"))),
        "a run that begins when the timer stops is stopped by the second question"
    );

    let world = self::world();
    let job = world.job("refresh");
    installed(&world, &job);
    world
        .programs
        .flag("systemctl", &format!("stuck.{service}"))
        .expect("a run that stays");
    world
        .programs
        .set("systemctl", &format!("active.{service}"), "activating")
        .expect("a run");
    let line = removal(&world.systemd(), &job);
    assert_eq!(line.status, Status::Failed);
    assert_eq!(
        line.detail,
        format!("{service} is still active after systemctl --user daemon-reload")
    );

    for (unit, name) in [(timer, "timer"), (service, "service")] {
        let world = self::world();
        let job = world.job("refresh");
        installed(&world, &job);
        if unit == service {
            world
                .programs
                .set("systemctl", &format!("active.{service}"), "activating")
                .expect("a run");
        }
        world
            .programs
            .fail_for(
                "systemctl",
                "stop",
                Some(unit),
                false,
                Fail::Exit("stop broke"),
                false,
            )
            .expect("fail");
        let line = removal(&world.systemd(), &job);
        assert_eq!(line.status, Status::Failed, "{name}");
        assert_eq!(
            line.detail,
            format!("systemctl --user stop {unit} failed: stop broke"),
            "{name}"
        );
        assert!(world.units.join(timer).exists() && world.units.join(service).exists());
        assert!(
            seen(&world, "systemctl")
                .contains(&format!("{}\t{both}", user(&format!("stop {unit}")))),
            "{name}: the stop is seen as asked, though it failed"
        );
        if unit == timer {
            assert!(
                world
                    .units
                    .join("timers.target.wants")
                    .join(timer)
                    .is_symlink()
            );
            assert_eq!(
                world.calls("systemctl").last(),
                Some(&user(&format!("stop {timer}"))),
                "the log ends at that stop"
            );
        }
    }

    let world = self::world();
    let job = world.job("refresh");
    world
        .programs
        .set("systemctl", &format!("held.{timer}"), "held")
        .expect("held");
    let line = removal(&world.systemd(), &job);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (Status::Removed, timer),
        "only the manager holds the timer"
    );
    assert!(
        world
            .calls("systemctl")
            .iter()
            .any(|call| call == "--user daemon-reload")
    );

    let world = self::world();
    let job = world.job("refresh");
    world
        .programs
        .set("systemctl", &format!("held.{service}"), "held")
        .expect("held");
    let line = removal(&world.systemd(), &job);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (Status::Removed, timer),
        "only the manager holds the service, as after a reload that failed"
    );
    assert!(
        world
            .calls("systemctl")
            .iter()
            .any(|call| call == "--user daemon-reload")
    );

    let world = self::world();
    let job = world.job("refresh");
    installed(&world, &job);
    fs::remove_file(world.units.join(timer)).expect("a timer file gone");
    fs::remove_file(world.units.join(service)).expect("a service file gone");
    assert!(
        world
            .units
            .join("timers.target.wants")
            .join(timer)
            .is_symlink()
    );
    let line = removal(&world.systemd(), &job);
    assert_eq!(
        line.status,
        Status::Removed,
        "a dangling link is a timer left: {}",
        line.detail
    );
    assert!(
        !world
            .units
            .join("timers.target.wants")
            .join(timer)
            .is_symlink()
    );

    let world = self::world();
    let job = world.job("refresh");
    installed(&world, &job);
    world
        .programs
        .fail(
            "systemctl",
            "daemon-reload",
            Fail::Exit("reload broke"),
            false,
        )
        .expect("fail");
    let from = fresh_log(&world);
    let line = removal(&world.systemd(), &job);
    assert_eq!(line.status, Status::Failed);
    assert_eq!(
        line.detail,
        "systemctl --user daemon-reload failed: reload broke"
    );
    let log = world.calls("systemctl")[from..].to_vec();
    let first_change = log
        .iter()
        .position(|call| call.contains("stop"))
        .expect("a change");
    assert_eq!(log[1..first_change], look(), "the look comes whole first");
    assert_eq!(
        log.last().map(String::as_str),
        Some("--user daemon-reload"),
        "nothing follows the reload"
    );

    let world = self::world();
    let job = world.job("refresh");
    installed(&world, &job);
    for unit in [timer, service] {
        world
            .programs
            .flag("systemctl", &format!("keep.{unit}"))
            .expect("kept");
        world
            .programs
            .set("systemctl", &format!("held.{unit}"), "held")
            .expect("held");
    }
    let line = removal(&world.systemd(), &job);
    assert_eq!(line.status, Status::Failed);
    assert_eq!(
        line.detail,
        format!("systemd still holds {service} after systemctl --user daemon-reload"),
        "both held: the service is named"
    );

    let world = self::world();
    let job = world.job("refresh");
    installed(&world, &job);
    world
        .programs
        .flag("systemctl", &format!("keep.{timer}"))
        .expect("kept");
    world
        .programs
        .set("systemctl", &format!("held.{timer}"), "held")
        .expect("held");
    let line = removal(&world.systemd(), &job);
    assert_eq!(line.status, Status::Failed);
    assert_eq!(
        line.detail,
        format!("systemd still holds {timer} after systemctl --user daemon-reload")
    );

    let world = self::world();
    let job = world.job("refresh");
    installed(&world, &job);
    world
        .programs
        .flag("systemctl", "silent.show")
        .expect("no line");
    let from = fresh_log(&world);
    let line = removal(&world.systemd(), &job);
    assert_eq!(line.status, Status::Failed);
    assert_eq!(
        line.detail,
        format!(
            "{} answered nothing",
            show(timer).replacen("--user", "systemctl --user", 1)
        )
    );
    assert!(
        world.calls("systemctl")[from..].iter().all(|call| {
            !call.contains("stop") && !call.contains("disable") && !call.contains("daemon-reload")
        }),
        "nothing changes before the look is whole"
    );
    assert!(world.units.join(timer).exists() && world.units.join(service).exists());
    assert!(
        world
            .units
            .join("timers.target.wants")
            .join(timer)
            .is_symlink()
    );
}

struct Shut(PathBuf);

impl Drop for Shut {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
    }
}

#[test]
fn a_launchd_removal_looks_whole_before_it_boots_out() {
    let _serial = serial();
    let label = "io.github.delucca.bilbo.refresh";
    let target = format!("gui/501/{label}");

    let world = world();
    let job = world.job("refresh");
    installed_by_launchd(&world, &job);
    let from = world.calls("launchctl").len();
    let line = removal(&world.launchd(), &job);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (Status::Removed, label),
        "{}",
        line.detail
    );
    assert_eq!(
        world.calls("launchctl")[from..],
        [
            format!("print {target}"),
            format!("bootout --wait {target}"),
            format!("print {target}"),
        ]
    );
    assert_eq!(
        fs::read_to_string(world.state.join("launchctl.seen")).expect("a record"),
        format!("bootout --wait {target}\tthere\n")
    );
    assert!(!plist(&world, &job).exists());

    let world = self::world();
    let job = world.job("refresh");
    fs::create_dir_all(plist(&world, &job).parent().expect("a folder")).expect("a folder");
    fs::write(plist(&world, &job), "old\n").expect("a plist");
    let line = removal(&world.launchd(), &job);
    assert_eq!(
        (line.status, line.detail.as_str()),
        (Status::Removed, label)
    );
    assert_eq!(
        world.calls("launchctl"),
        [format!("print {target}"), format!("print {target}")]
    );
    assert!(!plist(&world, &job).exists());

    let world = self::world();
    let job = world.job("refresh");
    installed_by_launchd(&world, &job);
    world
        .programs
        .fail("launchctl", "print", Fail::Exit("print broke"), false)
        .expect("fail");
    let from = world.calls("launchctl").len();
    let line = removal(&world.launchd(), &job);
    assert_eq!(line.status, Status::Failed);
    assert_eq!(
        line.detail,
        format!("launchctl print {target} failed: print broke")
    );
    assert_eq!(
        world.calls("launchctl")[from..],
        [format!("print {target}")]
    );
    assert!(plist(&world, &job).exists());
    assert_eq!(
        world.programs.loaded_labels(),
        std::slice::from_ref(&target)
    );

    let world = self::world();
    let job = world.job("refresh");
    installed_by_launchd(&world, &job);
    let probe = world.home.join("probe-closed");
    fs::create_dir_all(probe.join("inner")).expect("a folder");
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o000)).expect("a mode");
    let closed = fs::metadata(probe.join("inner")).is_err();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).expect("a mode");
    if closed {
        let agents = plist(&world, &job)
            .parent()
            .expect("a folder")
            .to_path_buf();
        let _shut = Shut(agents.clone());
        fs::set_permissions(&agents, fs::Permissions::from_mode(0o000)).expect("a mode");
        let error = fs::symlink_metadata(plist(&world, &job)).expect_err("shut");
        let from = world.calls("launchctl").len();
        let line = removal(&world.launchd(), &job);
        assert_eq!(line.status, Status::Failed);
        assert_eq!(
            line.detail,
            format!("cannot read {}: {error}", plist(&world, &job).display())
        );
        assert!(
            world.calls("launchctl")[from..]
                .iter()
                .all(|call| !call.starts_with("bootout")),
            "no bootout before the look is whole"
        );
    }
}

fn installed_by_launchd(world: &World, job: &Job) {
    let line = run(&world.launchd(), job);
    assert_eq!(line.status, Status::Installed, "{}", line.detail);
}
