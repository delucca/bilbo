//! Setup's sync step, and what the wizard prepares for it: the folder, the keys and the scope to join.

use std::collections::BTreeSet;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

use super::facts::Facts;
use crate::host::swap;
use crate::identity::keys::{self, Device, Identity, Owner};
use crate::identity::manifest::{self, Context, Known, Outcome, Recipient};
use crate::search::documents;
use crate::sync::remote::{self, Probe};
use crate::sync::scopes;
use crate::sync::transport::{self, Keys};

/// The step's detail when the device has no key.
pub const NO_KEY: &str = "no device key; run bilbo device init in a terminal";

/// Why no scope is created on a folder that holds scopes of this owner the device cannot open.
pub const BLOCKED: &str = "the folder holds scopes of this owner that this device cannot open; run bilbo device recover on this device";

/// Why no scope is created beside a manifest of this owner that has never listed the device.
pub const UNSEALED: &str =
    "a manifest of this owner in the store has never listed this device; run bilbo device recover";

/// What the wizard decided about sync.
#[derive(Default)]
pub enum Turn {
    /// Nothing was asked or turned on: the config's own scopes stand.
    #[default]
    Unchanged,
    /// Sync stays off for this run, for this reason; `scope` is the one picked, when it was.
    Off {
        scope: Option<String>,
        why: String,
    },
    /// The device keys cannot be read: the step fails with this message.
    Failed(String),
    On(Box<Turned>),
}

/// A scope the wizard turns sync on for.
pub struct Turned {
    pub name: String,
    /// `file://<folder>`, or a relay's URL.
    pub url: String,
    /// The folder of a `file://` URL.
    pub folder: Option<PathBuf>,
    pub enrol: Enrol,
    /// The id of the folder's scope of this name to copy into the store.
    pub take: Option<String>,
    /// A new scope is created: the folder and the store hold none of this name.
    pub mint: bool,
}

/// The keys the device gets, held until the summary is confirmed.
pub enum Enrol {
    /// A new owner, whose phrase was shown and confirmed.
    New {
        entropy: Zeroizing<[u8; 16]>,
        device: Device,
    },
    /// An owner whose phrase was typed in.
    Phrase {
        entropy: Zeroizing<[u8; 16]>,
        device: Device,
    },
    /// The device already has keys.
    Held,
}

/// The step's inputs: the scopes that sync once setup is done, and where the keys are.
#[derive(Default)]
pub struct SyncPlan {
    pub choice: Turn,
    /// The scopes that sync, with their URLs: the file's, then the wizard's.
    pub scopes: Vec<(String, String)>,
    /// The watcher is wanted.
    pub watch: bool,
    pub keys: Option<PathBuf>,
    pub root: PathBuf,
    /// No agent marker is set.
    pub terminal: bool,
}

pub fn plan(facts: &Facts, watch: bool, choice: Turn) -> SyncPlan {
    let mut scopes: Vec<(String, String)> = facts
        .scopes
        .iter()
        .filter(|(_, sync)| sync != "off")
        .cloned()
        .collect();
    if let Turn::On(turned) = &choice {
        scopes.retain(|(name, _)| *name != turned.name);
        scopes.push((turned.name.clone(), turned.url.clone()));
        scopes.sort();
    }
    SyncPlan {
        choice,
        scopes,
        watch,
        keys: facts.keys.clone(),
        root: facts.root.clone(),
        terminal: !facts.agent,
    }
}

/// `scope.<name>.sync = <url>` among the kept lines: the old line's place, or the end.
pub fn put_line(kept: &mut Vec<(String, String)>, name: &str, url: &str) {
    let key = format!("scope.{name}.sync");
    match kept.iter_mut().find(|(k, _)| *k == key) {
        Some(line) => line.1 = url.to_string(),
        None => kept.push((key, url.to_string())),
    }
}

/// The step's status and detail. A scope the wizard turned on is joined first, which is the only place setup
/// writes for sync; the checks then only read. A scope the wizard turned off is named after the others' line.
pub fn step(plan: &SyncPlan, notes: &Path) -> (&'static str, String) {
    if let Turn::Failed(why) = &plan.choice {
        return ("failed", why.clone());
    }
    if let Turn::On(turned) = &plan.choice
        && let Err(why) = join(plan, turned)
    {
        return ("failed", why);
    }
    let (status, mut detail) = checks(plan, notes);
    if let Turn::Off { scope, why } = &plan.choice {
        if plan.scopes.is_empty() {
            return ("skipped", why.clone());
        }
        let name = scope.as_deref().map_or(String::new(), |s| format!("{s} "));
        detail.push_str(&format!("; {name}skipped: {why}"));
    }
    (status, detail)
}

