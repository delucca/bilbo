use std::path::{Path, PathBuf};

use crate::disk;
use crate::keys::waited;
use crate::plugin::ended_badly;
use crate::program::{self, Bounds, End, Ran, first_stderr_line};
use crate::report::Status;
use crate::timer::{Files, Job, Left, Loaded, LogPath, Manager, Scheduler};

pub fn label(job: &Job) -> String {
    format!("io.github.delucca.bilbo.{}", job.verb)
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn render(job: &Job) -> String {
    let mut text = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n",
    );
    let environment = job.environment();
    if !environment.is_empty() {
        text.push_str("\t<key>EnvironmentVariables</key>\n\t<dict>\n");
        for (name, value) in &environment {
            text.push_str(&format!(
                "\t\t<key>{}</key>\n\t\t<string>{}</string>\n",
                xml(name),
                xml(value)
            ));
        }
        text.push_str("\t</dict>\n");
    }
    let program = job
        .program
        .as_deref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let log = xml(&job.log.to_string_lossy());
    text.push_str(&format!(
        "\t<key>Label</key>\n\t<string>{}</string>\n\t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{}</string>\n\t\t<string>{}</string>\n\t</array>\n\t<key>RunAtLoad</key>\n\t<false/>\n\t<key>StandardErrorPath</key>\n\t<string>{log}</string>\n\t<key>StandardOutPath</key>\n\t<string>{log}</string>\n\t<key>StartInterval</key>\n\t<integer>{}</integer>\n</dict>\n</plist>\n",
        xml(&label(job)),
        xml(&program),
        xml(job.verb),
        job.seconds()
    ));
    text
}

pub struct Launchd {
    program: Option<PathBuf>,
    units: PathBuf,
    uid: u32,
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

impl Launchd {
    pub fn new(program: Option<PathBuf>, units: PathBuf, uid: u32, bounds: Bounds) -> Launchd {
        Launchd {
            program,
            units,
            uid,
            bounds,
        }
    }

    pub fn path(&self, job: &Job) -> PathBuf {
        self.units.join(format!("{}.plist", label(job)))
    }

    fn domain(&self) -> String {
        format!("gui/{}", self.uid)
    }

    fn words(args: &[&str]) -> String {
        format!("launchctl {}", args.join(" "))
    }

    fn call(&self, program: &Path, args: &[&str]) -> Ran {
        program::run(program, args, self.bounds)
    }

    fn broke(&self, program: &Path, args: &[&str], ran: &Ran) -> String {
        failure(
            &Launchd::words(args),
            program,
            ran,
            &waited(self.bounds.wait),
        )
    }

    fn found(&self) -> Result<&Path, String> {
        self.program
            .as_deref()
            .ok_or_else(|| "launchctl is not on PATH".to_owned())
    }
}

impl Scheduler for Launchd {
    fn manager(&self) -> Manager {
        match self.found() {
            Ok(_) => Manager::Ready,
            Err(sentence) => Manager::Not(Status::Failed, sentence),
        }
    }

    fn label(&self, job: &Job) -> String {
        label(job)
    }

    fn files(&self, job: &Job) -> Result<Files, String> {
        disk::files(&[(self.path(job), render(job))])
    }

