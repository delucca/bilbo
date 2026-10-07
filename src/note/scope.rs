//! `bilbo scope`: lists the scopes this device declares, and `scope set` gives notes a scope. `set` changes one line
//! per note under `history/lock`, through restore's hidden file and an atomic exchange, so a write that races it is
//! never lost.

use std::fs;
use std::path::{Path, PathBuf};

use crate::Failure;
use crate::host::swap;
use crate::note::versions::{self, Lock};
use crate::note::{parse_name, read_id};
use crate::search::documents;
use crate::shared::config;
use crate::shared::frontmatter::split_key;
use crate::shared::hash;
use crate::shared::store;
use crate::sync::integrate::{self, Params};

/// What `apply` is about to exchange when it calls its hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exchange {
    /// Swap the new text in.
    First,
    /// Swap back, after the text that came out differed from what was read.
    Back,
}

/// Why a note was not handled.
#[derive(Debug, PartialEq, Eq)]
pub enum Fail {
    /// This note only: the reason, without its name.
    File(String),
    /// The whole run: the filesystem cannot swap.
    Abort(String),
}

pub struct Output {
    /// stderr lines, without "bilbo: ".
    pub warnings: Vec<String>,
    pub lines: Vec<String>,
    pub failed: bool,
}

pub fn run(args: &[String], env: &store::Env) -> Result<Output, Failure> {
    match args.first().map(String::as_str) {
        None => list(env),
        Some("set") => set(&args[1..], env),
        Some(arg) if arg.starts_with('-') => Err(Failure::Usage(format!("unknown option '{arg}'"))),
        Some(arg) => Err(Failure::Usage(format!("unexpected argument '{arg}'"))),
    }
}

fn list(env: &store::Env) -> Result<Output, Failure> {
    let settings = config::load(env).map_err(Failure::Config)?;
    let root = store::root(env).map_err(Failure::Config)?;
    let notes = root.join("notes");
    let stored = if notes.is_dir() {
        documents::read_notes(&notes)
            .map_err(|e| Failure::Refused(format!("cannot read {}: {e}", notes.display())))?
    } else {
        Vec::new()
    };
    let mut lines = Vec::new();
    for scope in &settings.scopes {
        let count = stored
            .iter()
            .filter(|n| n.scope.as_deref() == Some(&scope.name))
            .count();
        let paths = if scope.paths.is_empty() {
            "-".to_string()
        } else {
            scope.paths.join(", ")
        };
        let mut line = format!(
            "{}\t{count} notes\tsync {}\tembedder {}\tpaths {paths}",
            scope.name,
            scope.sync,
            scope.embedder.as_str(),
        );
        if settings.default_scope.as_deref() == Some(&scope.name) {
            line.push_str("\tdefault");
        }
        lines.push(line);
    }
    let unassigned = stored
        .iter()
        .filter(|n| {
            n.scope
                .as_deref()
                .is_none_or(|name| settings.scope(name).is_none())
        })
        .count();
    lines.push(format!(
        "(unassigned)\t{unassigned} notes\tembedder {}",
        settings.rule(None).as_str()
    ));
    let mut warnings = Vec::new();
    if settings.scopes.is_empty() {
        warnings.push(format!(
            "no scopes declared; add scope.<name>.* keys to {}",
            settings.shown_path()
        ));
    }
    Ok(Output {
        warnings,
        lines,
        failed: false,
    })
}

