//! `bilbo device`: shows this device, its owner and each scope's manifest, lists the owner's devices, and `init`,
//! `recover` and `revoke` write the keys and the manifest versions. The phrase forms run only in a terminal, and
//! every refusal comes before a phrase is drawn or read.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io;
use std::path::Path;

use zeroize::Zeroizing;

use crate::Failure;
use crate::host::prompt::Prompter;
use crate::identity::ceremony;
use crate::identity::keys::{self, Device, Identity, Owner};
use crate::identity::manifest::{self, Known, Outcome, Recipient, Written};
use crate::identity::phrase;
use crate::shared::config::{self, Settings};
use crate::shared::store;

pub struct Output {
    /// stderr lines, without "bilbo: ".
    pub warnings: Vec<String>,
    pub lines: Vec<String>,
    pub failed: bool,
}

/// What a syncing scope with no manifest is told, by `init` and `recover` alike: a fresh id here could fork it.
const UNSEALED: &str =
    "copy the store from an enrolled device, then run bilbo device recover again";

enum Form {
    Show,
    List,
    Init(Option<String>),
    Recover(Option<String>),
    Revoke(String),
}

struct Cx {
    settings: Settings,
    root: std::path::PathBuf,
    keys: std::path::PathBuf,
    /// Stdin and stderr are terminals and neither agent marker is set.
    human: bool,
    /// The sanitized host name, when it has a letter or a digit.
    host: Option<String>,
}

/// One scope's line of a step report.
struct Row {
    name: String,
    status: &'static str,
    detail: String,
}

impl Row {
    fn new(name: &str, status: &'static str, detail: String) -> Row {
        Row {
            name: name.to_string(),
            status,
            detail,
        }
    }

    fn of(name: &str, outcome: Written, created: bool) -> Row {
        let status = if created { "created" } else { "updated" };
        Row::new(name, status, version(&outcome))
    }
}

fn version(w: &Written) -> String {
    format!("{} manifest {} epoch {}", w.scope, w.n, w.epoch)
}

/// Runs the verb. `terminal` says stdin and stderr are both terminals; `p` draws the phrase screens.
pub fn run<P: Prompter>(
    args: &[String],
    env: &store::Env,
    terminal: bool,
    p: &mut P,
) -> Result<Output, Failure> {
    run_on(args, env, terminal, p, keys::host_name())
}

fn run_on<P: Prompter>(
    args: &[String],
    env: &store::Env,
    terminal: bool,
    p: &mut P,
    host: Option<String>,
) -> Result<Output, Failure> {
    let form = parse(args)?;
    let settings = config::load(env).map_err(Failure::Config)?;
    let root = store::root(env).map_err(Failure::Config)?;
    let keys = store::keys_dir(env)
        .ok_or_else(|| Failure::Config("no state folder: set XDG_STATE_HOME or HOME".into()))?;
    let identity = keys::read_identity(&keys).map_err(Failure::Refused)?;
    let mut warnings = Vec::new();
    if let Some(left) = keys::leftover(&keys)
        && (identity.is_some() || matches!(form, Form::Show | Form::List | Form::Revoke(_)))
    {
        warnings.push(format!(
            "{} is left from an interrupted init or recover; init or recover without keys removes it",
            left.display()
        ));
    }
    let human = terminal && !marked(&env.claudecode) && !marked(&env.codex_thread_id);
    let cx = Cx {
        settings,
        root,
        keys,
        human,
        host,
    };
    let mut out = match form {
        Form::Show => show(&cx, identity)?,
        Form::List => list(&cx, identity)?,
        Form::Init(name) => init(&cx, identity, name, p)?,
        Form::Recover(name) => recover(&cx, identity, name, p)?,
        Form::Revoke(target) => revoke(&cx, identity, &target)?,
    };
    warnings.append(&mut out.warnings);
    out.warnings = warnings;
    Ok(out)
}

/// An agent marker counts when it is set and not empty.
fn marked(var: &Option<OsString>) -> bool {
    var.as_ref().is_some_and(|v| !v.is_empty())
}

fn parse(args: &[String]) -> Result<Form, Failure> {
    let usage = |m: String| Failure::Usage(m);
    let Some(verb) = args.first() else {
        return Ok(Form::Show);
    };
    let rest = &args[1..];
    match verb.as_str() {
        "list" => no_more(rest).map(|()| Form::List),
        "init" => name_option(rest).map(Form::Init),
        "recover" => name_option(rest).map(Form::Recover),
        "revoke" => match rest {
            [] => Err(usage("revoke needs a device id or name".into())),
            [target] if target.starts_with('-') => Err(usage(format!("unknown option '{target}'"))),
            [target] => Ok(Form::Revoke(target.clone())),
            [_, extra, ..] => Err(usage(format!("unexpected argument '{extra}'"))),
        },
        arg if arg.starts_with('-') => Err(usage(format!("unknown option '{arg}'"))),
        arg => Err(usage(format!("unexpected argument '{arg}'"))),
    }
}

fn no_more(rest: &[String]) -> Result<(), Failure> {
    match rest.first() {
        None => Ok(()),
        Some(arg) if arg.starts_with('-') => Err(Failure::Usage(format!("unknown option '{arg}'"))),
        Some(arg) => Err(Failure::Usage(format!("unexpected argument '{arg}'"))),
    }
}

/// The value of `--name`, checked against the name rule.
fn name_option(rest: &[String]) -> Result<Option<String>, Failure> {
    let mut name = None;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        let value = if arg == "--name" {
            args.next()
                .ok_or_else(|| Failure::Usage("--name needs a value".into()))?
        } else if let Some(value) = arg.strip_prefix("--name=") {
            &value.to_string()
        } else if arg.starts_with('-') {
            return Err(Failure::Usage(format!("unknown option '{arg}'")));
        } else {
            return Err(Failure::Usage(format!("unexpected argument '{arg}'")));
        };
        if name.is_some() {
            return Err(Failure::Usage("--name given twice".into()));
        }
        if !keys::valid_name(value) {
            return Err(Failure::Usage(format!(
                "--name '{value}' is not a device name: lowercase letters and digits joined by single hyphens, at most 32 characters"
            )));
        }
        name = Some(value.clone());
    }
    Ok(name)
}

fn needs_terminal(form: &str) -> Failure {
    Failure::Refused(format!(
        "bilbo device {form} needs a terminal: run it yourself, in a terminal, not through an agent"
    ))
}

fn cancelled() -> Failure {
    Failure::Refused("cancelled; nothing was written".into())
}

/// A failed read of an answer: an interrupt is the user's cancel.
fn answer_failure(e: io::Error) -> Failure {
    if e.kind() == io::ErrorKind::Interrupted {
        cancelled()
    } else {
        Failure::Refused(format!("cannot read the answers: {e}"))
    }
}

fn survey(root: &Path, id: Option<&Identity>) -> Result<Vec<Known>, Failure> {
    let owner = id.map(|i| i.owner.sign.public());
    let who = id.map(|i| Recipient::device(&i.device));
    manifest::survey(root, owner.as_ref(), who.as_ref()).map_err(Failure::Refused)
}

fn syncing(settings: &Settings) -> impl Iterator<Item = &config::Scope> {
    settings.scopes.iter().filter(|s| s.sync != "off")
}

/// Whether `k` is the manifest a config scope named `name` means: this device's, read or once read.
fn matches(k: &Known, name: &str) -> bool {
    k.mine && (k.name() == Some(name) || k.last_name.as_deref() == Some(name))
}

/// A scope nobody can say the owner of, because none of its versions is valid or it cannot be read: it may be the
/// scope a config name means, so no id is minted beside it.
fn unattributed(k: &Known) -> bool {
    k.problem.is_some() && k.scope.owner().is_none()
}

fn fingerprint(owner: &[u8; 32]) -> String {
    keys::owner_fingerprint(owner)
}

/// The owners that signed the store's manifests, as fingerprints.
fn owners_of(known: &[Known]) -> Vec<String> {
    let owners: BTreeSet<String> = known
        .iter()
        .filter_map(|k| k.scope.owner())
        .map(|o| fingerprint(&o))
        .collect();
    owners.into_iter().collect()
}

fn show(cx: &Cx, id: Option<Identity>) -> Result<Output, Failure> {
    let known = survey(&cx.root, id.as_ref())?;
    let mut lines = Vec::new();
    let mut warnings = Vec::new();
    let mut failed = false;
    match &id {
        Some(i) => {
            lines.push(format!("device\t{}\t{}", i.device.name, i.device.id()));
            lines.push(format!("owner\t{}", fingerprint(&i.owner.sign.public())));
        }
        None => {
            lines.push("device\tnone".to_string());
            lines.push("owner\tnone".to_string());
            let owners = owners_of(&known);
            if !owners.is_empty() {
                warnings.push(format!(
                    "{} holds manifests of owner {}; in a terminal, run bilbo device recover with that owner's phrase",
                    cx.root.display(),
                    owners.join(", ")
                ));
            }
        }
    }
    let mine = id.as_ref().map(|i| fingerprint(&i.owner.sign.public()));
    let mut scopes: Vec<(String, String, String)> = Vec::new();
    for k in &known {
        let name = k.name().unwrap_or("-");
        let sid = &k.scope.id;
        let line = match k.scope.latest() {
            Some(latest) => {
                let m = &latest.manifest;
                let pending = if k.scope.pending.contains(&m.n) {
                    " pending"
                } else {
                    ""
                };
                format!(
                    "scope\t{name}\t{sid}\tmanifest {}{pending}\tepoch {}\t{} devices\t{}",
                    m.n,
                    m.epoch,
                    m.devices.len(),
                    m.transport
                )
            }
            None => format!("scope\t-\t{sid}\tinvalid"),
        };
        scopes.push((name.to_string(), sid.clone(), line));
        if let Some(problem) = &k.problem {
            warnings.push(format!("scope {sid}: {problem}"));
            failed = true;
        }
        if let (Some(mine), false, Some(owner)) = (&mine, k.mine, k.scope.owner()) {
            warnings.push(format!(
                "scope {sid} is signed by owner {}, not this device's owner {mine}",
                fingerprint(&owner)
            ));
            failed = true;
        }
        if let (Some(opened), Some(latest)) = (&k.opened, k.scope.latest())
            && let Some(s) = syncing(&cx.settings).find(|s| s.name == opened.name)
            && !manifest::transport_matches(&latest.manifest.transport, &s.sync)
        {
            warnings.push(format!(
                "scope {}: the manifest pins {} but the config says {}; run bilbo device init in a terminal to change the pin",
                opened.name, latest.manifest.transport, s.sync
            ));
            failed = true;
        }
    }
    for s in syncing(&cx.settings) {
        if known.iter().any(|k| matches(k, &s.name)) {
            continue;
        }
        scopes.push((
            s.name.clone(),
            String::new(),
            format!("scope\t{}\tunsealed\t{}", s.name, s.sync),
        ));
        warnings.push(match &id {
            None => format!(
                "scope {} syncs to {} but this device has no keys: in a terminal, run bilbo device recover with the phrase of an existing identity, or bilbo device init only to create a new one",
                s.name, s.sync
            ),
            Some(_) if known.iter().any(|k| k.mine && !k.ever_listed) => format!(
                "scope {} syncs to {} and no manifest this device reads has that name: in a terminal, run bilbo device recover with the owner's phrase",
                s.name, s.sync
            ),
            Some(_) => format!(
                "scope {} syncs to {} and has no manifest yet: run bilbo device init",
                s.name, s.sync
            ),
        });
    }
    scopes.sort();
    lines.extend(scopes.into_iter().map(|(_, _, line)| line));
    Ok(Output {
        warnings,
        lines,
        failed,
    })
}