fn checks(plan: &SyncPlan, notes: &Path) -> (&'static str, String) {
    if plan.scopes.is_empty() {
        return ("skipped", "no scope syncs".into());
    }
    let id = match plan.keys.as_deref().map(keys::read_identity) {
        Some(Ok(Some(id))) => id,
        Some(Err(why)) => return ("failed", why),
        _ => return ("skipped", NO_KEY.into()),
    };
    if !plan.watch {
        return ("failed", "sync needs the watcher; drop --no-watch".into());
    }
    let stored = documents::read_notes(notes).unwrap_or_default();
    let mut lines = Vec::new();
    let mut failures = Vec::new();
    for (name, url) in &plan.scopes {
        match reach(&id, url) {
            Ok(()) => {
                let count = stored
                    .iter()
                    .filter(|note| note.scope.as_deref() == Some(name))
                    .count();
                let unit = if count == 1 { "note" } else { "notes" };
                lines.push(format!("{name} through {url} ({count} {unit})"));
            }
            Err(why) => failures.push(why),
        }
    }
    if failures.is_empty() {
        ("ok", lines.join(", "))
    } else {
        ("failed", failures.join("; "))
    }
}

/// Whether `url` has a client, and its folder exists and takes writes, or its relay answers as one. Nothing is
/// created and no request is signed.
fn reach(id: &Identity, url: &str) -> Result<(), String> {
    if transport::is_relay_url(url) {
        return remote::identify(url).map_err(|probe| match probe {
            Probe::NotRelay => format!("{url} is not a bilbo relay"),
            Probe::Unreachable(why) => format!("{url} is not reachable: {why}"),
        });
    }
    let transport = transport::open(url, &Keys::of(id))?;
    let unreachable = |why: &str| format!("{url} is not reachable: {why}");
    transport.reachable().map_err(|why| unreachable(&why))?;
    match url.strip_prefix("file://") {
        Some(path) if !swap::writable(Path::new(path)) => {
            Err(unreachable("the folder is not writable"))
        }
        _ => Ok(()),
    }
}

/// What the folder holds for a scope of one name, as the keys in hand read it.
#[derive(Default)]
pub struct Inspected {
    /// The scope of that name to copy in.
    pub take: Option<String>,
    /// The stderr line when several scopes of that name are there.
    pub rivals: Option<String>,
    /// Why no scope may be created there.
    pub blocked: Option<String>,
}

/// Lists the owner's scopes on `url`, for a folder that exists and answers; a folder that does not says nothing.
pub fn inspect(url: &str, keys: &Keys, owner: &[u8; 32], who: &Recipient, name: &str) -> Inspected {
    let mut seen = Inspected::default();
    let Ok(transport) = transport::open(url, keys) else {
        return seen;
    };
    if transport.reachable().is_err() {
        return seen;
    }
    let listing = match scopes::list(&*transport, owner, who) {
        Ok(listing) => listing,
        Err(why) => {
            seen.blocked = Some(why);
            return seen;
        }
    };
    if let Some(pick) = scopes::pick(&listing.found, name) {
        seen.take = Some(pick.chosen.id.clone());
        if !pick.rivals.is_empty() {
            let others: Vec<&str> = pick.rivals.iter().map(|f| f.id.as_str()).collect();
            seen.rivals = Some(format!(
                "scope {name}: {url} holds {} scopes named {name}; took {} ({} devices), not {}",
                pick.rivals.len() + 1,
                pick.chosen.id,
                pick.chosen.devices,
                others.join(", ")
            ));
        }
    } else if listing.outsider() {
        seen.blocked = Some(BLOCKED.into());
    } else if let Some((id, why)) = listing.unattributed.first() {
        seen.blocked = Some(format!(
            "{url} holds scope {id} that does not verify: {why}"
        ));
    }
    seen
}

/// The fingerprints of the owners whose manifests the store holds.
pub fn owners_of(known: &[Known]) -> BTreeSet<String> {
    known
        .iter()
        .filter_map(|k| k.scope.owner())
        .map(|owner| keys::owner_fingerprint(&owner))
        .collect()
}

