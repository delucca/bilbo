use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::keys::waited;
use crate::report::{Line, Status};

#[derive(Debug, Clone)]
pub struct Job {
    pub step: &'static str,
    pub verb: &'static str,
    pub interval: Duration,
    pub program: Option<PathBuf>,
    pub log: PathBuf,
    pub variables: Vec<(String, OsString)>,
    pub path: Vec<PathBuf>,
    pub clause: Option<String>,
}

impl Job {
    pub fn environment(&self) -> Vec<(String, String)> {
        let mut found: Vec<(String, String)> = self
            .variables
            .iter()
            .map(|(name, value)| (name.clone(), value.to_string_lossy().into_owned()))
            .collect();
        let mut folders: Vec<&Path> = Vec::new();
        for folder in &self.path {
            if !folders.contains(&folder.as_path()) {
                folders.push(folder);
            }
        }
        if !folders.is_empty() {
            let joined: Vec<String> = folders
                .iter()
                .map(|folder| folder.to_string_lossy().into_owned())
                .collect();
            found.push(("PATH".to_owned(), joined.join(":")));
        }
        found
    }

    pub fn minutes(&self) -> u64 {
        self.interval.as_secs() / 60
    }

    pub fn seconds(&self) -> u64 {
        self.interval.as_secs()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Manager {
    Ready,
    Not(Status, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Files {
    Wanted,
    Absent,
    Differ,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Loaded {
    pub loaded: bool,
    pub something: bool,
    pub running: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogPath {
    Nothing,
    Regular,
    Link,
    Other,
}

pub trait Scheduler {
    fn manager(&self) -> Manager;
    fn label(&self, job: &Job) -> String;
    fn files(&self, job: &Job) -> Result<Files, String>;
    fn loaded(&self, job: &Job) -> Result<Loaded, String>;
    fn log_path(&self, job: &Job) -> Result<LogPath, String>;
    fn log_folder(&self, job: &Job) -> Result<bool, String>;
    fn make_log_folder(&self, job: &Job) -> Result<(), String>;
    fn unload(&self, job: &Job, seen: Loaded) -> Result<(), String>;
    fn write(&self, job: &Job) -> Result<(), String>;
    fn load(&self, job: &Job) -> Result<String, String>;
    fn left(&self, job: &Job) -> Result<Left, String>;
    fn take_out(&self, job: &Job, found: &Left) -> Result<Option<String>, String>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Left {
    pub there: Vec<PathBuf>,
    pub loaded: Vec<String>,
    pub active: Vec<String>,
    pub held: Vec<String>,
}

impl Left {
    pub fn is_empty(&self) -> bool {
        self.there.is_empty()
            && self.loaded.is_empty()
            && self.active.is_empty()
            && self.held.is_empty()
    }

    fn finding(&self, words: Option<&str>) -> Option<String> {
        let after = |text: String| match words {
            Some(words) => format!("{text} after {words}"),
            None => text,
        };
        if let Some(path) = self.there.first() {
            return Some(format!(
                "cannot delete {}: it is still there",
                path.display()
            ));
        }
        if let Some(label) = self.loaded.first() {
            return Some(after(format!("{label} is still loaded")));
        }
        if let Some(unit) = self.active.last() {
            return Some(after(format!("{unit} is still active")));
        }
        self.held
            .last()
            .map(|unit| after(format!("systemd still holds {unit}")))
    }
}

fn bad_char(text: &str) -> Option<char> {
    text.chars().find(|c| {
        let code = u32::from(*c);
        code < 0x20 || code == 0x7f || code == 0xfffe || code == 0xffff
    })
}

fn judged(what: &str, value: &std::ffi::OsStr) -> Result<(), String> {
    let Ok(text) = std::str::from_utf8(value.as_bytes()) else {
        return Err(format!("{what} is not UTF-8, which a unit cannot carry"));
    };
    match bad_char(text) {
        Some(c) => Err(format!(
            "{what} holds U+{:04X}, which a unit cannot carry",
            u32::from(c)
        )),
        None => Ok(()),
    }
}

fn values(job: &Job, program: &Path) -> Result<(), String> {
    judged("this bilbo's path", program.as_os_str())?;
    judged("the log's path", job.log.as_os_str())?;
    for (name, value) in &job.variables {
        judged(name, value)?;
    }
    for folder in &job.path {
        judged("a folder of PATH", folder.as_os_str())?;
        if folder.as_os_str().as_bytes().contains(&b':') {
            return Err(format!(
                "the folder {} holds a colon, which PATH cannot carry",
                folder.display()
            ));
        }
    }
    Ok(())
}

fn row(job: &Job, status: Status, detail: String) -> Line {
    Line {
        step: job.step,
        status,
        detail,
    }
}

fn failed(job: &Job, detail: String) -> Line {
    row(job, Status::Failed, detail)
}

fn detail(job: &Job, why: Option<&str>) -> String {
    let mut text = format!("every {}, log {}", waited(job.interval), job.log.display());
    if let Some(why) = why {
        text.push_str("; ");
        text.push_str(why);
    }
    if let Some(clause) = &job.clause {
        text.push_str("; ");
        text.push_str(clause);
    }
    text
}

pub fn removal(scheduler: &dyn Scheduler, job: &Job) -> Line {
    if let Manager::Not(status, sentence) = scheduler.manager() {
        return row(job, status, sentence);
    }
    let name = scheduler.label(job);
    let found = match scheduler.left(job) {
        Err(sentence) => return failed(job, sentence),
        Ok(found) if found.is_empty() => {
            return row(job, Status::Absent, format!("no timer {name}"));
        }
        Ok(found) => found,
    };
    let words = match scheduler.take_out(job, &found) {
        Ok(words) => words,
        Err(sentence) => return failed(job, sentence),
    };
    match scheduler.left(job) {
        Err(sentence) => failed(job, sentence),
        Ok(left) => match left.finding(words.as_deref()) {
            Some(sentence) => failed(job, sentence),
            None => row(job, Status::Removed, name),
        },
    }
}

pub fn step(scheduler: &dyn Scheduler, job: &Job) -> Line {
    if let Manager::Not(status, sentence) = scheduler.manager() {
        return row(job, status, sentence);
    }
    let Some(program) = &job.program else {
        return failed(
            job,
            "cannot find this bilbo's own path, which a timer runs".to_owned(),
        );
    };
    if let Err(sentence) = values(job, program) {
        return failed(job, sentence);
    }
    let log = match scheduler.log_path(job) {
        Ok(log) => log,
        Err(sentence) => return failed(job, sentence),
    };
    match log {
        LogPath::Nothing | LogPath::Regular => {}
        LogPath::Link => {
            return failed(
                job,
                format!("the log {} is a symbolic link", job.log.display()),
            );
        }
        LogPath::Other => {
            return failed(
                job,
                format!("the log {} is not a regular file", job.log.display()),
            );
        }
    }
    match decided(scheduler, job) {
        Ok((status, why)) => row(job, status, detail(job, why)),
        Err(sentence) => failed(job, sentence),
    }
}

fn decided(scheduler: &dyn Scheduler, job: &Job) -> Result<(Status, Option<&'static str>), String> {
    let files = scheduler.files(job)?;
    let seen = scheduler.loaded(job)?;
    let folder = scheduler.log_folder(job)?;
    let (status, why) = match (files, seen.loaded) {
        (Files::Wanted, true) if folder => return Ok((Status::Kept, None)),
        (Files::Wanted, true) => {
            scheduler.make_log_folder(job)?;
            return Ok((Status::Updated, Some("its log's folder was gone")));
        }
        (Files::Wanted, false) => {
            if !folder {
                scheduler.make_log_folder(job)?;
            }
            (Status::Updated, "it was not loaded")
        }
        (Files::Absent, _) if !seen.something => {
            scheduler.make_log_folder(job)?;
            scheduler.write(job)?;
            (Status::Installed, "")
        }
        _ => {
            if seen.something {
                scheduler.unload(job, seen)?;
            }
            scheduler.make_log_folder(job)?;
            scheduler.write(job)?;
            (Status::Updated, "its unit differed")
        }
    };
    let words = scheduler.load(job)?;
    if !scheduler.loaded(job)?.loaded {
        return Err(format!(
            "{} is not loaded after {words}",
            scheduler.label(job)
        ));
    }
    Ok((status, (!why.is_empty()).then_some(why)))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{Job, values};

    fn job() -> Job {
        Job {
            step: "refresh-timer",
            verb: "refresh",
            interval: Duration::from_secs(600),
            program: Some(PathBuf::from("/opt/bilbo/bin/bilbo")),
            log: PathBuf::from("/l/refresh.log"),
            variables: vec![("HOME".to_owned(), OsString::from("/home/me"))],
            path: vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/a"),
            ],
            clause: None,
        }
    }

    #[test]
    fn the_unit_environment_ends_with_a_path_of_each_folder_once() {
        assert_eq!(
            job().environment(),
            [
                ("HOME".to_owned(), "/home/me".to_owned()),
                ("PATH".to_owned(), "/a:/b".to_owned())
            ]
        );
        let mut bare = job();
        bare.variables.clear();
        bare.path.clear();
        assert!(bare.environment().is_empty());
        assert_eq!((job().minutes(), job().seconds()), (10, 600));
        let mut spelled = job();
        spelled.path = ["/opt/tools", "/usr/bin", "/opt/tools/"]
            .map(PathBuf::from)
            .to_vec();
        assert_eq!(
            spelled.environment().last(),
            Some(&("PATH".to_owned(), "/opt/tools:/usr/bin".to_owned()))
        );
    }

    #[test]
    fn a_value_names_the_first_character_a_unit_cannot_carry() {
        let program = PathBuf::from("/p");
        let mut odd = job();
        odd.log = PathBuf::from("/l\u{1}/x\n");
        assert_eq!(
            values(&odd, &program),
            Err("the log's path holds U+0001, which a unit cannot carry".to_owned())
        );
        let mut raw = job();
        raw.variables = vec![("XDG_DATA_HOME".to_owned(), OsString::from_vec(vec![0xff]))];
        assert_eq!(
            values(&raw, &program),
            Err("XDG_DATA_HOME is not UTF-8, which a unit cannot carry".to_owned())
        );
        let mut colon = job();
        colon.path = vec![PathBuf::from("/a:b")];
        assert_eq!(
            values(&colon, &program),
            Err("the folder /a:b holds a colon, which PATH cannot carry".to_owned())
        );
        let mut fine = job();
        fine.log = PathBuf::from("/l\u{fffd}\u{80}");
        assert_eq!(values(&fine, &program), Ok(()));
    }
}

#[cfg(test)]
pub mod samples {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::Duration;

    use super::Job;

    pub fn job(verb: &'static str) -> Job {
        let (step, seconds) = if verb == "refresh" {
            ("refresh-timer", 600)
        } else {
            ("derive-timer", 1800)
        };
        Job {
            step,
            verb,
            interval: Duration::from_secs(seconds),
            program: Some(PathBuf::from("/opt/bilbo/bin/bilbo")),
            log: PathBuf::from(format!("/home/me/.local/state/bilbo/timer/{verb}.log")),
            variables: vec![("HOME".to_owned(), OsString::from("/home/me"))],
            path: [
                "/home/me/.local/bin",
                "/usr/bin",
                "/bin",
                "/usr/sbin",
                "/sbin",
            ]
            .map(PathBuf::from)
            .to_vec(),
            clause: None,
        }
    }
}