fn list(cx: &Cx, id: Option<Identity>) -> Result<Output, Failure> {
    let Some(id) = id else {
        return Err(Failure::Refused(
            "this device has no keys: run bilbo device init in a terminal, or bilbo device recover with an existing recovery phrase".into(),
        ));
    };
    let known = survey(&cx.root, Some(&id))?;
    let this = id.device.id();
    let mut devices: BTreeMap<String, String> = BTreeMap::new();
    for k in known.iter().filter(|k| k.mine) {
        for entry in k.scope.latest().iter().flat_map(|v| &v.manifest.devices) {
            devices.insert(entry.id.clone(), entry.name.clone());
        }
    }
    devices.insert(this.clone(), id.device.name.clone());
    let mut rows: Vec<(String, String)> = devices.into_iter().map(|(i, n)| (n, i)).collect();
    rows.sort();
    let lines = rows
        .into_iter()
        .map(|(name, device)| {
            if device == this {
                format!("{name}\t{device}\tthis")
            } else {
                format!("{name}\t{device}")
            }
        })
        .collect();
    Ok(Output {
        warnings: Vec::new(),
        lines,
        failed: false,
    })
}

fn init<P: Prompter>(
    cx: &Cx,
    id: Option<Identity>,
    name: Option<String>,
    p: &mut P,
) -> Result<Output, Failure> {
    if id.is_some() && name.is_some() {
        return Err(Failure::Usage(
            "--name cannot rename an enrolled device".into(),
        ));
    }
    if let Some(s) = syncing(&cx.settings).next()
        && !cx.root.is_dir()
    {
        return Err(Failure::Refused(format!(
            "{} does not exist, and scope.{}.sync needs it: run bilbo setup",
            cx.root.display(),
            s.name
        )));
    }
    let mut lines = Vec::new();
    let id = match id {
        Some(id) => {
            lines.push(format!(
                "owner kept: {}",
                fingerprint(&id.owner.sign.public())
            ));
            lines.push(format!(
                "device kept: {} {}",
                id.device.name,
                id.device.id()
            ));
            id
        }
        None => {
            let known = survey(&cx.root, None)?;
            if !known.is_empty() {
                let owners = owners_of(&known);
                let who = if owners.is_empty() {
                    "manifests nobody can read".to_string()
                } else {
                    format!("manifests of owner {}", owners.join(", "))
                };
                return Err(Failure::Refused(format!(
                    "{} holds {who}: in a terminal, run bilbo device recover with that owner's phrase",
                    cx.root.display()
                )));
            }
            if !cx.human {
                return Err(needs_terminal("init"));
            }
            let name = chosen_name(cx, name)?;
            let id = ceremony_init(cx, &name, p)?;
            lines.push(format!(
                "owner created: {}",
                fingerprint(&id.owner.sign.public())
            ));
            lines.push(format!(
                "device created: {} {}",
                id.device.name,
                id.device.id()
            ));
            id
        }
    };
    let (rows, failed) = init_scopes(cx, &id)?;
    lines.extend(rows);
    Ok(Output {
        warnings: Vec::new(),
        lines,
        failed,
    })
}

/// `--name`, or the host name.
fn chosen_name(cx: &Cx, name: Option<String>) -> Result<String, Failure> {
    name.or_else(|| cx.host.clone()).ok_or_else(|| {
        Failure::Refused(
            "the host name holds no letter or digit to name this device: pass --name <name>".into(),
        )
    })
}

/// Shows a new phrase, and once it is confirmed writes the owner and device keys.
fn ceremony_init<P: Prompter>(cx: &Cx, name: &str, p: &mut P) -> Result<Identity, Failure> {
    keys::no_core_dump().map_err(Failure::Refused)?;
    keys::remove_leftover(&cx.keys).map_err(Failure::Refused)?;
    let entropy = Zeroizing::new(keys::random::<16>().map_err(Failure::Refused)?);
    let words = phrase::encode(&entropy);
    let owner = Owner::derive(&entropy);
    let print = fingerprint(&owner.sign.public());
    let positions = phrase::positions(keys::random::<3>().map_err(Failure::Refused)?);
    if !ceremony::confirm_written(p, &words, &print, positions).map_err(answer_failure)? {
        return Err(cancelled());
    }
    let device = Device::generate(name).map_err(Failure::Refused)?;
    let file = owner.file();
    keys::write_identity(&cx.keys, &file, &device).map_err(Failure::Refused)?;
    Ok(Identity {
        owner: file,
        device,
    })
}

/// `init`'s scope rows: a manifest for each syncing scope the config names, and a row for every other manifest.
fn init_scopes(cx: &Cx, id: &Identity) -> Result<(Vec<String>, bool), Failure> {
    let wanted: Vec<&config::Scope> = syncing(&cx.settings).collect();
    let lock = if wanted.is_empty() {
        None
    } else {
        Some(manifest::lock(&cx.root).map_err(Failure::Refused)?)
    };
    let known = survey(&cx.root, Some(id))?;
    let others = manifest::owner_devices(&known);
    let mut rows = Vec::new();
    let mut handled = BTreeSet::new();
    let mut failed = false;
    for s in wanted {
        let lock = lock.as_ref().expect("a syncing scope took the lock");
        let found = known.iter().find(|k| matches(k, &s.name));
        if let (None, Some(blind)) = (found, known.iter().find(|k| unattributed(k))) {
            failed = true;
            let why = blind.problem.as_deref().unwrap_or_default();
            rows.push(Row::new(
                &s.name,
                "failed",
                format!(
                    "no scope is created while {} is unreadable: {why}",
                    blind.scope.id
                ),
            ));
            continue;
        }
        match manifest::init_step(lock, id, &s.name, &s.sync, &known, &others, cx.human) {
            Outcome::Created(w) => {
                handled.insert(w.scope.clone());
                rows.push(Row::of(&s.name, w, true));
            }
            Outcome::Updated(w) => {
                handled.insert(w.scope.clone());
                rows.push(Row::of(&s.name, w, false));
            }
            Outcome::Kept => {
                if let Some(k) = found.filter(|k| k.opened.is_some()) {
                    handled.insert(k.scope.id.clone());
                    rows.push(Row::new(&s.name, "kept", k.scope.id.clone()));
                }
            }
            Outcome::Unsealed => rows.push(Row::new(&s.name, "unsealed", UNSEALED.into())),
            Outcome::Failed(why) => {
                failed = true;
                handled.extend(found.map(|k| k.scope.id.clone()));
                rows.push(Row::new(&s.name, "failed", why));
            }
        }
    }
    for k in known.iter().filter(|k| !handled.contains(&k.scope.id)) {
        let name = k.name().unwrap_or("-");
        if unattributed(k) {
            failed = true;
            rows.push(Row::new(
                name,
                "failed",
                k.problem.clone().unwrap_or_default(),
            ));
        } else {
            rows.push(Row::new(name, "kept", k.scope.id.clone()));
        }
    }
    Ok((report(rows), failed))
}

/// The rows as `scope <name> <status>: <detail>` lines, sorted by name.
fn report(mut rows: Vec<Row>) -> Vec<String> {
    rows.sort_by(|a, b| (&a.name, &a.detail).cmp(&(&b.name, &b.detail)));
    rows.into_iter()
        .map(|r| format!("scope {} {}: {}", r.name, r.status, r.detail))
        .collect()
}