/// The name of `k` as the owner reads it.
pub fn named_by(k: &Known, owner: &Owner) -> Option<String> {
    let read = manifest::open(&k.scope, &Recipient::Owner(&owner.box_secret));
    read.ok().flatten().map(|opened| opened.name)
}

/// Whether the store holds a manifest of this device's owner for a scope called `name`.
pub fn holds(known: &[Known], owner: Option<&Owner>, name: &str) -> bool {
    known.iter().any(|k| {
        k.mine
            && (k.name() == Some(name)
                || k.last_name.as_deref() == Some(name)
                || owner.is_some_and(|o| named_by(k, o).as_deref() == Some(name)))
    })
}

/// Whether every version of `k` is pending: a scope made here that no transport has seen.
pub fn all_pending(k: &Known) -> bool {
    let last = k.scope.versions.len() as u64;
    last > 0 && (1..=last).all(|n| k.scope.pending.contains(&n))
}

/// Whether `init_step` would refuse to create `name`: no manifest is named like it, and a manifest of the owner has
/// never listed this device or is invalid before any version it can read.
pub fn unsealed(known: &[Known], name: &str) -> bool {
    !known
        .iter()
        .any(|k| k.mine && k.last_name.as_deref() == Some(name))
        && known
            .iter()
            .any(|k| k.mine && (!k.ever_listed || (k.problem.is_some() && k.last_name.is_none())))
}

/// Creates the folder's last component when it is missing.
fn make_folder(folder: &Path) -> Result<(), String> {
    if folder.exists() {
        return Ok(());
    }
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(folder)
        .map_err(|e| format!("cannot create {}: {e}", folder.display()))
}

/// Writes what the wizard prepared, after the summary: the folder, the keys, the picked scope's manifest and the
/// scope's membership, as `bilbo device init` or `recover` writes them.
fn join(plan: &SyncPlan, turned: &Turned) -> Result<(), String> {
    let keys_dir = plan
        .keys
        .as_deref()
        .ok_or("no state folder: set XDG_STATE_HOME or HOME")?;
    if let Some(folder) = &turned.folder {
        make_folder(folder)?;
    }
    let owner = match &turned.enrol {
        Enrol::New { entropy, device } | Enrol::Phrase { entropy, device } => {
            let owner = Owner::derive(entropy);
            keys::write_identity(keys_dir, &owner.file(), device)?;
            Some(owner)
        }
        Enrol::Held => None,
    };
    let id = keys::read_identity(keys_dir)?.ok_or("the device keys are gone")?;
    let lock = manifest::lock(&plan.root)?;
    if let Some(scope) = &turned.take {
        if let Some(owner) = &owner {
            let mine = manifest::survey(
                &plan.root,
                Some(&id.owner.sign.public()),
                Some(&Recipient::Owner(&owner.box_secret)),
            )?;
            for k in mine.iter().filter(|k| {
                k.mine
                    && k.scope.id != *scope
                    && all_pending(k)
                    && named_by(k, owner).as_deref() == Some(&turned.name)
            }) {
                manifest::set_aside(&lock, &k.scope.id)?;
            }
        }
        copy(&lock, turned, &id, owner.as_ref(), scope)?;
    }
    let public = id.owner.sign.public();
    let known = manifest::survey(
        &plan.root,
        Some(&public),
        Some(&Recipient::device(&id.device)),
    )?;
    let named: Vec<&Known> = match &owner {
        Some(owner) => known
            .iter()
            .filter(|k| k.mine && named_by(k, owner).as_deref() == Some(&turned.name))
            .collect(),
        None => Vec::new(),
    };
    let (Some(owner), false) = (&owner, named.is_empty()) else {
        return create(plan, turned, &lock, &id, &known);
    };
    for k in named {
        if let Outcome::Failed(why) = manifest::recover_step(&lock, &id, &owner.box_secret, k) {
            return Err(why);
        }
    }
    Ok(())
}

