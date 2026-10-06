//! The device that shows the code: claims a mailbox, waits for the answer, asks the user, and enrolls the device that
//! answered.

use std::collections::BTreeSet;
use std::time::{Instant, SystemTime};

use zeroize::Zeroizing;

use super::{Cx, interval, wait};
use crate::Failure;
use crate::identity::keys::{self, Identity};
use crate::identity::manifest::{self, Known, Member, Recipient};
use crate::identity::pake::{self, Code, Grant, Hello, Outcome, Payload, Refusal};
use crate::shared::{config, hash, store};
use crate::sync::transport::{self, Put, Transport};

/// How many nameplates A tries before it gives up.
const CLAIMS: usize = 20;

const EXPIRED: &str = "the code expired; nothing was sent";

/// A scope this pairing carries, as A holds it.
struct Chosen {
    name: String,
    id: String,
    /// `any` or `local`.
    embedder: &'static str,
    /// The newest version A would have once B is listed, and its hash as far as the size of the message goes.
    n: u64,
    hash: [u8; 32],
}

/// What the checks before the mailbox settle.
struct Plan {
    id: Identity,
    root: std::path::PathBuf,
    /// The URL the pairing runs over, as written.
    url: String,
    scopes: Vec<Chosen>,
}

/// A claimed mailbox.
struct Mailbox {
    t: Box<dyn Transport>,
    nameplate: String,
    url: String,
}

impl Mailbox {
    fn put(&self, msg: &str, bytes: &[u8]) -> Result<(), String> {
        match self
            .t
            .create(&transport::message_path(&self.nameplate, msg), bytes)
        {
            Put::Created => Ok(()),
            Put::Exists => Err(format!("{msg}.msg already exists")),
            Put::Full(why) | Put::Unreachable(why) => Err(why),
        }
    }
}

/// Shows a code for `scopes` (every syncing scope when empty) over `via` (the one URL they share when `None`).
pub fn run(cx: &mut Cx, scopes: &[String], via: Option<&str>) -> Result<(), Failure> {
    let plan = check(cx, scopes, via)?;
    let keys = transport::Keys {
        opener: true,
        ..transport::Keys::of(&plan.id)
    };
    let t = transport::open(&plan.url, &keys).map_err(Failure::Refused)?;
    t.reachable()
        .map_err(|why| Failure::Refused(format!("cannot reach {}: {why}", plan.url)))?;
    t.sweep_mailboxes(SystemTime::now(), cx.limits.sweep)
        .map_err(Failure::Refused)?;
    let (code, shown) = claim(t.as_ref(), &plan.url)?;
    let shown_at = Instant::now();
    let mailbox = Mailbox {
        t,
        nameplate: code.nameplate(),
        url: plan.url.clone(),
    };
    (cx.err)(&format!("pairing code {}", code.text()));
    (cx.err)(&format!(
        "on the new device, run: bilbo pair {} --via {}",
        code.text(),
        plan.url
    ));
    (cx.err)("the code works once, for 10 minutes");

    let found = wait(
        shown_at,
        cx.limits.window,
        interval(&mailbox.url, cx.limits),
        || {
            mailbox
                .t
                .get(&transport::message_path(&mailbox.nameplate, "b"))
        },
    )
    .map_err(Failure::Refused)?;
    let Some(b_msg) = found else {
        let _ = mailbox.t.remove_mailbox(&mailbox.nameplate);
        return Err(Failure::Refused(EXPIRED.into()));
    };

    let (session, hello) = match pake::receive(shown, &b_msg) {
        Ok(got) => got,
        Err(Refusal::Newer) => {
            return Err(Failure::Refused(
                "the other device runs a newer bilbo; update this one and pair again".into(),
            ));
        }
        Err(Refusal::WrongCode) => {
            return Err(end(
                cx,
                &mailbox,
                pake::plain_reply(Outcome::WrongCode),
                "the other device used a wrong code; this code is used up, run bilbo pair again",
            ));
        }
        Err(Refusal::Malformed(why)) => {
            return Err(Failure::Refused(format!(
                "the other device sent a message that is not valid: {why}"
            )));
        }
    };
    let own = plan.id.owner.sign.public();
    if let Some(theirs) = hello.owner.filter(|theirs| *theirs != own) {
        return Err(end(
            cx,
            &mailbox,
            pake::reply(&session, Outcome::OtherOwner, None, Some(&own)),
            &format!(
                "{} belongs to another owner ({})",
                hello.name,
                keys::owner_fingerprint(&theirs)
            ),
        ));
    }
    let b_id = keys::device_id(&hello.sign);
    if taken(&plan, &hello, &b_id)? {
        return Err(end(
            cx,
            &mailbox,
            pake::reply(&session, Outcome::NameTaken, None, None),
            &format!(
                "a device named {} is already enrolled; pair again with --name on the new device",
                hello.name
            ),
        ));
    }

    let names: Vec<&str> = plan.scopes.iter().map(|s| s.name.as_str()).collect();
    (cx.err)(&format!("fingerprint {}", session.fingerprint()));
    (cx.err)(&format!(
        "pair {} {} into {}? compare the fingerprint on that device, then type y to confirm",
        hello.name,
        b_id,
        names.join(", ")
    ));
    let mut line = String::new();
    let read = cx.answer.read_line(&mut line);
    let confirmed = matches!(read, Ok(n) if n > 0)
        && matches!(line.trim().to_lowercase().as_str(), "y" | "yes");
    if shown_at.elapsed() >= cx.limits.window {
        return Err(end(
            cx,
            &mailbox,
            pake::reply(&session, Outcome::Expired, None, None),
            EXPIRED,
        ));
    }
    if !confirmed {
        return Err(end(
            cx,
            &mailbox,
            pake::reply(&session, Outcome::Declined, None, None),
            "not confirmed; nothing was sent",
        ));
    }

    let member = Member {
        id: b_id.clone(),
        name: hello.name.clone(),
        sign: hello.sign,
        box_public: hello.box_public,
    };
    let grants = enroll(&plan, &mailbox, &member).map_err(Failure::Refused)?;
    let payload = Payload {
        name: plan.id.device.name.clone(),
        id: plan.id.device.id(),
        seed: hello.owner.is_none().then(|| plan.id.owner.sign.seed()),
        scopes: grants,
    };
    let c_msg =
        pake::reply(&session, Outcome::Enrolled, Some(&payload), None).map_err(Failure::Refused)?;
    mailbox
        .put("c", &c_msg)
        .map_err(|why| {
            Failure::Refused(format!(
                "cannot answer the other device: {why}; {name} is listed in {scopes}: pair again to finish, or bilbo device revoke {name} to undo",
                name = hello.name,
                scopes = names.join(", ")
            ))
        })?;
    (cx.out)(&format!(
        "paired {} {}: {}",
        hello.name,
        b_id,
        names.join(", ")
    ));
    Ok(())
}