fn recover<P: Prompter>(
    cx: &Cx,
    id: Option<Identity>,
    name: Option<String>,
    p: &mut P,
) -> Result<Output, Failure> {
    if id.is_some() && name.is_some() {
        return Err(Failure::Usage(
            "--name cannot rename an enrolled device".into(),
        ));
    }
    if !cx.human {
        return Err(needs_terminal("recover"));
    }
    let before = survey(&cx.root, id.as_ref())?;
    let name = match &id {
        Some(_) => None,
        None => {
            let name = chosen_name(cx, name)?;
            let listed = before
                .iter()
                .filter_map(|k| k.scope.latest())
                .flat_map(|v| &v.manifest.devices)
                .any(|d| d.name == name);
            if listed {
                return Err(Failure::Refused(format!(
                    "a device named {name} is already listed in the store: pass --name <another name>"
                )));
            }
            Some(name)
        }
    };
    keys::no_core_dump().map_err(Failure::Refused)?;
    let entropy = ceremony::read_phrase(p).map_err(answer_failure)?;
    let owner = Owner::derive(&entropy);
    drop(entropy);
    let owner_public = owner.sign.public();
    let print = fingerprint(&owner_public);
    match &id {
        Some(i) if i.owner.sign.public() != owner_public => {
            return Err(Failure::Refused(format!(
                "the phrase derives owner {print}, but this device's owner is {}; nothing was written",
                fingerprint(&i.owner.sign.public())
            )));
        }
        None => {
            let signed = owners_of(&before);
            if !signed.is_empty() && !signed.contains(&print) {
                return Err(Failure::Refused(format!(
                    "the phrase derives owner {print}, but the manifests in {} are signed by {}; nothing was written",
                    cx.root.display(),
                    signed.join(", ")
                )));
            }
        }
        Some(_) => {}
    }
    let vouched = id.is_some() || owners_of(&before).contains(&print);
    if !ceremony::confirm_fingerprint(p, &print, !vouched).map_err(answer_failure)? {
        return Err(Failure::Refused(
            "the fingerprint does not match; nothing was written".into(),
        ));
    }
    let mut lines = Vec::new();
    let id = match (id, name) {
        (Some(id), _) => {
            lines.push(format!("owner kept: {print}"));
            lines.push(format!(
                "device kept: {} {}",
                id.device.name,
                id.device.id()
            ));
            id
        }
        (None, name) => {
            let device = Device::generate(&name.expect("a device without keys has a name"))
                .map_err(Failure::Refused)?;
            let file = owner.file();
            keys::write_identity(&cx.keys, &file, &device).map_err(Failure::Refused)?;
            lines.push(format!("owner recovered: {print}"));
            lines.push(format!("device created: {} {}", device.name, device.id()));
            Identity {
                owner: file,
                device,
            }
        }
    };
    let lock = store::scopes_dir(&cx.root)
        .is_dir()
        .then(|| manifest::lock(&cx.root))
        .transpose()
        .map_err(Failure::Refused)?;
    let known = survey(&cx.root, Some(&id))?;
    let mut outcomes: BTreeMap<String, Outcome> = BTreeMap::new();
    if let Some(lock) = &lock {
        for k in &known {
            let outcome = manifest::recover_step(lock, &id, &owner.box_secret, k);
            outcomes.insert(k.scope.id.clone(), outcome);
        }
    }
    drop(owner);
    let mut failed = false;
    let mut rows = Vec::new();
    for k in survey(&cx.root, Some(&id))? {
        let name = k.name().unwrap_or("-");
        rows.push(match outcomes.remove(&k.scope.id) {
            Some(Outcome::Updated(w)) | Some(Outcome::Created(w)) => Row::of(name, w, false),
            Some(Outcome::Failed(why)) => {
                failed = true;
                Row::new(name, "failed", why)
            }
            _ if unattributed(&k) => {
                failed = true;
                Row::new(name, "failed", k.problem.clone().unwrap_or_default())
            }
            _ => Row::new(name, "kept", k.scope.id.clone()),
        });
    }
    let named: BTreeSet<String> = rows.iter().map(|r| r.name.clone()).collect();
    for s in syncing(&cx.settings).filter(|s| !named.contains(&s.name)) {
        rows.push(Row::new(&s.name, "unsealed", UNSEALED.into()));
    }
    lines.extend(report(rows));
    Ok(Output {
        warnings: Vec::new(),
        lines,
        failed,
    })
}