/// `init`'s step for the picked scope.
fn create(
    plan: &SyncPlan,
    turned: &Turned,
    lock: &manifest::Lock,
    id: &Identity,
    known: &[Known],
) -> Result<(), String> {
    let public = id.owner.sign.public();
    let fresh = matches!(turned.enrol, Enrol::New { .. }) && transport::is_relay_url(&turned.url);
    let blocked = if fresh {
        None
    } else {
        inspect(
            &turned.url,
            &Keys::of(id),
            &public,
            &Recipient::device(&id.device),
            &turned.name,
        )
        .blocked
    };
    let ctx = Context {
        terminal: plan.terminal,
        blocked,
    };
    let others = manifest::owner_devices(known);
    match manifest::init_step(lock, id, &turned.name, &turned.url, known, &others, ctx) {
        Outcome::Created(_) | Outcome::Updated(_) | Outcome::Kept => Ok(()),
        Outcome::Unsealed => Err(UNSEALED.into()),
        Outcome::Failed(why) => Err(why),
    }
}

/// Copies the chain of the folder's scope `scope` into the store, and no other scope.
fn copy(
    lock: &manifest::Lock,
    turned: &Turned,
    id: &Identity,
    owner: Option<&Owner>,
    scope: &str,
) -> Result<(), String> {
    let keys = Keys {
        device: &id.device,
        owner: Some(owner.map_or(&id.owner.sign, |o| &o.sign)),
        opener: false,
    };
    let transport = transport::open(&turned.url, &keys)?;
    let public = id.owner.sign.public();
    let who = match owner {
        Some(owner) => Recipient::Owner(&owner.box_secret),
        None => Recipient::device(&id.device),
    };
    let listing = scopes::list(&*transport, &public, &who)?;
    let found = listing
        .found
        .iter()
        .find(|f| f.id == scope)
        .ok_or_else(|| format!("{} no longer holds the scope {scope}", turned.url))?;
    found
        .scope
        .versions
        .iter()
        .try_for_each(|v| manifest::adopt(lock, &found.id, v.manifest.n, &v.bytes))
}

#[cfg(test)]
mod tests {
    use super::super::fakes::*;
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/device");

