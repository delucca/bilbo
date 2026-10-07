//! `bilbo sync`: each syncing scope's state, the open conflicts and dropped text, and `declare`. Status reads only
//! local files and writes nothing; `declare` appends one line per conflict to the note's log under the history lock.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::Path;

use jiff::tz::TimeZone;
use serde::Deserialize;

use crate::Failure;
use crate::identity::keys;
use crate::identity::manifest::{self, Known, Recipient};
use crate::note::conflicts;
use crate::note::versions::{self, Declaration, NameError};
use crate::search::documents::{self, Stored};
use crate::shared::config;
use crate::shared::store;
use crate::sync::manifests::{self, Kind};
use crate::sync::replica::{self, Status};
use crate::sync::segment::Record;

/// The `created` form, to the minute.
const CREATED: &str = "%Y-%m-%dT%H:%M%:z";
/// The longest reason `declare` takes, in characters.
const REASON_MAX: usize = 500;
/// Flags older than this are not notices; the summary drops them when it is written, and a stale one is left out here.
const NOTICE_DAYS: i64 = 7;
const DAY: i64 = 24 * 60 * 60;

pub struct Output {
    /// stderr lines, without "bilbo: ".
    pub warnings: Vec<String>,
    pub lines: Vec<String>,
    pub failed: bool,
}

pub fn run(args: &[String], env: &store::Env) -> Result<Output, Failure> {
    match args.first().map(String::as_str) {
        None => status(env, jiff::Timestamp::now(), &TimeZone::system()),
        Some("declare") => declare(&args[1..], env),
        Some(arg) if arg.starts_with('-') => Err(Failure::Usage(format!("unknown option '{arg}'"))),
        Some(arg) => Err(Failure::Usage(format!("unexpected argument '{arg}'"))),
    }
}

/// The status report. It reads the config, the keys, the manifests, each scope's `state.json` and `changes.jsonl`,
/// the inbox and `open.json`, and the notes that `open.json` names.
fn status(env: &store::Env, now: jiff::Timestamp, tz: &TimeZone) -> Result<Output, Failure> {
    let root = store::root(env).map_err(Failure::Config)?;
    let notes = root.join("notes");
    if !notes.is_dir() {
        return Err(Failure::Refused(format!("no store at {}", root.display())));
    }
    let settings = config::load(env).map_err(Failure::Config)?;
    if !settings.scopes.iter().any(|s| s.sync != "off") {
        let path = settings
            .path
            .as_ref()
            .map_or("the config".to_string(), |p| p.display().to_string());
        return Err(Failure::Refused(format!(
            "no scope syncs; set scope.<name>.sync in {path}"
        )));
    }
    let keys = store::keys_dir(env)
        .ok_or_else(|| Failure::Config("no state folder: set XDG_STATE_HOME or HOME".into()))?;
    let identity = keys::read_identity(&keys).map_err(Failure::Refused)?;
    let stored = documents::read_notes(&notes)
        .map_err(|e| Failure::Refused(format!("cannot read {}: {e}", notes.display())))?;
    let owner = identity.as_ref().map(|i| i.owner.sign.public());
    let who = identity.as_ref().map(|i| Recipient::device(&i.device));
    let known = manifest::survey(&root, owner.as_ref(), who.as_ref()).map_err(Failure::Refused)?;
    let cx = Cx {
        root: &root,
        settings: &settings,
        me: identity.as_ref().map(|i| i.device.id()),
        now,
        tz,
    };
    Ok(report(
        &cx,
        &stored,
        &known,
        versions::watcher_running(&root),
    ))
}

struct Cx<'a> {
    root: &'a Path,
    settings: &'a config::Settings,
    /// This device's id; `None` without keys.
    me: Option<String>,
    now: jiff::Timestamp,
    tz: &'a TimeZone,
}

/// One syncing scope of the config with what this device holds of it.
struct Section<'a> {
    name: &'a str,
    url: &'a str,
    known: Option<&'a Known>,
    status: Option<Status>,
}

fn report(cx: &Cx, stored: &[Stored], known: &[Known], watching: bool) -> Output {
    let mut out = Output {
        warnings: Vec::new(),
        lines: Vec::new(),
        failed: false,
    };
    let syncing: BTreeSet<&str> = cx
        .settings
        .scopes
        .iter()
        .filter(|s| s.sync != "off")
        .map(|s| s.name.as_str())
        .collect();
    let mut sections = Vec::new();
    for scope in cx.settings.scopes.iter().filter(|s| s.sync != "off") {
        let known = pick(cx.root, known, &scope.name);
        let status = match known.map(|k| replica::status(cx.root, &k.scope.id)) {
            Some(Ok(status)) => status,
            Some(Err(e)) => {
                out.warnings.push(format!("sync {}: {e}", scope.name));
                out.failed = true;
                None
            }
            None => None,
        };
        sections.push(Section {
            name: &scope.name,
            url: &scope.sync,
            known,
            status,
        });
    }
    let waiting = match waiting(cx.root, &sections) {
        Ok(waiting) => waiting,
        Err(e) => {
            out.warnings.push(format!("cannot read the inbox: {e}"));
            out.failed = true;
            BTreeMap::new()
        }
    };
    for section in &sections {
        let count = stored
            .iter()
            .filter(|n| n.scope.as_deref() == Some(section.name))
            .count();
        let (pushed, pulled) = match &section.status {
            Some(s) => (s.pushed_at, s.pulled_at),
            None => (None, None),
        };
        out.lines.push(format!(
            "scope {} {}: {count} notes, pushed {}, pulled {}",
            section.name,
            section.url,
            at_second(pushed, cx.tz),
            at_second(pulled, cx.tz),
        ));
        if let Some(latest) = section.known.and_then(|k| k.scope.latest()) {
            for device in &latest.manifest.devices {
                out.lines.push(format!(
                    "device {} {}: {}",
                    section.name,
                    device.name,
                    device_state(cx, section.status.as_ref(), &device.id)
                ));
            }
        }
        for ((scope, device), n) in &waiting {
            if scope == section.name {
                let name = section
                    .known
                    .and_then(|k| k.scope.latest())
                    .and_then(|v| v.manifest.devices.iter().find(|d| d.id == *device))
                    .map_or(device.as_str(), |d| d.name.as_str());
                out.lines
                    .push(format!("waiting {} {name}: {n} versions", section.name));
            }
        }
    }
    let local = stored
        .iter()
        .filter(|n| n.scope.as_deref().is_none_or(|s| !syncing.contains(s)))
        .count();
    out.lines.push(format!("local: {local} notes sync nowhere"));
    notes_section(cx, &mut out);
    for section in &sections {
        let Some(k) = section.known else { continue };
        match manifests::recent(cx.root, &k.scope.id, cx.now) {
            Ok(changes) => {
                for change in changes {
                    let what = match (change.kind, &change.device) {
                        (Kind::Device, device) => format!(
                            "device {} added by {}",
                            device.as_deref().unwrap_or("unknown"),
                            change.signer
                        ),
                        (Kind::Epoch, _) => "epoch changed".to_string(),
                    };
                    out.lines.push(format!(
                        "change {} {}: {what} (manifest {})",
                        created(&change.at, cx.tz),
                        section.name,
                        change.n
                    ));
                }
            }
            Err(e) => {
                out.warnings.push(format!("sync {}: {e}", section.name));
                out.failed = true;
            }
        }
    }
    problems(cx, &sections, &mut out);
    if !watching {
        out.warnings
            .push("bilbo watch is not running; nothing syncs".into());
        out.failed = true;
    }
    out
}