fn revoke(cx: &Cx, id: Option<Identity>, target: &str) -> Result<Output, Failure> {
    if !cx.human {
        return Err(needs_terminal("revoke"));
    }
    let Some(id) = id else {
        return Err(Failure::Refused(
            "this device has no keys, so it cannot revoke: run bilbo device recover first".into(),
        ));
    };
    let lock = store::scopes_dir(&cx.root)
        .is_dir()
        .then(|| manifest::lock(&cx.root))
        .transpose()
        .map_err(Failure::Refused)?;
    let known = survey(&cx.root, Some(&id))?;
    let this = id.device.id();
    let mut listed: BTreeMap<String, String> = BTreeMap::new();
    for k in known.iter().filter(|k| k.mine) {
        for entry in k.scope.latest().iter().flat_map(|v| &v.manifest.devices) {
            listed.insert(entry.id.clone(), entry.name.clone());
        }
    }
    listed.insert(this.clone(), id.device.name.clone());
    let found: Vec<(&String, &String)> = listed
        .iter()
        .filter(|(i, n)| i.as_str() == target || n.as_str() == target)
        .collect();
    let (target_id, target_name) = match found.as_slice() {
        [] => return Err(Failure::Refused(format!("no device named {target}"))),
        [(i, _)] if **i == this => {
            return Err(Failure::Refused("a device cannot revoke itself".into()));
        }
        [(i, n)] => ((*i).clone(), (*n).clone()),
        many => {
            let ids: Vec<&str> = many.iter().map(|(i, _)| i.as_str()).collect();
            return Err(Failure::Usage(format!(
                "'{target}' names more than one device: {}; pass an id",
                ids.join(", ")
            )));
        }
    };
    let lock = lock.ok_or_else(|| Failure::Refused(format!("no device named {target}")))?;
    let mut rows = Vec::new();
    let mut warnings = Vec::new();
    let mut failed = false;
    for k in &known {
        match manifest::revoke_step(&lock, &id, k, &target_id) {
            Outcome::Updated(w) | Outcome::Created(w) => {
                let folder = k
                    .scope
                    .latest()
                    .is_some_and(|v| v.manifest.transport == "file://");
                if folder {
                    warnings.push(format!(
                        "scope {}: remove {target_name} from the account that syncs the folder, because revocation does not take away its write access there",
                        w.name
                    ));
                }
                let name = w.name.clone();
                rows.push(Row::of(&name, w, false));
            }
            Outcome::Failed(why) => {
                failed = true;
                rows.push(Row::new(k.name().unwrap_or("-"), "failed", why));
            }
            Outcome::Kept | Outcome::Unsealed => {}
        }
    }
    Ok(Output {
        warnings,
        lines: report(rows),
        failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::keys::{self, Owner};
    use crate::identity::manifest::{self, Known, Outcome, Recipient};
    use crate::identity::script::{Answer, Script, text};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const PERSONAL: &str = "scope.personal.sync = file:///Users/a/Sync/bilbo\n";
    const RELAY: &str = "https://relay.example.net";
    const ABANDON_FP: &str = "yb4b-5aju-v6zb-x2nm-nc5x-ompf";

    struct World(PathBuf);

    impl Drop for World {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn world(name: &str) -> World {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let n = COUNT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("bilbo-device-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("root")).unwrap();
        fs::create_dir_all(dir.join("home")).unwrap();
        fs::write(dir.join("config"), "").unwrap();
        World(dir)
    }

    impl World {
        fn root(&self) -> PathBuf {
            self.0.join("root")
        }

        fn keys(&self) -> PathBuf {
            self.0.join("state/bilbo/keys")
        }

        fn config(&self, text: &str) {
            fs::write(self.0.join("config"), text).unwrap();
        }

        fn env(&self, claudecode: Option<&str>, codex: Option<&str>) -> store::Env {
            store::Env {
                bilbo_home: Some(self.root().into()),
                xdg_data_home: None,
                home: Some(self.0.join("home").into()),
                bilbo_config: Some(self.0.join("config").into()),
                xdg_config_home: None,
                xdg_cache_home: None,
                xdg_state_home: Some(self.0.join("state").into()),
                claudecode: claudecode.map(Into::into),
                codex_thread_id: codex.map(Into::into),
            }
        }

        fn go(
            &self,
            args: &[&str],
            terminal: bool,
            answers: Vec<Answer>,
        ) -> (Result<Output, Failure>, Script) {
            self.go_as(args, terminal, answers, Some("testhost"), (None, None))
        }

        fn go_as(
            &self,
            args: &[&str],
            terminal: bool,
            answers: Vec<Answer>,
            host: Option<&str>,
            marks: (Option<&str>, Option<&str>),
        ) -> (Result<Output, Failure>, Script) {
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            let mut p = Script::new(answers);
            let out = run_on(
                &args,
                &self.env(marks.0, marks.1),
                terminal,
                &mut p,
                host.map(Into::into),
            );
            (out, p)
        }

        fn ok(&self, args: &[&str], terminal: bool, answers: Vec<Answer>) -> (Output, Script) {
            let (out, p) = self.go(args, terminal, answers);
            match out {
                Ok(out) => (out, p),
                Err(f) => panic!("refused: {:?}", message(&f)),
            }
        }

        fn tree(&self) -> BTreeMap<String, Vec<u8>> {
            let mut files = BTreeMap::new();
            collect(&self.0, &self.0, &mut files);
            files
        }

        /// Writes `who`'s keys as this machine's identity.
        fn enroll(&self, who: &Identity) {
            keys::write_identity(&self.keys(), &who.owner, &who.device).unwrap();
        }

        fn scope_ids(&self) -> Vec<String> {
            manifest::scope_ids(&self.root()).unwrap()
        }
    }

    fn collect(base: &Path, dir: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(base, &path, files);
            } else if let Ok(bytes) = fs::read(&path) {
                let rel = path.strip_prefix(base).unwrap().display().to_string();
                files.insert(rel, bytes);
            }
        }
    }

    /// The failure as its exit code and message.
    fn message(f: &Failure) -> (i32, String) {
        match f {
            Failure::Usage(m) | Failure::Config(m) => (2, m.clone()),
            Failure::Refused(m) => (1, m.clone()),
        }
    }

    fn refusal(r: (Result<Output, Failure>, Script)) -> (i32, String, Script) {
        match r.0 {
            Ok(out) => panic!("did not refuse: {:?}", out.lines),
            Err(f) => {
                let (code, m) = message(&f);
                (code, m, r.1)
            }
        }
    }

    fn identity(owner: u8, name: &str, seed: u8) -> Identity {
        Identity {
            owner: Owner::derive(&[owner; 16]).file(),
            device: Device::from_seeds(name, &[seed; 32], &[seed + 1; 32]),
        }
    }

    fn rivendell() -> Identity {
        identity(0, "rivendell", 1)
    }

    fn bagend() -> Identity {
        identity(0, "bagend", 3)
    }

    /// A scope `name` created by `who` in the store.
    fn scope(w: &World, who: &Identity, name: &str, url: &str) -> String {
        let lock = manifest::lock(&w.root()).unwrap();
        manifest::create(&lock, who, name, url, &[]).unwrap().scope
    }

    fn known_as(w: &World, who: &Identity) -> Vec<Known> {
        let owner = who.owner.sign.public();
        manifest::survey(
            &w.root(),
            Some(&owner),
            Some(&Recipient::device(&who.device)),
        )
        .unwrap()
    }

    /// `who` recovers into every scope that does not list it, with the all-`abandon` owner's phrase.
    fn join(w: &World, who: &Identity) {
        let lock = manifest::lock(&w.root()).unwrap();
        for k in &known_as(w, who) {
            let box_secret = Owner::derive(&[0; 16]).box_secret;
            let outcome = manifest::recover_step(&lock, who, &box_secret, k);
            assert!(matches!(outcome, Outcome::Updated(_) | Outcome::Kept));
        }
    }

    /// `rivendell` enrolled here, with `personal` pinned to `file://` listing it and `bagend`.
    fn pair() -> (World, String) {
        let w = world("pair");
        let sid = scope(&w, &bagend(), "personal", "file:///Users/a/Sync/bilbo");
        join(&w, &rivendell());
        w.enroll(&rivendell());
        w.config(PERSONAL);
        (w, sid)
    }

    fn answers_of(entropy: u8) -> Vec<Answer> {
        phrase_words(entropy).iter().map(|w| text(w)).collect()
    }

    fn phrase_words(entropy: u8) -> Vec<&'static str> {
        phrase::encode(&[entropy; 16])
            .iter()
            .map(|&i| phrase::word(i))
            .collect()
    }

    fn init_answers() -> Vec<Answer> {
        vec![Answer::Yes, Answer::Shown, Answer::Shown, Answer::Shown]
    }

    fn fingerprint_shown(p: &Script) -> String {
        let note = p
            .shown
            .iter()
            .find(|s| s.starts_with("note: Recovery phrase"))
            .unwrap();
        note.rsplit("Owner fingerprint: ")
            .next()
            .unwrap()
            .trim()
            .to_string()
    }

    fn entropy_of(p: &Script) -> Vec<u8> {
        let words = p.phrase().unwrap();
        let indexes: Vec<u16> = words.iter().map(|w| phrase::lookup(w).unwrap()).collect();
        phrase::decode(&indexes).unwrap().to_vec()
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    // Recovery phrase

    #[test]
    fn a_new_phrase_is_twelve_checked_words_and_its_fingerprint() {
        let w = world("new_phrase");
        let (out, p) = w.ok(&["init", "--name", "rivendell"], true, init_answers());
        let words = p.phrase().unwrap();
        assert_eq!(words.len(), 12);
        let entropy = entropy_of(&p);
        assert_eq!(entropy.len(), 16);
        let owner = Owner::derive(&entropy.clone().try_into().unwrap());
        assert_eq!(fingerprint_shown(&p), fingerprint(&owner.sign.public()));
        assert_eq!(
            out.lines[0],
            format!("owner created: {}", fingerprint_shown(&p))
        );
    }

    #[test]
    fn the_phrase_is_kept_nowhere() {
        let w = world("kept_nowhere");
        w.config(PERSONAL);
        let (out, p) = w.ok(&["init", "--name", "rivendell"], true, init_answers());
        let words = p.phrase().unwrap();
        let entropy = entropy_of(&p);
        let hex = keys::hex(&entropy);
        let stdout = out.lines.join("\n") + &out.warnings.join("\n");
        let mut files = w.tree();
        assert!(files.len() >= 4);
        files.insert("<stdout>".into(), stdout.into_bytes());
        for (path, bytes) in files {
            assert!(
                !contains(&bytes, hex.as_bytes()),
                "{path} holds the entropy"
            );
            assert!(
                !contains(&bytes, &entropy),
                "{path} holds the entropy bytes"
            );
            assert!(
                !contains(&bytes, words.join(" ").as_bytes()),
                "{path} holds the phrase"
            );
            assert!(
                !contains(&bytes, hex.to_uppercase().as_bytes()),
                "{path} holds the entropy in capitals"
            );
            for pair in words.windows(2) {
                for glue in [" ", "\n", ", ", "\",\""] {
                    let sequence = pair.join(glue);
                    assert!(
                        !contains(&bytes, sequence.as_bytes()),
                        "{path} holds '{sequence}'"
                    );
                }
            }
            if path == "<stdout>" {
                let text = String::from_utf8_lossy(&bytes);
                for word in &words {
                    assert!(
                        !text
                            .split(|c: char| !c.is_ascii_alphabetic())
                            .any(|t| t == *word),
                        "{path} holds the word {word}"
                    );
                }
            }
        }
    }

    // Phrase confirmation

    #[test]
    fn a_confirmed_phrase_writes_the_keys_and_reports_them() {
        let w = world("confirmed");
        w.config(PERSONAL);
        let (out, p) = w.ok(&["init", "--name", "rivendell"], true, init_answers());
        let id = keys::read_identity(&w.keys()).unwrap().unwrap();
        assert_eq!(out.lines.len(), 3);
        assert_eq!(
            out.lines[0],
            format!("owner created: {}", fingerprint_shown(&p))
        );
        assert_eq!(
            out.lines[1],
            format!("device created: rivendell {}", id.device.id())
        );
        let sid = &w.scope_ids()[0];
        assert_eq!(
            out.lines[2],
            format!("scope personal created: {sid} manifest 1 epoch 1")
        );
        assert!(!out.failed);
        assert_eq!(p.left(), 0);
    }

    #[test]
    fn a_prefix_in_capitals_counts_as_the_word() {
        let w = world("prefix");
        let answers = vec![
            Answer::Yes,
            Answer::ShownShort,
            Answer::ShownShort,
            Answer::ShownShort,
        ];
        let (out, _) = w.ok(&["init", "--name", "rivendell"], true, answers);
        assert!(out.lines[0].starts_with("owner created: "));
        assert!(w.keys().is_dir());
    }

    #[test]
    fn a_wrong_word_is_asked_again_and_nothing_is_written_meanwhile() {
        let w = world("wrong");
        let answers = vec![
            Answer::Yes,
            Answer::Wrong,
            Answer::Select(0),
            Answer::Shown,
            Answer::Shown,
            Answer::Shown,
        ];
        let (_, p) = w.ok(&["init", "--name", "rivendell"], true, answers);
        assert!(p.saw("does not match"));
        let asked: Vec<&String> = p
            .shown
            .iter()
            .filter(|l| l.starts_with("input: Word"))
            .collect();
        assert_eq!(asked.len(), 4);
        assert_eq!(asked[0], asked[1]);
        let w = world("wrong_cancel");
        let (code, m, p) = refusal(w.go(
            &["init", "--name", "rivendell"],
            true,
            vec![Answer::Yes, Answer::Wrong, Answer::Select(2)],
        ));
        assert_eq!(code, 1);
        assert!(m.contains("nothing was written"));
        assert!(!w.keys().exists());
        assert!(p.shown.last().unwrap() == "screen: end");
    }

    #[test]
    fn a_cancel_writes_nothing_and_exits_1() {
        for answers in [
            vec![Answer::No],
            vec![Answer::Yes, Answer::Interrupt],
            vec![Answer::Interrupt],
        ] {
            let w = world("cancel");
            let (code, m, p) = refusal(w.go(&["init", "--name", "rivendell"], true, answers));
            assert_eq!(code, 1);
            assert!(m.contains("nothing was written"), "{m}");
            assert!(!w.keys().exists());
            assert!(w.tree().keys().all(|f| f == "config"));
            assert!(!p.saw("Recovery phrase confirmed"));
        }
    }

    // Terminal-only forms

    #[test]
    fn an_agent_gets_one_stderr_line_and_no_file() {
        let w = world("agent_init");
        let (code, m, p) = refusal(w.go(&["init", "--name", "rivendell"], false, vec![]));
        assert_eq!(code, 1);
        assert!(m.contains("bilbo device init") && m.contains("terminal"));
        assert!(!m.contains('\n'));
        assert!(p.shown.is_empty());
        assert!(!w.0.join("state").exists());
    }

    #[test]
    fn an_agent_marker_blocks_the_terminal_forms_even_with_a_terminal() {
        let w = world("markers");
        let before = w.tree();
        for marks in [(Some("1"), None), (None, Some("x"))] {
            let r = w.go_as(
                &["recover", "--name", "a"],
                true,
                answers_of(0),
                Some("h"),
                marks,
            );
            let (code, m, p) = refusal(r);
            assert_eq!(code, 1);
            assert!(m.contains("terminal"));
            assert!(p.shown.is_empty() && p.left() == 12);
            let r = w.go_as(
                &["init", "--name", "a"],
                true,
                init_answers(),
                Some("h"),
                marks,
            );
            let (_, m, p) = refusal(r);
            assert!(m.contains("bilbo device init") && m.contains("terminal"));
            assert!(p.shown.is_empty());
        }
        assert_eq!(w.tree(), before);
        let r = w.go_as(&["revoke", "bagend"], true, vec![], None, (None, Some("x")));
        let (code, m, _) = refusal(r);
        assert_eq!(code, 1);
        assert!(m.contains("terminal"), "{m}");
        let (w, _) = pair();
        let before = w.tree();
        let r = w.go_as(&["revoke", "bagend"], true, vec![], None, (None, Some("x")));
        let (code, m, _) = refusal(r);
        assert_eq!(code, 1);
        assert!(m.contains("terminal"));
        assert_eq!(w.tree(), before);
    }

    #[test]
    fn the_terminal_rule_comes_before_name_refusals() {
        let w = world("terminal_first");
        scope(&w, &rivendell(), "personal", "file://");
        for form in ["recover", "init"] {
            let r = w.go_as(&[form], false, vec![], Some("rivendell"), (None, None));
            let (_, m, _) = refusal(r);
            assert!(m.contains("terminal"), "{m}");
            let r = w.go_as(&[form], true, vec![], None, (Some("1"), None));
            let (_, m, _) = refusal(r);
            assert!(m.contains("terminal"), "{m}");
        }
    }

    #[test]
    fn an_empty_marker_does_not_count() {
        let w = world("empty_marker");
        let r = w.go_as(
            &["init", "--name", "rivendell"],
            true,
            init_answers(),
            None,
            (Some(""), Some("")),
        );
        assert!(r.0.is_ok());
    }

    #[test]
    fn an_enrolled_device_seals_a_new_scope_without_a_terminal() {
        let (w, sid) = pair();
        w.config("scope.shared.sync = file:///Users/a/Sync/shared\n");
        let (out, p) = w.ok(&["init"], false, vec![]);
        assert!(!out.failed);
        assert!(p.shown.is_empty());
        let new = w.scope_ids().into_iter().find(|i| *i != sid).unwrap();
        let line = format!("scope shared created: {new} manifest 1 epoch 1");
        assert!(out.lines.contains(&line), "{:?}", out.lines);
    }

    // Phrase on screen

    #[test]
    fn the_phrase_is_only_inside_the_screen() {
        let w = world("screen");
        let (out, p) = w.ok(&["init", "--name", "rivendell"], true, init_answers());
        let at = |needle: &str| p.shown.iter().position(|l| l.contains(needle)).unwrap();
        let (begin, end) = (at("screen: begin"), at("screen: end"));
        assert!(begin < at("note: Recovery phrase") && at("note: Recovery phrase") < end);
        assert!(end < at("info: Recovery phrase confirmed"));
        let words = p.phrase().unwrap();
        for line in out.lines.iter().chain(&out.warnings) {
            for pair in words.windows(2) {
                assert!(!line.contains(&pair.join(" ")), "{pair:?} in {line}");
            }
        }
        let confirmed = p
            .shown
            .iter()
            .filter(|l| l.contains("Recovery phrase confirmed"))
            .count();
        assert_eq!(confirmed, 1);
    }

    #[test]
    fn a_cancel_leaves_the_screen_with_no_word_outside_it() {
        let w = world("screen_cancel");
        let (_, m, p) = refusal(w.go(
            &["init", "--name", "rivendell"],
            true,
            vec![Answer::Yes, Answer::Interrupt],
        ));
        let words = p.phrase().unwrap();
        let at = |needle: &str| p.shown.iter().position(|l| l.contains(needle)).unwrap();
        assert!(at("note: Recovery phrase") < at("screen: end"));
        assert_eq!(at("screen: end"), p.shown.len() - 1);
        for pair in words.windows(2) {
            assert!(!m.contains(&pair.join(" ")));
        }
    }

    // Owner key

    #[test]
    fn the_all_abandon_phrase_recovers_the_pinned_owner() {
        let w = world("known_phrase");
        let mut answers = answers_of(0);
        answers.push(Answer::Yes);
        let (out, _) = w.ok(&["recover", "--name", "bagend"], true, answers);
        assert_eq!(out.lines[0], format!("owner recovered: {ABANDON_FP}"));
    }

    #[test]
    fn the_same_phrase_gives_the_same_owner_on_two_devices() {
        let a = world("same_a");
        let (_, p) = a.ok(&["init", "--name", "rivendell"], true, init_answers());
        let words = p.phrase().unwrap();
        let b = world("same_b");
        let mut answers: Vec<Answer> = words.iter().map(|w| text(w)).collect();
        answers.push(Answer::Yes);
        b.ok(&["recover", "--name", "bagend"], true, answers);
        let owner_of = |w: &World| w.ok(&[], false, vec![]).0.lines[1].clone();
        assert_eq!(owner_of(&a), owner_of(&b));
        assert_ne!(owner_of(&a), format!("owner\t{ABANDON_FP}"));
    }

    #[test]
    fn another_phrase_gives_another_owner() {
        let (a, b) = (world("other_a"), world("other_b"));
        for (w, entropy) in [(&a, 0u8), (&b, 0x7f)] {
            let mut answers = answers_of(entropy);
            answers.push(Answer::Yes);
            w.ok(&["recover", "--name", "bagend"], true, answers);
        }
        let owner_of = |w: &World| w.ok(&[], false, vec![]).0.lines[1].clone();
        assert_ne!(owner_of(&a), owner_of(&b));
    }

    // Owner secrets on a device

    #[test]
    fn owner_key_holds_the_seed_and_the_public_key_only() {
        let w = world("owner_file");
        w.config(PERSONAL);
        let (_, p) = w.ok(&["init", "--name", "rivendell"], true, init_answers());
        let entropy: [u8; 16] = entropy_of(&p).try_into().unwrap();
        let owner = Owner::derive(&entropy);
        let text = fs::read_to_string(w.keys().join("owner.key")).unwrap();
        let members: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut names: Vec<&String> = members.as_object().unwrap().keys().collect();
        names.sort();
        assert_eq!(names, ["box_public", "format", "sign"]);
        assert!(!text.contains(&keys::hex(owner.box_secret.bytes())));
        let sid = &w.scope_ids()[0];
        let scope = manifest::read_scope(&w.root(), sid).unwrap();
        assert_eq!(
            scope.latest().unwrap().manifest.owner_box,
            keys::hex(&owner.box_secret.public())
        );
        assert_eq!(members["box_public"], keys::hex(&owner.box_secret.public()));
    }

    #[test]
    fn a_manifest_that_does_not_list_this_device_shows_a_dash() {
        let w = world("dash");
        let sid = scope(&w, &bagend(), "personal", "file:///Users/a/Sync/bilbo");
        w.enroll(&rivendell());
        let (out, _) = w.ok(&[], false, vec![]);
        assert_eq!(
            out.lines[2],
            format!("scope\t-\t{sid}\tmanifest 1 pending\tepoch 1\t1 devices\tfile://")
        );
        assert!(!out.failed);
    }

    // Owner fingerprint, device key

    #[test]
    fn show_prints_the_fingerprint_and_an_id_that_matches_the_manifest() {
        let (w, sid) = pair();
        let (out, _) = w.ok(&[], false, vec![]);
        let owner: Vec<&str> = out.lines[1].split('\t').collect();
        assert_eq!(owner[0], "owner");
        let groups: Vec<&str> = owner[1].split('-').collect();
        assert_eq!(groups.len(), 6);
        assert!(
            groups
                .iter()
                .all(|g| g.len() == 4 && g.bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7')))
        );
        let device: Vec<&str> = out.lines[0].split('\t').collect();
        assert_eq!(device[1], "rivendell");
        assert_eq!(device[2].len(), 26);
        let scope = manifest::read_scope(&w.root(), &sid).unwrap();
        let entry = scope
            .latest()
            .unwrap()
            .manifest
            .devices
            .iter()
            .find(|d| d.name == "rivendell")
            .unwrap();
        assert_eq!(
            device[2],
            keys::device_id(&keys::unhex(&entry.sign).unwrap())
        );
    }

    #[test]
    fn a_device_with_no_keys_and_no_manifest_has_no_owner() {
        let w = world("none");
        let (out, p) = w.ok(&[], false, vec![]);
        assert_eq!(out.lines, ["device\tnone", "owner\tnone"]);
        assert!(out.warnings.is_empty() && !out.failed);
        assert!(p.shown.is_empty());
    }

    #[test]
    fn a_damaged_key_file_is_refused_by_every_form() {
        let w = world("damaged");
        w.enroll(&rivendell());
        let device = w.keys().join("device.key");
        fs::write(
            &device,
            r#"{"format":1,"name":"Bag_End","sign":"00","box":"00"}"#,
        )
        .unwrap();
        let before = w.tree();
        for args in [
            &[][..],
            &["list"],
            &["init"],
            &["recover"],
            &["revoke", "x"],
        ] {
            let (code, m, p) = refusal(w.go(args, true, vec![]));
            assert_eq!(code, 1);
            assert!(m.contains("device.key"), "{m}");
            assert!(p.shown.is_empty());
        }
        assert_eq!(w.tree(), before);
    }

    // Device name

    #[test]
    fn the_default_name_is_the_host_name() {
        let w = world("default_name");
        let host = keys::sanitize_name("Daniels-MacBook-Pro.local");
        let r = w.go_as(
            &["init"],
            true,
            init_answers(),
            host.as_deref(),
            (None, None),
        );
        let out = r.0.ok().unwrap();
        assert!(out.lines[1].starts_with("device created: daniels-macbook-pro "));
    }

    #[test]
    fn a_bad_name_is_a_usage_error_naming_the_option() {
        let w = world("bad_name");
        for form in ["init", "recover"] {
            let (code, m, p) = refusal(w.go(&[form, "--name", "Bag_End"], true, vec![]));
            assert_eq!(code, 2);
            assert!(m.contains("--name"));
            assert!(p.shown.is_empty());
        }
        let (code, _, _) = refusal(w.go(&["init", "--name", &"a".repeat(33)], true, vec![]));
        assert_eq!(code, 2);
        assert!(!w.0.join("state").exists());
    }

    #[test]
    fn no_usable_host_name_asks_for_a_name() {
        let w = world("no_host");
        let r = w.go_as(&["init"], true, init_answers(), None, (None, None));
        let (code, m, p) = refusal(r);
        assert_eq!(code, 1);
        assert!(m.contains("--name"));
        assert!(p.shown.is_empty());
        assert!(!w.0.join("state").exists());
    }

    // Where keys live, writing keys

    #[test]
    fn keys_are_written_with_closed_modes() {
        let w = world("modes");
        w.ok(&["init", "--name", "rivendell"], true, init_answers());
        assert_eq!(mode(&w.keys()), 0o700);
        assert_eq!(mode(&w.keys().join("owner.key")), 0o600);
        assert_eq!(mode(&w.keys().join("device.key")), 0o600);
        assert!(!w.keys().starts_with(w.root()));
    }

    #[test]
    fn loose_permissions_refuse_and_name_the_folder() {
        let (w, _) = pair();
        fs::set_permissions(w.keys(), fs::Permissions::from_mode(0o755)).unwrap();
        let (code, m, _) = refusal(w.go(&["list"], false, vec![]));
        assert_eq!(code, 1);
        assert!(m.contains(&w.keys().display().to_string()));
    }

    #[test]
    fn a_copied_store_has_no_device_and_keeps_its_devices() {
        let (a, sid) = pair();
        let b = world("copied");
        let copy = manifest::read_scope(&a.root(), &sid).unwrap();
        let lock = manifest::lock(&b.root()).unwrap();
        for v in &copy.versions {
            manifest::adopt(&lock, &sid, v.manifest.n, &v.bytes).unwrap();
        }
        drop(lock);
        let (out, _) = b.ok(&[], false, vec![]);
        assert_eq!(&out.lines[..2], ["device\tnone", "owner\tnone"]);
        assert!(out.lines.iter().any(|l| l.contains("\t2 devices\t")));
        assert!(out.warnings.iter().any(|l| l.contains(ABANDON_FP)));
        let again = manifest::read_scope(&b.root(), &sid).unwrap();
        assert_eq!(again.latest().unwrap().manifest.devices.len(), 2);
    }

    #[test]
    fn a_leftover_keys_new_is_named_and_left_by_show() {
        let w = world("leftover");
        let left = w.0.join("state/bilbo/keys.new");
        fs::create_dir_all(&left).unwrap();
        fs::write(left.join("owner.key"), "x").unwrap();
        let before = w.tree();
        let (out, _) = w.ok(&[], false, vec![]);
        assert_eq!(out.lines[0], "device\tnone");
        assert_eq!(
            out.warnings
                .iter()
                .filter(|l| l.contains("keys.new"))
                .count(),
            1
        );
        assert_eq!(w.tree(), before);
    }

    #[test]
    fn the_next_init_removes_the_leftover_before_the_phrase() {
        let w = world("leftover_init");
        let left = w.0.join("state/bilbo/keys.new");
        fs::create_dir_all(&left).unwrap();
        fs::write(left.join("stale"), "x").unwrap();
        refusal(w.go(&["init", "--name", "rivendell"], true, vec![Answer::No]));
        assert!(!left.exists() && !w.keys().exists());
        fs::create_dir_all(&left).unwrap();
        w.ok(&["init", "--name", "rivendell"], true, init_answers());
        assert!(!left.exists());
        assert!(w.keys().join("owner.key").is_file());
    }

    // Show

    #[test]
    fn an_enrolled_device_shows_its_scope() {
        let (w, sid) = pair();
        let (out, _) = w.ok(&[], false, vec![]);
        assert_eq!(out.lines.len(), 3);
        assert_eq!(
            out.lines[2],
            format!("scope\tpersonal\t{sid}\tmanifest 2 pending\tepoch 1\t2 devices\tfile://")
        );
        assert!(out.warnings.is_empty() && !out.failed);
        assert!(out.lines[0].starts_with("device\trivendell\t"));
    }

    #[test]
    fn a_sync_url_with_no_keys_waits_for_the_phrase() {
        let w = world("waiting");
        w.config(PERSONAL);
        let (out, _) = w.ok(&[], false, vec![]);
        assert!(!out.failed);
        assert_eq!(out.warnings.len(), 1);
        let line = &out.warnings[0];
        assert!(line.contains("personal") && line.contains("terminal"));
        assert!(line.contains("bilbo device recover") && line.contains("bilbo device init"));
        assert_eq!(
            out.lines[2],
            "scope\tpersonal\tunsealed\tfile:///Users/a/Sync/bilbo"
        );
    }

    #[test]
    fn an_extra_argument_is_a_usage_error() {
        let w = world("extra");
        let (code, m, _) = refusal(w.go(&["now"], false, vec![]));
        assert_eq!(code, 2);
        assert!(m.contains("now"));
        let (code, _, _) = refusal(w.go(&["list", "now"], false, vec![]));
        assert_eq!(code, 2);
    }

    #[test]
    fn another_owners_manifest_is_a_problem() {
        let w = world("foreign");
        let sid = scope(&w, &identity(1, "gandalf", 5), "grey", "file://");
        w.enroll(&rivendell());
        let (out, _) = w.ok(&[], false, vec![]);
        assert!(out.failed);
        assert!(out.lines[2].starts_with(&format!("scope\t-\t{sid}\t")));
        let owner = fingerprint(&Owner::derive(&[1; 16]).sign.public());
        assert!(
            out.warnings
                .iter()
                .any(|l| l.contains(&sid) && l.contains(&owner))
        );
    }

    #[test]
    fn a_tampered_manifest_is_a_problem_and_revoke_refuses_it() {
        let (w, sid) = pair();
        let path = w
            .root()
            .join(format!(".bilbo/scopes/{sid}/manifest/2.json"));
        let mut bytes = fs::read(&path).unwrap();
        let at = bytes.windows(7).position(|b| b == b"\"name\":").unwrap();
        bytes[at + 10] ^= 1;
        fs::write(&path, &bytes).unwrap();
        let (out, _) = w.ok(&[], false, vec![]);
        assert!(out.failed);
        assert!(
            out.warnings
                .iter()
                .any(|l| l.contains("manifest/") && l.contains("invalid"))
        );
        let (out, _) = w.ok(&["revoke", "bagend"], true, vec![]);
        assert!(out.failed);
        assert!(out.lines[0].contains("failed"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn a_url_changed_in_the_config_is_a_problem() {
        let (w, _) = pair();
        w.config(&format!("scope.personal.sync = {RELAY}\n"));
        let (out, _) = w.ok(&[], false, vec![]);
        assert!(out.failed);
        let line = out
            .warnings
            .iter()
            .find(|l| l.contains("personal"))
            .unwrap();
        assert!(
            line.contains("file://") && line.contains(RELAY) && line.contains("bilbo device init")
        );
    }

    #[test]
    fn two_machines_may_name_different_folders_for_one_file_scope() {
        let (w, _) = pair();
        w.config("scope.personal.sync = file:///home/a/Sync/bilbo\n");
        let (out, _) = w.ok(&[], false, vec![]);
        assert!(out.warnings.is_empty() && !out.failed);
    }

    // Init

    #[test]
    fn a_rerun_keeps_everything_and_changes_no_file() {
        let w = world("rerun");
        w.config(PERSONAL);
        w.ok(&["init", "--name", "rivendell"], true, init_answers());
        let before = w.tree();
        let (out, p) = w.ok(&["init"], true, vec![]);
        assert!(
            out.lines.iter().all(|l| l.contains(" kept")),
            "{:?}",
            out.lines
        );
        assert_eq!(out.lines.len(), 3);
        assert!(p.shown.is_empty() && !out.failed);
        assert_eq!(w.tree(), before);
    }

    #[test]
    fn a_name_on_an_enrolled_device_is_a_usage_error() {
        let (w, _) = pair();
        let before = w.tree();
        for form in ["init", "recover"] {
            let (code, m, p) = refusal(w.go(&[form, "--name", "bagend"], true, vec![]));
            assert_eq!(code, 2);
            assert!(m.contains("--name"));
            assert!(p.shown.is_empty());
        }
        assert_eq!(w.tree(), before);
    }

    #[test]
    fn a_store_owned_by_someone_else_is_refused_before_the_phrase() {
        let w = world("someone_else");
        scope(&w, &identity(1, "gandalf", 5), "grey", "file://");
        let before = w.tree();
        let (code, m, p) = refusal(w.go(&["init", "--name", "rivendell"], true, init_answers()));
        assert_eq!(code, 1);
        let owner = fingerprint(&Owner::derive(&[1; 16]).sign.public());
        assert!(m.contains(&owner) && m.contains("bilbo device recover"));
        assert!(p.shown.is_empty());
        assert_eq!(w.tree(), before);
    }

    #[test]
    fn a_syncing_scope_needs_a_store() {
        let w = world("no_store");
        fs::remove_dir_all(w.root()).unwrap();
        w.config(PERSONAL);
        let (code, m, p) = refusal(w.go(&["init", "--name", "rivendell"], true, init_answers()));
        assert_eq!(code, 1);
        assert!(m.contains(&w.root().display().to_string()) && m.contains("bilbo setup"));
        assert!(p.shown.is_empty());
        assert!(!w.0.join("state").exists());
    }

    #[test]
    fn a_changed_url_needs_a_terminal_and_other_scopes_are_still_created() {
        let (w, sid) = pair();
        w.config(&format!(
            "scope.personal.sync = {RELAY}\nscope.shared.sync = file:///x\n"
        ));
        let (out, _) = w.ok(&["init"], false, vec![]);
        assert!(out.failed);
        assert!(
            out.lines
                .iter()
                .any(|l| l == "scope personal failed: changing the URL needs a terminal")
        );
        assert!(
            out.lines
                .iter()
                .any(|l| l.starts_with("scope shared created: "))
        );
        assert_eq!(
            manifest::read_scope(&w.root(), &sid)
                .unwrap()
                .versions
                .len(),
            2
        );
        let (out, _) = w.ok(&["init"], true, vec![]);
        assert!(
            out.lines
                .contains(&format!("scope personal updated: {sid} manifest 3 epoch 1"))
        );
        let scope = manifest::read_scope(&w.root(), &sid).unwrap();
        assert_eq!(scope.latest().unwrap().manifest.transport, RELAY);
    }

    #[test]
    fn off_keeps_the_scope_and_prints_no_problem() {
        let (w, sid) = pair();
        w.config("scope.personal.sync = off\n");
        let before = w.tree();
        let (out, _) = w.ok(&["init"], false, vec![]);
        assert!(out.lines.contains(&format!("scope personal kept: {sid}")));
        let (shown, _) = w.ok(&[], false, vec![]);
        assert!(shown.warnings.is_empty() && !shown.failed);
        assert_eq!(w.tree(), before);
    }

    #[test]
    fn a_renamed_scope_gets_a_new_id() {
        let (w, sid) = pair();
        w.config("scope.mine.sync = file:///Users/a/Sync/bilbo\n");
        let (out, _) = w.ok(&["init"], false, vec![]);
        assert!(
            out.lines
                .iter()
                .any(|l| l.starts_with("scope mine created: "))
        );
        assert!(out.lines.contains(&format!("scope personal kept: {sid}")));
        assert_eq!(w.scope_ids().len(), 2);
    }

    #[test]
    fn a_second_scope_lists_the_owners_devices() {
        let (w, _) = pair();
        w.config(&format!("{PERSONAL}scope.shared.sync = file:///x\n"));
        let (out, _) = w.ok(&["init"], false, vec![]);
        assert!(
            out.lines
                .iter()
                .any(|l| l.starts_with("scope shared created: "))
        );
        let (shown, _) = w.ok(&[], false, vec![]);
        let line = shown
            .lines
            .iter()
            .find(|l| l.starts_with("scope\tshared\t"))
            .unwrap();
        assert!(line.contains("\t2 devices\t"));
    }

    #[test]
    fn a_device_a_manifest_dropped_is_not_added_again() {
        let (w, sid) = pair();
        let lock = manifest::lock(&w.root()).unwrap();
        let known = known_as(&w, &bagend());
        let out = manifest::revoke_step(&lock, &bagend(), &known[0], &rivendell().device.id());
        assert!(matches!(out, Outcome::Updated(_)));
        drop(lock);
        let before = w.tree();
        let (out, _) = w.ok(&["init"], true, vec![]);
        assert!(out.lines.contains(&format!("scope - kept: {sid}")));
        assert!(!out.failed);
        assert_eq!(w.tree(), before);
    }

    #[test]
    fn a_manifest_this_device_never_read_leaves_its_scope_unsealed() {
        let w = world("never_read");
        let sid = scope(&w, &bagend(), "personal", "file:///x");
        w.enroll(&rivendell());
        w.config(PERSONAL);
        let (out, _) = w.ok(&["init"], true, vec![]);
        assert!(out.lines.contains(&format!("scope - kept: {sid}")));
        assert!(
            out.lines
                .contains(&format!("scope personal unsealed: {UNSEALED}"))
        );
        assert_eq!(w.scope_ids(), [sid]);
    }

    #[test]
    fn an_unreadable_scope_blocks_every_new_id() {
        let (w, sid) = pair();
        w.config(&format!(
            "scope.personal.sync = {RELAY}\nscope.shared.sync = file:///x\n"
        ));
        fs::create_dir(
            w.root()
                .join(format!(".bilbo/scopes/{sid}/manifest/3.json")),
        )
        .unwrap();
        let (out, _) = w.ok(&["init"], true, vec![]);
        assert!(out.failed);
        assert!(
            out.lines
                .iter()
                .any(|l| l.starts_with("scope personal failed: ") && l.contains("3.json"))
        );
        let blocked = out
            .lines
            .iter()
            .find(|l| l.starts_with("scope shared failed: "));
        assert!(blocked.unwrap().contains(&sid));
        assert_eq!(w.scope_ids(), [sid]);
    }

    #[test]
    fn an_unreadable_scope_is_a_failed_row_even_when_nothing_is_new() {
        let (w, sid) = pair();
        fs::create_dir(
            w.root()
                .join(format!(".bilbo/scopes/{sid}/manifest/3.json")),
        )
        .unwrap();
        let (out, _) = w.ok(&["init"], true, vec![]);
        assert!(out.failed);
        assert!(out.lines.iter().any(|l| l.starts_with("scope - failed: ")));
        assert!(w.ok(&[], false, vec![]).0.failed);
    }

    #[test]
    fn a_scope_made_while_init_waits_is_not_made_again() {
        let w = world("race");
        w.enroll(&rivendell());
        w.config(PERSONAL);
        let lock = manifest::lock(&w.root()).unwrap();
        let racer = std::thread::scope(|t| {
            let run = t.spawn(|| w.ok(&["init"], false, vec![]).0);
            std::thread::sleep(std::time::Duration::from_millis(300));
            manifest::create(&lock, &rivendell(), "personal", "file:///x", &[]).unwrap();
            drop(lock);
            run.join().unwrap()
        });
        assert_eq!(w.scope_ids().len(), 1);
        assert!(
            racer
                .lines
                .iter()
                .any(|l| l.starts_with("scope personal kept: ")),
            "{:?}",
            racer.lines
        );
        assert!(!racer.failed);
    }

    #[test]
    fn a_manifest_invalid_before_any_readable_version_leaves_the_scope_unsealed() {
        let w = world("unreadable_v1");
        let sid = scope(&w, &rivendell(), "personal", "file:///x");
        w.enroll(&rivendell());
        w.config(PERSONAL);
        let path = w
            .root()
            .join(format!(".bilbo/scopes/{sid}/manifest/1.json"));
        let mut m: manifest::Manifest = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let own = rivendell().device.id();
        let len = m.sealed[&own].len();
        m.sealed.insert(own, "00".repeat(len / 2));
        m.sig.clear();
        let mut message = b"bilbo-manifest-1\n".to_vec();
        message.extend(serde_json::to_vec(&m).unwrap());
        m.sig = keys::hex(&rivendell().owner.sign.sign(&message));
        let mut bytes = serde_json::to_vec(&m).unwrap();
        bytes.push(b'\n');
        fs::write(&path, bytes).unwrap();
        let (out, _) = w.ok(&["init"], true, vec![]);
        assert!(
            out.lines
                .contains(&format!("scope personal unsealed: {UNSEALED}")),
            "{:?}",
            out.lines
        );
        assert_eq!(w.scope_ids(), [sid]);
    }

    // Recover

    #[test]
    fn the_wrong_phrase_names_both_owners_and_writes_nothing() {
        let w = world("wrong_phrase");
        scope(&w, &rivendell(), "personal", "file://");
        let before = w.tree();
        let (code, m, _) = refusal(w.go(&["recover", "--name", "bagend"], true, answers_of(0x7f)));
        assert_eq!(code, 1);
        let other = fingerprint(&Owner::derive(&[0x7f; 16]).sign.public());
        assert!(m.contains(&other) && m.contains(ABANDON_FP));
        assert_eq!(w.tree(), before);
        assert!(!w.keys().exists());
    }

    #[test]
    fn a_taken_name_is_refused_before_any_word() {
        let w = world("taken");
        scope(&w, &rivendell(), "personal", "file://");
        let r = w.go_as(
            &["recover"],
            true,
            answers_of(0),
            Some("rivendell"),
            (None, None),
        );
        let (code, m, p) = refusal(r);
        assert_eq!(code, 1);
        assert!(m.contains("rivendell") && m.contains("--name"));
        assert!(p.shown.is_empty());
    }

    #[test]
    fn nothing_to_check_against_asks_whether_the_fingerprint_matches() {
        let w = world("ask");
        let mut answers = answers_of(0);
        answers.push(Answer::Yes);
        let (out, p) = w.ok(&["recover", "--name", "bagend"], true, answers);
        assert!(p.saw("note: Owner fingerprint\nyb4b-5aju-v6zb-x2nm-nc5x-ompf"));
        assert!(p.saw("confirm: Does it match"));
        assert!(out.lines[0].starts_with("owner recovered: "));
    }

    #[test]
    fn a_mismatch_writes_nothing() {
        let w = world("mismatch");
        let mut answers = answers_of(0);
        answers.push(Answer::No);
        let (code, m, _) = refusal(w.go(&["recover", "--name", "bagend"], true, answers));
        assert_eq!(code, 1);
        assert!(m.contains("nothing was written"));
        assert!(!w.keys().exists());
    }

    #[test]
    fn a_manifest_vouches_for_the_phrase() {
        let w = world("vouched");
        scope(&w, &rivendell(), "personal", "file://");
        let (out, p) = w.ok(&["recover", "--name", "bagend"], true, answers_of(0));
        assert!(p.saw("note: Owner fingerprint") && !p.saw("confirm:"));
        assert!(!out.failed);
    }

    #[test]
    fn a_wiped_laptop_recovers_into_its_scope() {
        let w = world("wiped");
        w.config(PERSONAL);
        let sid = scope(&w, &rivendell(), "personal", "file:///Users/a/Sync/bilbo");
        let (out, _) = w.ok(&["recover", "--name", "rivendell-2"], true, answers_of(0));
        let id = keys::read_identity(&w.keys()).unwrap().unwrap();
        assert_eq!(out.lines[0], format!("owner recovered: {ABANDON_FP}"));
        assert_eq!(
            out.lines[1],
            format!("device created: rivendell-2 {}", id.device.id())
        );
        assert_eq!(
            out.lines[2],
            format!("scope personal updated: {sid} manifest 2 epoch 1")
        );
        let (list, _) = w.ok(&["list"], false, vec![]);
        assert_eq!(list.lines.len(), 2);
        assert!(list.lines[0].starts_with("rivendell\t") && !list.lines[0].ends_with("this"));
        assert!(list.lines[1].starts_with("rivendell-2\t") && list.lines[1].ends_with("\tthis"));
    }

    #[test]
    fn a_fresh_machine_reports_its_scope_unsealed_and_makes_no_id() {
        let w = world("fresh");
        w.config(PERSONAL);
        let mut answers = answers_of(0);
        answers.push(Answer::Yes);
        let (out, _) = w.ok(&["recover", "--name", "bagend"], true, answers);
        assert_eq!(out.lines.len(), 3);
        assert_eq!(out.lines[2], format!("scope personal unsealed: {UNSEALED}"));
        assert!(!out.lines[2].contains("init"));
        assert!(!store::scopes_dir(&w.root()).exists());
        assert!(!out.failed);
    }

    #[test]
    fn an_interrupted_recover_is_finished() {
        let w = world("interrupted");
        w.config(&format!("{PERSONAL}scope.shared.sync = file:///x\n"));
        let personal = scope(&w, &rivendell(), "personal", "file:///Users/a/Sync/bilbo");
        let shared = scope(&w, &rivendell(), "shared", "file:///x");
        let lock = manifest::lock(&w.root()).unwrap();
        let owner = Owner::derive(&[0; 16]);
        let known = known_as(&w, &bagend());
        let first = known.iter().find(|k| k.scope.id == personal).unwrap();
        let out = manifest::recover_step(&lock, &bagend(), &owner.box_secret, first);
        assert!(matches!(out, Outcome::Updated(_)));
        drop(lock);
        w.enroll(&bagend());
        let (out, _) = w.ok(&["recover"], true, answers_of(0));
        assert_eq!(out.lines[0], format!("owner kept: {ABANDON_FP}"));
        assert!(out.lines[1].starts_with("device kept: bagend "));
        assert!(
            out.lines
                .contains(&format!("scope personal kept: {personal}"))
        );
        assert!(out.lines.contains(&format!(
            "scope shared updated: {shared} manifest 2 epoch 1"
        )));
    }

    #[test]
    fn a_scope_of_another_owner_is_kept_and_a_cannot_open_scope_is_a_dash() {
        let (w, _) = pair();
        let foreign = scope(&w, &identity(1, "gandalf", 5), "grey", "file://");
        let (out, _) = w.ok(&["init"], true, vec![]);
        assert!(out.lines.contains(&format!("scope - kept: {foreign}")));
        let before = manifest::read_scope(&w.root(), &foreign)
            .unwrap()
            .versions
            .len();
        assert_eq!(before, 1);
    }

    // List, revoke

    #[test]
    fn list_prints_every_device_by_name() {
        let (w, _) = pair();
        let (out, _) = w.ok(&["list"], false, vec![]);
        assert_eq!(
            out.lines,
            [
                format!("bagend\t{}", bagend().device.id()),
                format!("rivendell\t{}\tthis", rivendell().device.id()),
            ]
        );
    }

    #[test]
    fn list_without_keys_names_init() {
        let w = world("list_none");
        let (code, m, _) = refusal(w.go(&["list"], false, vec![]));
        assert_eq!(code, 1);
        assert!(m.contains("bilbo device init"));
    }

    #[test]
    fn revoking_a_lost_laptop_writes_a_new_epoch() {
        let (w, sid) = pair();
        let (out, _) = w.ok(&["revoke", "bagend"], true, vec![]);
        assert_eq!(
            out.lines,
            [format!("scope personal updated: {sid} manifest 3 epoch 2")]
        );
        assert!(!out.failed);
        let (list, _) = w.ok(&["list"], false, vec![]);
        assert_eq!(list.lines.len(), 1);
        assert!(list.lines[0].starts_with("rivendell\t"));
        let by_id = bagend().device.id();
        let (code, m, _) = refusal(w.go(&["revoke", &by_id], true, vec![]));
        assert_eq!((code, m), (1, format!("no device named {by_id}")));
    }

    #[test]
    fn a_device_cannot_revoke_itself() {
        let (w, _) = pair();
        let before = w.tree();
        let (code, m, _) = refusal(w.go(&["revoke", "rivendell"], true, vec![]));
        assert_eq!(code, 1);
        assert!(m.contains("cannot revoke itself"));
        assert_eq!(w.tree(), before);
    }

    #[test]
    fn an_unknown_device_is_refused_by_name() {
        let (w, _) = pair();
        let before = w.tree();
        let (code, m, _) = refusal(w.go(&["revoke", "mordor"], true, vec![]));
        assert_eq!((code, m.as_str()), (1, "no device named mordor"));
        assert_eq!(w.tree(), before);
    }

    #[test]
    fn revoke_needs_an_argument_and_keys() {
        let (w, _) = pair();
        let (code, _, _) = refusal(w.go(&["revoke"], true, vec![]));
        assert_eq!(code, 2);
        let (code, _, _) = refusal(w.go(&["revoke", "a", "b"], true, vec![]));
        assert_eq!(code, 2);
        let none = world("revoke_none");
        let (code, m, _) = refusal(none.go(&["revoke", "bagend"], true, vec![]));
        assert_eq!(code, 1);
        assert!(m.contains("no keys"));
    }

    #[test]
    fn a_name_two_devices_share_is_a_usage_error_listing_their_ids() {
        let (w, _) = pair();
        let twin = identity(0, "bagend", 7);
        scope(&w, &twin, "shared", "file://");
        join(&w, &rivendell());
        let (code, m, _) = refusal(w.go(&["revoke", "bagend"], true, vec![]));
        assert_eq!(code, 2);
        assert!(m.contains(&bagend().device.id()) && m.contains(&twin.device.id()));
    }

    #[test]
    fn a_folder_scope_gets_a_cloud_account_line_and_a_relay_scope_does_not() {
        let (w, _) = pair();
        scope(&w, &bagend(), "shared", RELAY);
        join(&w, &rivendell());
        let (out, _) = w.ok(&["revoke", "bagend"], true, vec![]);
        assert_eq!(out.lines.len(), 2);
        let cloud: Vec<&String> = out
            .warnings
            .iter()
            .filter(|l| l.contains("account"))
            .collect();
        assert_eq!(cloud.len(), 1);
        assert!(cloud[0].contains("personal") && cloud[0].contains("bagend"));
        assert!(cloud[0].contains("write access"));
        assert!(!out.warnings.iter().any(|l| l.contains("shared")));
    }
    // Golden fixtures for tests/device.rs

    /// Test keys: the all-`abandon` owner and two devices with fixed seeds. They guard nothing; never enroll them.
    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/device");

    /// Copies `from/*.json` to `to`, leaving the pending markers and anything hidden behind.
    fn copy_versions(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "json") {
                fs::copy(&path, to.join(path.file_name().unwrap())).unwrap();
            }
        }
    }

    /// Rebuilds `tests/fixtures/device/`: `rivendell/` and `bagend/` (key folders), `store/` (a `personal` scope pinned
    /// to `file://`, versions 1 and 2) and `foreign/` (a store with a scope of another owner). Ids and nonces are random, so a run
    /// replaces every file.
    #[test]
    #[ignore]
    fn write_fixtures() {
        let out = PathBuf::from(FIXTURES);
        let _ = fs::remove_dir_all(&out);
        let (w, sid) = pair();
        for who in [rivendell(), bagend()] {
            let k = world("fixture_keys");
            k.enroll(&who);
            let to = out.join(&who.device.name);
            fs::create_dir_all(&to).unwrap();
            for file in ["owner.key", "device.key"] {
                fs::copy(k.keys().join(file), to.join(file)).unwrap();
            }
        }
        copy_versions(
            &w.root().join(format!(".bilbo/scopes/{sid}/manifest")),
            &out.join(format!("store/.bilbo/scopes/{sid}/manifest")),
        );
        let other = world("fixture_foreign");
        let fid = scope(
            &other,
            &identity(1, "gandalf", 5),
            "grey",
            "file:///srv/grey",
        );
        copy_versions(
            &other.root().join(format!(".bilbo/scopes/{fid}/manifest")),
            &out.join(format!("foreign/.bilbo/scopes/{fid}/manifest")),
        );
    }

    fn fixture_scopes(dir: &Path) -> Vec<String> {
        manifest::scope_ids(dir).unwrap()
    }

    #[test]
    fn fixtures_open() {
        let store = PathBuf::from(FIXTURES).join("store");
        let ids = fixture_scopes(&store);
        assert_eq!(ids.len(), 1);
        let abandon = Owner::derive(&[0; 16]).sign.public();
        for (name, seed) in [("rivendell", 1), ("bagend", 3)] {
            let w = world("fixtures");
            let state = w.keys();
            fs::create_dir_all(&state).unwrap();
            fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
            for file in ["owner.key", "device.key"] {
                let to = state.join(file);
                fs::copy(PathBuf::from(FIXTURES).join(name).join(file), &to).unwrap();
                fs::set_permissions(&to, fs::Permissions::from_mode(0o600)).unwrap();
            }
            let id = keys::read_identity(&state).unwrap().unwrap();
            assert_eq!(id.device.name, name);
            assert_eq!(id.device.id(), identity(0, name, seed).device.id());
            assert_eq!(id.owner.sign.public(), abandon);
            copy_versions(
                &store.join(format!(".bilbo/scopes/{}/manifest", ids[0])),
                &w.root().join(format!(".bilbo/scopes/{}/manifest", ids[0])),
            );
            let known = known_as(&w, &id);
            assert_eq!(known.len(), 1);
            let k = &known[0];
            assert!(k.problem.is_none(), "{:?}", k.problem);
            assert!(k.mine && k.opened.is_some());
            assert_eq!(k.name(), Some("personal"));
            let latest = k.scope.latest().unwrap();
            assert_eq!(latest.manifest.n, 2);
            assert_eq!(latest.manifest.devices.len(), 2);
            assert_eq!(latest.manifest.transport, "file://");
        }
        let foreign = PathBuf::from(FIXTURES).join("foreign");
        let ids = fixture_scopes(&foreign);
        assert_eq!(ids.len(), 1);
        let scope = manifest::read_scope(&foreign, &ids[0]).unwrap();
        assert_eq!(scope.versions.len(), 1);
        assert_eq!(scope.owner(), Some(Owner::derive(&[1; 16]).sign.public()));
    }
}