/// Sends a result that carries no secret and returns the failure that says why.
fn end(cx: &mut Cx, mailbox: &Mailbox, bytes: Result<Vec<u8>, String>, why: &str) -> Failure {
    if let Err(problem) = bytes.and_then(|bytes| mailbox.put("c", &bytes)) {
        (cx.err)(&format!("cannot answer the other device: {problem}"));
    }
    Failure::Refused(why.into())
}

/// A's checks before the mailbox, in the spec's order.
fn check(cx: &mut Cx, scopes: &[String], via: Option<&str>) -> Result<Plan, Failure> {
    let settings = config::load(cx.env).map_err(Failure::Config)?;
    let root = store::root(cx.env).map_err(Failure::Config)?;
    let keys_dir = store::keys_dir(cx.env)
        .ok_or_else(|| Failure::Config("no state folder: set XDG_STATE_HOME or HOME".into()))?;
    let id = keys::read_identity(&keys_dir)
        .map_err(Failure::Refused)?
        .ok_or_else(|| {
            Failure::Refused(
                "this device has no owner key; turn on sync for a scope first, which sets the recovery phrase"
                    .into(),
            )
        })?;
    if settings.scopes.iter().all(|s| s.sync == "off") {
        return Err(Failure::Refused(
            "no scope syncs; set scope.<name>.sync and run bilbo device first".into(),
        ));
    }
    let mut wanted = BTreeSet::new();
    for name in scopes {
        match settings.scope(name) {
            Some(s) if s.sync != "off" => wanted.insert(name.as_str()),
            _ => return Err(Failure::Usage(format!("scope {name} does not sync"))),
        };
    }
    let paired: Vec<&config::Scope> = settings
        .scopes
        .iter()
        .filter(|s| s.sync != "off" && (wanted.is_empty() || wanted.contains(s.name.as_str())))
        .collect();
    if paired.len() > pake::SCOPES_MAX {
        return Err(Failure::Usage(format!(
            "one pairing carries at most {} scopes; name them with --scope",
            pake::SCOPES_MAX
        )));
    }
    let url = match via {
        Some(via) => {
            if !paired.iter().any(|s| s.sync == via) {
                return Err(Failure::Usage(format!(
                    "no scope paired syncs through {via}"
                )));
            }
            if let Some(other) = paired
                .iter()
                .find(|s| s.sync.starts_with("file://") && s.sync != via)
            {
                return Err(Failure::Usage(format!(
                    "scope {} syncs through {}, not {via}; pair it apart with --scope",
                    other.name, other.sync
                )));
            }
            via.to_string()
        }
        None => {
            let urls: BTreeSet<&str> = paired.iter().map(|s| s.sync.as_str()).collect();
            if urls.len() > 1 {
                return Err(Failure::Usage(format!(
                    "the scopes sync through different URLs ({}); pick the scopes with --scope and the URL with --via",
                    urls.into_iter().collect::<Vec<_>>().join(", ")
                )));
            }
            paired[0].sync.clone()
        }
    };
    if let Some(far) = std::iter::once(&url)
        .chain(paired.iter().map(|s| &s.sync))
        .find(|u| !u.starts_with("file://"))
    {
        let scheme = far.split("://").next().unwrap_or(far);
        return Err(Failure::Refused(format!(
            "this bilbo cannot reach {scheme}:// transports yet"
        )));
    }
    if !cx.human {
        return Err(Failure::Refused(
            "pairing is confirmed only in a terminal, by the user".into(),
        ));
    }

    let owner = id.owner.sign.public();
    let known = manifest::survey(&root, Some(&owner), Some(&Recipient::device(&id.device)))
        .map_err(Failure::Refused)?;
    let mut chosen = Vec::new();
    for scope in paired {
        let found = known.iter().find(|k| extendable(k, &scope.name));
        let Some(latest) = found.and_then(|k| k.scope.latest()) else {
            return Err(Failure::Refused(format!(
                "scope {} has no manifest this device can extend; run bilbo device",
                scope.name
            )));
        };
        chosen.push(Chosen {
            name: scope.name.clone(),
            id: found.map(|k| k.scope.id.clone()).unwrap_or_default(),
            embedder: scope.embedder.as_str(),
            n: latest.manifest.n + 1,
            hash: hash::sha256(&latest.bytes),
        });
    }
    fits(&id, &chosen)?;
    Ok(Plan {
        id,
        root,
        url,
        scopes: chosen,
    })
}