    /// A plan whose keys are the fixture `rhosgobel` in a folder of `dir`.
    fn keyed(dir: &Path, scopes: &[(&str, String)], watch: bool) -> SyncPlan {
        let keys = dir.join("keys");
        std::fs::create_dir_all(&keys).unwrap();
        std::fs::set_permissions(&keys, std::fs::Permissions::from_mode(0o700)).unwrap();
        for file in ["owner.key", "device.key"] {
            let target = keys.join(file);
            std::fs::copy(Path::new(FIXTURES).join("rhosgobel").join(file), &target).unwrap();
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        SyncPlan {
            choice: Turn::Unchanged,
            scopes: scopes
                .iter()
                .map(|(name, url)| (name.to_string(), url.clone()))
                .collect(),
            watch,
            keys: Some(keys),
            root: dir.join("root"),
            terminal: true,
        }
    }

    #[test]
    fn the_wizards_line_replaces_the_old_one_in_place_or_goes_last() {
        let mut kept = vec![
            ("scope.personal.sync".to_string(), "off".to_string()),
            ("scope.work.paths".to_string(), "/w".to_string()),
        ];
        put_line(&mut kept, "personal", "file:///a");
        put_line(&mut kept, "home", "file:///h");
        assert_eq!(
            kept,
            [
                ("scope.personal.sync".to_string(), "file:///a".to_string()),
                ("scope.work.paths".to_string(), "/w".to_string()),
                ("scope.home.sync".to_string(), "file:///h".to_string()),
            ]
        );
    }

    #[test]
    fn the_plan_keeps_the_syncing_scopes_and_the_wizards_pick_wins() {
        let dir = scratch("sync-plan");
        let mut facts = seen(&dir, None, super::super::facts::ConfigState::Present);
        facts.scopes = vec![
            ("work".into(), "off".into()),
            ("personal".into(), "file:///old".into()),
            ("shared".into(), "file:///s".into()),
        ];
        let plain = super::plan(&facts, true, Turn::Unchanged);
        assert_eq!(
            plain.scopes,
            [
                ("personal".to_string(), "file:///old".to_string()),
                ("shared".to_string(), "file:///s".to_string())
            ]
        );
        let turned = Turned {
            name: "personal".into(),
            url: "file:///new".into(),
            folder: Some("/new".into()),
            enrol: Enrol::Held,
            take: None,
            mint: true,
        };
        let picked = super::plan(&facts, true, Turn::On(Box::new(turned)));
        assert_eq!(
            picked.scopes[0],
            ("personal".to_string(), "file:///new".to_string())
        );
        assert_eq!(picked.scopes.len(), 2);
        facts.agent = true;
        assert!(!super::plan(&facts, true, Turn::Unchanged).terminal);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_syncing_is_skipped_with_the_wizards_reason_when_it_has_one() {
        let notes = Path::new("/nowhere");
        let mut plain = SyncPlan::default();
        assert_eq!(step(&plain, notes), ("skipped", "no scope syncs".into()));
        plain.choice = Turn::Off {
            scope: None,
            why: "because".into(),
        };
        assert_eq!(step(&plain, notes), ("skipped", "because".into()));
    }

    #[test]
    fn a_syncing_scope_without_keys_is_skipped_before_the_watcher_is_checked() {
        let syncing = SyncPlan {
            scopes: vec![("personal".into(), "file:///srv".into())],
            watch: false,
            ..SyncPlan::default()
        };
        assert_eq!(step(&syncing, Path::new("/n")), ("skipped", NO_KEY.into()));
    }

    #[test]
    fn the_watcher_the_scheme_and_the_folder_each_fail_the_step() {
        let dir = scratch("sync-step");
        let folder = dir.join("shared");
        std::fs::create_dir_all(&folder).unwrap();
        let url = format!("file://{}", folder.display());
        let notes = dir.join("notes");
        let no_watch = keyed(&dir, &[("personal", url.clone())], false);
        assert_eq!(
            step(&no_watch, &notes),
            ("failed", "sync needs the watcher; drop --no-watch".into())
        );
        let missing = format!("file://{}", dir.join("gone").display());
        let gone = keyed(&dir, &[("personal", missing.clone())], true);
        let (status, detail) = step(&gone, &notes);
        assert_eq!(status, "failed");
        assert!(
            detail.starts_with(&format!("{missing} is not reachable: ")),
            "{detail}"
        );
        assert!(!dir.join("gone").exists());
        let both = keyed(
            &dir,
            &[("personal", missing.clone()), ("work", url.clone())],
            true,
        );
        let (status, detail) = step(&both, &notes);
        assert_eq!(status, "failed");
        assert!(!detail.contains("work"), "{detail}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_relay_that_answers_as_one_is_ok() {
        let dir = scratch("sync-relay-ok");
        let relay = relay("sync-relay-ok-data", &[], 0);
        let url = relay.url();
        let syncing = keyed(&dir, &[("personal", url.clone())], true);
        assert_eq!(
            step(&syncing, &dir.join("notes")),
            ("ok", format!("personal through {url} (0 notes)"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_url_that_answers_otherwise_is_not_a_relay() {
        let dir = scratch("sync-relay-not");
        let other = Answering::start(404);
        let url = other.url();
        let syncing = keyed(&dir, &[("personal", url.clone())], true);
        assert_eq!(
            step(&syncing, &dir.join("notes")),
            ("failed", format!("{url} is not a bilbo relay"))
        );
        let heads = other.heads();
        assert_eq!(heads.len(), 1, "{heads:?}");
        assert!(heads[0].starts_with("GET /v1/ "), "{heads:?}");
        assert!(!heads[0].to_lowercase().contains("bilbo-"), "{heads:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_relay_that_is_down_is_not_reachable() {
        let dir = scratch("sync-relay-down");
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let url = format!("http://127.0.0.1:{port}");
        let syncing = keyed(&dir, &[("personal", url.clone())], true);
        let (status, detail) = step(&syncing, &dir.join("notes"));
        assert_eq!(status, "failed");
        assert!(
            detail.starts_with(&format!("{url} is not reachable: ")),
            "{detail}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_count_is_of_the_notes_whose_scope_key_names_the_scope() {
        let dir = scratch("sync-count");
        let folder = dir.join("shared");
        std::fs::create_dir_all(&folder).unwrap();
        let notes = dir.join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        for (file, scope) in [
            ("a", "scope: p\n"),
            ("b", "scope: p\n"),
            ("c", "scope: q\n"),
            ("d", ""),
        ] {
            std::fs::write(
                notes.join(format!("plan-{file}.md")),
                format!("---\nid: 01M3YJ7R6HK6NQ30DCDB1P4DYB\ncreated: 2026-10-02T14:23-03:00\n{scope}---\n\n# T\n\ntext\n"),
            )
            .unwrap();
        }
        let url = format!("file://{}", folder.display());
        let keyed = keyed(&dir, &[("p", url.clone())], true);
        assert_eq!(
            step(&keyed, &notes),
            ("ok", format!("p through {url} (2 notes)"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
