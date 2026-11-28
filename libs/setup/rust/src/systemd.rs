use std::path::{Path, PathBuf};

use crate::disk;
use crate::keys::waited;
use crate::plugin::ended_badly;
use crate::program::{self, Bounds, End, Ran, first_stderr_line};
use crate::report::Status;
use crate::timer::{Files, Job, Left, Loaded, LogPath, Manager, Scheduler};

pub fn timer_unit(job: &Job) -> String {
    format!("bilbo-{}.timer", job.verb)
}

pub fn service_unit(job: &Job) -> String {
    format!("bilbo-{}.service", job.verb)
}

fn quoted(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
}

fn description(job: &Job) -> &'static str {
    if job.verb == "refresh" {
        "Keep what every bilbo follow gained or changed"
    } else {
        "Write notes about the entries bilbo keeps"
    }
}

pub fn service(job: &Job) -> String {
    let mut text = format!(
        "[Unit]\nDescription={}\n\n[Service]\nType=oneshot\n",
        description(job)
    );
    for (name, value) in job.environment() {
        text.push_str(&format!(
            "Environment=\"{}\"\n",
            quoted(&format!("{name}={value}"))
        ));
    }
    let program = job
        .program
        .as_deref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let log = job.log.to_string_lossy().replace('%', "%%");
    text.push_str(&format!(
        "ExecStart=\"{}\" {}\nStandardOutput=append:{log}\nStandardError=append:{log}\n",
        quoted(&program),
        job.verb
    ));
    text
}

pub fn timer(job: &Job) -> String {
    let n = job.minutes();
    format!(
        "[Unit]\nDescription=Run bilbo {} every {n} min\n\n[Timer]\nOnActiveSec={n}min\nOnUnitActiveSec={n}min\n\n[Install]\nWantedBy=timers.target\n",
        job.verb
    )
}

pub struct Systemd {
    program: Option<PathBuf>,
    units: PathBuf,
    bounds: Bounds,
}

pub fn failure(words: &str, program: &Path, ran: &Ran, waited: &str) -> String {
    let why = match &ran.end {
        End::Exited(code) => {
            first_stderr_line(&ran.stderr).unwrap_or_else(|| format!("exit {code}"))
        }
        _ => ended_badly(ran, program, waited).unwrap_or_default(),
    };
    format!("{words} failed: {why}")
}

impl Systemd {
    pub fn new(program: Option<PathBuf>, units: PathBuf, bounds: Bounds) -> Systemd {
        Systemd {
            program,
            units,
            bounds,
        }
    }

    pub fn units(&self) -> &Path {
        &self.units
    }

    fn found(&self) -> Result<&Path, String> {
        self.program
            .as_deref()
            .ok_or_else(|| "systemctl is not on PATH".to_owned())
    }

    fn words(args: &[&str]) -> String {
        format!("systemctl {}", args.join(" "))
    }

    fn run(&self, program: &Path, args: &[&str]) -> Ran {
        program::run(program, args, self.bounds)
    }

    fn broke(&self, program: &Path, args: &[&str], ran: &Ran) -> String {
        failure(
            &Systemd::words(args),
            program,
            ran,
            &waited(self.bounds.wait),
        )
    }

    fn link(&self, job: &Job) -> PathBuf {
        self.units.join("timers.target.wants").join(timer_unit(job))
    }

    fn at_rest(&self, unit: &str) -> Result<bool, String> {
        let line = self.ask(&["--user", "is-active", unit])?;
        Ok(line == "inactive" || line == "failed")
    }

    fn ask(&self, args: &[&str]) -> Result<String, String> {
        let program = self.found()?;
        let ran = self.run(program, args);
        match ran.end {
            End::Exited(_) => {
                let line = String::from_utf8_lossy(&ran.stdout)
                    .lines()
                    .next()
                    .map(|line| line.trim_end_matches('\r').to_owned())
                    .filter(|line| !line.is_empty());
                line.ok_or_else(|| format!("{} answered nothing", Systemd::words(args)))
            }
            _ => Err(format!(
                "{}: {}",
                Systemd::words(args),
                ended_badly(&ran, program, &waited(self.bounds.wait)).unwrap_or_default()
            )),
        }
    }

    fn command(&self, args: &[&str]) -> Result<String, String> {
        let program = self.found()?;
        let ran = self.run(program, args);
        match ran.end {
            End::Exited(0) => Ok(Systemd::words(args)),
            _ => Err(self.broke(program, args, &ran)),
        }
    }

    pub fn files_of(&self, job: &Job) -> [(PathBuf, String); 2] {
        [
            (self.units.join(service_unit(job)), service(job)),
            (self.units.join(timer_unit(job)), timer(job)),
        ]
    }
}

