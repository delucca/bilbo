//! The index timer files and their launchd or systemd commands.

use crate::command::{Output, Runner, first_line};
use std::path::{Path, PathBuf};

pub const LABEL: &str = "io.github.delucca.bilbo.index";

const ID: &str = "/usr/bin/id";
const UNIT: &str = "bilbo-index.timer";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Launchd,
    Systemd,
}

/// Launchd on macOS, Systemd on Linux, None elsewhere.
pub fn platform() -> Option<Platform> {
    if cfg!(target_os = "macos") {
        Some(Platform::Launchd)
    } else if cfg!(target_os = "linux") {
        Some(Platform::Systemd)
    } else {
        None
    }
}

pub struct Job {
    pub exe: PathBuf,
    pub minutes: u32,
    pub log: PathBuf,
    pub env: Vec<(&'static str, String)>,
}

pub struct Place {
    pub home: PathBuf,
    pub config_home: PathBuf,
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn quoted(value: &str, dollar: bool) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '$' if dollar => out.push_str("$$"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn plist(job: &Job) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n<dict>\n",
    );
    if !job.env.is_empty() {
        out.push_str("\t<key>EnvironmentVariables</key>\n\t<dict>\n");
        for (name, value) in &job.env {
            out.push_str(&format!(
                "\t\t<key>{name}</key>\n\t\t<string>{}</string>\n",
                xml(value)
            ));
        }
        out.push_str("\t</dict>\n");
    }
    let log = xml(&job.log.to_string_lossy());
    out.push_str(&format!(
        "\t<key>Label</key>\n\t<string>{LABEL}</string>\n\
\t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{}</string>\n\t\t<string>index</string>\n\t</array>\n\
\t<key>RunAtLoad</key>\n\t<false/>\n\
\t<key>StandardErrorPath</key>\n\t<string>{log}</string>\n\
\t<key>StandardOutPath</key>\n\t<string>{log}</string>\n\
\t<key>StartInterval</key>\n\t<integer>{}</integer>\n\
</dict>\n</plist>\n",
        xml(&job.exe.to_string_lossy()),
        u64::from(job.minutes) * 60
    ));
    out
}

pub fn service(job: &Job) -> Result<String, String> {
    let exe = job.exe.to_string_lossy();
    let log = job.log.to_string_lossy();
    if exe.contains(['\n', '\r']) {
        return Err("the bilbo path holds a newline".into());
    }
    if log.contains(['\n', '\r']) {
        return Err("the log path holds a newline".into());
    }
    let mut out =
        String::from("[Unit]\nDescription=Index the bilbo store\n\n[Service]\nType=oneshot\n");
    for (name, value) in &job.env {
        if value.contains(['\n', '\r']) {
            return Err(format!("{name} holds a newline"));
        }
        out.push_str(&format!(
            "Environment={}\n",
            quoted(&format!("{name}={value}"), false)
        ));
    }
    let append = log.replace('%', "%%");
    out.push_str(&format!(
        "ExecStart={} index\nStandardOutput=append:{append}\nStandardError=append:{append}\n",
        quoted(&exe, true)
    ));
    Ok(out)
}

pub fn timer(job: &Job) -> String {
    let n = job.minutes;
    format!(
        "[Unit]\nDescription=Run bilbo index every {n} min\n\n[Timer]\nOnActiveSec={n}min\nOnUnitActiveSec={n}min\n\n[Install]\nWantedBy=timers.target\n"
    )
}

fn units(place: &Place) -> PathBuf {
    place.config_home.join("systemd").join("user")
}

fn wants(place: &Place) -> PathBuf {
    units(place).join("timers.target.wants").join(UNIT)
}

pub fn paths(platform: Platform, place: &Place) -> Vec<PathBuf> {
    match platform {
        Platform::Launchd => vec![
            place
                .home
                .join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist")),
        ],
        Platform::Systemd => vec![
            units(place).join("bilbo-index.service"),
            units(place).join(UNIT),
        ],
    }
}

pub fn files(
    platform: Platform,
    place: &Place,
    job: &Job,
) -> Result<Vec<(PathBuf, String)>, String> {
    let texts = match platform {
        Platform::Launchd => vec![plist(job)],
        Platform::Systemd => vec![service(job)?, timer(job)],
    };
    Ok(paths(platform, place).into_iter().zip(texts).collect())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Current {
    Missing,
    Same,
    Different,
}

/// Missing when no path exists, Same when every file holds exactly its text, else Different.
pub fn current(files: &[(PathBuf, String)]) -> Current {
    if files.iter().all(|(path, _)| !path.exists()) {
        Current::Missing
    } else if files
        .iter()
        .all(|(path, text)| std::fs::read_to_string(path).is_ok_and(|have| &have == text))
    {
        Current::Same
    } else {
        Current::Different
    }
}

pub fn installed(platform: Platform, place: &Place) -> bool {
    paths(platform, place).iter().any(|path| path.exists())
}

/// Whether the user manager answers: `systemctl --user is-system-running` prints a live state.
pub fn session(runner: &dyn Runner, systemctl: &Path) -> bool {
    runner
        .run(systemctl, &args(&["--user", "is-system-running"]))
        .is_ok_and(|out| {
            matches!(
                first_line(&out.stdout),
                "running" | "degraded" | "starting" | "initializing" | "maintenance"
            )
        })
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|a| a.to_string()).collect()
}

fn failure(program: &Path, args: &[String], out: &Output) -> String {
    let name = program
        .file_name()
        .map_or_else(|| program.to_string_lossy(), |n| n.to_string_lossy());
    let reason = match first_line(&out.stderr) {
        "" => match out.code {
            Some(code) => format!("exit {code}"),
            None => "ended by a signal".to_string(),
        },
        line => line.to_string(),
    };
    format!("{name} {} failed: {reason}", args.join(" "))
}

fn run_ok(runner: &dyn Runner, program: &Path, args: &[String]) -> Result<(), String> {
    let out = runner.run(program, args)?;
    if out.success() {
        Ok(())
    } else {
        Err(failure(program, args, &out))
    }
}

fn uid(runner: &dyn Runner) -> Result<String, String> {
    let out = runner
        .run(Path::new(ID), &args(&["-u"]))
        .map_err(|e| format!("cannot read the user id: {e}"))?;
    let id = first_line(&out.stdout);
    if out.success() && !id.is_empty() {
        Ok(id.to_string())
    } else {
        let reason = match first_line(&out.stderr) {
            "" => format!("{ID} -u answered nothing"),
            line => line.to_string(),
        };
        Err(format!("cannot read the user id: {reason}"))
    }
}

/// `launchctl bootout`, where a job that is not loaded (exit 3 or 113) is fine.
fn bootout(runner: &dyn Runner, tool: &Path, uid: &str) -> Result<(), String> {
    let args = args(&["bootout", &format!("gui/{uid}/{LABEL}")]);
    let out = runner.run(tool, &args)?;
    if out.success() || matches!(out.code, Some(3 | 113)) {
        Ok(())
    } else {
        Err(failure(tool, &args, &out))
    }
}

fn create_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let cannot = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    if let Some(dir) = path.parent() {
        create_dir(dir)?;
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp = path.with_file_name(format!(".{name}.tmp-{}", std::process::id()));
    std::fs::write(&temp, text)
        .and_then(|()| std::fs::rename(&temp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            cannot(e)
        })
}

fn delete(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("cannot delete {}: {e}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Creates the log folder, writes every file atomically, then loads the job.
/// Launchd: `id -u`, `bootout` (exit 3 or 113 ignored), `bootstrap`.
/// Systemd: `daemon-reload`, `enable` and `restart` of the timer.
/// When a write or load step fails, the files this call wrote (and the systemd wants link) are
/// deleted, so the next run sees no timer; the original error is the one returned.
pub fn install(
    platform: Platform,
    runner: &dyn Runner,
    tool: &Path,
    job: &Job,
    files: &[(PathBuf, String)],
) -> Result<(), String> {
    if let Some(dir) = job.log.parent() {
        create_dir(dir)?;
    }
    let loaded = write_all(files).and_then(|()| load(platform, runner, tool, files));
    if loaded.is_err() {
        for (path, _) in files {
            let _ = delete(path);
            if platform == Platform::Systemd
                && let Some(dir) = path.parent()
            {
                let _ = delete(&dir.join("timers.target.wants").join(UNIT));
            }
        }
    }
    loaded
}

fn write_all(files: &[(PathBuf, String)]) -> Result<(), String> {
    for (path, text) in files {
        write_atomic(path, text)?;
    }
    Ok(())
}

fn load(
    platform: Platform,
    runner: &dyn Runner,
    tool: &Path,
    files: &[(PathBuf, String)],
) -> Result<(), String> {
    match platform {
        Platform::Launchd => {
            let uid = uid(runner)?;
            bootout(runner, tool, &uid)?;
            let plist = files
                .first()
                .map(|(path, _)| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            run_ok(
                runner,
                tool,
                &["bootstrap".into(), format!("gui/{uid}"), plist],
            )
        }
        Platform::Systemd => {
            for step in [
                &["--user", "daemon-reload"][..],
                &["--user", "enable", UNIT],
                &["--user", "restart", UNIT],
            ] {
                run_ok(runner, tool, &args(step))?;
            }
            Ok(())
        }
    }
}

/// The installed interval in minutes: the plist's `StartInterval` seconds / 60, or the timer's
/// `OnUnitActiveSec=<n>min`. None when the file is missing or does not parse.
pub fn minutes(platform: Platform, place: &Place) -> Option<u32> {
    let path = match platform {
        Platform::Launchd => paths(platform, place).into_iter().next()?,
        Platform::Systemd => units(place).join(UNIT),
    };
    let text = std::fs::read_to_string(path).ok()?;
    match platform {
        Platform::Launchd => {
            let rest = text.split("<key>StartInterval</key>").nth(1)?;
            let seconds: u64 = rest
                .split("<integer>")
                .nth(1)?
                .split("</integer>")
                .next()?
                .trim()
                .parse()
                .ok()?;
            u32::try_from(seconds / 60).ok().filter(|m| *m > 0)
        }
        Platform::Systemd => text
            .lines()
            .find_map(|line| line.strip_prefix("OnUnitActiveSec="))?
            .trim()
            .strip_suffix("min")?
            .parse()
            .ok()
            .filter(|m| *m > 0),
    }
}

/// Without `tool` it fails and leaves every file, so a rerun with the tool on PATH can finish:
/// the manager would otherwise keep running a job whose files are gone.
/// Launchd: `id -u`, `bootout` (exit 3 or 113 ignored), delete the plist.
/// Systemd: with a live session, `disable --now`; delete both units and the wants link;
/// then `daemon-reload` (same condition). Without a session nothing is loaded, so only the files go.
pub fn uninstall(
    platform: Platform,
    runner: &dyn Runner,
    tool: Option<&Path>,
    place: &Place,
) -> Result<(), String> {
    let Some(tool) = tool else {
        return Err(format!(
            "{} not found on PATH",
            match platform {
                Platform::Launchd => "launchctl",
                Platform::Systemd => "systemctl",
            }
        ));
    };
    match platform {
        Platform::Launchd => {
            let uid = uid(runner)?;
            bootout(runner, tool, &uid)?;
        }
        Platform::Systemd => {
            let live = Some(tool).filter(|tool| session(runner, tool));
            if let Some(tool) = live {
                run_ok(runner, tool, &args(&["--user", "disable", "--now", UNIT]))?;
            }
            for path in paths(platform, place) {
                delete(&path)?;
            }
            delete(&wants(place))?;
            if let Some(tool) = live {
                run_ok(runner, tool, &args(&["--user", "daemon-reload"]))?;
            }
            return Ok(());
        }
    }
    for path in paths(platform, place) {
        delete(&path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-timer-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    /// Records every call and answers from a queue; an empty queue answers exit 0.
    struct Script {
        calls: RefCell<Vec<String>>,
        answers: RefCell<VecDeque<Output>>,
    }

    fn out(code: i32, stdout: &str, stderr: &str) -> Output {
        Output {
            code: Some(code),
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    fn script(answers: Vec<Output>) -> Script {
        Script {
            calls: RefCell::new(Vec::new()),
            answers: RefCell::new(answers.into()),
        }
    }

    impl Runner for Script {
        fn run(&self, program: &Path, args: &[String]) -> Result<Output, String> {
            self.calls
                .borrow_mut()
                .push(format!("{} {}", program.display(), args.join(" ")));
            Ok(self
                .answers
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| out(0, "", "")))
        }
    }

    fn job(minutes: u32, env: Vec<(&'static str, String)>) -> Job {
        Job {
            exe: "/opt/bilbo/bin/bilbo".into(),
            minutes,
            log: "/Users/a/.local/state/bilbo/index.log".into(),
            env,
        }
    }

    fn place(root: &Path) -> Place {
        Place {
            home: root.join("home"),
            config_home: root.join("config"),
        }
    }

    #[test]
    fn plist_for_15_minutes() {
        let expected = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
\t<key>Label</key>\n\
\t<string>io.github.delucca.bilbo.index</string>\n\
\t<key>ProgramArguments</key>\n\
\t<array>\n\
\t\t<string>/opt/bilbo/bin/bilbo</string>\n\
\t\t<string>index</string>\n\
\t</array>\n\
\t<key>RunAtLoad</key>\n\
\t<false/>\n\
\t<key>StandardErrorPath</key>\n\
\t<string>/Users/a/.local/state/bilbo/index.log</string>\n\
\t<key>StandardOutPath</key>\n\
\t<string>/Users/a/.local/state/bilbo/index.log</string>\n\
\t<key>StartInterval</key>\n\
\t<integer>900</integer>\n\
</dict>\n\
</plist>\n";
        assert_eq!(plist(&job(15, vec![])), expected);
    }

    #[test]
    fn plist_for_30_minutes_with_locations() {
        let env = vec![
            ("BILBO_HOME", "/data/bilbo".to_string()),
            ("XDG_DATA_HOME", "/data".to_string()),
        ];
        let text = plist(&job(30, env));
        let expected_env = "\t<key>EnvironmentVariables</key>\n\
\t<dict>\n\
\t\t<key>BILBO_HOME</key>\n\
\t\t<string>/data/bilbo</string>\n\
\t\t<key>XDG_DATA_HOME</key>\n\
\t\t<string>/data</string>\n\
\t</dict>\n\
\t<key>Label</key>\n";
        assert!(text.contains(&format!("<dict>\n{expected_env}")), "{text}");
        assert!(text.contains("<integer>1800</integer>"), "{text}");
    }

    #[test]
    fn plist_escapes_xml() {
        let mut j = job(15, vec![("BILBO_HOME", "/a&b/<c>".to_string())]);
        j.exe = "/o&p/bilbo".into();
        let text = plist(&j);
        assert!(
            text.contains("<string>/a&amp;b/&lt;c&gt;</string>"),
            "{text}"
        );
        assert!(text.contains("<string>/o&amp;p/bilbo</string>"), "{text}");
    }

    #[test]
    fn service_and_timer_for_15_and_30() {
        let env = vec![("BILBO_HOME", "/data/bilbo".to_string())];
        let mut j = job(15, env);
        j.log = "/home/a/.local/state/bilbo/index.log".into();
        let expected = "[Unit]\n\
Description=Index the bilbo store\n\
\n\
[Service]\n\
Type=oneshot\n\
Environment=\"BILBO_HOME=/data/bilbo\"\n\
ExecStart=\"/opt/bilbo/bin/bilbo\" index\n\
StandardOutput=append:/home/a/.local/state/bilbo/index.log\n\
StandardError=append:/home/a/.local/state/bilbo/index.log\n";
        assert_eq!(service(&j).unwrap(), expected);
        assert!(!service(&job(15, vec![])).unwrap().contains("Environment="));
        let expected_timer = |n: u32| {
            format!(
                "[Unit]\nDescription=Run bilbo index every {n} min\n\n[Timer]\nOnActiveSec={n}min\nOnUnitActiveSec={n}min\n\n[Install]\nWantedBy=timers.target\n"
            )
        };
        assert_eq!(timer(&j), expected_timer(15));
        j.minutes = 30;
        assert_eq!(timer(&j), expected_timer(30));
    }

    #[test]
    fn service_quotes_and_escapes() {
        let mut j = job(15, vec![("BILBO_HOME", "/d \"q\"/100%\\x".to_string())]);
        j.exe = "/opt/my bilbo/$b\"%/bilbo".into();
        j.log = "/state/100%/index.log".into();
        let text = service(&j).unwrap();
        assert!(
            text.contains("Environment=\"BILBO_HOME=/d \\\"q\\\"/100%%\\\\x\"\n"),
            "{text}"
        );
        assert!(
            text.contains("ExecStart=\"/opt/my bilbo/$$b\\\"%%/bilbo\" index\n"),
            "{text}"
        );
        assert!(
            text.contains("StandardOutput=append:/state/100%%/index.log\n"),
            "{text}"
        );
    }

    #[test]
    fn service_refuses_a_newline() {
        let j = job(15, vec![("XDG_DATA_HOME", "/a\nb".to_string())]);
        assert_eq!(service(&j).unwrap_err(), "XDG_DATA_HOME holds a newline");
        let mut j = job(15, vec![]);
        j.exe = "/a\nb".into();
        assert_eq!(service(&j).unwrap_err(), "the bilbo path holds a newline");
        let mut j = job(15, vec![]);
        j.log = "/a\nb".into();
        assert_eq!(service(&j).unwrap_err(), "the log path holds a newline");
    }

    #[test]
    fn paths_and_files_per_platform() {
        let p = Place {
            home: "/h".into(),
            config_home: "/c".into(),
        };
        assert_eq!(
            paths(Platform::Launchd, &p),
            [PathBuf::from(
                "/h/Library/LaunchAgents/io.github.delucca.bilbo.index.plist"
            )]
        );
        assert_eq!(
            paths(Platform::Systemd, &p),
            [
                PathBuf::from("/c/systemd/user/bilbo-index.service"),
                PathBuf::from("/c/systemd/user/bilbo-index.timer")
            ]
        );
        let f = files(Platform::Systemd, &p, &job(15, vec![])).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f[1].1, timer(&job(15, vec![])));
    }

    #[test]
    fn current_missing_same_different() {
        let s = scratch("current");
        let a = s.0.join("a");
        let b = s.0.join("b");
        let files = vec![
            (a.clone(), "one".to_string()),
            (b.clone(), "two".to_string()),
        ];
        assert_eq!(current(&files), Current::Missing);
        std::fs::write(&a, "one").unwrap();
        assert_eq!(current(&files), Current::Different);
        std::fs::write(&b, "two").unwrap();
        assert_eq!(current(&files), Current::Same);
        std::fs::write(&b, "other").unwrap();
        assert_eq!(current(&files), Current::Different);
    }

    #[test]
    fn installed_when_any_path_exists() {
        let s = scratch("installed");
        let p = place(&s.0);
        assert!(!installed(Platform::Systemd, &p));
        let f = files(Platform::Systemd, &p, &job(15, vec![])).unwrap();
        write_atomic(&f[1].0, &f[1].1).unwrap();
        assert!(installed(Platform::Systemd, &p));
    }

    #[test]
    fn install_launchd_commands() {
        let s = scratch("launchd");
        let p = place(&s.0);
        let mut j = job(15, vec![]);
        j.log = s.0.join("state/bilbo/index.log");
        let f = files(Platform::Launchd, &p, &j).unwrap();
        let runner = script(vec![out(0, "501\n", "")]);
        install(
            Platform::Launchd,
            &runner,
            Path::new("/x/launchctl"),
            &j,
            &f,
        )
        .unwrap();
        let plist = f[0].0.display();
        assert_eq!(
            *runner.calls.borrow(),
            [
                "/usr/bin/id -u".to_string(),
                format!("/x/launchctl bootout gui/501/{LABEL}"),
                format!("/x/launchctl bootstrap gui/501 {plist}"),
            ]
        );
        assert_eq!(std::fs::read_to_string(&f[0].0).unwrap(), f[0].1);
        assert!(s.0.join("state/bilbo").is_dir());
    }

    #[test]
    fn install_systemd_commands() {
        let s = scratch("systemd");
        let p = place(&s.0);
        let mut j = job(15, vec![]);
        j.log = s.0.join("state/bilbo/index.log");
        let f = files(Platform::Systemd, &p, &j).unwrap();
        let runner = script(vec![]);
        install(
            Platform::Systemd,
            &runner,
            Path::new("/x/systemctl"),
            &j,
            &f,
        )
        .unwrap();
        assert_eq!(
            *runner.calls.borrow(),
            [
                "/x/systemctl --user daemon-reload",
                "/x/systemctl --user enable bilbo-index.timer",
                "/x/systemctl --user restart bilbo-index.timer",
            ]
        );
        for (path, text) in &f {
            assert_eq!(&std::fs::read_to_string(path).unwrap(), text);
        }
        assert!(s.0.join("state/bilbo").is_dir());
    }

    #[test]
    fn install_ignores_bootout_3_and_113() {
        for code in [3, 113] {
            let s = scratch(&format!("bootout{code}"));
            let p = place(&s.0);
            let mut j = job(15, vec![]);
            j.log = s.0.join("state/index.log");
            let f = files(Platform::Launchd, &p, &j).unwrap();
            let runner = script(vec![
                out(0, "501\n", ""),
                out(code, "", "Boot-out failed: 3: No such process"),
            ]);
            install(
                Platform::Launchd,
                &runner,
                Path::new("/x/launchctl"),
                &j,
                &f,
            )
            .unwrap();
            assert_eq!(runner.calls.borrow().len(), 3);
        }
    }

    #[test]
    fn install_fails_on_bootstrap_5() {
        let s = scratch("bootstrap5");
        let p = place(&s.0);
        let mut j = job(15, vec![]);
        j.log = s.0.join("state/index.log");
        let f = files(Platform::Launchd, &p, &j).unwrap();
        let runner = script(vec![
            out(0, "501\n", ""),
            out(0, "", ""),
            out(5, "", "Bootstrap failed: 5: Input/output error\n"),
        ]);
        let err = install(
            Platform::Launchd,
            &runner,
            Path::new("/x/launchctl"),
            &j,
            &f,
        )
        .unwrap_err();
        assert_eq!(
            err,
            format!(
                "launchctl bootstrap gui/501 {} failed: Bootstrap failed: 5: Input/output error",
                f[0].0.display()
            )
        );
    }

    #[test]
    fn install_launchd_bootstrap_failure_leaves_no_plist() {
        let s = scratch("rollback-launchd");
        let p = place(&s.0);
        let mut j = job(15, vec![]);
        j.log = s.0.join("state/index.log");
        let f = files(Platform::Launchd, &p, &j).unwrap();
        let runner = script(vec![out(0, "501\n", ""), out(0, "", ""), out(5, "", "no")]);
        install(
            Platform::Launchd,
            &runner,
            Path::new("/x/launchctl"),
            &j,
            &f,
        )
        .unwrap_err();
        assert!(!installed(Platform::Launchd, &p));
        assert_eq!(current(&f), Current::Missing);
    }

    #[test]
    fn install_systemd_enable_failure_leaves_no_units() {
        let s = scratch("rollback-systemd");
        let p = place(&s.0);
        let mut j = job(15, vec![]);
        j.log = s.0.join("state/index.log");
        let f = files(Platform::Systemd, &p, &j).unwrap();
        let wants_link = wants(&p);
        std::fs::create_dir_all(wants_link.parent().unwrap()).unwrap();
        std::fs::write(&wants_link, "").unwrap();
        let runner = script(vec![out(0, "", ""), out(1, "", "enable broke")]);
        let err = install(
            Platform::Systemd,
            &runner,
            Path::new("/x/systemctl"),
            &j,
            &f,
        )
        .unwrap_err();
        assert!(err.ends_with("failed: enable broke"), "{err}");
        assert!(!installed(Platform::Systemd, &p));
        assert!(!wants_link.exists());
    }

    #[test]
    fn install_success_keeps_the_files() {
        for platform in [Platform::Launchd, Platform::Systemd] {
            let s = scratch("keep");
            let p = place(&s.0);
            let mut j = job(15, vec![]);
            j.log = s.0.join("state/index.log");
            let f = files(platform, &p, &j).unwrap();
            let runner = script(vec![out(0, "501\n", "")]);
            install(platform, &runner, Path::new("/x/tool"), &j, &f).unwrap();
            assert_eq!(current(&f), Current::Same);
        }
    }

    #[test]
    fn minutes_reads_what_install_writes() {
        for (platform, m) in [(Platform::Launchd, 15), (Platform::Systemd, 30)] {
            let s = scratch("minutes");
            let p = place(&s.0);
            assert_eq!(minutes(platform, &p), None);
            let f = files(platform, &p, &job(m, vec![])).unwrap();
            for (path, text) in &f {
                write_atomic(path, text).unwrap();
            }
            assert_eq!(minutes(platform, &p), Some(m));
            std::fs::write(&f[f.len() - 1].0, "garbage").unwrap();
            assert_eq!(minutes(platform, &p), None);
        }
    }

    #[test]
    fn install_fails_on_a_bad_user_id_and_a_bad_exit() {
        let s = scratch("badid");
        let p = place(&s.0);
        let mut j = job(15, vec![]);
        j.log = s.0.join("state/index.log");
        let f = files(Platform::Launchd, &p, &j).unwrap();
        let runner = script(vec![out(1, "", "no such user")]);
        let err = install(
            Platform::Launchd,
            &runner,
            Path::new("/x/launchctl"),
            &j,
            &f,
        )
        .unwrap_err();
        assert_eq!(err, "cannot read the user id: no such user");
        let f = files(Platform::Systemd, &p, &j).unwrap();
        let runner = script(vec![out(1, "", "")]);
        let err = install(
            Platform::Systemd,
            &runner,
            Path::new("/x/systemctl"),
            &j,
            &f,
        )
        .unwrap_err();
        assert_eq!(err, "systemctl --user daemon-reload failed: exit 1");
    }

    #[test]
    fn uninstall_launchd() {
        let s = scratch("un-launchd");
        let p = place(&s.0);
        let f = files(Platform::Launchd, &p, &job(15, vec![])).unwrap();
        write_atomic(&f[0].0, &f[0].1).unwrap();
        let runner = script(vec![out(0, "501\n", ""), out(3, "", "No such process")]);
        uninstall(
            Platform::Launchd,
            &runner,
            Some(Path::new("/x/launchctl")),
            &p,
        )
        .unwrap();
        assert_eq!(
            *runner.calls.borrow(),
            [
                "/usr/bin/id -u".to_string(),
                format!("/x/launchctl bootout gui/501/{LABEL}")
            ]
        );
        assert!(!f[0].0.exists());
    }

    fn systemd_installed(p: &Place) {
        for (path, text) in files(Platform::Systemd, p, &job(15, vec![])).unwrap() {
            write_atomic(&path, &text).unwrap();
        }
        write_atomic(&wants(p), "link").unwrap();
    }

    #[test]
    fn uninstall_systemd_with_session() {
        let s = scratch("un-systemd");
        let p = place(&s.0);
        systemd_installed(&p);
        let runner = script(vec![out(0, "running\n", "")]);
        uninstall(
            Platform::Systemd,
            &runner,
            Some(Path::new("/x/systemctl")),
            &p,
        )
        .unwrap();
        assert_eq!(
            *runner.calls.borrow(),
            [
                "/x/systemctl --user is-system-running",
                "/x/systemctl --user disable --now bilbo-index.timer",
                "/x/systemctl --user daemon-reload",
            ]
        );
        assert!(!installed(Platform::Systemd, &p));
        assert!(!wants(&p).exists());
    }

    #[test]
    fn uninstall_systemd_without_session() {
        let s = scratch("un-nosession");
        let p = place(&s.0);
        systemd_installed(&p);
        let runner = script(vec![out(1, "offline\n", "")]);
        uninstall(
            Platform::Systemd,
            &runner,
            Some(Path::new("/x/systemctl")),
            &p,
        )
        .unwrap();
        assert_eq!(
            *runner.calls.borrow(),
            ["/x/systemctl --user is-system-running"]
        );
        assert!(!installed(Platform::Systemd, &p));
        assert!(!wants(&p).exists());
    }

    #[test]
    fn uninstall_without_the_tool_fails_and_keeps_the_files() {
        for (platform, name) in [
            (Platform::Systemd, "systemctl"),
            (Platform::Launchd, "launchctl"),
        ] {
            let s = scratch(&format!("un-notool-{name}"));
            let p = place(&s.0);
            match platform {
                Platform::Systemd => systemd_installed(&p),
                Platform::Launchd => {
                    let f = files(platform, &p, &job(15, vec![])).unwrap();
                    write_atomic(&f[0].0, &f[0].1).unwrap();
                }
            }
            let runner = script(vec![]);
            let err = uninstall(platform, &runner, None, &p).unwrap_err();
            assert_eq!(err, format!("{name} not found on PATH"));
            assert!(runner.calls.borrow().is_empty());
            assert!(installed(platform, &p));
        }
    }

    #[test]
    fn session_states() {
        let tool = Path::new("/x/systemctl");
        for state in [
            "running",
            "degraded",
            "starting",
            "initializing",
            "maintenance",
        ] {
            let exit = if state == "running" { 0 } else { 1 };
            let runner = script(vec![out(exit, &format!("{state}\n"), "")]);
            assert!(session(&runner, tool), "{state}");
        }
        for answer in [
            out(1, "offline\n", ""),
            out(1, "", ""),
            out(1, "", "Failed to connect to bus: No medium found"),
        ] {
            assert!(!session(&script(vec![answer]), tool));
        }
    }
}
