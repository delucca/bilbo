//! The device that joins: answers a code, then checks and writes what the showing device sends.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Instant, SystemTime};

use super::{Cx, interval, wait};
use crate::Failure;
use crate::identity::keys::{self, Device, OwnerFile, SignKey};
use crate::identity::manifest::{self, Recipient};
use crate::identity::pake::{self, Code, Hello, Outcome, Payload, Refusal, Reply};
use crate::search::documents;
use crate::shared::{config, hash, store};
use crate::sync::scopes;
use crate::sync::transport::{self, Put, Transport};

const NEWER: &str = "the other device runs a newer bilbo; update this one and pair again";

/// This device as it answers: its keys, and its owner's signing public key when it is enrolled.
struct Me {
    device: Device,
    owner: Option<[u8; 32]>,
}

/// A scope the other device sent, as fetched and checked: its versions 1 to `n`, as the transport holds them.
struct Checked {
    name: String,
    id: String,
    embedder: String,
    files: Vec<Vec<u8>>,
}

/// What `fetch` checked: the scopes, and the owner's signing and box public keys.
struct Fetched {
    scopes: Vec<Checked>,
    owner: [u8; 32],
    owner_box: [u8; 32],
}

/// Answers `code` on the transport at `via`, as `name` when this device has no keys yet.
pub fn run(cx: &mut Cx, code: &str, via: &str, name: Option<&str>) -> Result<(), Failure> {
    let started = Instant::now();
    let code = Code::parse(code).map_err(Failure::Usage)?;
    let folder = folder_of(via)?;
    let settings = config::load(cx.env).map_err(Failure::Config)?;
    let config_path = config::path(cx.env)
        .map_err(Failure::Config)?
        .map(|(path, _)| path)
        .ok_or_else(|| {
            Failure::Config("no config file: set BILBO_CONFIG, XDG_CONFIG_HOME or HOME".into())
        })?;
    let root = store::root(cx.env).map_err(Failure::Config)?;
    if !root.join("notes").is_dir() {
        return Err(Failure::Refused(format!("no store at {}", root.display())));
    }
    if !Path::new(folder).is_dir() {
        return Err(Failure::Refused(format!("no folder at {folder}")));
    }
    let keys = store::keys_dir(cx.env)
        .ok_or_else(|| Failure::Config("no state folder: set XDG_STATE_HOME or HOME".into()))?;
    let pending = keys::pending_path(&keys);
    let me = match keys::read_identity(&keys).map_err(Failure::Refused)? {
        Some(id) => {
            if name.is_some_and(|n| n != id.device.name) {
                return Err(Failure::Usage(
                    "--name cannot rename an enrolled device".into(),
                ));
            }
            Me {
                owner: Some(id.owner.sign.public()),
                device: id.device,
            }
        }
        None => {
            let name = name.map(String::from).or_else(keys::host_name);
            let name = name.ok_or_else(|| {
                Failure::Refused(
                    "the host name holds no letter or digit to name this device: pass --name <name>"
                        .into(),
                )
            })?;
            if !keys::valid_name(&name) {
                return Err(Failure::Usage(format!("'{name}' is not a device name")));
            }
            Me {
                device: keys::pending_device(&pending, &name).map_err(Failure::Refused)?,
                owner: None,
            }
        }
    };
    let id = me.device.id();
    let signer = transport::Keys {
        device: &me.device,
        owner: None,
        opener: false,
    };
    let t = transport::open(via, &signer).map_err(Failure::Refused)?;
    t.sweep_mailboxes(SystemTime::now(), cx.limits.sweep)
        .map_err(Failure::Refused)?;
    let np = code.nameplate();
    let every = interval(via, cx.limits);
    let a_path = transport::message_path(&np, "a");
    let a_msg = wait(Instant::now(), cx.limits.appear, every, || t.get(&a_path))
        .map_err(Failure::Refused)?
        .ok_or_else(|| Failure::Refused(format!("no pairing {np} at {via}")))?;
    let hello = Hello {
        name: me.device.name.clone(),
        sign: me.device.sign.public(),
        box_public: me.device.box_secret.public(),
        owner: me.owner,
    };
    let (session, b_msg) = pake::answer(&code, &a_msg, &hello, &me.device.sign).map_err(refused)?;
    match t.create(&transport::message_path(&np, "b"), &b_msg) {
        Put::Created => {}
        Put::Exists => {
            return Err(Failure::Refused(format!(
                "code {np} was already used; run bilbo pair again on the other device"
            )));
        }
        Put::Full(why) | Put::Unreachable(why) => return Err(Failure::Refused(why)),
    }
    (cx.err)(&format!(
        "fingerprint {} for {} {id}; confirm on the device that showed the code",
        session.fingerprint(),
        me.device.name
    ));
    let c_path = transport::message_path(&np, "c");
    let c_msg = wait(started, cx.limits.window, every, || t.get(&c_path))
        .map_err(Failure::Refused)?
        .ok_or_else(|| Failure::Refused("no answer from the other device".into()))?;
    let reply = match pake::read_reply(&session, &c_msg) {
        Ok(reply) => reply,
        Err(Refusal::WrongCode) => {
            return Err(Failure::Refused(
                "the reply of the other device does not open; run bilbo pair again on the other device for a new code"
                    .into(),
            ));
        }
        Err(refusal) => return Err(refused(refusal)),
    };
    if !matches!(reply, Reply::Ended(Outcome::WrongCode)) {
        let _ = t.remove_mailbox(&np);
    }
    let received = Instant::now();
    let payload = match reply {
        Reply::Enrolled(payload) => payload,
        Reply::OtherOwner(theirs) => {
            let Some(own) = me.owner.map(|o| keys::owner_fingerprint(&o)) else {
                return Err(refused(Refusal::Malformed(
                    "an other-owner reply to a device with no owner".into(),
                )));
            };
            return Err(Failure::Refused(format!(
                "this device belongs to owner {own}, the other device to {}",
                keys::owner_fingerprint(&theirs)
            )));
        }
        Reply::Ended(outcome) => return Err(Failure::Refused(ended(outcome, &me.device.name))),
    };
    let Fetched {
        scopes: checked,
        owner,
        owner_box,
    } = fetch(cx, &*t, via, &me, &payload, received)?;
    agree_with_store(&root, &owner, &checked)?;
    let lock = manifest::lock(&root).map_err(Failure::Refused)?;
    for scope in &checked {
        let held = manifest::read_scope(lock.root(), &scope.id)
            .map_err(Failure::Refused)?
            .versions
            .len();
        for (i, bytes) in scope.files.iter().enumerate().skip(held) {
            manifest::adopt(&lock, &scope.id, i as u64 + 1, bytes).map_err(Failure::Refused)?;
        }
    }
    drop(lock);
    let lines = lines(&checked, via, &settings);
    config::set_keys(&config_path, &lines).map_err(Failure::Refused)?;
    if me.owner.is_none() {
        let seed = payload.seed.as_ref().expect("fetch checked the seed");
        let file = OwnerFile {
            sign: SignKey::from_seed(seed),
            box_public: owner_box,
        };
        keys::write_identity(&keys, &file, &me.device).map_err(Failure::Refused)?;
        if let Err(why) = keys::remove_pending(&pending) {
            (cx.err)(&why);
        }
    }
    let notes = documents::read_notes(&root.join("notes")).unwrap_or_default();
    for scope in &checked {
        let n = notes
            .iter()
            .filter(|note| note.scope.as_deref() == Some(scope.name.as_str()))
            .count();
        if n > 0 {
            (cx.err)(&format!(
                "{n} notes already carry scope: {} and sync from now on",
                scope.name
            ));
        }
    }
    let names: Vec<&str> = checked.iter().map(|s| s.name.as_str()).collect();
    (cx.out)(&format!(
        "paired with {}: {}",
        payload.name,
        names.join(", ")
    ));
    (cx.out)("bilbo watch starts syncing them within one cycle");
    Ok(())
}