impl Scheduler for Systemd {
    fn manager(&self) -> Manager {
        let program = match self.found() {
            Ok(program) => program,
            Err(sentence) => return Manager::Not(Status::Skipped, sentence),
        };
        let args = ["--user", "is-system-running"];
        let ran = self.run(program, &args);
        let words = Systemd::words(&args);
        match ran.end {
            End::Exited(_) => {
                let line = String::from_utf8_lossy(&ran.stdout)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches('\r')
                    .to_owned();
                match line.as_str() {
                    "running" | "degraded" | "starting" | "initializing" | "maintenance" => {
                        Manager::Ready
                    }
                    "offline" => Manager::Not(
                        Status::Skipped,
                        "no systemd user session answers".to_owned(),
                    ),
                    "" => Manager::Not(Status::Failed, format!("{words} answered nothing")),
                    other => Manager::Not(Status::Failed, format!("{words} answered '{other}'")),
                }
            }
            End::NotStarted(error) => Manager::Not(
                Status::Failed,
                format!("cannot start {}: {error}", program.display()),
            ),
            _ => Manager::Not(
                Status::Failed,
                format!(
                    "{words}: {}",
                    ended_badly(&ran, program, &waited(self.bounds.wait)).unwrap_or_default()
                ),
            ),
        }
    }

    fn label(&self, job: &Job) -> String {
        timer_unit(job)
    }

    fn files(&self, job: &Job) -> Result<Files, String> {
        disk::files(&self.files_of(job))
    }

    fn loaded(&self, job: &Job) -> Result<Loaded, String> {
        let unit = timer_unit(job);
        let active = self.ask(&["--user", "is-active", &unit])?;
        let at_rest = active == "inactive" || active == "failed";
        let enabled = if disk::is_there(&self.units.join(&unit))? {
            self.ask(&["--user", "is-enabled", &unit])? == "enabled"
        } else {
            false
        };
        Ok(Loaded {
            loaded: active == "active" && enabled,
            something: !at_rest || enabled,
            running: !at_rest,
        })
    }

    fn log_path(&self, job: &Job) -> Result<LogPath, String> {
        disk::look(&job.log)
    }

    fn log_folder(&self, job: &Job) -> Result<bool, String> {
        match job.log.parent() {
            Some(folder) => disk::folder_is_there(folder),
            None => Ok(true),
        }
    }

    fn make_log_folder(&self, job: &Job) -> Result<(), String> {
        disk::make_log_folder(&job.log)
    }

    fn left(&self, job: &Job) -> Result<Left, String> {
        let mut found = Left::default();
        let (timer, service) = (timer_unit(job), service_unit(job));
        for path in [
            self.units.join(&service),
            self.units.join(&timer),
            self.link(job),
        ] {
            if disk::is_there(&path)? {
                found.there.push(path);
            }
        }
        for unit in [&timer, &service] {
            if !self.at_rest(unit)? {
                found.active.push(unit.clone());
            }
        }
        for unit in [&timer, &service] {
            let line = self.ask(&["--user", "show", "--property=LoadState", "--value", unit])?;
            if line != "not-found" {
                found.held.push(unit.clone());
            }
        }
        Ok(found)
    }

    fn take_out(&self, job: &Job, found: &Left) -> Result<Option<String>, String> {
        let (timer, service) = (timer_unit(job), service_unit(job));
        if found.active.contains(&timer) {
            self.command(&["--user", "stop", &timer])?;
        }
        let timer_file = self.units.join(&timer);
        let service_file = self.units.join(&service);
        if found.there.contains(&timer_file) {
            self.command(&["--user", "disable", &timer])?;
        }
        if !self.at_rest(&service)? {
            self.command(&["--user", "stop", &service])?;
        }
        for file in [&service_file, &timer_file] {
            if found.there.contains(file) {
                disk::delete(file)?;
            }
        }
        let link = self.link(job);
        if disk::is_there(&link)? {
            disk::delete(&link)?;
        }
        self.command(&["--user", "daemon-reload"]).map(Some)
    }

    fn unload(&self, job: &Job, seen: Loaded) -> Result<(), String> {
        let unit = timer_unit(job);
        if seen.running {
            self.command(&["--user", "stop", &unit])?;
        }
        if disk::is_there(&self.units.join(&unit))? {
            self.command(&["--user", "disable", &unit])?;
        }
        Ok(())
    }

    fn write(&self, job: &Job) -> Result<(), String> {
        for (path, text) in self.files_of(job) {
            disk::write(&path, &text)?;
        }
        Ok(())
    }

    fn load(&self, job: &Job) -> Result<String, String> {
        let unit = timer_unit(job);
        self.command(&["--user", "daemon-reload"])?;
        self.command(&["--user", "enable", &unit])?;
        self.command(&["--user", "restart", &unit])
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::{Systemd, failure, service, timer};
    use crate::program::{Bounds, End, Ran};
    use crate::timer::samples::job;

    fn sample(name: &str) -> &'static str {
        match name {
            "bilbo-refresh.service" => {
                include_str!("../../../../docs/reference/timer/bilbo-refresh.service")
            }
            "bilbo-refresh.timer" => {
                include_str!("../../../../docs/reference/timer/bilbo-refresh.timer")
            }
            "bilbo-derive.service" => {
                include_str!("../../../../docs/reference/timer/bilbo-derive.service")
            }
            "bilbo-derive.timer" => {
                include_str!("../../../../docs/reference/timer/bilbo-derive.timer")
            }
            other => panic!("no sample {other}"),
        }
    }