/// The scope of the config named `name`: this device's manifest that holds or once held the name, the one with a
/// state first, then the one this device opens, then the newest.
fn pick<'a>(root: &Path, known: &'a [Known], name: &str) -> Option<&'a Known> {
    known
        .iter()
        .filter(|k| k.mine && (k.name() == Some(name) || k.last_name.as_deref() == Some(name)))
        .max_by_key(|k| {
            let state = store::scopes_dir(root)
                .join(&k.scope.id)
                .join("state.json")
                .is_file();
            let n = k.scope.latest().map_or(0, |v| v.manifest.n);
            (state, k.opened.is_some(), n)
        })
}

/// `this device`, `up to date`, `behind by <n> segments` or `stale since <time>`: what the acknowledgements say of the
/// segments with versions that this device wrote.
fn device_state(cx: &Cx, status: Option<&Status>, id: &str) -> String {
    if cx.me.as_deref() == Some(id) {
        return "this device".into();
    }
    let Some(status) = status else {
        return "up to date".into();
    };
    let acked = status.acked.get(id).copied().unwrap_or(0);
    let since = status.since.get(id).copied().unwrap_or(i64::MIN);
    let owed: Vec<i64> = status
        .sent
        .iter()
        .filter(|(seq, sent)| sent.versions && **seq > acked)
        .map(|(_, sent)| sent.at.max(since))
        .collect();
    let cutoff = cx.now.as_second() - i64::from(cx.settings.sync.stale_days) * DAY;
    match owed.iter().copied().min() {
        None => "up to date".into(),
        Some(oldest) if oldest < cutoff => {
            format!("stale since {}", at_second(Some(oldest), cx.tz))
        }
        Some(_) => format!("behind by {} segments", owed.len()),
    }
}

/// How many versions wait for a version they follow, per scope and writing device: the staged records whose parents
/// are not in the note's log, outside, or themselves staged and applicable.
fn waiting(root: &Path, sections: &[Section]) -> Result<BTreeMap<(String, String), usize>, String> {
    #[derive(Deserialize)]
    struct Staged {
        scope: String,
        #[serde(default)]
        record: Option<Record>,
    }
    let path = store::sync_dir(root).join("inbox.jsonl");
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let names: HashSet<&str> = sections.iter().map(|s| s.name).collect();
    let mut by_note: BTreeMap<String, Vec<(String, Record)>> = BTreeMap::new();
    for line in bytes.split(|b| *b == b'\n') {
        let Ok(Staged {
            scope,
            record: Some(record),
        }) = serde_json::from_slice(line)
        else {
            continue;
        };
        if names.contains(scope.as_str()) {
            by_note
                .entry(record.note.clone())
                .or_default()
                .push((scope, record));
        }
    }
    let mut counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for (note, staged) in by_note {
        let log = versions::load(root, &note)?;
        let mut known: HashSet<&str> = log.versions.iter().map(|v| v.version.as_str()).collect();
        let held = known.clone();
        loop {
            let ready: Vec<&str> = staged
                .iter()
                .map(|(_, r)| &r.version)
                .filter(|v| !known.contains(v.version.as_str()))
                .filter(|v| {
                    v.parents
                        .iter()
                        .all(|p| known.contains(p.as_str()) || v.outside.contains(p))
                })
                .map(|v| v.version.as_str())
                .collect();
            if ready.is_empty() {
                break;
            }
            known.extend(ready);
        }
        for (scope, record) in &staged {
            let v = &record.version;
            if !known.contains(v.version.as_str()) && !held.contains(v.version.as_str()) {
                let device = v.device.clone().unwrap_or_default();
                *counts.entry((scope.clone(), device)).or_default() += 1;
            }
        }
    }
    Ok(counts)
}

