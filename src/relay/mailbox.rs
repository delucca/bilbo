//! The pairing mailbox, the relay's one area a device with no manifest entry may write: who opens a nameplate and who
//! writes next, the size and count limits, expiry, and the per-address limits on unsigned requests.

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::net::IpAddr;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::UNIX_EPOCH;

use super::admit;
use super::http::Response;
use super::store::{Created, Data, Staged};
use crate::sync::transport;

/// The longest nameplate of the layout.
const NAMEPLATE_MAX: usize = 64;

/// The largest message, in bytes.
const MESSAGE_MAX: u64 = 4096;

/// The most messages a nameplate holds.
const MESSAGES: usize = 8;

/// The most nameplates open at once.
const NAMEPLATES: usize = 32;

/// How long a nameplate lives after its first message, in seconds.
const EXPIRY: u64 = 30 * 60;

/// The unsigned requests one peer address may send in `MINUTE` seconds.
const PER_MINUTE: usize = 60;
const MINUTE: u64 = 60;

/// The distinct nameplates one peer address may ask for unsigned in `WINDOW` seconds, which is also how long an idle
/// address is remembered.
const DISTINCT: usize = 4;
const WINDOW: u64 = 10 * 60;

/// A message's place, `pair/<nameplate>/<name>.msg`, as the route read it from the target.
pub struct Message<'a> {
    pub nameplate: &'a str,
    pub name: &'a str,
}

/// The open nameplates, who opened each, and the unsigned requests of each peer address, in memory only.
pub struct Mailbox {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    plates: BTreeMap<String, Plate>,
    peers: BTreeMap<IpAddr, Peer>,
}

struct Plate {
    /// The key that signed the first message; `None` for a nameplate found at start, which nobody may write to.
    opener: Option<[u8; 32]>,
    opened: u64,
    messages: usize,
    unsigned_used: bool,
}

/// The unsigned requests of one peer address.
#[derive(Default)]
struct Peer {
    /// When each request of the last minute came.
    requests: VecDeque<u64>,
    /// When each nameplate of the last 10 minutes was last asked for.
    plates: BTreeMap<String, u64>,
}

impl Plate {
    fn expired(&self, now: u64) -> bool {
        now.saturating_sub(self.opened) >= EXPIRY
    }
}

impl Peer {
    fn forget(&mut self, now: u64) {
        while self
            .requests
            .front()
            .is_some_and(|&t| now.saturating_sub(t) >= MINUTE)
        {
            self.requests.pop_front();
        }
        self.plates
            .retain(|_, &mut t| now.saturating_sub(t) < WINDOW);
    }

    /// Counts an unsigned request for `nameplate`, or says in how many seconds it would be served.
    fn count(&mut self, nameplate: &str, now: u64) -> Result<(), u64> {
        self.forget(now);
        if self.requests.len() >= PER_MINUTE {
            let oldest = self.requests.front().copied().unwrap_or(now);
            return Err((oldest + MINUTE).saturating_sub(now).max(1));
        }
        if !self.plates.contains_key(nameplate) && self.plates.len() >= DISTINCT {
            let oldest = self.plates.values().min().copied().unwrap_or(now);
            return Err((oldest + WINDOW).saturating_sub(now).max(1));
        }
        self.requests.push_back(now);
        self.plates.insert(nameplate.to_string(), now);
        Ok(())
    }
}

fn not_admitted() -> Response {
    Response::error(403, "not-admitted")
}

fn quota() -> Response {
    Response::error(507, "quota")
}

fn path(at: &Message) -> String {
    format!("pair/{}/{}.msg", at.nameplate, at.name)
}