    fn systemd() -> Systemd {
        Systemd::new(
            None,
            PathBuf::from("/home/me/.config/systemd/user"),
            Bounds {
                wait: Duration::from_secs(1),
                poll: Duration::from_millis(1),
            },
        )
    }

    #[test]
    fn a_systemd_job_is_the_reference_samples() {
        for verb in ["refresh", "derive"] {
            let files = systemd().files_of(&job(verb));
            let expected = [
                (
                    format!("bilbo-{verb}.service"),
                    sample(&format!("bilbo-{verb}.service")),
                ),
                (
                    format!("bilbo-{verb}.timer"),
                    sample(&format!("bilbo-{verb}.timer")),
                ),
            ];
            for ((path, text), (name, sample)) in files.iter().zip(expected) {
                assert_eq!(
                    path,
                    &PathBuf::from("/home/me/.config/systemd/user").join(name)
                );
                assert_eq!(text, sample);
            }
        }
    }

    #[test]
    fn a_unit_carries_the_variables_given_in_their_order_and_none_when_none_is() {
        let mut three = job("refresh");
        three.variables = ["HOME", "BILBO_CONFIG", "XDG_DATA_HOME"]
            .map(|name| (name.to_owned(), OsString::from("/v")))
            .to_vec();
        let names: Vec<String> = service(&three)
            .lines()
            .filter_map(|line| line.strip_prefix("Environment=\""))
            .map(|rest| rest.split('=').next().unwrap_or_default().to_owned())
            .collect();
        assert_eq!(names, ["HOME", "BILBO_CONFIG", "XDG_DATA_HOME", "PATH"]);
        let mut none = job("refresh");
        none.variables.clear();
        none.path.clear();
        assert!(!service(&none).contains("Environment="));
    }

    #[test]
    fn a_plist_writes_what_xml_reserves_and_a_unit_what_systemd_expands() {
        let mut plain = job("refresh");
        plain.program = Some(PathBuf::from("/opt/build$one/bin/bilbo"));
        assert!(service(&plain).contains("ExecStart=\"/opt/build$one/bin/bilbo\" refresh\n"));
        let mut odd = job("refresh");
        odd.program = Some(PathBuf::from("/o\"p\\q%r$s/bilbo&<>"));
        odd.log = PathBuf::from("/l%x\"y\\z$w&<>/y.log");
        odd.variables = vec![("HOME".to_owned(), OsString::from("/h\"\\%$&<>"))];
        odd.path = vec![PathBuf::from("/p\"q\\r%s$t&<>")];
        let text = service(&odd);
        let lines: Vec<&str> = text.lines().collect();
        for line in [
            "ExecStart=\"/o\\\"p\\\\q%%r$s/bilbo&<>\" refresh",
            "StandardOutput=append:/l%%x\"y\\z$w&<>/y.log",
            "StandardError=append:/l%%x\"y\\z$w&<>/y.log",
            "Environment=\"HOME=/h\\\"\\\\%%$&<>\"",
            "Environment=\"PATH=/p\\\"q\\\\r%%s$t&<>\"",
        ] {
            assert!(lines.contains(&line), "{line}\n{text}");
        }
    }

    #[test]
    fn a_schedulers_failure_is_named_by_its_own_first_line() {
        let program = Path::new("/bin/systemctl");
        let failed = |end: End, stderr: &[u8]| {
            let ran = Ran {
                end,
                stdout: Vec::new(),
                stderr: stderr.to_vec(),
            };
            failure("systemctl --user stop x", program, &ran, "30s")
        };
        let head = "systemctl --user stop x failed: ";
        assert_eq!(
            failed(End::Exited(1), b"Error: disk full\nmore\n"),
            format!("{head}Error: disk full")
        );
        assert_eq!(
            failed(End::Exited(1), b"\x1b[31mbad\x1b[0m\n"),
            format!("{head}\u{1b}[31mbad\u{1b}[0m")
        );
        assert_eq!(failed(End::Exited(2), b""), format!("{head}exit 2"));
        assert_eq!(
            failed(End::Exited(1), b"\nsecond\n"),
            format!("{head}exit 1")
        );
        assert_eq!(
            failed(End::Exited(1), b"a\xffb\n"),
            format!("{head}a\u{fffd}b")
        );
        assert_eq!(
            failed(End::NoExitCode, b"x\n"),
            format!("{head}a signal ended it")
        );
    }

    #[test]
    fn an_interval_is_written_in_seconds_for_launchd_and_minutes_for_systemd() {
        let mut two_hours = job("refresh");
        two_hours.interval = Duration::from_secs(7200);
        let text = timer(&two_hours);
        assert!(text.contains("OnActiveSec=120min\nOnUnitActiveSec=120min\n"));
        assert!(text.contains("every 120 min"));
    }
}