/// The conflict, dropped and notice lines of the notes the summary lists, judged on each file as it is now.
fn notes_section(cx: &Cx, out: &mut Output) {
    let summary = match conflicts::read(cx.root) {
        Ok(summary) => summary,
        Err(e) => {
            out.warnings.push(format!(
                "cannot read {}: {e}",
                conflicts::path(cx.root).display()
            ));
            out.failed = true;
            return;
        }
    };
    let scan = if summary.notes.values().any(|e| e.waits()) {
        match versions::scan(&cx.root.join("notes")) {
            Ok(scan) => Some(scan),
            Err(e) => {
                out.warnings.push(format!("cannot read the notes: {e}"));
                out.failed = true;
                None
            }
        }
    } else {
        None
    };
    let mut files: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut notices: Vec<(jiff::Timestamp, String, String, String)> = Vec::new();
    let window = jiff::SignedDuration::from_hours(24 * NOTICE_DAYS);
    for (id, entry) in &summary.notes {
        let found = scan.as_ref().and_then(|s| s.notes.get(id));
        let file = found.map_or(entry.file.clone(), |f| f.name.clone());
        if let (true, Some(found)) = (entry.waits(), found) {
            match versions::load(cx.root, id) {
                Ok(log) => {
                    let text = String::from_utf8_lossy(&found.bytes);
                    let judged = conflicts::judge(cx.root, Some(entry), &log, &text);
                    let mut lines = Vec::new();
                    if !judged.conflicts.is_empty() {
                        lines.push(format!(
                            "conflict notes/{file}: {}",
                            count(judged.conflicts.len(), "passage")
                        ));
                    }
                    let lost = lost_lines(&judged.dropped);
                    if lost > 0 {
                        lines.push(format!(
                            "dropped notes/{file}: {} not declared",
                            count(lost, "line")
                        ));
                    }
                    if !lines.is_empty() {
                        out.failed = true;
                        files.entry(file.clone()).or_default().extend(lines);
                    }
                }
                Err(e) => {
                    out.warnings
                        .push(format!("notes/{file}: history: read: {e}"));
                    out.failed = true;
                }
            }
        }
        for notice in &entry.notices {
            let Ok(at) = notice.at.parse::<jiff::Timestamp>() else {
                continue;
            };
            if cx.now.duration_since(at) < window {
                notices.push((at, notice.at.clone(), file.clone(), notice.flag.clone()));
            }
        }
    }
    out.lines.extend(files.into_values().flatten());
    notices.sort();
    for (_, at, file, flag) in notices {
        out.lines.push(format!(
            "notice {} notes/{file}: {flag}",
            created(&at, cx.tz)
        ));
    }
}

/// The dropped lines `bilbo check` would count: distinct lines per passage.
fn lost_lines(dropped: &[conflicts::Lost]) -> usize {
    let mut passages: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for lost in dropped {
        passages
            .entry(&lost.passage)
            .or_default()
            .extend(lost.lines.iter().map(String::as_str));
    }
    passages.values().map(BTreeSet::len).sum()
}

fn count(n: usize, one: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {one}s")
    }
}

/// The stderr lines of each scope: why watch stopped it, which device it could not read past, and a transport that
/// failed or was full at the last attempt. Each makes the exit code 1.
fn problems(cx: &Cx, sections: &[Section], out: &mut Output) {
    for section in sections {
        let Some(status) = &section.status else {
            continue;
        };
        if let Some(line) = &status.stopped {
            out.warnings.push(line.clone());
            out.failed = true;
        }
        let devices = section.known.and_then(|k| k.scope.latest());
        for (id, stop) in &status.stops {
            let name = devices
                .and_then(|v| v.manifest.devices.iter().find(|d| d.id == *id))
                .map_or(id.as_str(), |d| d.name.as_str());
            let mut line = format!(
                "sync {}: {name} stopped at segment {}: {}",
                section.name, stop.seq, stop.why
            );
            let refused = stop.why != "missing" && !stop.why.starts_with("cannot read it");
            if let (true, false, Some(k)) = (refused, stop.replaceable, section.known) {
                line.push_str(&format!(
                    "; on {name}, copy <root>/.bilbo/scopes/{}/out/{:020}.seg over it",
                    k.scope.id, stop.seq
                ));
            }
            out.warnings.push(line);
            out.failed = true;
        }
        if let Some(error) = &status.error {
            let since = at_second(Some(error.since), cx.tz);
            out.warnings.push(match error.kind.as_str() {
                "full" => format!(
                    "sync {}: full since {since}: {}",
                    section.name, error.message
                ),
                _ => format!(
                    "sync {}: {} not reachable since {since}: {}",
                    section.name, section.url, error.message
                ),
            });
            out.failed = true;
        }
    }
}

/// Unix seconds in the `created` form, or `never`.
fn at_second(secs: Option<i64>, tz: &TimeZone) -> String {
    secs.and_then(|s| jiff::Timestamp::from_second(s).ok())
        .map_or("never".to_string(), |t| {
            t.to_zoned(tz.clone()).strftime(CREATED).to_string()
        })
}

/// An RFC 3339 time in the `created` form: one recorded with an offset keeps it, as `bilbo history` shows it.
fn created(at: &str, tz: &TimeZone) -> String {
    match (at.get(..16), at.get(19..)) {
        (Some(head), Some(offset)) if at.len() == 25 && at.parse::<jiff::Timestamp>().is_ok() => {
            format!("{head}{offset}")
        }
        _ => match at.parse::<jiff::Timestamp>() {
            Ok(t) => t.to_zoned(tz.clone()).strftime(CREATED).to_string(),
            Err(_) => at.to_string(),
        },
    }
}