impl State {
    /// Whether `signer` may write to `nameplate`, and whether it has room. `enrolled` says whether a first message's
    /// signer may open a nameplate.
    fn admit(
        &self,
        nameplate: &str,
        signer: Option<&[u8; 32]>,
        enrolled: impl FnOnce(&[u8; 32]) -> bool,
        now: u64,
    ) -> Result<(), Response> {
        match self.plates.get(nameplate).filter(|p| !p.expired(now)) {
            None => match signer {
                Some(key) if enrolled(key) => {
                    if self.plates.values().filter(|p| !p.expired(now)).count() >= NAMEPLATES {
                        Err(quota())
                    } else {
                        Ok(())
                    }
                }
                _ => Err(not_admitted()),
            },
            Some(plate) => {
                let allowed = match signer {
                    Some(key) => plate.opener.as_ref() == Some(key),
                    None => !plate.unsigned_used,
                };
                if !allowed {
                    Err(not_admitted())
                } else if plate.messages >= MESSAGES {
                    Err(quota())
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Removes the nameplates opened 30 minutes or more before `now` that `data` can drop.
    fn expire(&mut self, data: &Data, now: u64) {
        self.plates
            .retain(|name, plate| !plate.expired(now) || data.remove_nameplate(name).is_err());
    }
}

impl Mailbox {
    /// Counts the open nameplates under `<data>/pair/` and removes those opened 30 minutes or more before `now`.
    pub fn open(data: &Data, now: u64) -> Result<Mailbox, String> {
        let mut state = State::default();
        let folder = data.root().join("pair");
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Mailbox {
                    state: Mutex::new(state),
                });
            }
            Err(e) => return Err(format!("cannot read {}: {e}", folder.display())),
        };
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot read {}: {e}", folder.display()))?;
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !transport::is_mailbox_name(&name, NAMEPLATE_MAX)
                || !entry.file_type().is_ok_and(|t| t.is_dir())
            {
                continue;
            }
            let (opened, messages) = stored(&entry.path())?;
            let plate = Plate {
                opener: None,
                opened: opened.unwrap_or(0),
                messages,
                unsigned_used: true,
            };
            if messages == 0 || plate.expired(now) {
                data.remove_nameplate(&name)?;
            } else {
                state.plates.insert(name, plate);
            }
        }
        Ok(Mailbox {
            state: Mutex::new(state),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The checks before a mailbox request's body: who may write (`signer` is the key whose signature, time and
    /// nonce the route verified, `None` when unsigned), the per-address limits of an unsigned request, and room for
    /// a new message. `length` is the `PUT`'s `Content-Length`, `None` for a `GET`.
    pub fn head(
        &self,
        scopes: &admit::Scopes,
        at: &Message,
        signer: Option<&[u8; 32]>,
        peer: IpAddr,
        length: Option<u64>,
        now: u64,
    ) -> Result<(), Response> {
        let mut state = self.lock();
        if signer.is_none() {
            let counted = state
                .peers
                .entry(peer)
                .or_default()
                .count(at.nameplate, now);
            if let Err(wait) = counted {
                return Err(Response::error(429, "rate").with("Retry-After", &wait.to_string()));
            }
        }
        let Some(length) = length else {
            return Ok(());
        };
        state.admit(at.nameplate, signer, |key| scopes.enrolled(key), now)?;
        if length > MESSAGE_MAX {
            return Err(Response::error(413, "too-large"));
        }
        Ok(())
    }

    /// Creates a message whose `head` passed, from its received body: 201, 200 for the same bytes, 409 `exists`.
    pub fn put(
        &self,
        data: &Data,
        at: &Message,
        signer: Option<&[u8; 32]>,
        body: Staged,
        now: u64,
    ) -> Response {
        let mut state = self.lock();
        if state
            .plates
            .get(at.nameplate)
            .is_some_and(|p| p.expired(now))
        {
            state.expire(data, now);
        }
        // `head` checked the signer against the manifests; a nameplate that is still absent here needs only a key.
        if let Err(refusal) = state.admit(at.nameplate, signer, |_| true, now) {
            return refusal;
        }
        match body.link(data, &path(at)) {
            Created::New => {
                let plate = state
                    .plates
                    .entry(at.nameplate.to_string())
                    .or_insert_with(|| Plate {
                        opener: signer.copied(),
                        opened: now,
                        messages: 0,
                        unsigned_used: false,
                    });
                plate.messages += 1;
                plate.unsigned_used |= signer.is_none();
                Response::new(201, Vec::new())
            }
            Created::Same => Response::new(200, Vec::new()),
            Created::Other => Response::error(409, "exists"),
            Created::Full(_) => quota(),
            Created::Failed(_) => Response::error(500, "internal"),
        }
    }

    /// A message's bytes, or 404 `not-found`, an expired nameplate included.
    pub fn get(&self, data: &Data, at: &Message, now: u64) -> Response {
        let mut state = self.lock();
        state.expire(data, now);
        match data.read(&path(at)) {
            Ok(Some(bytes)) if state.plates.contains_key(at.nameplate) => Response::new(200, bytes),
            Ok(_) => Response::error(404, "not-found"),
            Err(_) => Response::error(500, "internal"),
        }
    }

    /// Removes the nameplates opened 30 minutes or more before `now`, and forgets idle peer addresses.
    pub fn sweep(&self, data: &Data, now: u64) {
        let mut state = self.lock();
        state.expire(data, now);
        state.peers.retain(|_, peer| {
            peer.forget(now);
            !peer.requests.is_empty() || !peer.plates.is_empty()
        });
    }
}

/// When the first message of the nameplate folder `dir` was written, by modification time, and how many it holds.
fn stored(dir: &std::path::Path) -> Result<(Option<u64>, usize), String> {
    let mut opened: Option<u64> = None;
    let mut messages = 0;
    let read = |e: std::io::Error| format!("cannot read {}: {e}", dir.display());
    for file in fs::read_dir(dir).map_err(read)? {
        let file = file.map_err(read)?;
        if !file.file_name().to_string_lossy().ends_with(".msg") {
            continue;
        }
        messages += 1;
        let modified = file.metadata().and_then(|m| m.modified()).map_err(read)?;
        let secs = modified
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        opened = Some(opened.map_or(secs, |o| o.min(secs)));
    }
    Ok((opened, messages))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::identity::keys::{self, Device, Identity, Owner};
    use crate::identity::manifest;
    use crate::relay::admit::{Held, Standing};

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-mailbox-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn identity(owner: &Owner, name: &str, seed: u8) -> Identity {
        Identity {
            owner: owner.file(),
            device: Device::from_seeds(name, &[seed; 32], &[seed + 1; 32]),
        }
    }

    /// A relay holding one valid scope, owned by an admitted owner, that lists `rivendell` and `bagend`.
    struct World {
        data: Data,
        scopes: admit::Scopes,
        rivendell: [u8; 32],
        bagend: [u8; 32],
        stranger: [u8; 32],
        _manifests: Scratch,
        _root: Scratch,
    }

    fn world(name: &str) -> World {
        let manifests = scratch(&format!("{name}-manifests"));
        let root = scratch(name);
        let owner = Owner::derive(&[7; 16]);
        let rivendell = identity(&owner, "rivendell", 11);
        let bagend = identity(&owner, "bagend", 21);
        let stranger = identity(&owner, "stranger", 31);
        let lock = manifest::lock(&manifests.0).unwrap();
        let others = [manifest::Member::of(&bagend.device)];
        let id = manifest::create(&lock, &rivendell, "personal", "file:///x", &others)
            .unwrap()
            .scope;
        drop(lock);
        let held = Held {
            chain: manifest::read_scope(&manifests.0, &id).unwrap(),
            standing: Standing::Valid,
            bytes: 0,
            reserved: 0,
            highest: BTreeMap::new(),
        };
        let print = keys::owner_fingerprint(&owner.sign.public());
        World {
            data: Data::open(&root.0.join("data")).unwrap(),
            scopes: admit::Scopes::new(&[print], BTreeMap::from([(id, held)])),
            rivendell: rivendell.device.sign.public(),
            bagend: bagend.device.sign.public(),
            stranger: stranger.device.sign.public(),
            _manifests: manifests,
            _root: root,
        }
    }

    fn peer(last: u8) -> IpAddr {
        IpAddr::from([10, 0, 0, last])
    }

    fn at<'a>(nameplate: &'a str, name: &'a str) -> Message<'a> {
        Message { nameplate, name }
    }

    const START: u64 = 1_000_000;

    impl World {
        fn head(
            &self,
            mailbox: &Mailbox,
            to: &Message,
            signer: Option<&[u8; 32]>,
            from: IpAddr,
            length: Option<u64>,
            now: u64,
        ) -> Result<(), u16> {
            mailbox
                .head(&self.scopes, to, signer, from, length, now)
                .map_err(|r| r.status)
        }

        /// A `PUT` as the route runs it: `head`, then `put`.
        fn put(
            &self,
            mailbox: &Mailbox,
            to: &Message,
            signer: Option<&[u8; 32]>,
            from: IpAddr,
            body: &[u8],
            now: u64,
        ) -> u16 {
            let length = Some(body.len() as u64);
            if let Err(status) = self.head(mailbox, to, signer, from, length, now) {
                return status;
            }
            let staged = self.data.stage(body.len() as u64, &mut &body[..]).unwrap();
            mailbox.put(&self.data, to, signer, staged, now).status
        }

        fn open(&self, now: u64) -> Mailbox {
            Mailbox::open(&self.data, now).unwrap()
        }
    }

    #[test]
    fn an_enrolled_device_opens_a_nameplate() {
        let w = world("opens");
        let m = w.open(START);
        let status = w.put(
            &m,
            &at("42", "a"),
            Some(&w.rivendell),
            peer(1),
            b"one",
            START,
        );
        assert_eq!(status, 201);
        assert_eq!(m.get(&w.data, &at("42", "a"), START).body, b"one");
    }

    #[test]
    fn the_new_device_answers_and_polls() {
        let w = world("answers");
        let m = w.open(START);
        assert_eq!(
            w.put(&m, &at("42", "a"), Some(&w.rivendell), peer(1), b"a", START),
            201
        );
        assert_eq!(w.put(&m, &at("42", "b"), None, peer(2), b"b", START), 201);
        assert_eq!(m.get(&w.data, &at("42", "c"), START).status, 404);
        assert_eq!(
            w.put(&m, &at("42", "c"), Some(&w.rivendell), peer(1), b"c", START),
            201
        );
        assert_eq!(m.get(&w.data, &at("42", "c"), START).status, 200);
    }

    #[test]
    fn a_stranger_cannot_open_a_nameplate() {
        let w = world("stranger");
        let m = w.open(START);
        assert_eq!(w.put(&m, &at("91", "a"), None, peer(2), b"a", START), 403);
        let signed = w.put(&m, &at("91", "a"), Some(&w.stranger), peer(2), b"a", START);
        assert_eq!(signed, 403);
        assert!(!w.data.root().join("pair/91").exists());
        assert_eq!(m.get(&w.data, &at("91", "a"), START).status, 404);
    }

    #[test]
    fn a_second_unsigned_message_is_refused_and_the_opener_still_writes() {
        let w = world("second");
        let m = w.open(START);
        assert_eq!(
            w.put(&m, &at("42", "a"), Some(&w.rivendell), peer(1), b"a", START),
            201
        );
        assert_eq!(w.put(&m, &at("42", "b"), None, peer(2), b"b", START), 201);
        assert_eq!(w.put(&m, &at("42", "c"), None, peer(2), b"x", START), 403);
        assert_eq!(
            w.put(&m, &at("42", "c"), Some(&w.rivendell), peer(1), b"c", START),
            201
        );
    }

    #[test]
    fn another_enrolled_device_cannot_reply() {
        let w = world("other");
        let m = w.open(START);
        assert_eq!(
            w.put(&m, &at("42", "a"), Some(&w.rivendell), peer(1), b"a", START),
            201
        );
        assert_eq!(
            w.put(&m, &at("42", "c"), Some(&w.bagend), peer(3), b"c", START),
            403
        );
    }

    #[test]
    fn junk_slots_leave_the_opener_its_eight_messages() {
        let w = world("junk");
        let m = w.open(START);
        assert_eq!(
            w.put(&m, &at("42", "a"), Some(&w.rivendell), peer(1), b"a", START),
            201
        );
        assert_eq!(w.put(&m, &at("42", "b"), None, peer(2), b"b", START), 201);
        for name in ["x", "y", "z"] {
            assert_eq!(w.put(&m, &at("42", name), None, peer(2), b"j", START), 403);
        }
        for name in ["c", "d", "e", "f", "g", "h"] {
            let status = w.put(
                &m,
                &at("42", name),
                Some(&w.rivendell),
                peer(1),
                b"m",
                START,
            );
            assert_eq!(status, 201, "{name}");
        }
        let ninth = w.put(&m, &at("42", "i"), Some(&w.rivendell), peer(1), b"m", START);
        assert_eq!(ninth, 507);
    }

    #[test]
    fn a_second_answer_to_one_slot() {
        let w = world("exists");
        let m = w.open(START);
        assert_eq!(
            w.put(
                &m,
                &at("42", "a"),
                Some(&w.rivendell),
                peer(1),
                b"one",
                START
            ),
            201
        );
        let same = w.put(
            &m,
            &at("42", "a"),
            Some(&w.rivendell),
            peer(1),
            b"one",
            START,
        );
        assert_eq!(same, 200);
        let other = w.put(
            &m,
            &at("42", "a"),
            Some(&w.rivendell),
            peer(1),
            b"two",
            START,
        );
        assert_eq!(other, 409);
        assert_eq!(m.get(&w.data, &at("42", "a"), START).body, b"one");
    }

    #[test]
    fn a_message_is_at_most_4_kib() {
        let w = world("size");
        let m = w.open(START);
        let big = vec![0u8; 5000];
        assert_eq!(
            w.put(&m, &at("42", "a"), Some(&w.rivendell), peer(1), &big, START),
            413
        );
        let edge = vec![0u8; 4096];
        assert_eq!(
            w.put(
                &m,
                &at("42", "a"),
                Some(&w.rivendell),
                peer(1),
                &edge,
                START
            ),
            201
        );
        assert!(!w.data.root().join("pair/43").exists());
    }

    #[test]
    fn at_most_32_nameplates_are_open() {
        let w = world("quota");
        let m = w.open(START);
        for n in 1..=32 {
            let plate = n.to_string();
            let status = w.put(
                &m,
                &at(&plate, "a"),
                Some(&w.rivendell),
                peer(1),
                b"a",
                START,
            );
            assert_eq!(status, 201, "{n}");
        }
        let status = w.put(&m, &at("33", "a"), Some(&w.rivendell), peer(1), b"a", START);
        assert_eq!(status, 507);
        assert_eq!(
            w.put(&m, &at("32", "c"), Some(&w.rivendell), peer(1), b"c", START),
            201
        );
        // An expired nameplate frees its place.
        let later = START + EXPIRY;
        assert_eq!(
            w.put(&m, &at("33", "a"), Some(&w.rivendell), peer(1), b"a", later),
            201
        );
    }

    #[test]
    fn an_expired_nameplate_is_gone() {
        let w = world("expired");
        let m = w.open(START);
        assert_eq!(
            w.put(&m, &at("42", "a"), Some(&w.rivendell), peer(1), b"a", START),
            201
        );
        let almost = START + EXPIRY - 1;
        assert_eq!(m.get(&w.data, &at("42", "a"), almost).status, 200);
        let later = START + 31 * 60;
        assert_eq!(m.get(&w.data, &at("42", "a"), later).status, 404);
        assert!(!w.data.root().join("pair/42").exists());
    }

    #[test]
    fn expiry_counts_from_the_first_message() {
        let w = world("first");
        let m = w.open(START);
        assert_eq!(
            w.put(&m, &at("42", "a"), Some(&w.rivendell), peer(1), b"a", START),
            201
        );
        let late = START + EXPIRY - 5;
        assert_eq!(
            w.put(&m, &at("42", "c"), Some(&w.rivendell), peer(1), b"c", late),
            201
        );
        assert_eq!(m.get(&w.data, &at("42", "c"), START + EXPIRY).status, 404);
    }

    #[test]
    fn a_new_nameplate_may_reuse_an_expired_number() {
        let w = world("reuse");
        let m = w.open(START);
        assert_eq!(
            w.put(
                &m,
                &at("42", "a"),
                Some(&w.rivendell),
                peer(1),
                b"old",
                START
            ),
            201
        );
        let later = START + EXPIRY;
        let status = w.put(&m, &at("42", "a"), Some(&w.bagend), peer(1), b"new", later);
        assert_eq!(status, 201);
        assert_eq!(m.get(&w.data, &at("42", "a"), later).body, b"new");
        assert_eq!(
            w.put(&m, &at("42", "c"), Some(&w.rivendell), peer(1), b"c", later),
            403
        );
    }

    #[test]
    fn the_sweep_removes_expired_nameplates_and_idle_peers() {
        let w = world("sweep");
        let m = w.open(START);
        assert_eq!(
            w.put(&m, &at("42", "a"), Some(&w.rivendell), peer(1), b"a", START),
            201
        );
        assert_eq!(
            w.head(&m, &at("42", "c"), None, peer(2), None, START),
            Ok(())
        );
        m.sweep(&w.data, START + 60);
        assert!(w.data.root().join("pair/42").exists());
        assert_eq!(m.lock().peers.len(), 1);
        m.sweep(&w.data, START + WINDOW);
        assert!(m.lock().peers.is_empty());
        m.sweep(&w.data, START + EXPIRY);
        assert!(!w.data.root().join("pair/42").exists());
        assert!(m.lock().plates.is_empty());
    }

    #[test]
    fn the_61st_unsigned_request_in_a_minute_is_rate_limited() {
        let w = world("flood");
        let m = w.open(START);
        let to = at("42", "c");
        for i in 0..60 {
            assert_eq!(
                w.head(&m, &to, None, peer(2), None, START + i / 30),
                Ok(()),
                "{i}"
            );
        }
        let refused = m
            .head(&w.scopes, &to, None, peer(2), None, START + 1)
            .unwrap_err();
        assert_eq!(refused.status, 429);
        let wait = refused.headers.iter().find(|(n, _)| n == "Retry-After");
        assert_eq!(wait.map(|(_, v)| v.as_str()), Some("59"));
        assert_eq!(w.head(&m, &to, None, peer(3), None, START + 1), Ok(()));
        assert_eq!(w.head(&m, &to, None, peer(2), None, START + 60), Ok(()));
    }

    #[test]
    fn probing_five_nameplates_is_refused_at_the_fifth() {
        let w = world("probe");
        let m = w.open(START);
        for n in 1..=4 {
            let plate = n.to_string();
            let now = START + n;
            assert_eq!(
                w.head(&m, &at(&plate, "c"), None, peer(2), None, now),
                Ok(())
            );
        }
        assert_eq!(
            w.head(&m, &at("5", "c"), None, peer(2), None, START + 5),
            Err(429)
        );
        for n in 1..=4 {
            let plate = n.to_string();
            assert_eq!(
                w.head(&m, &at(&plate, "c"), None, peer(2), None, START + 6),
                Ok(())
            );
        }
        assert_eq!(
            w.head(&m, &at("5", "c"), None, peer(3), None, START + 5),
            Ok(())
        );
        assert_eq!(
            w.head(&m, &at("5", "c"), None, peer(2), None, START + WINDOW + 6),
            Ok(())
        );
    }

    #[test]
    fn the_opener_is_never_rate_limited() {
        let w = world("opener");
        let m = w.open(START);
        assert_eq!(
            w.put(&m, &at("42", "a"), Some(&w.rivendell), peer(1), b"a", START),
            201
        );
        let to = at("42", "b");
        for i in 0..300 {
            let now = START + i * 2;
            let status = w.head(&m, &to, Some(&w.rivendell), peer(1), None, now);
            assert_eq!(status, Ok(()), "poll {i}");
        }
        assert!(m.lock().peers.is_empty());
    }

    #[test]
    fn a_stranger_put_counts_against_the_address() {
        let w = world("strangerrate");
        let m = w.open(START);
        for n in 1..=4 {
            let plate = n.to_string();
            assert_eq!(w.put(&m, &at(&plate, "a"), None, peer(2), b"a", START), 403);
        }
        assert_eq!(w.put(&m, &at("5", "a"), None, peer(2), b"a", START), 429);
    }

    fn age(path: &std::path::Path, seconds: u64) {
        let when = SystemTime::now() - Duration::from_secs(seconds);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    #[test]
    fn start_up_counts_the_open_nameplates_and_removes_expired_ones() {
        let w = world("restart");
        let first = w.open(START);
        assert_eq!(
            w.put(
                &first,
                &at("1", "a"),
                Some(&w.rivendell),
                peer(1),
                b"a",
                START
            ),
            201
        );
        assert_eq!(
            w.put(
                &first,
                &at("2", "a"),
                Some(&w.rivendell),
                peer(1),
                b"a",
                START
            ),
            201
        );
        drop(first);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        age(&w.data.root().join("pair/1/a.msg"), 31 * 60);
        let second = w.open(now);
        assert!(!w.data.root().join("pair/1").exists());
        assert!(w.data.root().join("pair/2").exists());
        assert_eq!(second.lock().plates.len(), 1);
        assert_eq!(second.get(&w.data, &at("2", "a"), now).body, b"a");
        // Nobody holds the key of a nameplate found at start.
        let status = w.put(
            &second,
            &at("2", "c"),
            Some(&w.rivendell),
            peer(1),
            b"c",
            now,
        );
        assert_eq!(status, 403);
    }

    #[test]
    fn start_up_without_a_pair_folder_is_empty() {
        let w = world("nopair");
        assert!(w.open(START).lock().plates.is_empty());
    }
}