/// Whether A can write a next version of the scope `name`: it is the owner's, valid, and A holds the latest epoch's key.
fn extendable(known: &Known, name: &str) -> bool {
    let (Some(opened), Some(latest)) = (&known.opened, known.scope.latest()) else {
        return false;
    };
    known.mine
        && known.scope.invalid.is_none()
        && opened.name == name
        && opened.keys.contains_key(&latest.manifest.epoch)
}

/// Refuses scopes whose payload, with the owner seed, would not fit one message.
fn fits(id: &Identity, chosen: &[Chosen]) -> Result<(), Failure> {
    let payload = Payload {
        name: id.device.name.clone(),
        id: id.device.id(),
        seed: Some(Zeroizing::new([0; 32])),
        scopes: chosen
            .iter()
            .map(|s| Grant {
                name: s.name.clone(),
                id: s.id.clone(),
                embedder: s.embedder.to_string(),
                n: s.n,
                hash: s.hash,
                url: None,
            })
            .collect(),
    };
    let sealed = || -> Result<Vec<u8>, String> {
        let code = Code::random()?;
        let (shown, a_msg) = pake::show(&code)?;
        let dummy = keys::SignKey::from_seed(&[1; 32]);
        let hello = Hello {
            name: "fit".into(),
            sign: dummy.public(),
            box_public: [0; 32],
            owner: None,
        };
        let (_, b_msg) = pake::answer(&code, &a_msg, &hello, &dummy)
            .map_err(|_| "cannot size the message".to_string())?;
        let (session, _) =
            pake::receive(shown, &b_msg).map_err(|_| "cannot size the message".to_string())?;
        pake::reply(&session, Outcome::Enrolled, Some(&payload), None)
    };
    sealed().map(drop).map_err(|why| {
        Failure::Usage(format!(
            "the scopes do not fit one pairing message ({why}); pair fewer with --scope"
        ))
    })
}

/// Claims a mailbox by creating its `a.msg`.
fn claim(t: &dyn Transport, url: &str) -> Result<(Code, pake::Shown), Failure> {
    for _ in 0..CLAIMS {
        let code = Code::random().map_err(Failure::Refused)?;
        let (shown, a_msg) = pake::show(&code).map_err(Failure::Refused)?;
        match t.create(&transport::message_path(&code.nameplate(), "a"), &a_msg) {
            Put::Created => return Ok((code, shown)),
            Put::Exists => {}
            Put::Full(why) | Put::Unreachable(why) => {
                return Err(Failure::Refused(format!(
                    "cannot create a pairing mailbox at {url}: {why}"
                )));
            }
        }
    }
    Err(Failure::Refused(format!(
        "no free pairing number at {url}; try again later"
    )))
}

/// Whether B's name is A's own or another device's in the latest version of any of the owner's scopes. A device listed
/// under B's own id does not take its own name.
fn taken(plan: &Plan, hello: &Hello, b_id: &str) -> Result<bool, Failure> {
    let owner = plan.id.owner.sign.public();
    let known = manifest::survey(
        &plan.root,
        Some(&owner),
        Some(&Recipient::device(&plan.id.device)),
    )
    .map_err(Failure::Refused)?;
    let own = (plan.id.device.name.as_str(), plan.id.device.id());
    let listed = known
        .iter()
        .filter(|k| k.mine)
        .filter_map(|k| k.scope.latest())
        .flat_map(|v| &v.manifest.devices)
        .map(|d| (d.name.as_str(), d.id.clone()));
    Ok(std::iter::once(own)
        .chain(listed)
        .any(|(name, id)| name == hello.name && id != b_id))
}

