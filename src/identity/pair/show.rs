//! The device that shows the code: claims a mailbox, waits for the answer, asks the user, and enrolls the device that
//! answered.

use std::collections::BTreeSet;
use std::time::{Instant, SystemTime};

use zeroize::Zeroizing;

use super::{Cx, interval, wait};
use crate::Failure;
use crate::host::prompt::Prompter;
use crate::host::terminal;
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
    /// The URL the scope syncs through on A, as written.
    url: String,
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
pub fn run(
    cx: &mut Cx,
    p: &mut impl Prompter,
    scopes: &[String],
    via: Option<&str>,
) -> Result<(), Failure> {
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
    let _ = p.intro("bilbo pair");
    let command = format!("bilbo pair {} --via {}", code.text(), plan.url);
    let line = format!(
        "The code is {}. It works once, for 10 minutes.",
        code.text()
    );
    // cliclack wraps a note's text, which would put the box border inside a command a person copies.
    if terminal::width_of(&command) + 6 <= cx.width {
        let _ = p.note("On the new device, run", &format!("{command}\n\n{line}"));
    } else {
        let _ = p.info(&format!("On the new device, run:\n{command}"));
        let _ = p.note("Pairing code", &line);
    }
    let result = converse(cx, p, &plan, &mailbox, shown, shown_at);
    if result.is_err() {
        let _ = p.cancel("Not paired");
    }
    result
}

