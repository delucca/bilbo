use std::path::{Path, PathBuf};

use crate::program::{End, Ran};
use crate::report::{Line, Status};

pub const PLUGIN_ID: &str = "bilbo@bilbo";
pub const KEY_PREFIX: &str = "bilbo@bilbo:";
pub const PACKAGE: &str = "plugins/bilbo";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Marketplace {
    NotUsed,
    None,
    Here,
    Elsewhere(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stale {
    Version(String),
    InstalledFrom(String),
    ReadFrom(String),
    NoFolder,
    CopyDiffers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub on: bool,
    pub stale: Option<Stale>,
}

impl Installed {
    fn current(&self) -> bool {
        self.on && self.stale.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Other {
    pub id: String,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub marketplace: Marketplace,
    pub plugin: Option<Installed>,
    pub other: Option<Other>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Uninstall,
    Remove,
    Add,
    Install,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub status: Status,
    pub acts: Vec<Act>,
    pub why: Option<String>,
}

fn why_updated(seen: &Seen) -> String {
    match &seen.marketplace {
        Marketplace::Elsewhere(text) => return format!("its marketplace came from {text}"),
        Marketplace::None => return "its marketplace was gone".to_owned(),
        Marketplace::NotUsed | Marketplace::Here => {}
    }
    match &seen.plugin {
        Some(plugin) if !plugin.on => "it was turned off".to_owned(),
        Some(Installed {
            stale: Some(stale), ..
        }) => match stale {
            Stale::Version(version) => format!("it was version {version}"),
            Stale::InstalledFrom(path) => format!("it was installed from {path}"),
            Stale::ReadFrom(folder) => format!("it was read from {folder}"),
            Stale::NoFolder => "no folder it is read from was named".to_owned(),
            Stale::CopyDiffers => "its copy differed".to_owned(),
        },
        _ => String::new(),
    }
}

pub fn decide(seen: &Seen) -> Change {
    use Act::{Add, Install, Remove, Uninstall};
    let (status, acts) = match (&seen.marketplace, &seen.plugin) {
        (Marketplace::Here | Marketplace::NotUsed, Some(plugin)) if plugin.current() => {
            (Status::Kept, vec![])
        }
        (Marketplace::None, None) => (Status::Installed, vec![Add, Install]),
        (Marketplace::Here | Marketplace::NotUsed, None) => (Status::Installed, vec![Install]),
        (Marketplace::Here | Marketplace::NotUsed, Some(_)) => {
            (Status::Updated, vec![Uninstall, Install])
        }
        (Marketplace::None, Some(_)) => (Status::Updated, vec![Uninstall, Add, Install]),
        (Marketplace::Elsewhere(_), None) => (Status::Updated, vec![Remove, Add, Install]),
        (Marketplace::Elsewhere(_), Some(_)) => {
            (Status::Updated, vec![Uninstall, Remove, Add, Install])
        }
    };
    let why = (status == Status::Updated).then(|| why_updated(seen));
    Change { status, acts, why }
}

pub type Canon<'a> = dyn Fn(&Path) -> Option<PathBuf> + 'a;

pub struct Package {
    pub folder: PathBuf,
    pub plugin: PathBuf,
}

pub struct Trusting {
    pub step: &'static str,
    pub hooks: Box<dyn Hooks>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub plugin: bool,
    pub marketplace: bool,
    pub other: Option<Other>,
}

pub trait Agent {
    fn step(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn id(&self) -> &'static str;
    fn manifest(&self, folder: &Path) -> Result<(), String>;
    fn look(&self, folder: &Path) -> Result<Seen, String>;
    fn run(&self, act: Act, folder: &Path) -> Result<(), String>;
    fn words(&self, act: Act, folder: &Path) -> String;
    fn hooks(&self, folder: &Path) -> Option<Trusting>;
    fn held(&self) -> Result<Held, String>;
}

fn failed(agent: &dyn Agent, detail: String) -> Line {
    Line {
        step: agent.step(),
        status: Status::Failed,
        detail,
    }
}

fn other_sentence(agent: &dyn Agent, other: &Other) -> String {
    let scope = other
        .scope
        .as_ref()
        .map(|scope| format!(" ({scope} scope)"))
        .unwrap_or_default();
    format!(
        "{} has {}{scope} installed from the marketplace bilbo; bilbo setup changes no marketplace another installation uses",
        agent.name(),
        other.id
    )
}

fn what_is_listed(agent: &dyn Agent, seen: &Seen) -> Option<String> {
    match &seen.marketplace {
        Marketplace::None => return Some("no marketplace bilbo".to_owned()),
        Marketplace::Elsewhere(text) => {
            return Some(format!("the marketplace bilbo from {text}"));
        }
        Marketplace::Here | Marketplace::NotUsed => {}
    }
    let Some(plugin) = &seen.plugin else {
        return Some(format!("no plugin {}", agent.id()));
    };
    if !plugin.on {
        return Some("the plugin turned off".to_owned());
    }
    if let Some(stale) = &plugin.stale {
        return Some(match stale {
            Stale::Version(version) => format!("the plugin at version {version}"),
            Stale::InstalledFrom(path) => format!("the plugin installed from {path}"),
            Stale::ReadFrom(folder) => format!("the plugin read from {folder}"),
            Stale::NoFolder => "the plugin with no folder it is read from".to_owned(),
            Stale::CopyDiffers => "a copy of the plugin that differs".to_owned(),
        });
    }
    seen.other
        .as_ref()
        .map(|other| format!("{}, also from the marketplace bilbo", other.id))
}

pub fn glance(agent: &dyn Agent) -> Result<Held, Line> {
    let held = agent.held().map_err(|sentence| failed(agent, sentence))?;
    match &held.other {
        Some(other) => Err(failed(agent, other_sentence(agent, other))),
        None => Ok(held),
    }
}

fn listed_now(agent: &dyn Agent, held: &Held) -> String {
    let id = agent.id();
    match (held.plugin, held.marketplace) {
        (true, true) => format!("the plugin {id} and the marketplace bilbo"),
        (true, false) => format!("the plugin {id}"),
        _ => "the marketplace bilbo".to_owned(),
    }
}

pub fn removal(agent: &dyn Agent, held: &Held) -> Line {
    let row = |status, detail: String| Line {
        step: agent.step(),
        status,
        detail,
    };
    let mut acts = Vec::new();
    if held.plugin {
        acts.push(Act::Uninstall);
    }
    if held.marketplace {
        acts.push(Act::Remove);
    }
    let Some(last) = acts.last().copied() else {
        return row(Status::Absent, format!("no plugin {}", agent.id()));
    };
    let nowhere = Path::new("");
    for act in &acts {
        if let Err(sentence) = agent.run(*act, nowhere) {
            return failed(agent, sentence);
        }
    }
    match agent.held() {
        Err(sentence) => failed(agent, sentence),
        Ok(after) if after.plugin || after.marketplace => failed(
            agent,
            format!(
                "{} still lists {} after {}",
                agent.name(),
                listed_now(agent, &after),
                agent.words(last, nowhere)
            ),
        ),
        Ok(_) => row(
            Status::Removed,
            match (held.plugin, held.marketplace) {
                (true, true) => format!("{} and the marketplace bilbo", agent.id()),
                (true, false) => agent.id().to_owned(),
                _ => "the marketplace bilbo".to_owned(),
            },
        ),
    }
}

fn entries(n: usize) -> String {
    if n == 1 {
        "1 trust entry".to_owned()
    } else {
        format!("{n} trust entries")
    }
}

pub fn forget(step: &'static str, hooks: &mut dyn Hooks) -> Line {
    let line = |status, detail| Line {
        step,
        status,
        detail,
    };
    let outcome = forgotten(hooks);
    hooks.close();
    match outcome {
        Ok(0) => line(
            Status::Absent,
            "Codex keeps no trust of bilbo@bilbo".to_owned(),
        ),
        Ok(n) => line(Status::Removed, format!("{} of bilbo@bilbo", entries(n))),
        Err(sentence) => line(Status::Failed, sentence),
    }
}

fn forgotten(hooks: &mut dyn Hooks) -> Result<usize, String> {
    let keys = hooks.kept()?;
    if keys.is_empty() {
        return Ok(0);
    }
    hooks.forget(&keys)?;
    let left = hooks.kept()?.len();
    if left > 0 {
        return Err(format!(
            "Codex still keeps {} of bilbo@bilbo",
            entries(left)
        ));
    }
    Ok(keys.len())
}

pub fn step(agent: &dyn Agent, folder: &Path) -> Line {
    if let Err(sentence) = agent.manifest(folder) {
        return failed(agent, sentence);
    }
    let seen = match agent.look(folder) {
        Ok(seen) => seen,
        Err(sentence) => return failed(agent, sentence),
    };
    if let Some(other) = &seen.other {
        return failed(agent, other_sentence(agent, other));
    }
    let change = decide(&seen);
    let folder_text = folder.display();
    let head = format!("{} from {folder_text}", agent.id());
    let Some(last) = change.acts.last().copied() else {
        return Line {
            step: agent.step(),
            status: change.status,
            detail: head,
        };
    };
    for act in &change.acts {
        if let Err(sentence) = agent.run(*act, folder) {
            return failed(agent, sentence);
        }
    }
    let after = match agent.look(folder) {
        Ok(after) => after,
        Err(sentence) => return failed(agent, sentence),
    };
    if let Some(what) = what_is_listed(agent, &after) {
        return failed(
            agent,
            format!(
                "{} lists {what} after {}",
                agent.name(),
                agent.words(last, folder)
            ),
        );
    }
    let detail = match change.why {
        Some(why) => format!("{head}; {why}"),
        None => head,
    };
    Line {
        step: agent.step(),
        status: change.status,
        detail,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    Trusted,
    ToTrust,
    Changed,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    pub key: String,
    pub hash: String,
    pub standing: Standing,
}

pub trait Hooks {
    fn list(&mut self) -> Result<Vec<Hook>, String>;
    fn trust(&mut self, hooks: &[Hook]) -> Result<(), String>;
    fn kept(&mut self) -> Result<Vec<String>, String>;
    fn forget(&mut self, keys: &[String]) -> Result<(), String>;
    fn close(&mut self);
}

fn count(n: usize) -> String {
    if n == 1 {
        "1 hook".to_owned()
    } else {
        format!("{n} hooks")
    }
}

fn unknown_status(hooks: &[Hook]) -> Option<String> {
    hooks.iter().find_map(|hook| match &hook.standing {
        Standing::Unknown(status) => Some(format!(
            "Codex reports the trust status '{status}' for a hook of bilbo@bilbo"
        )),
        _ => None,
    })
}

fn trusted(hooks: &mut dyn Hooks, to_trust: &[Hook], listed: &[Hook]) -> Result<(), String> {
    hooks.trust(to_trust)?;
    let after = hooks.list()?;
    if let Some(sentence) = unknown_status(&after) {
        return Err(sentence);
    }
    let same = after.len() == listed.len()
        && after.iter().all(|later| {
            listed
                .iter()
                .any(|first| first.key == later.key && first.hash == later.hash)
        });
    if !same {
        return Err("Codex lists other hooks of bilbo@bilbo after the write".to_owned());
    }
    let left = after
        .iter()
        .filter(|hook| hook.standing != Standing::Trusted)
        .count();
    if left > 0 {
        return Err(format!(
            "Codex still lists {} of bilbo@bilbo as not trusted",
            count(left)
        ));
    }
    Ok(())
}

pub fn trust(step: &'static str, hooks: &mut dyn Hooks) -> Line {
    let line = |status, detail| Line {
        step,
        status,
        detail,
    };
    let outcome = trust_all(hooks);
    hooks.close();
    match outcome {
        Ok((status, listed)) => line(
            status,
            format!("Codex trusts {} of bilbo@bilbo", count(listed)),
        ),
        Err(sentence) => line(Status::Failed, sentence),
    }
}

fn trust_all(hooks: &mut dyn Hooks) -> Result<(Status, usize), String> {
    let listed = hooks.list()?;
    if let Some(sentence) = unknown_status(&listed) {
        return Err(sentence);
    }
    let to_trust: Vec<Hook> = listed
        .iter()
        .filter(|hook| matches!(hook.standing, Standing::ToTrust | Standing::Changed))
        .cloned()
        .collect();
    let status = if to_trust.is_empty() {
        Status::Kept
    } else if to_trust
        .iter()
        .any(|hook| hook.standing == Standing::Changed)
    {
        Status::Updated
    } else {
        Status::Installed
    };
    if !to_trust.is_empty() {
        trusted(hooks, &to_trust, &listed)?;
    }
    Ok((status, listed.len()))
}

pub fn plain_line(stderr: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(stderr);
    text.lines().map(plain).find(|line| !line.trim().is_empty())
}

pub fn first_line(stderr: &[u8]) -> Option<String> {
    plain_line(stderr).map(|line| {
        let line = line.trim();
        let line = line.strip_prefix('✘').map_or(line, str::trim_start);
        line.strip_prefix("Error: ").unwrap_or(line).to_owned()
    })
}

fn plain(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn ended_badly(ran: &Ran, program: &Path, waited: &str) -> Option<String> {
    match &ran.end {
        End::Exited(0) => None,
        End::Exited(code) => Some(match plain_line(&ran.stderr) {
            Some(line) => format!("it exited with {code}: {line}"),
            None => format!("it exited with {code}"),
        }),
        End::NoExitCode => Some("a signal ended it".to_owned()),
        End::StillRunning => Some(format!("it did not end within {waited}")),
        End::OutputOpen => Some(format!("it left its output open past {waited}")),
        End::Unreadable(error) => Some(format!("its output could not be read: {error}")),
        End::NotStarted(error) => Some(format!("cannot start {}: {error}", program.display())),
    }
}

pub fn message(ran: &Ran, own: Option<String>, program: &Path, waited: &str) -> String {
    match &ran.end {
        End::Exited(code) => own
            .or_else(|| first_line(&ran.stderr))
            .unwrap_or_else(|| format!("exit {code}")),
        End::NoExitCode => own
            .or_else(|| first_line(&ran.stderr))
            .unwrap_or_else(|| "a signal ended it".to_owned()),
        _ => ended_badly(ran, program, waited).unwrap_or_default(),
    }
}

pub fn unreadable(words: &str, why: &str) -> String {
    format!("cannot read {words}: {why}")
}

pub const NOT_JSON: &str = "its output is not the JSON bilbo reads";

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::Path;

    use super::{
        Act, Hook, Hooks, Installed, Marketplace, Seen, Stale, Standing, Status, decide,
        ended_badly, first_line, forget, message, trust,
    };
    use crate::program::{End, Ran};

    fn on() -> Option<Installed> {
        Some(Installed {
            on: true,
            stale: None,
        })
    }

    fn off() -> Option<Installed> {
        Some(Installed {
            on: false,
            stale: None,
        })
    }

    fn old() -> Option<Installed> {
        Some(Installed {
            on: true,
            stale: Some(Stale::Version("0.1".to_owned())),
        })
    }

    fn seen(marketplace: Marketplace, plugin: Option<Installed>) -> Seen {
        Seen {
            marketplace,
            plugin,
            other: None,
        }
    }

    #[test]
    fn the_change_follows_what_the_tool_lists() {
        use Act::{Add, Install, Remove, Uninstall};
        let elsewhere = || Marketplace::Elsewhere("/old".to_owned());
        let rows: Vec<(Seen, Status, Vec<Act>)> = vec![
            (seen(Marketplace::Here, on()), Status::Kept, vec![]),
            (seen(Marketplace::NotUsed, on()), Status::Kept, vec![]),
            (
                seen(Marketplace::None, None),
                Status::Installed,
                vec![Add, Install],
            ),
            (
                seen(Marketplace::Here, None),
                Status::Installed,
                vec![Install],
            ),
            (
                seen(Marketplace::NotUsed, None),
                Status::Installed,
                vec![Install],
            ),
            (
                seen(Marketplace::Here, off()),
                Status::Updated,
                vec![Uninstall, Install],
            ),
            (
                seen(Marketplace::Here, old()),
                Status::Updated,
                vec![Uninstall, Install],
            ),
            (
                seen(Marketplace::NotUsed, off()),
                Status::Updated,
                vec![Uninstall, Install],
            ),
            (
                seen(Marketplace::NotUsed, old()),
                Status::Updated,
                vec![Uninstall, Install],
            ),
            (
                seen(Marketplace::None, on()),
                Status::Updated,
                vec![Uninstall, Add, Install],
            ),
            (
                seen(Marketplace::None, off()),
                Status::Updated,
                vec![Uninstall, Add, Install],
            ),
            (
                seen(elsewhere(), None),
                Status::Updated,
                vec![Remove, Add, Install],
            ),
            (
                seen(elsewhere(), on()),
                Status::Updated,
                vec![Uninstall, Remove, Add, Install],
            ),
            (
                seen(elsewhere(), off()),
                Status::Updated,
                vec![Uninstall, Remove, Add, Install],
            ),
        ];
        for (seen, status, acts) in rows {
            let change = decide(&seen);
            assert_eq!(
                (change.status, change.acts.clone()),
                (status, acts),
                "{seen:?}"
            );
            let mentions = |act: Act| change.acts.contains(&act);
            if seen.marketplace == Marketplace::NotUsed || seen.marketplace == Marketplace::Here {
                assert!(!mentions(Add) && !mentions(Remove), "{seen:?}");
            }
        }
        let why = |seen: Seen| decide(&seen).why.expect("a reason");
        assert_eq!(
            why(seen(Marketplace::Elsewhere("/old".to_owned()), None)),
            "its marketplace came from /old"
        );
        assert_eq!(
            why(seen(Marketplace::None, off())),
            "its marketplace was gone"
        );
        assert_eq!(why(seen(Marketplace::Here, off())), "it was turned off");
        assert_eq!(why(seen(Marketplace::Here, old())), "it was version 0.1");
    }

    struct Double {
        lists: RefCell<Vec<Result<Vec<Hook>, String>>>,
        written: RefCell<Vec<Vec<Hook>>>,
        closed: RefCell<u32>,
        kept: RefCell<Vec<Result<Vec<String>, String>>>,
        forgotten: RefCell<Vec<Vec<String>>>,
        forget_fails: bool,
    }

    impl Double {
        fn new(lists: Vec<Result<Vec<Hook>, String>>) -> Double {
            Double {
                lists: RefCell::new(lists),
                written: RefCell::new(Vec::new()),
                closed: RefCell::new(0),
                kept: RefCell::new(Vec::new()),
                forgotten: RefCell::new(Vec::new()),
                forget_fails: false,
            }
        }
    }

    struct Over<'a>(&'a Double);

    impl Hooks for Over<'_> {
        fn list(&mut self) -> Result<Vec<Hook>, String> {
            self.0.lists.borrow_mut().remove(0)
        }

        fn trust(&mut self, hooks: &[Hook]) -> Result<(), String> {
            self.0.written.borrow_mut().push(hooks.to_vec());
            Ok(())
        }

        fn kept(&mut self) -> Result<Vec<String>, String> {
            self.0.kept.borrow_mut().remove(0)
        }

        fn forget(&mut self, keys: &[String]) -> Result<(), String> {
            self.0.forgotten.borrow_mut().push(keys.to_vec());
            if self.0.forget_fails {
                Err("config/batchWrite failed".to_owned())
            } else {
                Ok(())
            }
        }

        fn close(&mut self) {
            *self.0.closed.borrow_mut() += 1;
        }
    }

    fn hook(key: &str, hash: &str, standing: Standing) -> Hook {
        Hook {
            key: key.to_owned(),
            hash: hash.to_owned(),
            standing,
        }
    }

    fn run(double: &Double) -> (Status, String) {
        let line = trust("codex-hooks", &mut Over(double));
        assert_eq!(*double.closed.borrow(), 1, "the program is closed once");
        (line.status, line.detail)
    }

    #[test]
    fn a_hook_is_trusted_by_its_standing() {
        let all = vec![
            hook("a", "1", Standing::Trusted),
            hook("b", "2", Standing::Trusted),
        ];
        let double = Double::new(vec![Ok(all)]);
        assert_eq!(
            run(&double),
            (
                Status::Kept,
                "Codex trusts 2 hooks of bilbo@bilbo".to_owned()
            )
        );
        assert!(
            double.written.borrow().is_empty(),
            "nothing is written when all are trusted"
        );

        let first = vec![
            hook("a", "1", Standing::ToTrust),
            hook("b", "2", Standing::Trusted),
        ];
        let after = vec![
            hook("a", "1", Standing::Trusted),
            hook("b", "2", Standing::Trusted),
        ];
        let double = Double::new(vec![Ok(first), Ok(after)]);
        assert_eq!(
            run(&double),
            (
                Status::Installed,
                "Codex trusts 2 hooks of bilbo@bilbo".to_owned()
            )
        );
        assert_eq!(
            double.written.borrow().clone(),
            vec![vec![hook("a", "1", Standing::ToTrust)]]
        );

        let first = vec![hook("a", "1", Standing::Changed)];
        let after = vec![hook("a", "1", Standing::Trusted)];
        let double = Double::new(vec![Ok(first), Ok(after)]);
        assert_eq!(
            run(&double),
            (
                Status::Updated,
                "Codex trusts 1 hook of bilbo@bilbo".to_owned()
            )
        );

        let first = vec![hook("a", "1", Standing::Unknown("blocked".to_owned()))];
        let double = Double::new(vec![Ok(first)]);
        assert_eq!(
            run(&double),
            (
                Status::Failed,
                "Codex reports the trust status 'blocked' for a hook of bilbo@bilbo".to_owned()
            )
        );
        assert!(double.written.borrow().is_empty());
    }

    #[test]
    fn a_failed_lists_reason_keeps_its_line_as_the_tool_wrote_it() {
        let program = Path::new("/p/tool");
        let why = |ran: &Ran| ended_badly(ran, program, "30s");
        assert_eq!(
            why(&ran(
                End::Exited(1),
                "\n  \n\u{1b}[31mError: broken\u{1b}[0m\n"
            )),
            Some("it exited with 1: Error: broken".to_owned())
        );
        assert_eq!(
            why(&ran(End::Exited(1), "✘ gone\n")),
            Some("it exited with 1: ✘ gone".to_owned())
        );
        assert_eq!(
            why(&ran(End::Exited(1), "  Error: broken  \n")),
            Some("it exited with 1:   Error: broken  ".to_owned())
        );
        assert_eq!(
            why(&ran(End::Exited(2), "  \n")),
            Some("it exited with 2".to_owned())
        );
        assert_eq!(
            why(&ran(End::NoExitCode, "a line\n")),
            Some("a signal ended it".to_owned())
        );
    }

    #[test]
    fn the_hooks_are_listed_again_after_the_write() {
        let first = || {
            vec![
                hook("a", "1", Standing::ToTrust),
                hook("b", "2", Standing::ToTrust),
            ]
        };
        let still = vec![
            hook("a", "1", Standing::Trusted),
            hook("b", "2", Standing::ToTrust),
        ];
        let both_still = vec![
            hook("a", "1", Standing::ToTrust),
            hook("b", "2", Standing::ToTrust),
        ];
        let other_hash = vec![
            hook("a", "1", Standing::Trusted),
            hook("b", "9", Standing::Trusted),
        ];
        let fewer = vec![hook("a", "1", Standing::Trusted)];
        let reordered = vec![
            hook("b", "2", Standing::Trusted),
            hook("a", "1", Standing::Trusted),
        ];
        let blocked = vec![
            hook("a", "1", Standing::Trusted),
            hook("b", "2", Standing::Unknown("blocked".to_owned())),
        ];
        let double = Double::new(vec![Ok(first()), Ok(reordered)]);
        assert_eq!(
            run(&double),
            (
                Status::Installed,
                "Codex trusts 2 hooks of bilbo@bilbo".to_owned()
            )
        );
        for (after, sentence) in [
            (
                Ok(blocked),
                "Codex reports the trust status 'blocked' for a hook of bilbo@bilbo",
            ),
            (
                Ok(still),
                "Codex still lists 1 hook of bilbo@bilbo as not trusted",
            ),
            (
                Ok(both_still),
                "Codex still lists 2 hooks of bilbo@bilbo as not trusted",
            ),
            (
                Ok(other_hash),
                "Codex lists other hooks of bilbo@bilbo after the write",
            ),
            (
                Ok(fewer),
                "Codex lists other hooks of bilbo@bilbo after the write",
            ),
            (
                Err("Codex lists no hook of bilbo@bilbo for SessionStart".to_owned()),
                "Codex lists no hook of bilbo@bilbo for SessionStart",
            ),
        ] {
            let double = Double::new(vec![Ok(first()), after]);
            assert_eq!(run(&double), (Status::Failed, sentence.to_owned()));
        }
    }

    fn ran(end: End, stderr: &str) -> Ran {
        Ran {
            end,
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn a_failed_commands_reason_is_its_message_then_its_first_line() {
        let program = Path::new("/p/tool");
        let said =
            |ran: &Ran, own: Option<&str>| message(ran, own.map(str::to_owned), program, "30s");
        let two = ran(
            End::Exited(1),
            "\n  \n\u{1b}[31m✘ first line\u{1b}[0m\nsecond\n",
        );
        assert_eq!(said(&two, None), "first line");
        assert_eq!(said(&two, Some("its own")), "its own");
        assert_eq!(
            said(
                &ran(End::Exited(1), "Error: it broke\nstack\nstack\n"),
                None
            ),
            "it broke"
        );
        assert_eq!(said(&ran(End::Exited(1), "  ✘ gone  \n"), None), "gone");
        assert_eq!(said(&ran(End::Exited(2), "  \n"), None), "exit 2");
        assert_eq!(said(&ran(End::NoExitCode, "x"), Some("own")), "own");
        assert_eq!(said(&ran(End::NoExitCode, "\n✘ line\n"), None), "line");
        assert_eq!(said(&ran(End::NoExitCode, ""), None), "a signal ended it");
        assert_eq!(
            said(&ran(End::StillRunning, "line"), Some("own")),
            "it did not end within 30s"
        );
        assert_eq!(
            said(&ran(End::StillRunning, ""), None),
            "it did not end within 30s"
        );
        assert_eq!(
            said(&ran(End::OutputOpen, ""), None),
            "it left its output open past 30s"
        );
        assert_eq!(
            first_line(b"Error: Error: twice\n"),
            Some("Error: twice".to_owned())
        );
        assert_eq!(first_line(b"\n\n"), None);
    }

    fn keys(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn forgetting(double: &Double) -> (Status, String) {
        let line = forget("codex-hooks", &mut Over(double));
        assert_eq!(*double.closed.borrow(), 1, "the program is closed once");
        (line.status, line.detail)
    }

    #[test]
    fn a_trust_removal_is_believed_after_a_second_read() {
        let double = Double::new(Vec::new());
        *double.kept.borrow_mut() = vec![Ok(Vec::new())];
        assert_eq!(
            forgetting(&double),
            (
                Status::Absent,
                "Codex keeps no trust of bilbo@bilbo".to_owned()
            )
        );
        assert!(double.forgotten.borrow().is_empty());

        let double = Double::new(Vec::new());
        *double.kept.borrow_mut() = vec![Ok(keys(&["a", "b"])), Ok(Vec::new())];
        assert_eq!(
            forgetting(&double),
            (Status::Removed, "2 trust entries of bilbo@bilbo".to_owned())
        );
        assert_eq!(double.forgotten.borrow().clone(), vec![keys(&["a", "b"])]);

        let double = Double::new(Vec::new());
        *double.kept.borrow_mut() = vec![Ok(keys(&["a"])), Ok(Vec::new())];
        assert_eq!(forgetting(&double).1, "1 trust entry of bilbo@bilbo");

        let double = Double::new(Vec::new());
        *double.kept.borrow_mut() = vec![Ok(keys(&["a", "b"])), Ok(keys(&["b"]))];
        assert_eq!(
            forgetting(&double),
            (
                Status::Failed,
                "Codex still keeps 1 trust entry of bilbo@bilbo".to_owned()
            )
        );

        let mut double = Double::new(Vec::new());
        double.forget_fails = true;
        *double.kept.borrow_mut() = vec![Ok(keys(&["a"]))];
        assert_eq!(
            forgetting(&double),
            (Status::Failed, "config/batchWrite failed".to_owned())
        );

        let double = Double::new(Vec::new());
        *double.kept.borrow_mut() = vec![Err("it broke".to_owned())];
        assert_eq!(forgetting(&double), (Status::Failed, "it broke".to_owned()));
        assert!(double.forgotten.borrow().is_empty());
    }
}