/// `declare <note> <reason>`: records that the dropped text of the note's latest resolved conflict was dropped on
/// purpose, one declaration per conflict version, under the history lock.
fn declare(args: &[String], env: &store::Env) -> Result<Output, Failure> {
    let (note, reason) = match args {
        [] | [_] => {
            return Err(Failure::Usage("declare needs a note and a reason".into()));
        }
        [note, reason] => (note, reason),
        [_, _, extra, ..] => {
            return Err(Failure::Usage(format!("unexpected argument '{extra}'")));
        }
    };
    if note.starts_with('-') {
        return Err(Failure::Usage(format!("unknown option '{note}'")));
    }
    if reason.contains(['\n', '\r']) {
        return Err(Failure::Usage("the reason must be one line".into()));
    }
    let length = reason.chars().count();
    if length == 0 || length > REASON_MAX {
        return Err(Failure::Usage(format!(
            "the reason must be 1 to {REASON_MAX} characters, not {length}"
        )));
    }
    let root = store::root(env).map_err(Failure::Config)?;
    let notes = root.join("notes");
    if !notes.is_dir() {
        return Err(Failure::Refused(format!("no store at {}", root.display())));
    }
    let keys = store::keys_dir(env)
        .ok_or_else(|| Failure::Config("no state folder: set XDG_STATE_HOME or HOME".into()))?;
    keys::read_identity(&keys).map_err(Failure::Refused)?;
    let scan = versions::scan(&notes)
        .map_err(|e| Failure::Refused(format!("cannot read {}: {e}", notes.display())))?;
    let named = versions::resolve(&root, note, &scan).map_err(|e| match e {
        NameError::Usage(message) => Failure::Usage(message),
        NameError::NoHistory => Failure::Refused(format!("no history for {note}")),
        NameError::Failed(message) => Failure::Refused(message),
    })?;
    let nothing = || Failure::Refused(format!("{note} has no dropped text to declare"));
    let file = named.file.clone().ok_or_else(nothing)?;
    let lock = versions::lock(&root).map_err(Failure::Refused)?;
    let bytes = fs::read(notes.join(&file))
        .map_err(|e| Failure::Refused(format!("cannot read notes/{file}: {e}")))?;
    let summary = conflicts::read(&root)
        .map_err(|e| Failure::Refused(format!("cannot read the open-conflict summary: {e}")))?;
    let log = versions::load(&root, &named.id).map_err(Failure::Refused)?;
    let judged = conflicts::judge(
        &root,
        summary.notes.get(&named.id),
        &log,
        &String::from_utf8_lossy(&bytes),
    );
    if judged.dropped.is_empty() {
        return Err(nothing());
    }
    let conflicts: BTreeSet<&str> = judged.dropped.iter().map(|l| l.conflict.as_str()).collect();
    for conflict in conflicts {
        let declaration = Declaration {
            declare: conflict.to_string(),
            reason: reason.clone(),
            at: versions::now_at(),
            device: None,
        };
        versions::append_declaration(&lock, &named.id, &declaration).map_err(Failure::Refused)?;
    }
    Ok(Output {
        warnings: Vec::new(),
        lines: vec![format!(
            "declared {file}: {} lines dropped on purpose",
            lost_lines(&judged.dropped)
        )],
        failed: false,
    })
}

/// `bilbo sync --help`; its Usage block is also the synopsis a usage error shows.
pub const HELP: &str = r#"bilbo sync: show each syncing scope's state, or declare dropped text.

Usage:
  bilbo sync
  bilbo sync declare <note> <reason>

sync reads only local files and changes nothing. declare records that the
text a conflict's resolution dropped from <note> was dropped on purpose.
<note> is named as in 'bilbo history'; <reason> is one line of 1 to 500
characters.

Output of sync, in this order, each line only when it applies:
  scope <name> <url>: <n> notes, pushed <time>, pulled <time>
  device <scope> <device>: <state>, for each device of the scope
  waiting <scope> <device>: <n> versions whose parents have not arrived
  local: <n> notes sync nowhere
  conflict notes/<file>: <n> passages
  dropped notes/<file>: <n> lines not declared
  notice <time> notes/<file>: <flag>, for each flag of the last 7 days
  change <time> <scope>: <what changed> (manifest <n>), for the last 30 days
<time> is in the created form, or never.
Output of declare: declared <file name>: <n> lines dropped on purpose

Exit: 0 nothing needs you; 1 something needs you (a conflict, undeclared
dropped text, a failing transport or device, or no running 'bilbo watch'),
no store, no scope syncs, or nothing to declare; 2 usage or config error.

Examples:
  bilbo sync
  bilbo sync declare release-tags "superseded by the v2 tag scheme"