/// What follows the box: waits for the new device, asks the user, and enrolls it.
fn converse(
    cx: &mut Cx,
    p: &mut impl Prompter,
    plan: &Plan,
    mailbox: &Mailbox,
    shown: pake::Shown,
    shown_at: Instant,
) -> Result<(), Failure> {
    let found = p
        .wait("Waiting for the new device", || {
            wait(
                shown_at,
                cx.limits.window,
                interval(&mailbox.url, cx.limits),
                || {
                    mailbox
                        .t
                        .get(&transport::message_path(&mailbox.nameplate, "b"))
                },
            )
        })
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
                mailbox,
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
            mailbox,
            pake::reply(&session, Outcome::OtherOwner, None, Some(&own)),
            &format!(
                "{} belongs to another owner ({})",
                hello.name,
                keys::owner_fingerprint(&theirs)
            ),
        ));
    }
    let b_id = keys::device_id(&hello.sign);
    if taken(plan, &hello, &b_id)? {
        return Err(end(
            cx,
            mailbox,
            pake::reply(&session, Outcome::NameTaken, None, None),
            &format!(
                "a device named {} is already enrolled; pair again with --name on the new device",
                hello.name
            ),
        ));
    }

    let names: Vec<&str> = plan.scopes.iter().map(|s| s.name.as_str()).collect();
    let _ = p.info(&format!(
        "{} {} asks to join {}",
        hello.name,
        b_id,
        names.join(", ")
    ));
    // Esc, Ctrl-C and the end of input decline like no does.
    let confirmed = matches!(
        p.confirm(
            &format!(
                "Fingerprint {}: does {} show the same?",
                session.fingerprint(),
                hello.name
            ),
            false
        ),
        Ok(true)
    );
    if shown_at.elapsed() >= cx.limits.window {
        return Err(end(
            cx,
            mailbox,
            pake::reply(&session, Outcome::Expired, None, None),
            EXPIRED,
        ));
    }
    if !confirmed {
        return Err(end(
            cx,
            mailbox,
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
    let grants = enroll(plan, mailbox, &member).map_err(Failure::Refused)?;
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
    let _ = p.outro("Paired");
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

/// A's checks before the mailbox, in the order the device-pairing spec gives them.
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
                .find(|s| s.sync != via && !transport::is_relay_url(&s.sync))
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
            url: scope.sync.clone(),
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
            Put::Full(why) => return Err(Failure::Refused(why)),
            Put::Unreachable(why) => {
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
        let own;
        let t: &dyn Transport = if chosen.url == mailbox.url {
            &*mailbox.t
        } else {
            own = transport::open(&chosen.url, &transport::Keys::of(&plan.id))?;
            &*own
        };
        for version in &scope.versions {
            let path = transport::manifest_path(&scope.id, version.manifest.n);
            let moved = || {
                format!(
                    "the {} manifest on {} moved on; let bilbo watch catch up, then pair again",
                    chosen.name, chosen.url
                )
            };
            match t.create(&path, &version.bytes) {
                Put::Created => {}
                Put::Exists => match t.get(&path)? {
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
            url: (chosen.url != mailbox.url).then(|| chosen.url.clone()),
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
    use crate::identity::script::{Answer, Script};
    use std::fs;
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

        fn env(&self, child: Option<&str>) -> store::Env {
            store::Env {
                bilbo_home: Some(self.root().into()),
                home: Some(self.0.join("home").into()),
                bilbo_config: Some(self.0.join("config").into()),
                xdg_state_home: Some(self.0.join("state").into()),
                claude_code_child_session: child.map(Into::into),
                ..store::Env::from_vars(|_| None)
            }
        }

        /// A as `rhosgobel`, enrolled with the scope `personal` that syncs through the folder.
        fn enrolled(&self) -> (Identity, String) {
            let id = rhosgobel();
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

    fn rhosgobel() -> Identity {
        identity(0, "rhosgobel", 1)
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
        /// What A's prompter showed, one entry per call.
        shown: Vec<String>,
        /// What B read from `c.msg`, `None` when it answered nothing or no `c.msg` came.
        reply: Option<Reply>,
        b_id: String,
    }

    impl Ran {
        fn refused(&self) -> String {
            match &self.result {
                Err(Failure::Refused(m) | Failure::Unmatched { message: m, .. }) => m.clone(),
                Err(Failure::Usage(m)) | Err(Failure::Config(m)) => m.clone(),
                Ok(()) => panic!("did not refuse; stdout {:?}", self.out),
            }
        }
    }

    /// The code in the command A shows, in its `note:` or, when the command is long, its `info:`.
    fn code_line(lines: &mpsc::Receiver<String>) -> Option<Code> {
        while let Ok(line) = lines.recv_timeout(Duration::from_secs(5)) {
            if let Some((_, rest)) = line.split_once("\nbilbo pair ") {
                return Some(Code::parse(rest.split(' ').next().unwrap()).unwrap());
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
        answers: Vec<Answer>,
        limits: &Limits,
        terminal: Option<usize>,
        child: Option<&str>,
    ) -> Ran {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let env = w.env(child);
        let (tx, rx) = mpsc::channel();
        let mut script = Script::tapped(answers, tx);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = std::thread::scope(|s| {
            let player = s.spawn(|| play(w, b, rx));
            let result = {
                let mut on_out = |l: &str| out.push(l.to_string());
                let mut on_err = |l: &str| err.push(l.to_string());
                pair(
                    &args,
                    &env,
                    terminal,
                    &mut script,
                    limits,
                    &mut on_out,
                    &mut on_err,
                )
            };
            let shown = std::mem::take(&mut script.shown);
            drop(script);
            (result, player.join().unwrap(), shown)
        });
        Ran {
            result: result.0,
            out,
            err,
            shown: result.2,
            reply: result.1,
            b_id: b.who.device.id(),
        }
    }

    fn go(w: &World, args: &[&str], b: &B, answers: Vec<Answer>) -> Ran {
        go_with(w, args, b, answers, &limits(), Some(WIDE), None)
    }

    /// Wide enough for any command a test folder makes.
    const WIDE: usize = 400;

    fn yes() -> Vec<Answer> {
        vec![Answer::Yes]
    }

    #[test]
    fn a_confirmed_pairing_lists_b_and_sends_the_payload() {
        let w = world("happy");
        let (a, scope) = w.enrolled();
        let b = new_device("mirkwood", 20);
        let ran = go(&w, &[], &b, yes());
        assert!(ran.result.is_ok(), "{:?}", ran.refused());
        assert_eq!(ran.out, [format!("paired mirkwood {}: personal", ran.b_id)]);
        assert!(
            ran.shown
                .iter()
                .any(|l| l.starts_with("confirm: Fingerprint "))
        );
        let Some(Reply::Enrolled(payload)) = ran.reply else {
            panic!("no payload");
        };
        assert_eq!(
            (payload.name.as_str(), payload.id),
            ("rhosgobel", a.device.id())
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
    fn a_pairing_on_a_terminal_is_drawn_in_order() {
        let w = world("drawn");
        w.enrolled();
        let ran = go(&w, &[], &new_device("mirkwood", 20), yes());
        assert!(ran.result.is_ok(), "{:?}", ran.refused());
        let code = ran
            .shown
            .iter()
            .find_map(|l| l.split_once("\nbilbo pair ")?.1.split(' ').next())
            .unwrap();
        let fingerprint = ran
            .shown
            .iter()
            .find_map(|l| {
                l.strip_prefix("confirm: Fingerprint ")?
                    .split(": does")
                    .next()
            })
            .unwrap();
        assert_eq!(
            ran.shown,
            [
                "intro: bilbo pair".to_string(),
                format!(
                    "note: On the new device, run\nbilbo pair {code} --via {}\n\nThe code is {code}. It works once, for 10 minutes.",
                    w.url()
                ),
                "wait: Waiting for the new device".into(),
                format!("info: mirkwood {} asks to join personal", ran.b_id),
                format!(
                    "confirm: Fingerprint {fingerprint}: does mirkwood show the same? initial=false"
                ),
                "outro: Paired".into(),
            ]
        );
        assert!(ran.err.is_empty(), "{:?}", ran.err);
    }

    #[test]
    fn a_command_too_wide_for_the_box_goes_above_it() {
        let w = world("narrow");
        w.enrolled();
        let ran = go_with(
            &w,
            &[],
            &new_device("mirkwood", 20),
            yes(),
            &limits(),
            Some(40),
            None,
        );
        assert!(ran.result.is_ok(), "{:?}", ran.refused());
        let code = ran
            .shown
            .iter()
            .find_map(|l| l.split_once("\nbilbo pair ")?.1.split(' ').next())
            .unwrap();
        assert_eq!(ran.shown[0], "intro: bilbo pair");
        assert_eq!(
            ran.shown[1],
            format!(
                "info: On the new device, run:\nbilbo pair {code} --via {}",
                w.url()
            )
        );
        assert_eq!(
            ran.shown[2],
            format!("note: Pairing code\nThe code is {code}. It works once, for 10 minutes.")
        );
    }

    #[test]
    fn a_refusal_after_the_box_closes_the_drawing() {
        let w = world("closed");
        w.enrolled();
        let mut b = new_device("mirkwood", 20);
        b.mode = Mode::WrongWord;
        let ran = go(&w, &[], &b, yes());
        assert!(ran.refused().contains("wrong code"));
        assert_eq!(
            ran.shown.last().map(String::as_str),
            Some("cancel: Not paired")
        );
    }

    #[test]
    fn the_mailbox_holds_no_scope_name_url_or_seed() {
        let w = world("opaque");
        let (a, _) = w.enrolled();
        let ran = go(&w, &[], &new_device("mirkwood", 20), yes());
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
            who: identity(0, "bywater", 3),
            enrolled: true,
            mode: Mode::Right,
        };
        let ran = go(&w, &["--scope", "shared"], &b, yes());
        assert!(ran.result.is_ok(), "{:?}", ran.refused());
        assert_eq!(ran.out, [format!("paired bywater {}: shared", ran.b_id)]);
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
            who: identity(7, "bywater", 3),
            enrolled: true,
            mode: Mode::Right,
        };
        let ran = go(&w, &[], &b, yes());
        let theirs = keys::owner_fingerprint(&b.who.owner.sign.public());
        assert_eq!(
            ran.refused(),
            format!("bywater belongs to another owner ({theirs})")
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
        let bywater = Member::of(&identity(0, "bywater", 3).device);
        let other = w.scope(&a, "shared", &[bywater]);
        let own = go(&w, &[], &new_device("rhosgobel", 20), yes());
        assert!(
            own.refused()
                .contains("a device named rhosgobel is already enrolled")
        );
        assert!(matches!(own.reply, Some(Reply::Ended(Outcome::NameTaken))));
        let apart = go(
            &w,
            &["--scope", "personal"],
            &new_device("bywater", 21),
            yes(),
        );
        assert_eq!(
            apart.refused(),
            "a device named bywater is already enrolled; pair again with --name on the new device"
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
        assert!(go(&w, &[], &b, yes()).result.is_ok());
        let again = go(&w, &[], &b, yes());
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
        let ran = go(&w, &[], &b, yes());
        assert_eq!(
            ran.refused(),
            "the other device used a wrong code; this code is used up, run bilbo pair again"
        );
        assert!(matches!(ran.reply, Some(Reply::Ended(Outcome::WrongCode))));
        assert!(!ran.shown.iter().any(|l| l.starts_with("confirm: ")));
        assert_eq!(w.versions(&scope), 1);
        assert_eq!(w.mailboxes().len(), 1);
    }

    #[test]
    fn declining_or_ending_input_sends_nothing() {
        let cases = [
            ("no", vec![Answer::No]),
            ("eof", vec![]),
            ("esc", vec![Answer::Interrupt]),
            ("enter", vec![Answer::Default]),
        ];
        for (name, answers) in cases {
            let w = world(name);
            let (_, scope) = w.enrolled();
            let ran = go(&w, &[], &new_device("mirkwood", 20), answers);
            assert_eq!(ran.refused(), "not confirmed; nothing was sent", "{name}");
            assert!(matches!(ran.reply, Some(Reply::Ended(Outcome::Declined))));
            assert_eq!(w.versions(&scope), 1);
            assert!(ran.out.is_empty());
            assert_eq!(
                ran.shown.last().map(String::as_str),
                Some("cancel: Not paired")
            );
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
        let ran = go_with(
            &w,
            &[],
            &new_device("mirkwood", 20),
            vec![Answer::Late(Duration::from_millis(600), true)],
            &short,
            Some(WIDE),
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
        let ran = go_with(&w, &[], &b, vec![], &short, Some(WIDE), None);
        assert_eq!(ran.refused(), EXPIRED);
        assert!(w.mailboxes().is_empty());
    }

    #[test]
    fn a_newer_format_gets_no_reply() {
        let w = world("newer");
        let (_, scope) = w.enrolled();
        let mut b = new_device("mirkwood", 20);
        b.mode = Mode::Newer;
        let ran = go(&w, &[], &b, yes());
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
            go(&w, &[], &new_device("mirkwood", 20), yes())
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
        let ran = go(&w, &[], &new_device("mirkwood", 20), yes());
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
        let ran = go(&w, &[], &b, yes());
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
        go_with(&w, &[], &b, vec![], &short, Some(WIDE), None);
        assert_eq!(w.mailboxes(), ["8"]);
    }

    /// Runs A against the refusal checks: no mailbox may exist after.
    fn refusal(w: &World, args: &[&str]) -> (bool, String) {
        let mut b = new_device("mirkwood", 20);
        b.mode = Mode::Silent;
        let ran = go(w, args, &b, yes());
        assert!(!w.sync().join("pair").exists(), "{args:?} made a mailbox");
        assert!(ran.out.is_empty() && ran.err.is_empty() && ran.shown.is_empty());
        (
            matches!(ran.result, Err(Failure::Refused(_))),
            ran.refused(),
        )
    }

    #[test]
    fn the_checks_before_the_mailbox() {
        let w = world("checks");
        let url = w.url();
        let id = rhosgobel();
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
            "scope.personal.sync = {url}\nscope.client.sync = off\n"
        ));
        assert_eq!(
            refusal(&w, &["--scope", "client"]),
            (false, "scope client does not sync".into())
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
        for (terminal, marker) in [(None, None), (Some(100), Some("1"))] {
            let ran = go_with(&w, &[], &b, vec![], &limits(), terminal, marker);
            assert_eq!(ran.refused(), message);
            assert!(!w.sync().join("pair").exists());
        }
    }

    #[test]
    fn claudecode_alone_is_a_person_at_a_terminal() {
        let w = world("ide_terminal");
        w.enrolled();
        let mut b = new_device("mirkwood", 20);
        b.mode = Mode::Silent;
        // `CLAUDECODE` is not an agent marker: the env holds none, and the terminal gets a code.
        let ran = go_with(&w, &[], &b, vec![], &limits(), Some(WIDE), None);
        assert!(
            !matches!(&ran.result, Err(Failure::Refused(m)) if m.contains("only in a terminal"))
        );
        assert!(w.sync().join("pair").exists());
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
        let ran = go(w, &[], &b, yes());
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
        let ran = go(&w, &[], &new_device("mirkwood", 20), yes());
        let Some(Reply::Enrolled(payload)) = ran.reply else {
            panic!("no payload");
        };
        assert_eq!(payload.scopes[0].embedder, "local");
    }

    /// A transport whose mailbox is full.
    struct Full;

    impl Transport for Full {
        fn create(&self, _: &str, _: &[u8]) -> Put {
            Put::Full("relay http://r has no room for a pairing now; try again later".into())
        }
        fn reachable(&self) -> Result<(), String> {
            unreachable!()
        }
        fn keeps(&self) -> bool {
            unreachable!()
        }
        fn scopes(&self) -> Result<Vec<String>, String> {
            unreachable!()
        }
        fn devices(&self, _: &str) -> Result<Vec<String>, String> {
            unreachable!()
        }
        fn list_after(&self, _: &str, _: &str, _: u64) -> Result<Vec<u64>, String> {
            unreachable!()
        }
        fn probe(&self, _: &str, _: &str, _: u64) -> Result<Vec<u64>, String> {
            unreachable!()
        }
        fn get(&self, _: &str) -> Result<Option<Vec<u8>>, String> {
            unreachable!()
        }
        fn highest_manifest(&self, _: &str) -> Result<Option<u64>, String> {
            unreachable!()
        }
        fn replace(&self, _: &str, _: &[u8]) -> Result<(), String> {
            unreachable!()
        }
        fn sweep(&self, _: SystemTime) -> Result<(), String> {
            unreachable!()
        }
        fn remove_mailbox(&self, _: &str) -> Result<(), String> {
            unreachable!()
        }
        fn sweep_mailboxes(&self, _: SystemTime, _: Duration) -> Result<(), String> {
            unreachable!()
        }
    }

    #[test]
    fn a_full_mailbox_is_reported_in_the_relays_own_words() {
        let Err(Failure::Refused(why)) = claim(&Full, "http://r") else {
            panic!("a full mailbox must refuse");
        };
        assert_eq!(
            why,
            "relay http://r has no room for a pairing now; try again later"
        );
    }
}