/// The folder `via` names. A URL this bilbo cannot reach, or one a config line could not hold, is refused.
fn folder_of(via: &str) -> Result<&str, Failure> {
    if let Some(path) = via.strip_prefix("file://") {
        let plain = path.starts_with('/')
            && !path.contains(['?', '#'])
            && !path.chars().any(char::is_control);
        return if plain {
            Ok(path)
        } else {
            Err(Failure::Usage(format!(
                "{via} is not a file:// URL with an absolute path"
            )))
        };
    }
    if via.starts_with("https://") || (via.starts_with("http://") && config::is_local(via)) {
        let scheme = via.split("://").next().unwrap_or(via);
        return Err(Failure::Refused(format!(
            "this bilbo cannot reach {scheme}:// transports yet"
        )));
    }
    if via.starts_with("http://") {
        return Err(Failure::Usage(format!(
            "{via} is plain http:// on another host; use https:// or a file:// folder"
        )));
    }
    Err(Failure::Usage(format!(
        "{via} is not a transport URL; use file:///<absolute path>"
    )))
}

fn refused(refusal: Refusal) -> Failure {
    Failure::Refused(match refusal {
        Refusal::Newer => NEWER.into(),
        Refusal::WrongCode => "the message of the other device does not open".into(),
        Refusal::Malformed(why) => format!("the message of the other device is not valid: {why}"),
    })
}

/// What `outcome` tells the user, for a result that carries nothing.
fn ended(outcome: Outcome, name: &str) -> String {
    match outcome {
        Outcome::WrongCode => {
            "wrong code; run bilbo pair again on the other device for a new one".into()
        }
        Outcome::Declined => "the other device declined; nothing was received".into(),
        Outcome::Expired => "the code expired on the other device; nothing was received".into(),
        Outcome::NameTaken => format!("the name {name} is taken; run bilbo pair again with --name"),
        Outcome::OtherOwner | Outcome::Enrolled => {
            "the other device answered with a result this bilbo cannot use".into()
        }
    }
}

fn mismatch(name: &str) -> Failure {
    Failure::Refused(format!(
        "the {name} manifests on the transport do not match what the other device sent; nothing was written"
    ))
}