    fn loaded(&self, job: &Job) -> Result<Loaded, String> {
        let program = self.found()?;
        let target = format!("{}/{}", self.domain(), label(job));
        let args = ["print", target.as_str()];
        let ran = self.call(program, &args);
        let loaded = match ran.end {
            End::Exited(0) => true,
            End::Exited(113) => false,
            _ => return Err(self.broke(program, &args, &ran)),
        };
        Ok(Loaded {
            loaded,
            something: loaded,
            running: loaded,
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
        if Scheduler::loaded(self, job)?.loaded {
            found.loaded.push(label(job));
        }
        let path = self.path(job);
        if disk::is_there(&path)? {
            found.there.push(path);
        }
        Ok(found)
    }

    fn take_out(&self, job: &Job, found: &Left) -> Result<Option<String>, String> {
        let mut words = None;
        if !found.loaded.is_empty() {
            let program = self.found()?;
            let target = format!("{}/{}", self.domain(), label(job));
            let args = ["bootout", "--wait", target.as_str()];
            let ran = self.call(program, &args);
            match ran.end {
                End::Exited(0 | 3) => words = Some(Launchd::words(&args)),
                _ => return Err(self.broke(program, &args, &ran)),
            }
        }
        if !found.there.is_empty() {
            disk::delete(&self.path(job))?;
        }
        Ok(words)
    }

    fn unload(&self, job: &Job, _seen: Loaded) -> Result<(), String> {
        let program = self.found()?;
        let target = format!("{}/{}", self.domain(), label(job));
        let args = ["bootout", "--wait", target.as_str()];
        let ran = self.call(program, &args);
        match ran.end {
            End::Exited(0 | 3) => Ok(()),
            _ => Err(self.broke(program, &args, &ran)),
        }
    }

    fn write(&self, job: &Job) -> Result<(), String> {
        disk::write(&self.path(job), &render(job))
    }

    fn load(&self, job: &Job) -> Result<String, String> {
        let program = self.found()?;
        let path = self.path(job);
        let domain = self.domain();
        let args = ["bootstrap", domain.as_str(), &path.to_string_lossy()];
        let ran = self.call(program, &args);
        match ran.end {
            End::Exited(0) => Ok(Launchd::words(&args)),
            _ => Err(self.broke(program, &args, &ran)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::{Launchd, failure, label, render};
    use crate::program::{Bounds, End, Ran};
    use crate::timer::samples::job;

    const REFRESH: &str =
        include_str!("../../../../docs/reference/timer/io.github.delucca.bilbo.refresh.plist");
    const DERIVE: &str =
        include_str!("../../../../docs/reference/timer/io.github.delucca.bilbo.derive.plist");

    #[test]
    fn a_launchd_job_is_the_reference_sample() {
        assert_eq!(render(&job("refresh")), REFRESH);
        assert_eq!(render(&job("derive")), DERIVE);
        let launchd = Launchd::new(
            None,
            PathBuf::from("/home/me/Library/LaunchAgents"),
            501,
            Bounds {
                wait: Duration::from_secs(1),
                poll: Duration::from_millis(1),
            },
        );
        assert_eq!(
            launchd.path(&job("derive")),
            PathBuf::from("/home/me/Library/LaunchAgents/io.github.delucca.bilbo.derive.plist")
        );
        assert_eq!(label(&job("refresh")), "io.github.delucca.bilbo.refresh");
    }

    #[test]
    fn a_unit_carries_the_variables_given_in_their_order_and_none_when_none_is() {
        let mut three = job("refresh");
        three.variables = ["HOME", "BILBO_CONFIG", "XDG_DATA_HOME"]
            .map(|name| (name.to_owned(), OsString::from(format!("/v/{name}"))))
            .to_vec();
        let text = render(&three);
        let names: Vec<&str> = text
            .lines()
            .filter_map(|line| line.trim().strip_prefix("<key>")?.strip_suffix("</key>"))
            .take(5)
            .collect();
        assert_eq!(
            names,
            [
                "EnvironmentVariables",
                "HOME",
                "BILBO_CONFIG",
                "XDG_DATA_HOME",
                "PATH"
            ]
        );
        let mut none = job("refresh");
        none.variables.clear();
        none.path.clear();
        assert!(!render(&none).contains("EnvironmentVariables"));
    }

    #[test]
    fn a_plist_writes_what_xml_reserves_and_a_unit_what_systemd_expands() {
        let mut odd = job("refresh");
        odd.program = Some(PathBuf::from("/o&p/<b>/bilbo%$\"\\"));
        odd.log = PathBuf::from("/l&<>/%$\"\\x.log");
        odd.variables = vec![("HOME".to_owned(), OsString::from("/h&<>%$\"\\"))];
        odd.path = vec![PathBuf::from("/p&<>%$\"\\")];
        let text = render(&odd);
        let lines: Vec<&str> = text.lines().collect();
        for line in [
            "\t\t<string>/o&amp;p/&lt;b&gt;/bilbo%$\"\\</string>",
            "\t\t<string>/h&amp;&lt;&gt;%$\"\\</string>",
            "\t\t<string>/p&amp;&lt;&gt;%$\"\\</string>",
        ] {
            assert!(lines.contains(&line), "{line}\n{text}");
        }
        let log = "\t<string>/l&amp;&lt;&gt;/%$\"\\x.log</string>";
        assert_eq!(
            lines.iter().filter(|line| **line == log).count(),
            2,
            "{text}"
        );
    }

    #[test]
    fn a_schedulers_failure_is_named_by_its_own_first_line() {
        let program = Path::new("/bin/launchctl");
        let failed = |end: End, stderr: &[u8]| {
            let ran = Ran {
                end,
                stdout: Vec::new(),
                stderr: stderr.to_vec(),
            };
            failure("launchctl bootout x", program, &ran, "30s")
        };
        let head = "launchctl bootout x failed: ";
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
        assert!(render(&two_hours).contains("<integer>7200</integer>"));
    }
}