/// Lists B in each scope and puts every local version on the transport, then names what B fetches. The new version is
/// written locally, as pending, and to the transport before any reply.
fn enroll(plan: &Plan, mailbox: &Mailbox, member: &Member) -> Result<Vec<Grant>, String> {
    let lock = manifest::lock(&plan.root)?;
    let mut grants = Vec::new();
    for chosen in &plan.scopes {
        let mut scope = manifest::read_scope(&plan.root, &chosen.id)?;
        let opened = manifest::open(&scope, &Recipient::device(&plan.id.device))
            .map_err(|invalid| invalid.to_string())?;
        let lists = scope.latest().is_some_and(|v| v.manifest.lists(&member.id));
        if !lists {
            let (Some(opened), Some(latest)) = (opened, scope.latest()) else {
                return Err(format!(
                    "scope {} has no manifest this device can extend; run bilbo device",
                    chosen.name
                ));
            };
            let key = opened
                .keys
                .get(&latest.manifest.epoch)
                .ok_or_else(|| format!("this device holds no key for scope {}", chosen.name))?;
            manifest::add_device(&lock, &scope, key, member, &plan.id)?;
            scope = manifest::read_scope(&plan.root, &chosen.id)?;
        }
        if let Some(invalid) = &scope.invalid {
            return Err(format!("scope {} is not valid: {invalid}", chosen.name));
        }
        for version in &scope.versions {
            let path = transport::manifest_path(&scope.id, version.manifest.n);
            let moved = || {
                format!(
                    "the {} manifest on {} moved on; let bilbo watch catch up, then pair again",
                    chosen.name, mailbox.url
                )
            };
            match mailbox.t.create(&path, &version.bytes) {
                Put::Created => {}
                Put::Exists => match mailbox.t.get(&path)? {
                    Some(there) if there == version.bytes => {}
                    _ => return Err(moved()),
                },
                Put::Full(why) | Put::Unreachable(why) => return Err(why),
            }
        }
        let latest = scope.latest().ok_or("the scope has no version")?;
        grants.push(Grant {
            name: chosen.name.clone(),
            id: scope.id.clone(),
            embedder: chosen.embedder.to_string(),
            n: latest.manifest.n,
            hash: hash::sha256(&latest.bytes),
            url: None,
        });
    }
    Ok(grants)
}

#[cfg(test)]
mod tests {
    use super::super::{Limits, run as pair};
    use super::*;
    use crate::identity::keys::{Device, Owner};
    use crate::identity::pake::Reply;
    use std::fs;
    use std::io::{BufRead, Cursor, Read};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

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
            std::env::temp_dir().join(format!("bilbo-show-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for sub in ["root", "home", "sync"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
        }
        fs::write(dir.join("config"), "").unwrap();
        World(dir)
    }

    impl World {
        fn root(&self) -> PathBuf {
            self.0.join("root")
        }

        fn sync(&self) -> PathBuf {
            self.0.join("sync")
        }

        fn url(&self) -> String {
            format!("file://{}", self.sync().display())
        }

        fn keys(&self) -> PathBuf {
            self.0.join("state/bilbo/keys")
        }

        fn config(&self, text: &str) {
            fs::write(self.0.join("config"), text).unwrap();
        }

        fn env(&self, claudecode: Option<&str>) -> store::Env {
            store::Env {
                bilbo_home: Some(self.root().into()),
                xdg_data_home: None,
                home: Some(self.0.join("home").into()),
                bilbo_config: Some(self.0.join("config").into()),
                xdg_config_home: None,
                xdg_cache_home: None,
                xdg_state_home: Some(self.0.join("state").into()),
                claudecode: claudecode.map(Into::into),
                codex_thread_id: None,
            }
        }

        /// A as `rivendell`, enrolled with the scope `personal` that syncs through the folder.
        fn enrolled(&self) -> (Identity, String) {
            let id = rivendell();
            keys::write_identity(&self.keys(), &id.owner, &id.device).unwrap();
            self.config(&format!("scope.personal.sync = {}\n", self.url()));
            let scope = self.scope(&id, "personal", &[]);
            (id, scope)
        }

        fn scope(&self, id: &Identity, name: &str, others: &[Member]) -> String {
            let lock = manifest::lock(&self.root()).unwrap();
            manifest::create(&lock, id, name, &self.url(), others)
                .unwrap()
                .scope
        }

        fn versions(&self, scope: &str) -> u64 {
            manifest::read_scope(&self.root(), scope)
                .unwrap()
                .versions
                .len() as u64
        }