fn set(args: &[String], env: &store::Env) -> Result<Output, Failure> {
    let (force, name, files) = parse_set(args)?;
    let settings = config::load(env).map_err(Failure::Config)?;
    if settings.scope(&name).is_none() {
        let declared = settings.scope_names();
        return Err(Failure::Usage(format!(
            "scope '{name}' is not declared in {}; declared scopes: {}",
            settings.shown_path(),
            if declared.is_empty() {
                "none".to_string()
            } else {
                declared.join(", ")
            }
        )));
    }
    let root = store::root(env).map_err(Failure::Config)?;
    let notes = root.join("notes");
    let mut out = Output {
        warnings: Vec::new(),
        lines: Vec::new(),
        failed: false,
    };
    let syncs = |name: &str| settings.scope(name).is_some_and(|s| s.sync != "off");
    let params = Params {
        syncs: &syncs,
        now: jiff::Timestamp::now(),
        stale_days: settings.sync.stale_days,
    };
    let mut lock: Option<Lock> = None;
    for file in &files {
        let shown = match locate(file, &notes) {
            Ok(shown) => shown,
            Err(reason) => {
                out.failed = true;
                out.warnings.push(format!("{file}: {reason}"));
                continue;
            }
        };
        if lock.is_none() {
            let held = versions::lock(&root).and_then(|held| {
                let staged = integrate::staged(&root)?;
                let swept = versions::sweep_restore_leftovers(&held, &versions::now_at(), &staged)?;
                Ok((held, swept))
            });
            match held {
                Ok((held, swept)) => {
                    out.warnings.extend(swept);
                    lock = Some(held);
                }
                Err(message) => {
                    out.failed = true;
                    out.warnings.push(message);
                    break;
                }
            }
        }
        let Some(held) = &lock else { continue };
        match apply(held, &params, &shown, &name, force, &mut |_| Ok(())) {
            Ok(done) => out.lines.push(format!("notes/{shown}: {done}")),
            Err(Fail::File(reason)) => {
                out.failed = true;
                out.warnings.push(format!("notes/{shown}: {reason}"));
            }
            Err(Fail::Abort(message)) => {
                out.failed = true;
                out.warnings.push(message);
                break;
            }
        }
    }
    Ok(out)
}

fn parse_set(args: &[String]) -> Result<(bool, String, Vec<String>), Failure> {
    let mut force = false;
    let mut positional = Vec::new();
    let mut options = true;
    for arg in args {
        if options && arg == "--" {
            options = false;
        } else if options && arg == "--force" {
            force = true;
        } else if options && arg.starts_with('-') {
            return Err(Failure::Usage(format!("unknown option '{arg}'")));
        } else {
            positional.push(arg.clone());
        }
    }
    if positional.len() < 2 {
        return Err(Failure::Usage(
            "scope set needs a scope name and at least one file".into(),
        ));
    }
    let name = positional.remove(0);
    Ok((force, name, positional))
}