/// Fetches and checks each scope of `payload`, and returns them with the owner's signing and box public keys.
fn fetch(
    cx: &Cx,
    t: &dyn Transport,
    via: &str,
    me: &Me,
    payload: &Payload,
    received: Instant,
) -> Result<Fetched, Failure> {
    let owner = match (&me.owner, &payload.seed) {
        (Some(own), None) => *own,
        (Some(own), Some(seed)) if SignKey::from_seed(seed).public() == *own => *own,
        (None, Some(seed)) => SignKey::from_seed(seed).public(),
        _ => {
            return Err(Failure::Refused(
                "the other device sent an owner key this device cannot use; nothing was written"
                    .into(),
            ));
        }
    };
    if payload.scopes.is_empty() {
        return Err(Failure::Refused(
            "the other device sent no scopes; nothing was written".into(),
        ));
    }
    let mut seen = BTreeSet::new();
    for grant in &payload.scopes {
        if grant.url.is_some() {
            return Err(Failure::Refused(
                "this bilbo cannot reach https:// transports yet".into(),
            ));
        }
        let unique = seen.insert(grant.name.as_str()) & seen.insert(grant.id.as_str());
        if !unique || !keys::is_id(&grant.id) || grant.n == 0 {
            return Err(mismatch(&grant.name));
        }
    }
    let every = interval(via, cx.limits);
    let mut checked = Vec::new();
    let mut owner_box = None;
    for grant in &payload.scopes {
        let found = wait(received, cx.limits.manifests, every, || {
            let scope = scopes::chain(t, &grant.id)?;
            Ok((scope.versions.len() as u64 >= grant.n).then_some(scope))
        })
        .map_err(Failure::Refused)?;
        let Some(mut scope) = found else {
            let arrived = matches!(
                t.get(&transport::manifest_path(&grant.id, grant.n)),
                Ok(Some(_))
            );
            return Err(if arrived {
                mismatch(&grant.name)
            } else {
                Failure::Refused(format!(
                    "the {} manifest did not reach {via} in time; run bilbo pair again",
                    grant.name
                ))
            });
        };
        scope.versions.truncate(grant.n as usize);
        let latest = scope.latest().expect("n is at least 1");
        let me_id = me.device.id();
        let listed = latest.manifest.devices.iter().any(|d| {
            d.id == me_id
                && d.sign == keys::hex(&me.device.sign.public())
                && d.box_key == keys::hex(&me.device.box_secret.public())
        });
        let opened = manifest::open(&scope, &Recipient::device(&me.device));
        let box_key = keys::unhex::<32>(&latest.manifest.owner_box);
        let sound = hash::sha256(&latest.bytes) == grant.hash
            && scope.owner() == Some(owner)
            && listed
            && opened.is_ok_and(|o| o.is_some_and(|o| o.n == grant.n && o.name == grant.name))
            && box_key.is_some()
            && owner_box.is_none_or(|first| Some(first) == box_key);
        if !sound {
            return Err(mismatch(&grant.name));
        }
        owner_box = box_key;
        checked.push(Checked {
            name: grant.name.clone(),
            id: grant.id.clone(),
            embedder: grant.embedder.clone(),
            files: scope.versions.iter().map(|v| v.bytes.clone()).collect(),
        });
    }
    let owner_box = owner_box.expect("a payload holds a scope");
    Ok(Fetched {
        scopes: checked,
        owner,
        owner_box,
    })
}

/// Refuses a store with a scope of another owner, or whose versions of a scope sent are not the transport's.
fn agree_with_store(root: &Path, owner: &[u8; 32], checked: &[Checked]) -> Result<(), Failure> {
    for id in manifest::scope_ids(root).map_err(Failure::Refused)? {
        let held = manifest::read_scope(root, &id).map_err(Failure::Refused)?;
        if let Some(theirs) = held.owner().filter(|o| o != owner) {
            return Err(Failure::Refused(format!(
                "the store at {} holds a scope of owner {}, but the other device's owner is {}; nothing was written",
                root.display(),
                keys::owner_fingerprint(&theirs),
                keys::owner_fingerprint(owner)
            )));
        }
    }
    for scope in checked {
        let held = manifest::read_scope(root, &scope.id).map_err(Failure::Refused)?;
        let same = held.invalid.is_none()
            && held
                .versions
                .iter()
                .zip(&scope.files)
                .all(|(version, file)| version.bytes == *file);
        if !same {
            return Err(Failure::Refused(format!(
                "the {} manifests in this store differ from those on the transport; nothing was written",
                scope.name
            )));
        }
    }
    Ok(())
}