Docs: https://github.com/delucca/bilbo/wiki/Commands#sync
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::keys::{Device, Identity, Owner};
    use crate::identity::manifest::Member;
    use crate::note::conflicts::{Entry, Lost, Notice, Open, Summary};
    use crate::note::versions::Version;
    use serde_json::json;
    use std::path::PathBuf;

    const NOTE: &str = "01M3YJ7R6HK6NQ30DCDB1P4DYB";
    const OTHER: &str = "01M3YJ7R6HK6NQ30DCDB1P4DYC";
    const T0: i64 = 1_800_000_000;
    const T0_TEXT: &str = "2027-01-15T08:00+00:00";
    const DAY: i64 = 86_400;
    const NO_WATCHER: &str = "bilbo watch is not running; nothing syncs";

    struct World {
        dir: PathBuf,
        file: Option<std::fs::File>,
    }

    impl Drop for World {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn world(name: &str) -> World {
        let dir =
            std::env::temp_dir().join(format!("bilbo-sync-cli-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("home/notes")).unwrap();
        fs::create_dir_all(dir.join("state/bilbo")).unwrap();
        World { dir, file: None }
    }

    fn identity(name: &str, seed: u8) -> Identity {
        Identity {
            owner: Owner::derive(&[0; 16]).file(),
            device: Device::from_seeds(name, &[seed; 32], &[seed + 1; 32]),
        }
    }

    fn me() -> Identity {
        identity("rhosgobel", 1)
    }

    fn bywater() -> Identity {
        identity("bywater", 3)
    }

    fn morthond() -> Identity {
        identity("morthond", 5)
    }

    impl World {
        fn root(&self) -> PathBuf {
            self.dir.join("home")
        }

        fn env(&self) -> store::Env {
            let dir = self.dir.clone();
            store::Env::from_vars(move |name| match name {
                "BILBO_HOME" => Some(dir.join("home").into()),
                "BILBO_CONFIG" => Some(dir.join("config").into()),
                "XDG_STATE_HOME" => Some(dir.join("state").into()),
                _ => None,
            })
        }

        fn config(&self, text: &str) {
            fs::write(self.dir.join("config"), text).unwrap();
        }

        /// This device's keys, and a syncing `personal` whose manifest lists it and `others`.
        fn personal(&self, others: &[&Identity]) -> String {
            let keys = self.dir.join("state/bilbo/keys");
            let who = me();
            keys::write_identity(&keys, &who.owner, &who.device).unwrap();
            self.config("scope.personal.sync = file:///srv/bilbo\n");
            let members: Vec<Member> = others.iter().map(|i| Member::of(&i.device)).collect();
            let lock = manifest::lock(&self.root()).unwrap();
            manifest::create(&lock, &who, "personal", "file://", &members)
                .unwrap()
                .scope
        }

        fn note(&self, file: &str, id: &str, scope: Option<&str>, body: &str) {
            let scope = scope.map_or(String::new(), |s| format!("scope: {s}\n"));
            let text = format!(
                "---\nid: {id}\ncreated: 2026-10-03T10:00-03:00\n{scope}---\n\n# T\n\n{body}"
            );
            fs::write(self.root().join("notes").join(file), text).unwrap();
        }

        fn state(&self, scope: &str, fields: serde_json::Value) {
            let mut state = json!({
                "name": "personal", "device": me().device.id(), "own": 0, "cursors": {},
                "acks": {}, "sent": {}, "owed": false, "last_ack": null, "listed": [],
                "cutoffs": {}, "marked": [], "since": {}, "pulled_at": null,
                "pushed_at": null, "stops": {}, "error": null, "stopped": null, "halted": null,
            });
            for (key, value) in fields.as_object().unwrap() {
                state[key] = value.clone();
            }
            let dir = self.root().join(".bilbo/scopes").join(scope);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("state.json"), state.to_string()).unwrap();
        }

        fn summary(&self, entries: &[(&str, Entry)]) {
            let mut summary = Summary::default();
            for (id, entry) in entries {
                summary.notes.insert(id.to_string(), entry.clone());
            }
            let lock = versions::lock(&self.root()).unwrap();
            conflicts::write(&lock, &summary).unwrap();
        }

        /// Holds the watcher's lock, as a running `bilbo watch` does.
        fn watching(&mut self) {
            self.file = None;
            let path = versions::watch_lock_path(&self.root());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(path)
                .unwrap();
            file.lock().unwrap();
            self.file = Some(file);
        }

        fn at(&self, secs: i64) -> Result<Output, Failure> {
            status(
                &self.env(),
                jiff::Timestamp::from_second(T0 + secs).unwrap(),
                &TimeZone::UTC,
            )
        }

        fn out(&self, secs: i64) -> Output {
            match self.at(secs) {
                Ok(out) => out,
                Err(_) => panic!("status failed"),
            }
        }

        fn declare(&self, note: &str, reason: &str) -> Result<Output, Failure> {
            declare(&[note.to_string(), reason.to_string()], &self.env())
        }
    }

    fn refusal(result: Result<Output, Failure>) -> (u8, String) {
        match result {
            Ok(_) => panic!("expected a failure"),
            Err(Failure::Usage(m)) => (2, m),
            Err(Failure::Config(m)) => (2, m),
            Err(Failure::Refused(m)) => (1, m),
        }
    }

    fn open(passage: &str, version: &str) -> Open {
        Open {
            version: version.into(),
            passage: passage.into(),
            sides: vec!["a".repeat(64), "b".repeat(64)],
        }
    }

    fn block(sides: &[&str]) -> String {
        let mut out = String::new();
        for (n, side) in sides.iter().enumerate() {
            let mark = if n == 0 {
                "<<<<<<< bilbo "
            } else {
                "======= bilbo "
            };
            out.push_str(&format!(
                "{mark}{:012x} 2026-10-03T14:23-03:00\n{side}\n",
                n + 1
            ));
        }
        out.push_str(">>>>>>> bilbo\n");
        out
    }

    fn lost(conflict: &str, passage: &str, lines: &[&str]) -> Lost {
        Lost {
            conflict: conflict.into(),
            passage: passage.into(),
            lines: lines.iter().map(|l| l.to_string()).collect(),
        }
    }

    fn sent(at: i64, versions: bool) -> serde_json::Value {
        json!({"at": at, "versions": versions})
    }

    fn device_lines(out: &Output) -> Vec<&str> {
        out.lines
            .iter()
            .filter(|l| l.starts_with("device "))
            .map(String::as_str)
            .collect()
    }

    #[test]
    fn two_devices_in_step() {
        let mut w = world("step");
        let id = w.personal(&[&bywater()]);
        for n in 0..3u8 {
            w.note(
                &format!("plan-p{n}.md"),
                &format!("01M3YJ7R6HK6NQ30DCDB1P4D{n}A"),
                Some("personal"),
                "x\n",
            );
        }
        w.note("plan-l1.md", "01M3YJ7R6HK6NQ30DCDB1P4D1B", None, "x\n");
        w.note(
            "plan-l2.md",
            "01M3YJ7R6HK6NQ30DCDB1P4D2B",
            Some("work"),
            "x\n",
        );
        w.state(
            &id,
            json!({
                "pulled_at": T0 - 60, "pushed_at": T0 - 120, "own": 2,
                "sent": {"1": sent(T0 - 300, true), "2": sent(T0 - 200, true)},
                "acks": {bywater().device.id(): {me().device.id(): 2}},
            }),
        );
        w.watching();
        let out = w.out(0);
        let mut devices = [
            (me().device.id(), "rhosgobel", "this device"),
            (bywater().device.id(), "bywater", "up to date"),
        ];
        devices.sort();
        let mut expected = vec![
            "scope personal file:///srv/bilbo: 3 notes, pushed 2027-01-15T07:58+00:00, pulled 2027-01-15T07:59+00:00".to_string(),
        ];
        expected.extend(
            devices
                .iter()
                .map(|(_, name, state)| format!("device personal {name}: {state}")),
        );
        expected.push("local: 2 notes sync nowhere".into());
        assert_eq!(out.lines, expected);
        assert!(out.warnings.is_empty() && !out.failed);
    }

    #[test]
    fn a_scope_never_synced_says_never() {
        let w = world("never");
        w.personal(&[]);
        let out = w.out(0);
        assert_eq!(
            out.lines[0],
            "scope personal file:///srv/bilbo: 0 notes, pushed never, pulled never"
        );
        assert_eq!(out.lines[1], "device personal rhosgobel: this device");
    }

    #[test]
    fn a_device_behind_by_segments_and_one_gone_for_good() {
        let w = world("behind");
        let id = w.personal(&[&bywater(), &morthond()]);
        let (b, m) = (bywater().device.id(), morthond().device.id());
        let now = 200 * DAY;
        w.state(
            &id,
            json!({
                "own": 7,
                "sent": {
                    "1": sent(T0, true), "2": sent(T0 + 10, true),
                    "5": sent(T0 + now - 7 * DAY, true), "6": sent(T0 + now - 6 * DAY, true),
                    "7": sent(T0 + now - 5 * DAY, false),
                },
                "acks": {b.clone(): {me().device.id(): 4}, m.clone(): {me().device.id(): 2}},
            }),
        );
        let out = w.out(now);
        let lines = device_lines(&out);
        assert!(lines.contains(&"device personal bywater: behind by 2 segments"));
        assert!(lines.contains(&"device personal morthond: behind by 2 segments"));
        // the segment bywater never took, 200 days old, makes it stale
        w.state(
            &id,
            json!({
                "own": 1, "sent": {"1": sent(T0, true)},
                "acks": {b: {me().device.id(): 1}, m: {me().device.id(): 0}},
            }),
        );
        let out = w.out(now);
        let lines = device_lines(&out);
        assert!(lines.contains(&"device personal bywater: up to date"));
        assert!(
            lines.contains(&format!("device personal morthond: stale since {T0_TEXT}").as_str())
        );
    }

    #[test]
    fn a_device_listed_only_recently_is_not_stale_for_an_old_segment() {
        let w = world("since");
        let id = w.personal(&[&morthond()]);
        let m = morthond().device.id();
        w.state(
            &id,
            json!({
                "own": 1, "sent": {"1": sent(T0, true)},
                "since": {m.clone(): T0 + 199 * DAY},
                "acks": {m: {me().device.id(): 0}},
            }),
        );
        let out = w.out(200 * DAY);
        assert!(device_lines(&out).contains(&"device personal morthond: behind by 1 segments"));
    }

    #[test]
    fn declare_refuses_a_damaged_key_folder() {
        let w = world("damaged");
        w.personal(&[]);
        let keys = w.dir.join("state/bilbo/keys");
        fs::remove_file(keys.join("device.key")).unwrap();
        let (code, message) = refusal(w.declare("release", "x"));
        assert_eq!(code, 1);
        assert!(message.contains("damaged"), "{message}");
    }

    #[test]
    fn a_wrong_clock_on_the_acknowledging_device_changes_nothing() {
        let w = world("clock");
        let id = w.personal(&[&morthond()]);
        w.state(
            &id,
            json!({
                "own": 2, "sent": {"1": sent(T0, true), "2": sent(T0 + 60, true)},
                "acks": {morthond().device.id(): {me().device.id(): 2}},
            }),
        );
        let out = w.out(3600);
        assert!(device_lines(&out).contains(&"device personal morthond: up to date"));
    }

    #[test]
    fn an_open_conflict_names_the_file_and_fails() {
        let w = world("conflict");
        w.personal(&[]);
        let body = format!("## Nix\n\n{}", block(&["use A", "use B"]));
        w.note("gotcha-nix.md", NOTE, Some("personal"), &body);
        let entry = Entry {
            file: "gotcha-nix.md".into(),
            conflict: vec![open("Nix", &"c".repeat(64))],
            ..Entry::default()
        };
        w.summary(&[(NOTE, entry)]);
        let out = w.out(0);
        assert!(
            out.lines
                .contains(&"conflict notes/gotcha-nix.md: 1 passage".to_string())
        );
        assert!(out.failed);
    }

    #[test]
    fn dropped_text_counts_lines_and_a_restored_line_leaves() {
        let w = world("dropped");
        w.personal(&[]);
        w.note(
            "plan-release.md",
            NOTE,
            Some("personal"),
            "## Date\n\nkept\n",
        );
        let entry = Entry {
            file: "plan-release.md".into(),
            dropped: vec![lost(&"c".repeat(64), "Date", &["one", "two", "three"])],
            ..Entry::default()
        };
        w.summary(&[(NOTE, entry.clone())]);
        let out = w.out(0);
        assert!(
            out.lines
                .contains(&"dropped notes/plan-release.md: 3 lines not declared".to_string())
        );
        assert!(out.failed);
        w.note(
            "plan-release.md",
            NOTE,
            Some("personal"),
            "## Date\n\nkept\nthree\none\n",
        );
        let out = w.out(0);
        assert!(
            out.lines
                .contains(&"dropped notes/plan-release.md: 1 line not declared".to_string())
        );
        w.note(
            "plan-release.md",
            NOTE,
            Some("personal"),
            "## Date\n\nkept\nthree\none\ntwo\n",
        );
        let out = w.out(0);
        assert!(out.lines.iter().all(|l| !l.starts_with("dropped")));
    }

    #[test]
    fn changes_and_notices_of_the_window_are_listed() {
        let w = world("changes");
        let id = w.personal(&[]);
        let changes = format!(
            "{}\n{}\n{}\n",
            json!({"at": "2027-01-12T08:00:00+00:00", "n": 4, "kind": "device", "device": "morthond", "signer": "owner key"}),
            json!({"at": "2027-01-13T08:00:00+00:00", "n": 5, "kind": "epoch", "signer": "owner key"}),
            json!({"at": "2026-11-01T08:00:00+00:00", "n": 3, "kind": "device", "device": "old", "signer": "owner key"}),
        );
        let dir = w.root().join(".bilbo/scopes").join(&id);
        fs::write(dir.join("changes.jsonl"), changes).unwrap();
        let entry = Entry {
            file: "plan-x.md".into(),
            notices: vec![
                Notice {
                    at: "2027-01-14T10:00:00-03:00".into(),
                    flag: "edit-beat-delete".into(),
                },
                Notice {
                    at: "2027-01-01T10:00:00-03:00".into(),
                    flag: "stale-base".into(),
                },
            ],
            ..Entry::default()
        };
        w.summary(&[(NOTE, entry)]);
        let out = w.out(0);
        let tail: Vec<&str> = out
            .lines
            .iter()
            .skip_while(|l| !l.starts_with("local"))
            .skip(1)
            .map(String::as_str)
            .collect();
        assert_eq!(
            tail,
            [
                "notice 2027-01-14T10:00-03:00 notes/plan-x.md: edit-beat-delete",
                "change 2027-01-12T08:00+00:00 personal: device morthond added by owner key (manifest 4)",
                "change 2027-01-13T08:00+00:00 personal: epoch changed (manifest 5)",
            ]
        );
    }

    #[test]
    fn versions_held_back_for_a_parent_are_listed_per_device() {
        let w = world("waiting");
        w.personal(&[&bywater()]);
        let staged = |id: &str, parent: &str| {
            let version = Version {
                version: id.repeat(64),
                parents: vec![parent.repeat(64)],
                file: "plan-x.md".into(),
                blob: "d".repeat(64),
                event: "edited".into(),
                at: "2027-01-15T07:00:00+00:00".into(),
                device: Some(bywater().device.id()),
                ..Version::default()
            };
            let mut record = serde_json::to_value(&version).unwrap();
            record["note"] = json!(NOTE);
            json!({"seen": "2027-01-15T07:00:00Z", "scope": "personal", "record": record})
                .to_string()
        };
        // 1 follows 0, which never arrived; 2 follows 1; 3 follows 2
        let lines = [staged("1", "0"), staged("2", "1"), staged("3", "2")];
        let dir = w.root().join(".bilbo/sync");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("inbox.jsonl"), lines.join("\n") + "\n").unwrap();
        let out = w.out(0);
        assert!(
            out.lines
                .contains(&"waiting personal bywater: 3 versions".to_string())
        );
    }

    #[test]
    fn problems_go_to_stderr_and_fail() {
        let mut w = world("problems");
        let id = w.personal(&[&bywater()]);
        let b = bywater().device.id();
        w.state(
            &id,
            json!({
                "stopped": "sync personal: the manifest pins file:///other, not file:///srv/bilbo; fix scope.personal.sync",
                "stops": {b.clone(): {"seq": 3, "why": "missing", "replaceable": true}},
                "error": {"kind": "unreachable", "since": T0 - 3600, "message": "the folder does not exist"},
            }),
        );
        w.watching();
        let out = w.out(0);
        assert_eq!(
            out.warnings,
            [
                "sync personal: the manifest pins file:///other, not file:///srv/bilbo; fix scope.personal.sync",
                "sync personal: bywater stopped at segment 3: missing",
                "sync personal: file:///srv/bilbo not reachable since 2027-01-15T07:00+00:00: the folder does not exist",
            ]
        );
        assert!(out.failed);
        assert!(out.lines[0].starts_with("scope personal file:///srv/bilbo: 0 notes"));
        w.state(
            &id,
            json!({
                "stops": {b.clone(): {"seq": 3, "why": "it does not verify", "replaceable": false}},
                "error": {"kind": "full", "since": T0 - 60, "message": "the relay is full"},
            }),
        );
        let out = w.out(0);
        assert_eq!(
            out.warnings,
            [
                format!(
                    "sync personal: bywater stopped at segment 3: it does not verify; on bywater, copy <root>/.bilbo/scopes/{id}/out/00000000000000000003.seg over it"
                ),
                "sync personal: full since 2027-01-15T07:59+00:00: the relay is full".to_string(),
            ]
        );
    }

    #[test]
    fn a_read_failure_and_a_replaceable_segment_add_no_copy_advice() {
        let mut w = world("advice");
        let id = w.personal(&[&bywater()]);
        let b = bywater().device.id();
        for (why, replaceable) in [
            ("cannot read it: denied", false),
            ("it does not verify", true),
        ] {
            w.state(
                &id,
                json!({"stops": {b.clone(): {"seq": 2, "why": why, "replaceable": replaceable}}}),
            );
            w.watching();
            let out = w.out(0);
            assert_eq!(
                out.warnings,
                [format!(
                    "sync personal: bywater stopped at segment 2: {why}"
                )]
            );
        }
    }

    #[test]
    fn no_watcher_fails_and_a_clean_run_with_one_succeeds() {
        let mut w = world("watcher");
        w.personal(&[]);
        let out = w.out(0);
        assert_eq!(out.warnings, [NO_WATCHER]);
        assert!(out.failed);
        w.watching();
        let out = w.out(0);
        assert!(out.warnings.is_empty() && !out.failed);
    }

    #[test]
    fn nothing_changes_under_the_root() {
        let w = world("readonly");
        let id = w.personal(&[&bywater()]);
        w.note(
            "plan-x.md",
            NOTE,
            Some("personal"),
            &format!("## A\n\n{}", block(&["x", "y"])),
        );
        let entry = Entry {
            file: "plan-x.md".into(),
            conflict: vec![open("A", &"c".repeat(64))],
            ..Entry::default()
        };
        w.summary(&[(NOTE, entry)]);
        w.state(&id, json!({"own": 1, "sent": {"1": sent(T0, true)}}));
        fn snapshot(dir: &Path, into: &mut Vec<(PathBuf, Vec<u8>, std::time::SystemTime)>) {
            let mut entries: Vec<_> = fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            entries.sort();
            for path in entries {
                let meta = fs::symlink_metadata(&path).unwrap();
                let bytes = if meta.is_file() {
                    fs::read(&path).unwrap()
                } else {
                    Vec::new()
                };
                into.push((path.clone(), bytes, meta.modified().unwrap()));
                if meta.is_dir() {
                    snapshot(&path, into);
                }
            }
        }
        let (mut before, mut after) = (Vec::new(), Vec::new());
        snapshot(&w.root(), &mut before);
        let out = w.out(0);
        assert!(out.failed);
        snapshot(&w.root(), &mut after);
        assert_eq!(before, after);
    }

    #[test]
    fn no_store_and_no_syncing_scope_are_refusals() {
        let w = world("refuse");
        w.config("scope.personal.sync = off\n");
        assert_eq!(
            refusal(w.at(0)),
            (
                1,
                format!(
                    "no scope syncs; set scope.<name>.sync in {}",
                    w.dir.join("config").display()
                )
            )
        );
        fs::remove_dir_all(w.root().join("notes")).unwrap();
        assert_eq!(
            refusal(w.at(0)),
            (1, format!("no store at {}", w.root().display()))
        );
    }

    #[test]
    fn an_argument_other_than_declare_is_a_usage_error() {
        let w = world("args");
        for args in [vec!["now"], vec!["--now"]] {
            let args: Vec<String> = args.into_iter().map(String::from).collect();
            let (code, message) = refusal(run(&args, &w.env()));
            assert_eq!(code, 2);
            assert!(message.contains("now"));
        }
    }

    #[test]
    fn declare_takes_one_line_of_one_to_five_hundred_characters() {
        let w = world("reason");
        for reason in ["a\nb", "a\rb", "", &"x".repeat(501)] {
            assert_eq!(refusal(w.declare("release", reason)).0, 2, "{reason:?}");
        }
        let (code, _) = refusal(declare(&["release".to_string()], &w.env()));
        assert_eq!(code, 2);
        let long = "x".repeat(500);
        // past the usage checks it reaches the note lookup
        assert_eq!(
            refusal(w.declare("release", &long)),
            (1, "no history for release".into())
        );
    }

    fn resolved(w: &World) -> Entry {
        let lock = versions::lock(&w.root()).unwrap();
        let conflicted = format!("# T\n\n## Date\n\n{}", block(&["use A\nand B", "use C"]));
        let blob = versions::write_blob(&lock, conflicted.as_bytes()).unwrap();
        let version = Version {
            version: "c".repeat(64),
            file: "plan-release.md".into(),
            blob,
            event: "merged".into(),
            at: "2027-01-15T07:00:00+00:00".into(),
            ..Version::default()
        };
        versions::append(&lock, NOTE, &version).unwrap();
        Entry {
            file: "plan-release.md".into(),
            conflict: vec![open("Date", &"c".repeat(64))],
            ..Entry::default()
        }
    }

    #[test]
    fn declaring_a_drop_before_the_watcher_records_it() {
        let w = world("declare");
        w.personal(&[]);
        let entry = resolved(&w);
        w.summary(&[(NOTE, entry)]);
        w.note(
            "plan-release.md",
            NOTE,
            Some("personal"),
            "## Date\n\nuse A\n",
        );
        let before = w.out(0);
        assert!(
            before
                .lines
                .contains(&"dropped notes/plan-release.md: 2 lines not declared".to_string())
        );
        let out = w
            .declare("release", "B's date was superseded")
            .unwrap_or_else(|_| panic!("declare failed"));
        assert_eq!(
            out.lines,
            ["declared plan-release.md: 2 lines dropped on purpose"]
        );
        let log = versions::load(&w.root(), NOTE).unwrap();
        assert_eq!(log.declarations.len(), 1);
        assert_eq!(log.declarations[0].declare, "c".repeat(64));
        assert_eq!(log.declarations[0].reason, "B's date was superseded");
        let after = w.out(0);
        assert!(after.lines.iter().all(|l| !l.starts_with("dropped")));
        assert_eq!(
            refusal(w.declare("release", "again")),
            (1, "release has no dropped text to declare".into())
        );
    }

    #[test]
    fn declaring_names_every_conflict_version_of_the_dropped_text() {
        let w = world("declare-two");
        w.personal(&[]);
        w.note(
            "plan-release.md",
            NOTE,
            Some("personal"),
            "## Date\n\nkept\n",
        );
        let entry = Entry {
            file: "plan-release.md".into(),
            dropped: vec![
                lost(&"c".repeat(64), "Date", &["one", "two"]),
                lost(&"d".repeat(64), "Date", &["two", "three"]),
            ],
            ..Entry::default()
        };
        w.summary(&[(NOTE, entry)]);
        let lock = versions::lock(&w.root()).unwrap();
        let version = Version {
            version: "c".repeat(64),
            file: "plan-release.md".into(),
            blob: versions::write_blob(&lock, b"x").unwrap(),
            event: "edited".into(),
            at: "2027-01-15T07:00:00+00:00".into(),
            ..Version::default()
        };
        versions::append(&lock, NOTE, &version).unwrap();
        drop(lock);
        let out = w
            .declare("release", "ok")
            .unwrap_or_else(|_| panic!("declare failed"));
        assert_eq!(
            out.lines,
            ["declared plan-release.md: 3 lines dropped on purpose"]
        );
        let declared: Vec<String> = versions::load(&w.root(), NOTE)
            .unwrap()
            .declarations
            .into_iter()
            .map(|d| d.declare)
            .collect();
        assert_eq!(declared, ["c".repeat(64), "d".repeat(64)]);
    }

    #[test]
    fn a_note_without_dropped_text_has_nothing_to_declare() {
        let w = world("nothing");
        w.personal(&[]);
        w.note("plan-release.md", OTHER, Some("personal"), "text\n");
        let lock = versions::lock(&w.root()).unwrap();
        let version = Version {
            version: "c".repeat(64),
            file: "plan-release.md".into(),
            blob: versions::write_blob(&lock, b"x").unwrap(),
            event: "added".into(),
            at: "2027-01-15T07:00:00+00:00".into(),
            ..Version::default()
        };
        versions::append(&lock, OTHER, &version).unwrap();
        drop(lock);
        assert_eq!(
            refusal(w.declare("release", "x")),
            (1, "release has no dropped text to declare".into())
        );
        assert!(
            versions::load(&w.root(), OTHER)
                .unwrap()
                .declarations
                .is_empty()
        );
    }
}