/// The file name of `arg` when it is a regular file directly in `notes/` with a note's name, else why not.
fn locate(arg: &str, notes: &Path) -> Result<String, String> {
    let path = PathBuf::from(arg);
    let meta = fs::symlink_metadata(&path).map_err(|e| format!("cannot read: {e}"))?;
    if !meta.file_type().is_file() {
        return Err("not a regular file".into());
    }
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let here = fs::canonicalize(parent).ok();
    if here.is_none() || here != fs::canonicalize(notes).ok() {
        return Err(format!("not a note in {}", notes.display()));
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("name: must be valid UTF-8")?;
    parse_name(name)?;
    Ok(name.to_string())
}

/// What `edit` decides for a note's text.
#[derive(Debug)]
enum Edit {
    /// Nothing changes; the line to print.
    Keep(String),
    /// The new text, and the line to print.
    Write(Vec<u8>, String),
}

/// Gives the note `name` the scope `scope` and returns the line to print after `notes/<name>: `. `p` is what
/// `integrate::record_local` needs for a save the watcher has not recorded. `hook` is called before each exchange; an
/// error from it stands for a failed exchange.
pub fn apply(
    lock: &Lock,
    p: &Params,
    name: &str,
    scope: &str,
    force: bool,
    hook: &mut dyn FnMut(Exchange) -> Result<(), String>,
) -> Result<String, Fail> {
    let root = lock.root();
    let path = root.join("notes").join(name);
    let mut read = fs::read(&path).map_err(|e| Fail::File(format!("cannot read: {e}")))?;
    let (mut written, mut done) = match plan(&read, scope, force)? {
        Edit::Keep(done) => return Ok(done),
        Edit::Write(written, done) => (written, done),
    };
    let id = id_of(&read)?;
    let temp = versions::restore_path(root, &id);
    let hidden = format!(
        "notes/{}",
        temp.file_name().unwrap_or_default().to_string_lossy()
    );
    if fs::symlink_metadata(&temp).is_ok() {
        return Err(Fail::File(format!(
            "{hidden} is left from an earlier run; bilbo watch records it once it has seen the note"
        )));
    }
    // A save the watcher has not recorded is recorded first; with a stale-base entry it is merged against the base,
    // which may rewrite the file.
    integrate::record_local(lock, p, &id, name, Some(&read), &versions::now_at())
        .map_err(Fail::File)?;
    let now = fs::read(&path).map_err(|e| Fail::File(format!("cannot read: {e}")))?;
    if now != read {
        read = now;
        match plan(&read, scope, force)? {
            Edit::Keep(kept) => return Ok(kept),
            Edit::Write(bytes, line) => (written, done) = (bytes, line),
        }
    }
    versions::write_temp(&temp, &written, Some(name)).map_err(Fail::File)?;
    if let Err(e) = hook(Exchange::First).and_then(|()| swap::exchange(&path, &temp)) {
        let _ = fs::remove_file(&temp);
        return Err(if e == swap::UNSUPPORTED {
            Fail::Abort(format!("cannot set scopes on this filesystem: {e}"))
        } else {
            Fail::File(e)
        });
    }
    let out = fs::read(&temp).map_err(|e| {
        Fail::File(format!(
            "cannot read {hidden}: {e}; it holds the text the note had"
        ))
    })?;
    if out == read {
        remove(&temp, &hidden, "set")?;
        return match record(lock, p, &id, name, &read, &written) {
            Ok(()) => Ok(done),
            Err(e) => Err(Fail::File(format!("{done}, but {e}"))),
        };
    }
    if let Err(e) = hook(Exchange::Back).and_then(|()| swap::exchange(&path, &temp)) {
        return Err(Fail::File(format!(
            "changed while bilbo scope set ran, and {e}; the other text is at {hidden}"
        )));
    }
    match fs::read(&temp) {
        Ok(out) if out == written => remove(
            &temp,
            &hidden,
            "changed while bilbo scope set ran; run it again",
        )?,
        Ok(_) => {
            return Err(Fail::File(format!(
                "changed while bilbo scope set ran twice; the later write is at {hidden}"
            )));
        }
        Err(e) => {
            return Err(Fail::File(format!(
                "changed while bilbo scope set ran, and cannot read {hidden}: {e}"
            )));
        }
    }
    Err(Fail::File(
        "changed while bilbo scope set ran; run it again".into(),
    ))
}

/// Records the text `apply` wrote as a version that follows the head group holding `read`, the bytes it replaced, so
/// the watcher does not take it for a save, and makes it the written version of a stale-base entry, which keeps its
/// base.
fn record(
    lock: &Lock,
    p: &Params,
    id: &str,
    name: &str,
    read: &[u8],
    written: &[u8],
) -> Result<(), String> {
    let log = versions::load(lock.root(), id)?;
    let parents = match integrate::held_group(&log.versions, name, &hash::sha256_hex(read)) {
        Some(group) => group,
        None => match versions::heads(&log.versions).as_slice() {
            [only] => only.iter().map(|v| v.version.clone()).collect(),
            _ => versions::last_parents(lock.root(), id)?,
        },
    };
    let digest = hash::sha256_hex(written);
    let latest = log
        .versions
        .iter()
        .find(|v| parents.first() == Some(&v.version));
    let Some(event) = versions::event_for(latest, Some((name, &digest))) else {
        return Ok(());
    };
    let version = versions::record(
        lock,
        id,
        &parents,
        name,
        Some(written),
        event,
        &versions::now_at(),
    )?;
    integrate::keep_base(lock, id, &version.version)?;
    integrate::refresh_open(lock, p, &[id.to_string()])
}

/// What `edit` decides for the bytes `read`.
fn plan(read: &[u8], scope: &str, force: bool) -> Result<Edit, Fail> {
    let text = std::str::from_utf8(read).map_err(|_| Fail::File("not valid UTF-8".into()))?;
    edit(text, scope, force).map_err(Fail::File)
}

fn id_of(read: &[u8]) -> Result<String, Fail> {
    std::str::from_utf8(read)
        .ok()
        .and_then(read_id)
        .ok_or_else(|| Fail::File("id: missing or not a canonical id".into()))
}

fn remove(temp: &Path, hidden: &str, lead: &str) -> Result<(), Fail> {
    fs::remove_file(temp)
        .map_err(|e| Fail::File(format!("{lead}, but cannot remove {hidden}: {e}")))
}

/// The text with its one `scope` line set to `scope`, changing no other byte, or why the note is refused.
fn edit(text: &str, scope: &str, force: bool) -> Result<Edit, String> {
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut at = text.len() - body.len();
    for line in body.split_inclusive('\n') {
        lines.push((at, line));
        at += line.len();
    }
    let bare = |line: &str| {
        line.trim_end_matches('\n')
            .trim_end_matches('\r')
            .to_string()
    };
    if lines.first().map(|(_, l)| bare(l)).as_deref() != Some("---") {
        return Err("frontmatter: missing; line 1 must be '---'".into());
    }
    let Some(close) = lines[1..].iter().position(|(_, l)| bare(l) == "---") else {
        return Err("frontmatter: no closing '---' line".into());
    };
    let close = close + 1;
    let scopes: Vec<(usize, &str)> = lines[1..close]
        .iter()
        .filter(|(_, l)| !l.starts_with([' ', '-']))
        .filter(|(_, l)| split_key(&bare(l)).is_some_and(|(key, _)| key == "scope"))
        .copied()
        .collect();
    if scopes.len() > 1 {
        return Err("scope: given more than once".into());
    }
    let bytes = text.as_bytes();
    let Some(&(start, line)) = scopes.first() else {
        let (start, closing) = lines[close];
        let ending = if closing.trim_end_matches('\n').ends_with('\r') {
            "\r\n"
        } else {
            "\n"
        };
        let mut out = bytes[..start].to_vec();
        out.extend_from_slice(format!("scope: {scope}{ending}").as_bytes());
        out.extend_from_slice(&bytes[start..]);
        return Ok(Edit::Write(out, format!("set {scope}")));
    };
    let line = bare(line);
    if line == format!("scope: {scope}") {
        return Ok(Edit::Keep(format!("kept {scope}")));
    }
    let old = line
        .strip_prefix("scope: ")
        .filter(|value| !value.is_empty() && *value == value.trim());
    if !force {
        return Ok(Edit::Keep(match old {
            Some(old) => format!("kept {old}; --force replaces it"),
            None => format!("kept '{line}' as written; --force rewrites it"),
        }));
    }
    let mut out = bytes[..start].to_vec();
    out.extend_from_slice(format!("scope: {scope}").as_bytes());
    out.extend_from_slice(&bytes[start + line.len()..]);
    Ok(Edit::Write(
        out,
        match old {
            Some(old) => format!("replaced {old} with {scope}"),
            None => format!("rewrote '{line}' as scope: {scope}"),
        },
    ))
}

/// `bilbo scope --help`; its Usage block is also the synopsis a usage error shows.
pub const HELP: &str = r#"bilbo scope: list this device's scopes, or give notes a scope.

Usage:
  bilbo scope
  bilbo scope set [--force] <name> <file>...

Options:
  --force  set: replace a scope a note already holds
  --       set: end the options; every later argument is a name or a file
Scopes are declared in the config as scope.<name>.* keys. set changes only
the scope line of each note's frontmatter; each <file> is a note in
<root>/notes/, and set handles every file even when one fails.

Output of scope: one line per declared scope, sorted by name, then one line
for the notes with no scope:
  <name> TAB <n> notes TAB sync <value> TAB embedder <rule> TAB paths <list>
  (unassigned) TAB <n> notes TAB embedder <rule>
paths is - when unset, and the line of the scope.default scope ends with
TAB default.
Output of set, one line per note:
  notes/<file>: set <name>
  notes/<file>: kept <name>
  notes/<file>: kept <other>; --force replaces it
  notes/<file>: replaced <other> with <name>
  notes/<file>: kept '<line>' as written; --force rewrites it
  notes/<file>: rewrote '<line>' as scope: <name>
A file set refuses is named on stderr instead.

Exit: 0 success; 1 set refused a file; 2 usage or config error, an
undeclared scope included.

Examples:
  bilbo scope
  bilbo scope set work ~/.local/share/bilbo/notes/gotcha-acme-deploy.md

Docs: https://github.com/delucca/bilbo/wiki/Commands#scope
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::versions::{EDITED, load, record_difference};

    const ID: &str = "01M3YJ7R6HK6NQ30DCDB1P4D01";
    const HEAD: &str = "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4D01\ncreated: 2026-10-02T14:23-03:00\n";

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-scope-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("notes")).unwrap();
        Scratch(dir)
    }

    fn note(extra: &str) -> String {
        format!("{HEAD}{extra}---\n\n# A\n\nbody\n")
    }

    fn put(root: &Path, text: &str) {
        fs::write(root.join("notes/plan-a.md"), text).unwrap();
    }

    fn read(root: &Path) -> String {
        fs::read_to_string(root.join("notes/plan-a.md")).unwrap()
    }

    fn files(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(root.join("notes"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// What `bilbo watch` would record for the note as it is now.
    fn watch(root: &Path) {
        let bytes = fs::read(root.join("notes/plan-a.md")).unwrap();
        let lock = versions::lock(root).unwrap();
        record_difference(&lock, ID, Some(("plan-a.md", &bytes)), &versions::now_at()).unwrap();
    }

    fn no_sync(_: &str) -> bool {
        false
    }

    fn params() -> Params<'static> {
        Params {
            syncs: &no_sync,
            now: "2026-10-04T15:00:00Z".parse().unwrap(),
            stale_days: 180,
        }
    }

    fn go_to(
        root: &Path,
        scope: &str,
        hook: &mut dyn FnMut(Exchange) -> Result<(), String>,
    ) -> Result<String, Fail> {
        let lock = versions::lock(root).unwrap();
        apply(&lock, &params(), "plan-a.md", scope, false, hook)
    }

    fn go(
        root: &Path,
        force: bool,
        hook: &mut dyn FnMut(Exchange) -> Result<(), String>,
    ) -> Result<String, Fail> {
        let lock = versions::lock(root).unwrap();
        apply(&lock, &params(), "plan-a.md", "work", force, hook)
    }

    fn written(edit: Result<Edit, String>) -> (String, String) {
        match edit.unwrap() {
            Edit::Write(bytes, done) => (String::from_utf8(bytes).unwrap(), done),
            Edit::Keep(done) => panic!("kept: {done}"),
        }
    }

    #[test]
    fn a_missing_key_is_inserted_before_the_closing_line() {
        let (text, done) = written(edit(&note(""), "work", false));
        assert_eq!(text, note("scope: work\n"));
        assert_eq!(done, "set work");
    }

    #[test]
    fn an_inserted_line_follows_the_files_line_endings_and_keeps_a_bom() {
        let crlf = note("").replace('\n', "\r\n");
        let (text, _) = written(edit(&crlf, "work", false));
        assert_eq!(text, note("scope: work\n").replace('\n', "\r\n"));
        let bom = format!("\u{feff}{}", note(""));
        let (text, _) = written(edit(&bom, "work", false));
        assert_eq!(text, format!("\u{feff}{}", note("scope: work\n")));
    }

    #[test]
    fn a_key_with_the_same_value_or_another_is_kept() {
        let same = note("scope: work\n");
        assert!(matches!(edit(&same, "work", true), Ok(Edit::Keep(d)) if d == "kept work"));
        let other = note("scope: personal\n");
        assert!(matches!(
            edit(&other, "work", false),
            Ok(Edit::Keep(d)) if d == "kept personal; --force replaces it"
        ));
    }

    #[test]
    fn force_replaces_one_line_and_mends_an_invalid_value() {
        let other = note("scope: personal\n");
        let (text, done) = written(edit(&other, "work", true));
        assert_eq!(text, note("scope: work\n"));
        assert_eq!(done, "replaced personal with work");
        let (text, done) = written(edit(&note("scope: Work\n"), "work", true));
        assert_eq!(text, note("scope: work\n"));
        assert_eq!(done, "replaced Work with work");
        let crlf = note("scope: personal\n").replace('\n', "\r\n");
        let (text, _) = written(edit(&crlf, "work", true));
        assert_eq!(text, note("scope: work\n").replace('\n', "\r\n"));
    }

    #[test]
    fn a_line_that_is_not_the_canonical_form_is_named_as_written() {
        for line in ["scope:work", "scope: work ", "scope:"] {
            let text = note(&format!("{line}\n"));
            assert!(matches!(
                edit(&text, "work", false),
                Ok(Edit::Keep(d)) if d == format!("kept '{line}' as written; --force rewrites it")
            ));
            let (mended, done) = written(edit(&text, "work", true));
            assert_eq!(mended, note("scope: work\n"));
            assert_eq!(done, format!("rewrote '{line}' as scope: work"));
        }
    }

    #[test]
    fn a_scope_line_inside_the_sources_list_is_not_the_key() {
        let sources = note("sources:\n  - \"doc: scope: x\"\n");
        let (text, _) = written(edit(&sources, "work", false));
        assert_eq!(text, note("sources:\n  - \"doc: scope: x\"\nscope: work\n"));
    }

    #[test]
    fn a_file_without_closed_frontmatter_or_with_two_keys_is_refused() {
        assert!(
            edit("# Title\n", "work", false)
                .unwrap_err()
                .contains("frontmatter")
        );
        assert!(
            edit("---\nid: x\n# Title\n", "work", false)
                .unwrap_err()
                .contains("closing")
        );
        let two = note("scope: work\nscope: personal\n");
        assert!(edit(&two, "work", true).unwrap_err().contains("scope"));
    }

    #[test]
    fn sets_the_key_and_leaves_no_hidden_file() {
        let s = scratch("plain");
        put(&s.0, &note(""));
        let done = go(&s.0, false, &mut |_| Ok(())).unwrap();
        assert_eq!(done, "set work");
        assert_eq!(read(&s.0), note("scope: work\n"));
        assert_eq!(files(&s.0), ["plan-a.md"]);
    }

    #[test]
    fn a_note_that_is_not_utf8_is_refused_untouched() {
        let s = scratch("utf8");
        let mut bytes = note("").into_bytes();
        bytes.extend_from_slice(&[0xff, 0xfe]);
        fs::write(s.0.join("notes/plan-a.md"), &bytes).unwrap();
        let err = go(&s.0, false, &mut |_| Ok(())).unwrap_err();
        assert_eq!(err, Fail::File("not valid UTF-8".into()));
        assert_eq!(fs::read(s.0.join("notes/plan-a.md")).unwrap(), bytes);
        assert_eq!(files(&s.0), ["plan-a.md"]);
    }

    #[test]
    fn a_kept_note_is_not_touched_and_creates_no_hidden_file() {
        let s = scratch("kept");
        put(&s.0, &note("scope: work\n"));
        let hook = &mut |_| Err("exchanged".to_string());
        assert_eq!(go(&s.0, false, hook).unwrap(), "kept work");
        assert_eq!(files(&s.0), ["plan-a.md"]);
    }

    #[test]
    fn a_write_before_the_swap_is_kept_and_reported() {
        let s = scratch("race");
        put(&s.0, &note(""));
        watch(&s.0);
        let agent = note("") + "agent\n";
        let mut writes = 0;
        let err = go(&s.0, false, &mut |step| {
            if step == Exchange::First {
                writes += 1;
                put(&s.0, &agent);
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(
            err,
            Fail::File("changed while bilbo scope set ran; run it again".into())
        );
        assert_eq!(writes, 1);
        assert_eq!(read(&s.0), agent);
        assert_eq!(files(&s.0), ["plan-a.md"]);
    }

    #[test]
    fn a_second_write_between_the_swaps_is_parked_and_swept_into_history() {
        let s = scratch("second");
        put(&s.0, &note(""));
        watch(&s.0);
        let first = note("") + "first\n";
        let second = note("") + "second\n";
        let err = go(&s.0, false, &mut |step| {
            match step {
                Exchange::First => put(&s.0, &first),
                Exchange::Back => put(&s.0, &second),
            }
            Ok(())
        })
        .unwrap_err();
        let hidden = format!("notes/.bilbo-restore-{ID}");
        let Fail::File(message) = err else {
            panic!("not a file failure")
        };
        assert!(message.contains(&hidden), "{message}");
        assert_eq!(read(&s.0), first);
        assert_eq!(
            fs::read_to_string(s.0.join(&hidden)).unwrap(),
            second,
            "the later write stays at the hidden name"
        );
        let lock = versions::lock(&s.0).unwrap();
        let messages =
            versions::sweep_restore_leftovers(&lock, &versions::now_at(), &Default::default())
                .unwrap();
        assert_eq!(messages.len(), 1);
        drop(lock);
        assert_eq!(files(&s.0), ["plan-a.md"]);
        let log = load(&s.0, ID).unwrap();
        let latest = log.latest().unwrap();
        assert_eq!(latest.event, EDITED);
        assert_eq!(versions::content(&s.0, latest).unwrap(), second.as_bytes());
    }

    #[test]
    fn a_filesystem_that_cannot_swap_aborts_and_changes_nothing() {
        let s = scratch("unsupported");
        put(&s.0, &note(""));
        let err = go(&s.0, false, &mut |_| Err(swap::UNSUPPORTED.to_string())).unwrap_err();
        assert_eq!(
            err,
            Fail::Abort(
                "cannot set scopes on this filesystem: it cannot swap files atomically".into()
            )
        );
        assert_eq!(read(&s.0), note(""));
        assert_eq!(files(&s.0), ["plan-a.md"]);
    }

    #[test]
    fn a_parked_file_of_a_note_without_history_blocks_the_next_set() {
        let s = scratch("parked");
        put(&s.0, &note(""));
        fs::write(s.0.join(format!("notes/.bilbo-restore-{ID}")), "parked").unwrap();
        let Fail::File(message) = go(&s.0, false, &mut |_| Ok(())).unwrap_err() else {
            panic!("not a file failure")
        };
        assert!(
            message.contains(&format!("notes/.bilbo-restore-{ID}")),
            "{message}"
        );
        assert!(message.contains("bilbo watch records it"), "{message}");
        assert_eq!(read(&s.0), note(""));
    }

    fn events(root: &Path) -> Vec<String> {
        load(root, ID)
            .unwrap()
            .versions
            .iter()
            .map(|v| v.event.clone())
            .collect()
    }

    #[test]
    fn the_version_it_wrote_is_recorded_so_the_watcher_adds_none() {
        let s = scratch("recorded");
        put(&s.0, &note(""));
        watch(&s.0);
        go(&s.0, false, &mut |_| Ok(())).unwrap();
        assert_eq!(events(&s.0), ["added", "edited"]);
        let log = load(&s.0, ID).unwrap();
        let latest = log.latest().unwrap();
        assert_eq!(
            versions::content(&s.0, latest).unwrap(),
            note("scope: work\n").as_bytes()
        );
        assert_eq!(latest.parents, [log.versions[0].version.clone()]);
        watch(&s.0);
        assert_eq!(events(&s.0), ["added", "edited"]);
    }

    #[test]
    fn a_kept_or_raced_note_records_nothing() {
        let s = scratch("not-recorded");
        put(&s.0, &note("scope: work\n"));
        watch(&s.0);
        let agent = note("scope: work\n") + "agent\n";
        go(&s.0, false, &mut |_| Ok(())).unwrap();
        go(&s.0, true, &mut |_| Ok(())).unwrap();
        assert_eq!(events(&s.0), ["added"]);
        put(&s.0, &note("scope: personal\n"));
        watch(&s.0);
        let err = go(&s.0, true, &mut |step| {
            if step == Exchange::First {
                put(&s.0, &agent);
            }
            Ok(())
        });
        assert!(err.is_err());
        assert_eq!(events(&s.0), ["added", "edited"]);
    }

    fn two_passages(setup: &str, rollout: &str) -> String {
        format!("{HEAD}---\n\n# A\n\n## Setup\n{setup}\n\n## Rollout\n{rollout}\n")
    }

    /// H, then the edit sync wrote (B's `## Setup`), which the file holds, with its stale-base entry.
    fn synced(root: &Path) -> Vec<String> {
        put(root, &two_passages("base setup", "base rollout"));
        watch(root);
        put(root, &two_passages("B setup", "base rollout"));
        watch(root);
        let ids: Vec<String> = load(root, ID)
            .unwrap()
            .versions
            .iter()
            .map(|v| v.version.clone())
            .collect();
        let dir = store::sync_dir(root);
        fs::create_dir_all(&dir).unwrap();
        let entry = serde_json::json!({ ID: { "base": ids[0], "written": ids[1] } });
        fs::write(dir.join("stale-base.json"), entry.to_string()).unwrap();
        ids
    }

    #[test]
    fn a_stale_save_after_scope_set_is_still_merged() {
        let s = scratch("stale-after");
        let ids = synced(&s.0);

        let done = go_to(&s.0, "personal", &mut |_| Ok(())).unwrap();

        assert_eq!(done, "set personal");
        let log = load(&s.0, ID).unwrap();
        let wrote = log.latest().unwrap();
        assert_eq!(wrote.event, EDITED);
        assert_eq!(
            integrate::stale_base(&s.0, ID).unwrap(),
            Some(integrate::Base {
                base: ids[0].clone(),
                written: wrote.version.clone()
            })
        );
        let lock = versions::lock(&s.0).unwrap();
        integrate::record_local(
            &lock,
            &params(),
            ID,
            "plan-a.md",
            Some(two_passages("base setup", "agent rollout").as_bytes()),
            &versions::now_at(),
        )
        .unwrap();
        assert_eq!(
            read(&s.0),
            format!(
                "{HEAD}scope: personal\n---\n\n# A\n\n## Setup\nB setup\n\n## Rollout\nagent rollout\n"
            )
        );
        let log = load(&s.0, ID).unwrap();
        let last = log.latest().unwrap();
        assert_eq!(last.event, "merged");
        assert_eq!(last.flags, ["stale-base"]);
        assert!(log.versions.iter().any(|v| v.version == wrote.version));
    }

    #[test]
    fn a_staged_version_is_left_at_its_hidden_name_by_the_sweep() {
        let s = scratch("staged");
        put(&s.0, &note(""));
        watch(&s.0);
        let inbound = note("scope: work\n") + "inbound\n";
        fs::write(s.0.join(format!("notes/.bilbo-restore-{ID}")), &inbound).unwrap();
        let dir = store::sync_dir(&s.0);
        fs::create_dir_all(&dir).unwrap();
        let line = serde_json::json!({
            "seen": "2026-10-04T12:00:00-03:00",
            "scope": "work",
            "record": {
                "note": ID,
                "version": "f".repeat(64),
                "parents": [],
                "file": "plan-a.md",
                "blob": hash::sha256_hex(inbound.as_bytes()),
                "event": "edited",
                "at": "2026-10-04T12:00:00-03:00",
            },
        });
        fs::write(dir.join("inbox.jsonl"), format!("{line}\n")).unwrap();
        let staged = integrate::staged(&s.0).unwrap();
        let lock = versions::lock(&s.0).unwrap();

        let messages =
            versions::sweep_restore_leftovers(&lock, &versions::now_at(), &staged).unwrap();

        assert!(messages.is_empty(), "{messages:?}");
        assert!(s.0.join(format!("notes/.bilbo-restore-{ID}")).exists());
        assert_eq!(events(&s.0), ["added"]);
    }

    #[test]
    fn the_version_it_wrote_follows_the_head_the_file_held_not_the_last_line() {
        let s = scratch("two-heads");
        let lock = versions::lock(&s.0).unwrap();
        let at = versions::now_at();
        let record = |parents: &[String], text: &str| {
            let event = if parents.is_empty() { "added" } else { EDITED };
            versions::record(
                &lock,
                ID,
                parents,
                "plan-a.md",
                Some(text.as_bytes()),
                event,
                &at,
            )
            .unwrap()
        };
        let first = record(&[], &note(""));
        let mine = record(
            std::slice::from_ref(&first.version),
            &note("").replace("body", "mine"),
        );
        let theirs = record(
            std::slice::from_ref(&first.version),
            &note("").replace("body", "theirs"),
        );
        drop(lock);
        put(&s.0, &note("").replace("body", "mine"));

        go(&s.0, false, &mut |_| Ok(())).unwrap();

        let log = load(&s.0, ID).unwrap();
        let wrote = log.versions.last().unwrap();
        assert_eq!(wrote.parents, [mine.version]);
        assert_ne!(wrote.parents, [theirs.version]);
    }
}