/// The config lines pairing sets: each scope's transport, and `local` where either device asks for it.
fn lines(checked: &[Checked], via: &str, settings: &config::Settings) -> Vec<(String, String)> {
    let mut lines = Vec::new();
    for scope in checked {
        lines.push((format!("scope.{}.sync", scope.name), via.to_string()));
        let local = settings
            .scope(&scope.name)
            .is_some_and(|s| s.embedder == config::Rule::Local);
        if scope.embedder == "local" && !local {
            lines.push((format!("scope.{}.embedder", scope.name), "local".into()));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use zeroize::Zeroizing;

    use super::*;
    use crate::identity::keys::{Identity, Owner};
    use crate::identity::manifest::Member;
    use crate::identity::pair::Limits;
    use crate::identity::pake::{Grant, Session};
    use crate::sync::transport::Folder;

    const CODE: &str = "42-orbit-tunnel-velvet";

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
            std::env::temp_dir().join(format!("bilbo-join-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for sub in ["b/root/notes", "b/home", "sync", "a/root"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
        }
        World(dir)
    }

    impl World {
        fn sync(&self) -> PathBuf {
            self.0.join("sync")
        }

        fn via(&self) -> String {
            format!("file://{}", self.sync().display())
        }

        fn root(&self) -> PathBuf {
            self.0.join("b/root")
        }

        fn keys(&self) -> PathBuf {
            self.0.join("b/state/bilbo/keys")
        }

        fn pending(&self) -> PathBuf {
            keys::pending_path(&self.keys())
        }

        fn config(&self) -> PathBuf {
            self.0.join("b/config/bilbo/config")
        }

        fn write_config(&self, text: &str) {
            fs::create_dir_all(self.config().parent().unwrap()).unwrap();
            fs::write(self.config(), text).unwrap();
        }

        fn env(&self) -> store::Env {
            store::Env {
                bilbo_home: Some(self.root().into()),
                xdg_data_home: None,
                home: Some(self.0.join("b/home").into()),
                bilbo_config: None,
                xdg_config_home: Some(self.0.join("b/config").into()),
                xdg_cache_home: None,
                xdg_state_home: Some(self.0.join("b/state").into()),
                claudecode: None,
                codex_thread_id: None,
            }
        }

        fn folder(&self) -> Folder {
            Folder::new(self.sync(), "a")
        }
    }

    fn rivendell() -> Identity {
        Identity {
            owner: Owner::derive(&[0; 16]).file(),
            device: Device::from_seeds("rivendell", &[1; 32], &[2; 32]),
        }
    }

    fn bagend() -> Device {
        Device::from_seeds("bagend", &[3; 32], &[4; 32])
    }

    /// The scope `personal` as device A holds it: version 1, and version 2 listing `b` when there is one.
    struct Scope {
        id: String,
        versions: Vec<Vec<u8>>,
    }

    fn scope_for(w: &World, a: &Identity, b: Option<&Device>) -> Scope {
        let root = w.0.join("a/root");
        let lock = manifest::lock(&root).unwrap();
        let written = manifest::create(&lock, a, "personal", "file:///a", &[]).unwrap();
        if let Some(b) = b {
            let scope = manifest::read_scope(&root, &written.scope).unwrap();
            let opened = manifest::open(&scope, &Recipient::device(&a.device))
                .unwrap()
                .unwrap();
            let key = &opened.keys[&opened.epoch];
            manifest::add_device(&lock, &scope, key, &Member::of(b), a).unwrap();
        }
        let read = manifest::read_scope(&root, &written.scope).unwrap();
        Scope {
            id: written.scope,
            versions: read.versions.iter().map(|v| v.bytes.clone()).collect(),
        }
    }

    /// Puts the first `upto` versions on the transport.
    fn publish(w: &World, scope: &Scope, upto: usize) {
        for (i, bytes) in scope.versions.iter().take(upto).enumerate() {
            let path = transport::manifest_path(&scope.id, i as u64 + 1);
            assert_eq!(w.folder().create(&path, bytes), Put::Created);
        }
    }

    fn grant(scope: &Scope, n: usize, embedder: &str) -> Grant {
        Grant {
            name: "personal".into(),
            id: scope.id.clone(),
            embedder: embedder.into(),
            n: n as u64,
            hash: hash::sha256(&scope.versions[n - 1]),
            url: None,
        }
    }

    fn payload(a: &Identity, grants: Vec<Grant>, seed: bool) -> Payload {
        Payload {
            name: "rivendell".into(),
            id: a.device.id(),
            seed: seed.then(|| a.owner.sign.seed()),
            scopes: grants,
        }
    }

    fn enrolled(payload: &Payload, session: &Session) -> Vec<u8> {
        pake::reply(session, Outcome::Enrolled, Some(payload), None).unwrap()
    }

    fn limits() -> Limits {
        Limits {
            window: Duration::from_secs(5),
            appear: Duration::from_secs(5),
            manifests: Duration::from_secs(5),
            poll_file: Duration::from_millis(2),
            poll_https: Duration::from_millis(2),
            sweep: Duration::from_secs(30 * 60),
        }
    }

    fn quick() -> Limits {
        Limits {
            window: Duration::from_millis(300),
            appear: Duration::from_millis(100),
            manifests: Duration::from_millis(100),
            ..limits()
        }
    }

    struct Run {
        result: Result<(), (u8, String)>,
        out: Vec<String>,
        err: Vec<String>,
    }

    impl Run {
        fn refused(&self) -> &str {
            match &self.result {
                Err((1, message)) => message,
                other => panic!("not a refusal: {other:?} {:?}", self.err),
            }
        }

        fn usage(&self) -> &str {
            match &self.result {
                Err((2, message)) => message,
                other => panic!("not a usage error: {other:?}"),
            }
        }
    }

    fn failed(failure: Failure) -> (u8, String) {
        match failure {
            Failure::Usage(message) | Failure::Config(message) => (2, message),
            Failure::Refused(message) => (1, message),
        }
    }

    fn join(w: &World, code: &str, via: &str, name: Option<&str>, limits: &Limits) -> Run {
        let env = w.env();
        let mut answer = std::io::empty();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = {
            let mut put_out = |line: &str| out.push(line.to_string());
            let mut put_err = |line: &str| err.push(line.to_string());
            let mut cx = Cx {
                env: &env,
                human: false,
                answer: &mut answer,
                limits,
                out: &mut put_out,
                err: &mut put_err,
            };
            run(&mut cx, code, via, name)
        };
        Run {
            result: result.map_err(failed),
            out,
            err,
        }
    }

    /// What device A heard of the answer.
    type Heard = Result<(Session, Hello), Refusal>;

    fn show(w: &World) -> pake::Shown {
        let (shown, a_msg) = pake::show(&Code::parse(CODE).unwrap()).unwrap();
        assert_eq!(
            w.folder()
                .create(&transport::message_path("42", "a"), &a_msg),
            Put::Created
        );
        shown
    }

    fn send(w: &World, bytes: &[u8]) {
        let path = transport::message_path("42", "c");
        assert_eq!(w.folder().create(&path, bytes), Put::Created);
    }

    /// Shows the code, runs B on a thread, and hands `act` what A heard once B has answered.
    fn pair(
        w: &World,
        code: &str,
        name: Option<&str>,
        limits: &Limits,
        act: impl FnOnce(Heard),
    ) -> Run {
        let shown = show(w);
        std::thread::scope(|s| {
            let b = s.spawn(|| join(w, code, &w.via(), name, limits));
            let deadline = Instant::now() + Duration::from_secs(10);
            let b_msg = loop {
                if let Some(b_msg) = w.folder().get(&transport::message_path("42", "b")).unwrap() {
                    break b_msg;
                }
                if b.is_finished() {
                    let run = b.join().unwrap();
                    panic!("B ended without answering: {:?} {:?}", run.result, run.err);
                }
                assert!(Instant::now() < deadline, "B never answered");
                std::thread::sleep(Duration::from_millis(2));
            };
            act(pake::receive(shown, &b_msg));
            b.join().unwrap()
        })
    }

    fn mismatch_text(name: &str) -> String {
        format!(
            "the {name} manifests on the transport do not match what the other device sent; nothing was written"
        )
    }

    /// A fresh device `mirkwood` listed in `personal` version 2, which is on the transport.
    struct Fresh {
        w: World,
        a: Identity,
        b: Device,
        scope: Scope,
    }

    fn fresh(name: &str) -> Fresh {
        let w = world(name);
        let a = rivendell();
        let b = keys::pending_device(&w.pending(), "mirkwood").unwrap();
        let scope = scope_for(&w, &a, Some(&b));
        publish(&w, &scope, 2);
        Fresh { w, a, b, scope }
    }

    fn nothing_written(w: &World, config: Option<&str>) {
        assert!(!w.keys().exists());
        assert!(manifest::scope_ids(&w.root()).unwrap().is_empty());
        assert_eq!(fs::read_to_string(w.config()).ok().as_deref(), config);
        assert!(!w.config().with_file_name("config.bak").exists());
    }

    #[test]
    fn a_device_without_keys_joins() {
        let f = fresh("joins");
        f.w.write_config("embedder.url = http://127.0.0.1:8081\nembedder.model = m\n");
        let mut heard = None;
        let run = pair(&f.w, CODE, Some("mirkwood"), &limits(), |h| {
            let (session, hello) = h.unwrap();
            let grants = vec![grant(&f.scope, 2, "any")];
            send(&f.w, &enrolled(&payload(&f.a, grants, true), &session));
            heard = Some((session.fingerprint(), hello));
        });
        let (fingerprint, hello) = heard.unwrap();
        assert!(run.result.is_ok(), "{:?} {:?}", run.result, run.err);
        assert_eq!(
            run.out,
            [
                "paired with rivendell: personal",
                "bilbo watch starts syncing them within one cycle"
            ]
        );
        assert_eq!(
            run.err,
            [format!(
                "fingerprint {fingerprint} for mirkwood {}; confirm on the device that showed the code",
                f.b.id()
            )]
        );
        assert_eq!((hello.name.as_str(), hello.owner), ("mirkwood", None));
        assert_eq!(hello.sign, f.b.sign.public());
        let id = keys::read_identity(&f.w.keys()).unwrap().unwrap();
        assert_eq!(id.device.id(), f.b.id());
        assert_eq!(id.device.name, "mirkwood");
        assert_eq!(id.owner.sign.public(), f.a.owner.sign.public());
        assert_eq!(id.owner.box_public, f.a.owner.box_public);
        assert!(!f.w.pending().exists());
        let held = manifest::read_scope(&f.w.root(), &f.scope.id).unwrap();
        assert_eq!(held.versions.len(), 2);
        assert!(held.pending.is_empty());
        assert!(
            !f.w.root()
                .join(format!(".bilbo/scopes/{}/manifest/2.pending", f.scope.id))
                .exists()
        );
        let text = fs::read_to_string(f.w.config()).unwrap();
        assert_eq!(
            text,
            format!(
                "embedder.url = http://127.0.0.1:8081\nembedder.model = m\nscope.personal.sync = {}\n",
                f.w.via()
            )
        );
        assert!(f.w.config().with_file_name("config.bak").exists());
        assert!(!f.w.sync().join("pair/42").exists());
    }

    #[test]
    fn a_loosely_typed_code_joins() {
        let f = fresh("loose");
        let run = pair(
            &f.w,
            "42 ORBI tunn velvet",
            Some("mirkwood"),
            &limits(),
            |h| {
                let (session, _) = h.unwrap();
                let grants = vec![grant(&f.scope, 2, "any")];
                send(&f.w, &enrolled(&payload(&f.a, grants, true), &session));
            },
        );
        assert!(run.result.is_ok(), "{:?}", run.result);
    }

    #[test]
    fn refusals_before_the_mailbox_touch_nothing() {
        let w = world("before");
        let via = w.via();
        let go = |code: &str, via: &str, name: Option<&str>| join(&w, code, via, name, &quick());
        let run = go("42-orbit-tunel-velvet", &via, None);
        assert!(run.usage().contains("'tunel' is not a pairing word"));
        let run = go(CODE, "http://bagend:8090", Some("mirkwood"));
        assert!(run.usage().contains("http://bagend:8090"));
        for url in ["https://relay.example", "http://127.0.0.1:8090"] {
            let run = go(CODE, url, Some("mirkwood"));
            let scheme = url.split("://").next().unwrap();
            assert_eq!(
                run.refused(),
                format!("this bilbo cannot reach {scheme}:// transports yet")
            );
        }
        for url in ["ftp://relay", "file://relative/path", "file:///a?b"] {
            let run = go(CODE, url, Some("mirkwood"));
            assert!(run.usage().contains(url), "{url}");
        }
        let run = go(CODE, "file:///nope", Some("mirkwood"));
        assert_eq!(run.refused(), "no folder at /nope");
        let run = go(CODE, &via, Some("Bag_End"));
        assert!(run.usage().contains("'Bag_End' is not a device name"));
        fs::remove_dir(w.root().join("notes")).unwrap();
        let run = go(CODE, &via, Some("mirkwood"));
        assert_eq!(run.refused(), format!("no store at {}", w.root().display()));
        assert!(!w.sync().join("pair").exists());
        assert!(!w.keys().exists() && !w.pending().exists());
    }

    #[test]
    fn a_missing_mailbox_is_reported_and_the_code_stays() {
        let w = world("nomailbox");
        show(&w);
        let run = join(
            &w,
            "43-orbit-tunnel-velvet",
            &w.via(),
            Some("mirkwood"),
            &quick(),
        );
        assert_eq!(run.refused(), format!("no pairing 43 at {}", w.via()));
        assert!(w.sync().join("pair/42/a.msg").exists());
        assert!(!w.sync().join("pair/42/b.msg").exists());
    }

    #[test]
    fn a_newer_code_writes_no_answer() {
        let w = world("newer");
        let path = transport::message_path("42", "a");
        let created = w.folder().create(&path, br#"{"format":2,"spake":"00"}"#);
        assert_eq!(created, Put::Created);
        let run = join(&w, CODE, &w.via(), Some("mirkwood"), &quick());
        assert_eq!(run.refused(), NEWER);
        assert!(!w.sync().join("pair/42/b.msg").exists());
    }

    #[test]
    fn a_wrong_code_keeps_the_mailbox_and_burns_the_code() {
        let w = world("wrong");
        let run = pair(
            &w,
            "42-orbit-tunnel-vessel",
            Some("mirkwood"),
            &limits(),
            |h| {
                assert_eq!(h.err(), Some(Refusal::WrongCode));
                send(&w, &pake::plain_reply(Outcome::WrongCode).unwrap());
            },
        );
        assert_eq!(
            run.refused(),
            "wrong code; run bilbo pair again on the other device for a new one"
        );
        assert!(run.out.is_empty());
        assert!(w.sync().join("pair/42").exists());
        nothing_written(&w, None);
        assert!(w.pending().join("device.key").exists());
        let again = join(&w, CODE, &w.via(), Some("mirkwood"), &quick());
        assert_eq!(
            again.refused(),
            "code 42 was already used; run bilbo pair again on the other device"
        );
    }

    #[test]
    fn a_result_without_a_secret_ends_the_pairing_and_removes_the_mailbox() {
        for (outcome, text) in [
            (
                Outcome::Declined,
                "the other device declined; nothing was received",
            ),
            (
                Outcome::Expired,
                "the code expired on the other device; nothing was received",
            ),
            (
                Outcome::NameTaken,
                "the name mirkwood is taken; run bilbo pair again with --name",
            ),
        ] {
            let f = fresh("plain");
            let run = pair(&f.w, CODE, Some("mirkwood"), &limits(), |h| {
                let (session, _) = h.unwrap();
                send(&f.w, &pake::reply(&session, outcome, None, None).unwrap());
            });
            assert_eq!(run.refused(), text);
            assert!(run.out.is_empty());
            assert!(!f.w.sync().join("pair/42").exists());
            assert!(!f.w.keys().exists());
            assert!(manifest::scope_ids(&f.w.root()).unwrap().is_empty());
        }
    }

    #[test]
    fn an_enrolled_device_of_another_owner_sees_both_fingerprints() {
        let f = fresh("other-owner");
        let theirs = Owner::derive(&[1; 16]);
        keys::write_identity(&f.w.keys(), &theirs.file(), &bagend()).unwrap();
        let ours = f.a.owner.sign.public();
        let mut owner = None;
        let run = pair(&f.w, CODE, None, &limits(), |h| {
            let (session, hello) = h.unwrap();
            owner = hello.owner;
            let bytes = pake::reply(&session, Outcome::OtherOwner, None, Some(&ours)).unwrap();
            send(&f.w, &bytes);
        });
        assert_eq!(owner, Some(theirs.sign.public()));
        assert_eq!(
            run.refused(),
            format!(
                "this device belongs to owner {}, the other device to {}",
                keys::owner_fingerprint(&theirs.sign.public()),
                keys::owner_fingerprint(&ours)
            )
        );
        assert!(!f.w.sync().join("pair/42").exists());
        assert!(manifest::scope_ids(&f.w.root()).unwrap().is_empty());
    }

    #[test]
    fn an_other_owner_reply_to_a_device_without_an_owner_is_malformed() {
        let f = fresh("no-owner-reply");
        let ours = f.a.owner.sign.public();
        let run = pair(&f.w, CODE, None, &limits(), |h| {
            let (session, _) = h.unwrap();
            let bytes = pake::reply(&session, Outcome::OtherOwner, None, Some(&ours)).unwrap();
            send(&f.w, &bytes);
        });
        assert_eq!(
            run.refused(),
            "the message of the other device is not valid: an other-owner reply to a device with no owner"
        );
        nothing_written(&f.w, None);
    }

    #[test]
    fn no_answer_in_the_window_leaves_the_mailbox() {
        let w = world("silent");
        let limits = Limits {
            window: Duration::from_millis(150),
            ..limits()
        };
        let run = pair(&w, CODE, Some("mirkwood"), &limits, |h| {
            h.unwrap();
        });
        assert_eq!(run.refused(), "no answer from the other device");
        assert!(w.sync().join("pair/42").exists());
        nothing_written(&w, None);
    }

    #[test]
    fn a_manifest_that_never_arrives_writes_nothing() {
        let f = fresh("never");
        fs::remove_file(
            f.w.sync()
                .join(format!("scopes/{}/manifest/2.json", f.scope.id)),
        )
        .unwrap();
        let run = pair(&f.w, CODE, Some("mirkwood"), &quick(), |h| {
            let (session, _) = h.unwrap();
            let grants = vec![grant(&f.scope, 2, "any")];
            send(&f.w, &enrolled(&payload(&f.a, grants, true), &session));
        });
        assert_eq!(
            run.refused(),
            format!(
                "the personal manifest did not reach {} in time; run bilbo pair again",
                f.w.via()
            )
        );
        nothing_written(&f.w, None);
    }

    #[test]
    fn a_late_manifest_is_waited_for() {
        let f = fresh("late");
        fs::remove_dir_all(f.w.sync().join("scopes")).unwrap();
        let run = pair(&f.w, CODE, Some("mirkwood"), &limits(), |h| {
            let (session, _) = h.unwrap();
            let grants = vec![grant(&f.scope, 2, "any")];
            send(&f.w, &enrolled(&payload(&f.a, grants, true), &session));
            std::thread::sleep(Duration::from_millis(80));
            publish(&f.w, &f.scope, 2);
        });
        assert!(run.result.is_ok(), "{:?}", run.result);
        assert_eq!(
            manifest::read_scope(&f.w.root(), &f.scope.id)
                .unwrap()
                .versions
                .len(),
            2
        );
    }

    /// Runs a pairing whose payload `tamper` changes, with the transport as `fresh` left it.
    fn tampered(name: &str, tamper: impl FnOnce(&Fresh, &mut Payload)) -> (Fresh, Run) {
        let f = fresh(name);
        let run = pair(&f.w, CODE, Some("mirkwood"), &quick(), |h| {
            let (session, _) = h.unwrap();
            let mut sent = payload(&f.a, vec![grant(&f.scope, 2, "any")], true);
            tamper(&f, &mut sent);
            send(&f.w, &enrolled(&sent, &session));
        });
        (f, run)
    }

    #[test]
    fn manifests_that_do_not_match_what_was_sent_write_nothing() {
        let mismatch = mismatch_text("personal");
        let (f, run) = tampered("hash", |_, sent| sent.scopes[0].hash[0] ^= 1);
        assert_eq!(run.refused(), mismatch);
        nothing_written(&f.w, None);
        let (f, run) = tampered("seed", |_, sent| {
            sent.seed = Some(Owner::derive(&[1; 16]).sign.seed());
        });
        assert_eq!(run.refused(), mismatch);
        nothing_written(&f.w, None);
        let (f, run) = tampered("id", |_, sent| sent.scopes[0].id = "../x".into());
        assert_eq!(
            run.refused(),
            "the message of the other device is not valid: the payload is not valid"
        );
        nothing_written(&f.w, None);
        let (f, run) = tampered("version", |_, sent| sent.scopes[0].n = 3);
        assert_eq!(
            run.refused(),
            format!(
                "the personal manifest did not reach {} in time; run bilbo pair again",
                f.w.via()
            )
        );
        nothing_written(&f.w, None);
        let (f, run) = tampered("gap", |f, _| {
            fs::remove_file(
                f.w.sync()
                    .join(format!("scopes/{}/manifest/1.json", f.scope.id)),
            )
            .unwrap();
        });
        assert_eq!(run.refused(), mismatch);
        nothing_written(&f.w, None);
        let (f, run) = tampered("damaged", |f, _| {
            let path =
                f.w.sync()
                    .join(format!("scopes/{}/manifest/2.json", f.scope.id));
            fs::write(path, b"{}").unwrap();
        });
        assert_eq!(run.refused(), mismatch);
        nothing_written(&f.w, None);
    }

    #[test]
    fn a_version_that_does_not_list_this_device_is_refused() {
        let w = world("unlisted");
        let a = rivendell();
        keys::pending_device(&w.pending(), "mirkwood").unwrap();
        let scope = scope_for(&w, &a, None);
        publish(&w, &scope, 1);
        let run = pair(&w, CODE, Some("mirkwood"), &quick(), |h| {
            let (session, _) = h.unwrap();
            let grants = vec![grant(&scope, 1, "any")];
            send(&w, &enrolled(&payload(&a, grants, true), &session));
        });
        assert_eq!(run.refused(), mismatch_text("personal"));
        nothing_written(&w, None);
    }

    #[test]
    fn a_relay_url_in_the_payload_is_refused_before_anything_is_written() {
        let (f, run) = tampered("relay", |_, sent| {
            sent.scopes[0].url = Some("https://relay.example".into());
        });
        assert_eq!(
            run.refused(),
            "this bilbo cannot reach https:// transports yet"
        );
        nothing_written(&f.w, None);
    }

    #[test]
    fn a_store_of_another_owner_is_refused_with_both_fingerprints() {
        let f = fresh("foreign");
        let theirs = Identity {
            owner: Owner::derive(&[1; 16]).file(),
            device: Device::from_seeds("gandalf", &[5; 32], &[6; 32]),
        };
        let lock = manifest::lock(&f.w.root()).unwrap();
        manifest::create(&lock, &theirs, "grey", "file:///grey", &[]).unwrap();
        drop(lock);
        let run = pair(&f.w, CODE, Some("mirkwood"), &limits(), |h| {
            let (session, _) = h.unwrap();
            let grants = vec![grant(&f.scope, 2, "any")];
            send(&f.w, &enrolled(&payload(&f.a, grants, true), &session));
        });
        let message = run.refused();
        for owner in [&theirs.owner.sign.public(), &f.a.owner.sign.public()] {
            assert!(
                message.contains(&keys::owner_fingerprint(owner)),
                "{message}"
            );
        }
        assert!(message.ends_with("nothing was written"), "{message}");
        assert!(!f.w.keys().exists() && !f.w.config().exists());
        assert_eq!(manifest::scope_ids(&f.w.root()).unwrap().len(), 1);
    }

    #[test]
    fn the_embedder_rule_only_tightens() {
        for (before, grant_rule, lines) in [
            (
                "embedder.url = http://127.0.0.1:8081\nembedder.model = m\nscope.personal.sync = off\n# mine\n",
                "local",
                "scope.personal.embedder = local",
            ),
            ("scope.personal.embedder = local\n", "any", ""),
            ("", "any", ""),
        ] {
            let f = fresh("embedder");
            f.w.write_config(before);
            let run = pair(&f.w, CODE, Some("mirkwood"), &limits(), |h| {
                let (session, _) = h.unwrap();
                let grants = vec![grant(&f.scope, 2, grant_rule)];
                send(&f.w, &enrolled(&payload(&f.a, grants, true), &session));
            });
            assert!(run.result.is_ok(), "{:?}", run.result);
            let text = fs::read_to_string(f.w.config()).unwrap();
            let sync = format!("scope.personal.sync = {}", f.w.via());
            assert!(text.contains(&sync), "{text}");
            assert!(!text.contains("scope.personal.sync = off"), "{text}");
            assert!(!text.contains("paths"), "{text}");
            if before.starts_with("embedder.url") {
                assert_eq!(
                    text,
                    format!(
                        "embedder.url = http://127.0.0.1:8081\nembedder.model = m\n{sync}\n# mine\n{lines}\n"
                    )
                );
            }
            assert_eq!(
                text.matches("scope.personal.embedder").count(),
                usize::from(!lines.is_empty() || before.contains("embedder = local"))
            );
            assert_eq!(
                fs::read_to_string(f.w.config().with_file_name("config.bak")).unwrap(),
                before
            );
        }
    }

    #[test]
    fn notes_that_already_carry_the_scope_are_counted_before_the_stdout_lines() {
        let f = fresh("notes");
        let head = "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4D01\ncreated: 2026-10-02T14:23-03:00\n";
        for (file, scope) in [
            ("plan-a.md", "scope: personal\n"),
            ("plan-b.md", "scope: personal\n"),
            ("plan-c.md", "scope: other\n"),
            ("plan-d.md", ""),
        ] {
            let text = format!("{head}{scope}---\n\n# A\n\nbody\n");
            fs::write(f.w.root().join("notes").join(file), text).unwrap();
        }
        let run = pair(&f.w, CODE, Some("mirkwood"), &limits(), |h| {
            let (session, _) = h.unwrap();
            let grants = vec![grant(&f.scope, 2, "any")];
            send(&f.w, &enrolled(&payload(&f.a, grants, true), &session));
        });
        assert!(run.result.is_ok(), "{:?}", run.result);
        assert_eq!(
            run.err.last().map(String::as_str),
            Some("2 notes already carry scope: personal and sync from now on")
        );
        assert_eq!(run.out.len(), 2);
    }

    #[test]
    fn an_unwritable_config_leaves_no_keys_and_pairing_again_keeps_the_id() {
        let f = fresh("config");
        let blocker = f.w.config().parent().unwrap().to_path_buf();
        fs::create_dir_all(blocker.parent().unwrap()).unwrap();
        fs::write(&blocker, "a file").unwrap();
        let reply = |h: Heard| {
            let (session, _) = h.unwrap();
            let grants = vec![grant(&f.scope, 2, "any")];
            send(&f.w, &enrolled(&payload(&f.a, grants, true), &session));
        };
        let run = pair(&f.w, CODE, Some("mirkwood"), &limits(), reply);
        assert!(
            run.refused().starts_with("cannot write "),
            "{:?}",
            run.result
        );
        assert!(run.refused().contains("config"), "{:?}", run.result);
        assert!(run.out.is_empty());
        assert!(!f.w.keys().exists());
        assert!(f.w.pending().join("device.key").exists());
        fs::remove_file(&blocker).unwrap();
        let run = pair(&f.w, CODE, Some("mirkwood"), &limits(), reply);
        assert!(run.result.is_ok(), "{:?}", run.result);
        let id = keys::read_identity(&f.w.keys()).unwrap().unwrap();
        assert_eq!(id.device.id(), f.b.id());
        assert!(!f.w.pending().exists());
        let held = manifest::read_scope(&f.w.root(), &f.scope.id).unwrap();
        assert_eq!(held.versions.len(), 2);
    }

    #[test]
    fn a_leftover_keys_new_is_cleared_by_the_write() {
        let f = fresh("leftover");
        let staging = keys::staging_path(&f.w.keys());
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("stale.key"), "x").unwrap();
        let run = pair(&f.w, CODE, Some("mirkwood"), &limits(), |h| {
            let (session, _) = h.unwrap();
            let grants = vec![grant(&f.scope, 2, "any")];
            send(&f.w, &enrolled(&payload(&f.a, grants, true), &session));
        });
        assert!(run.result.is_ok(), "{:?}", run.result);
        assert!(keys::leftover(&f.w.keys()).is_none());
        assert!(keys::read_identity(&f.w.keys()).unwrap().is_some());
    }

    #[test]
    fn an_enrolled_device_joins_another_scope_and_keeps_its_keys() {
        let w = world("enrolled");
        let a = rivendell();
        let b = bagend();
        keys::write_identity(&w.keys(), &Owner::derive(&[0; 16]).file(), &b).unwrap();
        let before = (
            fs::read(w.keys().join("owner.key")).unwrap(),
            fs::read(w.keys().join("device.key")).unwrap(),
        );
        let scope = scope_for(&w, &a, Some(&b));
        publish(&w, &scope, 2);
        let rename = join(&w, CODE, &w.via(), Some("mirkwood"), &quick());
        assert_eq!(rename.usage(), "--name cannot rename an enrolled device");
        let mut owner = None;
        let run = pair(&w, CODE, Some("bagend"), &limits(), |h| {
            let (session, hello) = h.unwrap();
            owner = hello.owner;
            let grants = vec![grant(&scope, 2, "any")];
            send(&w, &enrolled(&payload(&a, grants, false), &session));
        });
        assert!(run.result.is_ok(), "{:?}", run.result);
        assert_eq!(owner, Some(a.owner.sign.public()));
        let after = (
            fs::read(w.keys().join("owner.key")).unwrap(),
            fs::read(w.keys().join("device.key")).unwrap(),
        );
        assert_eq!(before, after);
        assert!(!w.pending().exists());
        let text = fs::read_to_string(w.config()).unwrap();
        assert_eq!(text, format!("scope.personal.sync = {}\n", w.via()));
        assert_eq!(
            manifest::read_scope(&w.root(), &scope.id)
                .unwrap()
                .versions
                .len(),
            2
        );
    }

    #[test]
    fn an_enrolled_device_refuses_a_seed_that_is_not_its_owner() {
        let w = world("enrolled-seed");
        let a = rivendell();
        let b = bagend();
        keys::write_identity(&w.keys(), &Owner::derive(&[0; 16]).file(), &b).unwrap();
        let scope = scope_for(&w, &a, Some(&b));
        publish(&w, &scope, 2);
        let run = pair(&w, CODE, None, &quick(), |h| {
            let (session, _) = h.unwrap();
            let mut sent = payload(&a, vec![grant(&scope, 2, "any")], false);
            sent.seed = Some(Zeroizing::new(*Owner::derive(&[1; 16]).sign.seed()));
            send(&w, &enrolled(&sent, &session));
        });
        assert!(run.refused().contains("owner key"), "{:?}", run.result);
        assert!(manifest::scope_ids(&w.root()).unwrap().is_empty());
    }
}