        fn mailboxes(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(self.sync().join("pair"))
                .map(|d| {
                    d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
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

    fn limits() -> Limits {
        Limits {
            window: Duration::from_secs(10),
            appear: Duration::from_secs(1),
            manifests: Duration::from_secs(1),
            poll_file: Duration::from_millis(5),
            poll_https: Duration::from_millis(5),
            sweep: Duration::from_secs(30 * 60),
        }
    }

    /// How the scripted B answers.
    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Right,
        WrongWord,
        Newer,
        /// Never answers.
        Silent,
        /// Answers, then plants its own `c.msg` before A can.
        Forged,
    }

    struct B {
        who: Identity,
        enrolled: bool,
        mode: Mode,
    }

    fn new_device(name: &str, seed: u8) -> B {
        B {
            who: identity(9, name, seed),
            enrolled: false,
            mode: Mode::Right,
        }
    }

    struct Ran {
        result: Result<(), Failure>,
        out: Vec<String>,
        err: Vec<String>,
        /// What B read from `c.msg`, `None` when it answered nothing or no `c.msg` came.
        reply: Option<Reply>,
        b_id: String,
    }

    impl Ran {
        fn refused(&self) -> String {
            match &self.result {
                Err(Failure::Refused(m)) => m.clone(),
                Err(Failure::Usage(m)) | Err(Failure::Config(m)) => m.clone(),
                Ok(()) => panic!("did not refuse; stdout {:?}", self.out),
            }
        }
    }

    fn code_line(lines: &mpsc::Receiver<String>) -> Option<Code> {
        while let Ok(line) = lines.recv_timeout(Duration::from_secs(5)) {
            if let Some(text) = line.strip_prefix("pairing code ") {
                return Some(Code::parse(text).unwrap());
            }
        }
        None
    }

    fn play(w: &World, b: &B, lines: mpsc::Receiver<String>) -> Option<Reply> {
        let code = code_line(&lines)?;
        if b.mode == Mode::Silent {
            return None;
        }
        let t = transport::Folder::new(w.sync(), "b");
        let a_path = transport::message_path(&code.nameplate(), "a");
        let mut a_msg = None;
        for _ in 0..1000 {
            a_msg = t.get(&a_path).unwrap();
            if a_msg.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let a_msg = a_msg?;
        let b_path = transport::message_path(&code.nameplate(), "b");
        if b.mode == Mode::Newer {
            assert_eq!(t.create(&b_path, br#"{"format":2}"#), Put::Created);
            return None;
        }
        let typed = if b.mode == Mode::WrongWord {
            let text = code.text();
            let (head, last) = text.rsplit_once('-').unwrap();
            let other = if last == "abandon" {
                "ability"
            } else {
                "abandon"
            };
            Code::parse(&format!("{head}-{other}")).unwrap()
        } else {
            code
        };
        let hello = Hello {
            name: b.who.device.name.clone(),
            sign: b.who.device.sign.public(),
            box_public: b.who.device.box_secret.public(),
            owner: b.enrolled.then(|| b.who.owner.sign.public()),
        };
        let (session, b_msg) = pake::answer(&typed, &a_msg, &hello, &b.who.device.sign).unwrap();
        assert_eq!(t.create(&b_path, &b_msg), Put::Created);
        let c_path = transport::message_path(&typed.nameplate(), "c");
        if b.mode == Mode::Forged {
            assert_eq!(t.create(&c_path, b"forged"), Put::Created);
            return None;
        }
        for _ in 0..400 {
            if let Some(c_msg) = t.get(&c_path).unwrap() {
                return Some(pake::read_reply(&session, &c_msg).unwrap());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }

    fn go_with(
        w: &World,
        args: &[&str],
        b: &B,
        answer: &mut dyn BufRead,
        limits: &Limits,
        terminal: bool,
        claudecode: Option<&str>,
    ) -> Ran {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let env = w.env(claudecode);
        let (tx, rx) = mpsc::channel();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = std::thread::scope(|s| {
            let player = s.spawn(|| play(w, b, rx));
            let result = {
                let mut on_out = |l: &str| out.push(l.to_string());
                let mut on_err = |l: &str| {
                    err.push(l.to_string());
                    let _ = tx.send(l.to_string());
                };
                pair(
                    &args,
                    &env,
                    terminal,
                    answer,
                    limits,
                    &mut on_out,
                    &mut on_err,
                )
            };
            drop(tx);
            (result, player.join().unwrap())
        });
        Ran {
            result: result.0,
            out,
            err,
            reply: result.1,
            b_id: b.who.device.id(),
        }
    }

    fn go(w: &World, args: &[&str], b: &B, answer: &str) -> Ran {
        go_with(
            w,
            args,
            b,
            &mut Cursor::new(answer.as_bytes().to_vec()),
            &limits(),
            true,
            None,
        )
    }

    #[test]
    fn a_confirmed_pairing_lists_b_and_sends_the_payload() {
        let w = world("happy");
        let (a, scope) = w.enrolled();
        let b = new_device("mirkwood", 20);
        let ran = go(&w, &[], &b, "y\n");
        assert!(ran.result.is_ok(), "{:?}", ran.refused());
        assert_eq!(ran.out, [format!("paired mirkwood {}: personal", ran.b_id)]);
        let code = ran
            .err
            .iter()
            .find_map(|l| l.strip_prefix("pairing code "))
            .unwrap();
        assert!(ran.err.contains(&format!(
            "on the new device, run: bilbo pair {code} --via {}",
            w.url()
        )));
        assert!(ran.err.iter().any(|l| l.starts_with("fingerprint ")));
        let Some(Reply::Enrolled(payload)) = ran.reply else {
            panic!("no payload");
        };
        assert_eq!(
            (payload.name.as_str(), payload.id),
            ("rivendell", a.device.id())
        );
        assert_eq!(payload.seed.as_deref(), Some(&*a.owner.sign.seed()));
        let grant = &payload.scopes[0];
        assert_eq!(
            (grant.name.as_str(), grant.id.as_str(), grant.n),
            ("personal", scope.as_str(), 2)
        );
        let local = manifest::read_scope(&w.root(), &scope).unwrap();
        let latest = local.latest().unwrap();
        assert!(latest.manifest.lists(&ran.b_id));
        assert_eq!(grant.hash, hash::sha256(&latest.bytes));
        assert!(
            local.pending.contains(&2),
            "the new version waits for the read-back"
        );
        let t = transport::Folder::new(w.sync(), "x");
        for n in [1, 2] {
            let there = t
                .get(&transport::manifest_path(&scope, n))
                .unwrap()
                .unwrap();
            assert_eq!(there, local.versions[n as usize - 1].bytes);
        }
        assert_eq!(w.mailboxes().len(), 1, "A leaves the mailbox to B");
    }

    #[test]
    fn the_mailbox_holds_no_scope_name_url_or_seed() {
        let w = world("opaque");
        let (a, _) = w.enrolled();
        let ran = go(&w, &[], &new_device("mirkwood", 20), "y\n");
        assert!(ran.result.is_ok());
        let seed = keys::hex(&*a.owner.sign.seed());
        let np = &w.mailboxes()[0];
        for msg in ["a", "b", "c"] {
            let text = fs::read_to_string(w.sync().join(format!("pair/{np}/{msg}.msg"))).unwrap();
            for secret in ["personal", w.url().as_str(), seed.as_str()] {
                assert!(!text.contains(secret), "{msg}.msg holds {secret}");
            }
        }
    }

    #[test]
    fn an_enrolled_device_of_the_same_owner_gets_no_seed() {
        let w = world("enrolled");
        let (a, _) = w.enrolled();
        let shared = w.scope(&a, "shared", &[]);
        w.config(&format!(
            "scope.personal.sync = {0}\nscope.shared.sync = {0}\n",
            w.url()
        ));
        let b = B {
            who: identity(0, "bagend", 3),
            enrolled: true,
            mode: Mode::Right,
        };
        let ran = go(&w, &["--scope", "shared"], &b, "yes\n");
        assert!(ran.result.is_ok(), "{:?}", ran.refused());
        assert_eq!(ran.out, [format!("paired bagend {}: shared", ran.b_id)]);
        let Some(Reply::Enrolled(payload)) = ran.reply else {
            panic!("no payload");
        };
        assert!(payload.seed.is_none());
        assert_eq!(payload.scopes.len(), 1);
        assert_eq!(w.versions(&shared), 2);
    }

    #[test]
    fn another_owner_is_refused_and_told_the_showing_owner() {
        let w = world("owner");
        let (a, scope) = w.enrolled();
        let b = B {
            who: identity(7, "bagend", 3),
            enrolled: true,
            mode: Mode::Right,
        };
        let ran = go(&w, &[], &b, "y\n");
        let theirs = keys::owner_fingerprint(&b.who.owner.sign.public());
        assert_eq!(
            ran.refused(),
            format!("bagend belongs to another owner ({theirs})")
        );
        let Some(Reply::OtherOwner(owner)) = ran.reply else {
            panic!("no other-owner reply");
        };
        assert_eq!(owner, a.owner.sign.public());
        assert_eq!(w.versions(&scope), 1);
    }

    #[test]
    fn a_name_in_use_anywhere_is_refused() {
        let w = world("names");
        let (a, scope) = w.enrolled();
        let bagend = Member::of(&identity(0, "bagend", 3).device);
        let other = w.scope(&a, "shared", &[bagend]);
        let own = go(&w, &[], &new_device("rivendell", 20), "y\n");
        assert!(
            own.refused()
                .contains("a device named rivendell is already enrolled")
        );
        assert!(matches!(own.reply, Some(Reply::Ended(Outcome::NameTaken))));
        let apart = go(
            &w,
            &["--scope", "personal"],
            &new_device("bagend", 21),
            "y\n",
        );
        assert_eq!(
            apart.refused(),
            "a device named bagend is already enrolled; pair again with --name on the new device"
        );
        assert!(matches!(
            apart.reply,
            Some(Reply::Ended(Outcome::NameTaken))
        ));
        assert_eq!((w.versions(&scope), w.versions(&other)), (1, 1));
    }

    #[test]
    fn pairing_again_after_an_interrupted_pairing_writes_no_new_version() {
        let w = world("again");
        let (_, scope) = w.enrolled();
        let b = new_device("mirkwood", 20);
        assert!(go(&w, &[], &b, "y\n").result.is_ok());
        let again = go(&w, &[], &b, "y\n");
        assert!(again.result.is_ok(), "{:?}", again.refused());
        assert!(matches!(again.reply, Some(Reply::Enrolled(_))));
        assert_eq!(w.versions(&scope), 2);
    }

    #[test]
    fn a_wrong_code_sends_nothing_and_leaves_the_mailbox() {
        let w = world("wrong");
        let (_, scope) = w.enrolled();
        let mut b = new_device("mirkwood", 20);
        b.mode = Mode::WrongWord;
        let ran = go(&w, &[], &b, "y\n");
        assert_eq!(
            ran.refused(),
            "the other device used a wrong code; this code is used up, run bilbo pair again"
        );
        assert!(matches!(ran.reply, Some(Reply::Ended(Outcome::WrongCode))));
        assert!(!ran.err.iter().any(|l| l.starts_with("fingerprint ")));
        assert_eq!(w.versions(&scope), 1);
        assert_eq!(w.mailboxes().len(), 1);
    }

    #[test]
    fn declining_or_ending_input_sends_nothing() {
        for (name, answer) in [("no", "n\n"), ("eof", ""), ("other", "maybe\n")] {
            let w = world(name);
            let (_, scope) = w.enrolled();
            let ran = go(&w, &[], &new_device("mirkwood", 20), answer);
            assert_eq!(ran.refused(), "not confirmed; nothing was sent", "{name}");
            assert!(matches!(ran.reply, Some(Reply::Ended(Outcome::Declined))));
            assert_eq!(w.versions(&scope), 1);
            assert!(ran.out.is_empty());
        }
    }

    /// A reader that answers only after `delay`.
    struct Late(Duration);

    impl Read for Late {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            std::thread::sleep(self.0);
            Cursor::new(b"y\n").read(buf)
        }
    }

    #[test]
    fn a_confirmation_after_the_window_sends_nothing() {
        let w = world("late");
        let (_, scope) = w.enrolled();
        let short = Limits {
            window: Duration::from_millis(400),
            ..limits()
        };
        let mut late = std::io::BufReader::new(Late(Duration::from_millis(600)));
        let ran = go_with(
            &w,
            &[],
            &new_device("mirkwood", 20),
            &mut late,
            &short,
            true,
            None,
        );
        assert_eq!(ran.refused(), EXPIRED);
        assert!(matches!(ran.reply, Some(Reply::Ended(Outcome::Expired))));
        assert_eq!(w.versions(&scope), 1);
    }

    #[test]
    fn an_unanswered_code_expires_and_removes_its_mailbox() {
        let w = world("expiry");
        w.enrolled();
        let short = Limits {
            window: Duration::from_millis(100),
            ..limits()
        };
        let mut b = new_device("mirkwood", 20);
        b.mode = Mode::Silent;
        let ran = go_with(
            &w,
            &[],
            &b,
            &mut Cursor::new(Vec::new()),
            &short,
            true,
            None,
        );
        assert_eq!(ran.refused(), EXPIRED);
        assert!(w.mailboxes().is_empty());
    }

    #[test]
    fn a_newer_format_gets_no_reply() {
        let w = world("newer");
        let (_, scope) = w.enrolled();
        let mut b = new_device("mirkwood", 20);
        b.mode = Mode::Newer;
        let ran = go(&w, &[], &b, "y\n");
        assert_eq!(
            ran.refused(),
            "the other device runs a newer bilbo; update this one and pair again"
        );
        let np = &w.mailboxes()[0];
        assert!(!w.sync().join(format!("pair/{np}/c.msg")).exists());
        assert_eq!(w.versions(&scope), 1);
    }

    #[test]
    fn local_versions_the_transport_lacks_are_published_before_the_reply() {
        let w = world("publish");
        let (_, scope) = w.enrolled();
        let t = transport::Folder::new(w.sync(), "x");
        assert!(
            t.get(&transport::manifest_path(&scope, 1))
                .unwrap()
                .is_none()
        );
        assert!(
            go(&w, &[], &new_device("mirkwood", 20), "y\n")
                .result
                .is_ok()
        );
        for n in [1, 2] {
            assert!(
                t.get(&transport::manifest_path(&scope, n))
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[test]
    fn a_manifest_that_moved_on_stops_before_the_reply() {
        let w = world("moved");
        let (_, scope) = w.enrolled();
        let t = transport::Folder::new(w.sync(), "x");
        assert_eq!(
            t.create(&transport::manifest_path(&scope, 2), b"another"),
            Put::Created
        );
        let ran = go(&w, &[], &new_device("mirkwood", 20), "y\n");
        assert_eq!(
            ran.refused(),
            format!(
                "the personal manifest on {} moved on; let bilbo watch catch up, then pair again",
                w.url()
            )
        );
        assert!(ran.reply.is_none());
        let np = &w.mailboxes()[0];
        assert!(!w.sync().join(format!("pair/{np}/c.msg")).exists());
    }

    #[test]
    fn a_c_msg_that_already_exists_says_b_is_listed() {
        let w = world("forged");
        let (_, scope) = w.enrolled();
        let mut b = new_device("mirkwood", 20);
        b.mode = Mode::Forged;
        let ran = go(&w, &[], &b, "y\n");
        assert_eq!(
            ran.refused(),
            format!(
                "cannot answer the other device: c.msg already exists; mirkwood is listed in personal: pair again to finish, or bilbo device revoke mirkwood to undo"
            )
        );
        assert_eq!(w.versions(&scope), 2);
    }

    #[test]
    fn a_stale_mailbox_is_swept_and_a_fresh_one_stays() {
        let w = world("sweep");
        w.enrolled();
        let t = transport::Folder::new(w.sync(), "x");
        for np in ["7", "8"] {
            assert_eq!(
                t.create(&transport::message_path(np, "a"), b"x"),
                Put::Created
            );
        }
        let old = SystemTime::now() - Duration::from_secs(31 * 60);
        fs::File::options()
            .write(true)
            .open(w.sync().join("pair/7/a.msg"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        let mut b = new_device("mirkwood", 20);
        b.mode = Mode::Silent;
        let short = Limits {
            window: Duration::from_millis(50),
            ..limits()
        };
        go_with(
            &w,
            &[],
            &b,
            &mut Cursor::new(Vec::new()),
            &short,
            true,
            None,
        );
        assert_eq!(w.mailboxes(), ["8"]);
    }

    /// Runs A against the refusal checks: no mailbox may exist after.
    fn refusal(w: &World, args: &[&str]) -> (bool, String) {
        let mut b = new_device("mirkwood", 20);
        b.mode = Mode::Silent;
        let ran = go(w, args, &b, "y\n");
        assert!(!w.sync().join("pair").exists(), "{args:?} made a mailbox");
        assert!(ran.out.is_empty() && ran.err.is_empty());
        (
            matches!(ran.result, Err(Failure::Refused(_))),
            ran.refused(),
        )
    }

    #[test]
    fn the_checks_before_the_mailbox() {
        let w = world("checks");
        let url = w.url();
        let id = rivendell();
        // No owner key.
        w.config(&format!("scope.personal.sync = {url}\n"));
        assert_eq!(
            refusal(&w, &[]),
            (
                true,
                "this device has no owner key; turn on sync for a scope first, which sets the recovery phrase"
                    .to_string()
            )
        );
        keys::write_identity(&w.keys(), &id.owner, &id.device).unwrap();
        // No syncing scope.
        w.config("scope.personal.sync = off\n");
        assert_eq!(
            refusal(&w, &[]),
            (
                true,
                "no scope syncs; set scope.<name>.sync and run bilbo device first".to_string()
            )
        );
        // A scope that does not sync.
        w.config(&format!(
            "scope.personal.sync = {url}\nscope.uber.sync = off\n"
        ));
        assert_eq!(
            refusal(&w, &["--scope", "uber"]),
            (false, "scope uber does not sync".into())
        );
        assert_eq!(
            refusal(&w, &["--scope", "nope"]),
            (false, "scope nope does not sync".into())
        );
        // Scopes in two folders, written two ways.
        w.config("scope.personal.sync = file:///srv/a\nscope.shared.sync = file:///srv/b\n");
        let (refused, message) = refusal(&w, &[]);
        assert!(!refused && message.contains("file:///srv/a") && message.contains("file:///srv/b"));
        assert!(message.contains("--via"));
        w.config("scope.personal.sync = file:///srv/sync/\nscope.shared.sync = file:///srv/sync\n");
        let (refused, message) = refusal(&w, &[]);
        assert!(
            !refused
                && message.contains("file:///srv/sync/")
                && message.contains("file:///srv/sync,")
        );
        // A --via no scope uses.
        w.config("scope.personal.sync = file:///srv/sync\n");
        assert_eq!(
            refusal(&w, &["--via", "file:///srv/other"]),
            (
                false,
                "no scope paired syncs through file:///srv/other".into()
            )
        );
        // Too many scopes.
        let many: String = (0..13)
            .map(|i| format!("scope.s{i}.sync = file:///srv/sync\n"))
            .collect();
        w.config(&many);
        let (refused, message) = refusal(&w, &[]);
        assert!(!refused && message.contains("at most 12") && message.contains("--scope"));
        // A relay.
        w.config("scope.personal.sync = https://relay.example\n");
        assert_eq!(
            refusal(&w, &[]),
            (
                true,
                "this bilbo cannot reach https:// transports yet".into()
            )
        );
        // A loopback relay names its own scheme.
        w.config("scope.personal.sync = http://127.0.0.1:8081\n");
        assert_eq!(
            refusal(&w, &[]),
            (
                true,
                "this bilbo cannot reach http:// transports yet".into()
            )
        );
        // A scope with no manifest this device can extend.
        w.config(&format!("scope.personal.sync = {url}\n"));
        assert_eq!(
            refusal(&w, &[]),
            (
                true,
                "scope personal has no manifest this device can extend; run bilbo device".into()
            )
        );
    }

    #[test]
    fn only_a_terminal_without_an_agent_marker_shows_a_code() {
        let w = world("terminal");
        w.enrolled();
        let message = "pairing is confirmed only in a terminal, by the user";
        let mut b = new_device("mirkwood", 20);
        b.mode = Mode::Silent;
        for (terminal, marker) in [(false, None), (true, Some("1"))] {
            let ran = go_with(
                &w,
                &[],
                &b,
                &mut Cursor::new(Vec::new()),
                &limits(),
                terminal,
                marker,
            );
            assert_eq!(ran.refused(), message);
            assert!(!w.sync().join("pair").exists());
        }
    }

    #[test]
    fn all_nameplates_taken_is_refused_without_a_new_mailbox() {
        let w = world("full");
        w.enrolled();
        let t = transport::Folder::new(w.sync(), "x");
        for np in 1..=999 {
            assert_eq!(
                t.create(&transport::message_path(&np.to_string(), "a"), b"x"),
                Put::Created
            );
        }
        let (refused, message) = refusal_keep(&w);
        assert!(refused);
        assert_eq!(
            message,
            format!("no free pairing number at {}; try again later", w.url())
        );
        assert_eq!(w.mailboxes().len(), 999);
    }

    fn refusal_keep(w: &World) -> (bool, String) {
        let mut b = new_device("mirkwood", 20);
        b.mode = Mode::Silent;
        let ran = go(w, &[], &b, "y\n");
        (
            matches!(ran.result, Err(Failure::Refused(_))),
            ran.refused(),
        )
    }

    #[test]
    fn scopes_too_long_for_one_message_are_a_usage_error() {
        let w = world("oversize");
        let (a, _) = w.enrolled();
        let long = "x".repeat(4000);
        w.scope(&a, &long, &[]);
        w.config(&format!(
            "scope.personal.sync = {0}\nscope.{long}.sync = {0}\n",
            w.url()
        ));
        let (refused, message) = refusal(&w, &["--scope", &long]);
        assert!(!refused);
        assert!(message.contains("do not fit one pairing message") && message.contains("--scope"));
    }

    #[test]
    fn the_grant_carries_the_scopes_embedder_rule() {
        let w = world("embedder");
        w.enrolled();
        w.config(&format!(
            "scope.personal.sync = {}\nscope.personal.embedder = local\n",
            w.url()
        ));
        let ran = go(&w, &[], &new_device("mirkwood", 20), "y\n");
        let Some(Reply::Enrolled(payload)) = ran.reply else {
            panic!("no payload");
        };
        assert_eq!(payload.scopes[0].embedder, "local");
    }
}
