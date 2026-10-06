//! The relay's requests: the target grammar under `/v1/`, the order of checks, signatures and the nonce cache, scope
//! reads, listings and creates, the mailbox's place, and the log lines.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::io::Read;
use std::sync::{Mutex, PoisonError};

use super::State;
use super::admit::{self, Action, Put, Refusal};
use super::http::{Handler, Head, Request, Response};
use super::mailbox::Message;
use super::store::{Created, Data, Staged};
use crate::identity::keys;
use crate::shared::hash;
use crate::sync::remote::sign;
use crate::sync::transport;

/// How long an accepted nonce is remembered, in seconds.
const REPLAY: u64 = 600;

/// The most nonces remembered at once.
const NONCES: usize = 100_000;

/// The most seqs one folder listing holds.
const PAGE: u64 = 1000;

/// The seconds a client is told to wait after 503 `busy`.
const BUSY: &str = "5";

/// How long the 4xx refusals counted by `Refusals` wait for their line, in seconds.
const MINUTE: u64 = 60;

/// The longest nameplate and message name of the layout.
const NAMEPLATE_MAX: usize = 64;
const MESSAGE_MAX: usize = 16;

/// The nonces of signed requests accepted in the last 600 seconds, per key, at most 100,000.
#[derive(Default)]
pub struct Nonces {
    seen: Mutex<Seen>,
}

#[derive(Default)]
struct Seen {
    /// When each `(key, nonce)` was accepted.
    at: BTreeMap<([u8; 32], [u8; 16]), u64>,
    /// The same, oldest first.
    order: VecDeque<(u64, [u8; 32], [u8; 16])>,
}

/// Why a nonce is not accepted.
#[derive(Debug, PartialEq)]
enum Reject {
    /// 401 `replay`: the key used it in the last 600 seconds.
    Replayed,
    /// 503 `busy`: the cache holds 100,000 nonces.
    Full,
}

impl Nonces {
    /// Records `nonce` of `key` at `now`, after its signature verified.
    fn accept(&self, key: &[u8; 32], nonce: &[u8; 16], now: u64) -> Result<(), Reject> {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        while let Some(&(at, old_key, old_nonce)) = seen.order.front() {
            if now.saturating_sub(at) < REPLAY {
                break;
            }
            seen.order.pop_front();
            seen.at.remove(&(old_key, old_nonce));
        }
        if seen.at.contains_key(&(*key, *nonce)) {
            return Err(Reject::Replayed);
        }
        if seen.at.len() >= NONCES {
            return Err(Reject::Full);
        }
        seen.at.insert((*key, *nonce), now);
        seen.order.push_back((now, *key, *nonce));
        Ok(())
    }
}

/// The 4xx refusals not logged one by one, counted by reason until the next minute's line.
#[derive(Default)]
pub struct Refusals {
    counted: Mutex<Counted>,
}

#[derive(Default)]
struct Counted {
    reasons: BTreeMap<String, u64>,
    /// When the first refusal since the last line was counted.
    since: Option<u64>,
}

impl Refusals {
    fn count(&self, reason: &str, now: u64) {
        let mut counted = self.counted.lock().unwrap_or_else(PoisonError::into_inner);
        *counted.reasons.entry(reason.to_string()).or_default() += 1;
        counted.since.get_or_insert(now);
    }

    /// The line for the refusals counted since the last one, when the first is a minute old.
    fn due(&self, now: u64) -> Option<String> {
        let mut counted = self.counted.lock().unwrap_or_else(PoisonError::into_inner);
        if counted
            .since
            .is_none_or(|since| now.saturating_sub(since) < MINUTE)
        {
            return None;
        }
        counted.since = None;
        let reasons = std::mem::take(&mut counted.reasons);
        let total: u64 = reasons.values().sum();
        let by_reason: Vec<String> = reasons.iter().map(|(r, n)| format!("{r} {n}")).collect();
        Some(format!(
            "refused {total} other requests in the last minute: {}",
            by_reason.join(", ")
        ))
    }
}

thread_local! {
    /// The scope and bytes `head` booked for the body its connection's thread reads next, for `answer` to hand to the
    /// create or give back, and for `finish` to give back when `answer` never ran. `http::serve` runs the three on
    /// one thread.
    static BOOKED: RefCell<Option<(String, u64)>> = const { RefCell::new(None) };
}

/// A request target as the tree's grammar spells it.
#[derive(Debug, PartialEq)]
enum Route<'a> {
    /// `/v1/`
    Root,
    /// `/v1/scopes/`
    Owned,
    /// `scopes/<id>/manifest/<n>.json`
    Manifest { scope: &'a str, n: u64 },
    /// `scopes/<id>/manifest/latest`
    Latest { scope: &'a str },
    /// `scopes/<id>/devices/`
    Devices { scope: &'a str },
    /// `scopes/<id>/devices/<id>/` with its `?after=`
    Folder {
        scope: &'a str,
        device: &'a str,
        after: u64,
    },
    /// `scopes/<id>/devices/<id>/<seq>.seg`
    Segment {
        scope: &'a str,
        device: &'a str,
        seq: u64,
    },
    /// `pair/<nameplate>/<name>.msg`
    Message { nameplate: &'a str, name: &'a str },
}

impl<'a> Route<'a> {
    /// Reads `target` byte for byte: no percent-escape, dot segment, empty segment or query beyond a folder's
    /// `after`.
    fn parse(target: &'a str) -> Option<Route<'a>> {
        let rest = target.strip_prefix("/v1/")?;
        let (path, query) = match rest.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (rest, None),
        };
        let parts: Vec<&str> = path.split('/').collect();
        if let ["scopes", scope, "devices", device, ""] = parts.as_slice() {
            if !keys::is_id(scope) || !keys::is_id(device) {
                return None;
            }
            let after = match query {
                None => 0,
                Some(query) => decimal(query.strip_prefix("after=")?)?,
            };
            return Some(Route::Folder {
                scope,
                device,
                after,
            });
        }
        if query.is_some() {
            return None;
        }
        match parts.as_slice() {
            [""] => Some(Route::Root),
            ["scopes", ""] => Some(Route::Owned),
            ["scopes", scope, "manifest", "latest"] if keys::is_id(scope) => {
                Some(Route::Latest { scope })
            }
            ["scopes", scope, "manifest", name] if keys::is_id(scope) => {
                let n = transport::manifest_number(name)?;
                Some(Route::Manifest { scope, n })
            }
            ["scopes", scope, "devices", ""] if keys::is_id(scope) => {
                Some(Route::Devices { scope })
            }
            ["scopes", scope, "devices", device, name]
                if keys::is_id(scope) && keys::is_id(device) =>
            {
                let seq = transport::segment_seq(name)?;
                Some(Route::Segment { scope, device, seq })
            }
            ["pair", nameplate, file] if transport::is_mailbox_name(nameplate, NAMEPLATE_MAX) => {
                let name = file.strip_suffix(".msg")?;
                transport::is_mailbox_name(name, MESSAGE_MAX)
                    .then_some(Route::Message { nameplate, name })
            }
            _ => None,
        }
    }

    /// Whether the route names an object, the only kind a `PUT` may.
    fn is_object(&self) -> bool {
        matches!(
            self,
            Route::Manifest { .. } | Route::Segment { .. } | Route::Message { .. }
        )
    }

    /// Whether the route is under `/v1/scopes/`, where every request is signed.
    fn is_scoped(&self) -> bool {
        !matches!(self, Route::Root | Route::Message { .. })
    }

    /// What a log line names of the route: never a nameplate.
    fn what(&self) -> String {
        match self {
            Route::Root => "root".into(),
            Route::Owned => "scopes".into(),
            Route::Manifest { scope, .. } | Route::Latest { scope } => format!("manifest {scope}"),
            Route::Devices { scope } | Route::Folder { scope, .. } => format!("devices {scope}"),
            Route::Segment { scope, device, .. } => format!("segment {scope} {device}"),
            Route::Message { .. } => "mailbox".into(),
        }
    }
}

/// Plain decimal digits with no sign and no leading zero.
fn decimal(text: &str) -> Option<u64> {
    if text.is_empty()
        || !text.bytes().all(|b| b.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return None;
    }
    text.parse().ok()
}

/// A body booked against its scope's cap by `head`, given back unless a create took it over.
struct Booked<'a> {
    scopes: &'a admit::Scopes,
    scope: String,
    amount: u64,
}

impl Booked<'_> {
    /// What the create releases itself.
    fn hand_over(&mut self) -> u64 {
        std::mem::take(&mut self.amount)
    }
}

impl Drop for Booked<'_> {
    fn drop(&mut self) {
        if self.amount > 0 {
            self.scopes.release(&self.scope, self.amount);
        }
    }
}

fn bad_request() -> Response {
    Response::error(400, "bad-request")
}

fn unsigned() -> Response {
    Response::error(401, "signature")
}

fn internal() -> Response {
    Response::error(500, "internal")
}

fn json(body: String) -> Response {
    Response::new(200, body.into_bytes()).with("Content-Type", "application/json")
}

/// The status each admission refusal answers with.
fn refusal(why: Refusal) -> Response {
    let status = match why {
        Refusal::NotAdmitted | Refusal::Invalid => 403,
        Refusal::NotNext => 409,
        Refusal::Manifest(_) => 422,
        Refusal::Quota => 507,
        Refusal::TooLarge => 413,
    };
    Response::error(status, why.reason())
}

/// The reason in a refusal's body.
fn reason_of(response: &Response) -> String {
    response.reason().to_string()
}

/// A request's signature headers. A header given twice is a bad signature.
fn signature(request: &Request) -> Result<Option<sign::Signed>, Response> {
    let names = [sign::KEY, sign::TIME, sign::NONCE, sign::SIGNATURE];
    let twice = names
        .iter()
        .any(|name| request.headers.iter().filter(|(n, _)| n == name).count() > 1);
    if twice {
        return Err(unsigned());
    }
    sign::read(&|name| request.header(name).map(str::to_string)).map_err(|_| unsigned())
}

impl State<'_> {
    /// The checks that need no body.
    fn check_head(&self, request: &Request, now: u64) -> Result<(), Response> {
        BOOKED.take();
        let route = Route::parse(&request.target).ok_or_else(bad_request)?;
        let put = match request.method.as_str() {
            "GET" => false,
            "PUT" if route.is_object() => true,
            _ => return Err(Response::error(405, "bad-request")),
        };
        if route == Route::Root {
            return Ok(());
        }
        let signed = signature(request)?;
        if signed.is_none() && route.is_scoped() {
            return Err(unsigned());
        }
        if signed
            .as_ref()
            .is_some_and(|s| s.time.abs_diff(now) > sign::WINDOW)
        {
            return Err(Response::error(401, "clock"));
        }
        if !put {
            return Ok(());
        }
        let length = request.length.unwrap_or(0);
        let claimed = signed.as_ref().map(|s| &s.key);
        let scope = match route {
            Route::Manifest { scope, .. } => {
                if length > admit::MANIFEST_MAX {
                    return Err(refusal(Refusal::TooLarge));
                }
                scope
            }
            Route::Segment { scope, .. } => {
                if length > admit::segment_max(&self.flags) {
                    return Err(refusal(Refusal::TooLarge));
                }
                scope
            }
            Route::Message { nameplate, name } => {
                let at = Message { nameplate, name };
                return self.mailbox.head(
                    &self.scopes,
                    &at,
                    claimed,
                    request.peer,
                    Some(length),
                    now,
                );
            }
            _ => return Ok(()),
        };
        // Whether the key may write is judged after its signature: a refusal for the scope's standing waits too.
        match self.scopes.reserve(&self.flags, scope, length) {
            Ok(booked) => BOOKED.set(Some((scope.to_string(), booked))),
            Err(Refusal::Quota) => return Err(refusal(Refusal::Quota)),
            Err(_) => {}
        }
        Ok(())
    }

    /// Logs a 500 or 507, and a 4xx other than 404 when a key the relay knows verified, or counts the 4xx.
    fn settle(&self, response: Response, request: &Request, known: bool) -> Response {
        let status = response.status;
        let counted = (400..500).contains(&status) && status != 404;
        if counted && !known {
            self.refusals.count(response.reason(), self.now());
        } else if counted || status == 500 || status == 507 {
            let what = Route::parse(&request.target).map_or_else(|| "request".into(), |r| r.what());
            let word = if counted { "refused" } else { "failed" };
            (self.log)(&format!(
                "{word} {status} {} {} {what}",
                response.reason(),
                request.method
            ));
        }
        response
    }

    /// Whether `key` is one the relay knows: a device listed in a held scope's latest manifest, or an admitted owner.
    fn knows(&self, key: &[u8; 32]) -> bool {
        self.scopes.enrolled(key) || self.scopes.owner_scopes(key).is_ok()
    }

    fn respond(
        &self,
        request: &Request,
        body: &mut dyn Read,
        known: &mut bool,
    ) -> Result<Response, Response> {
        let now = self.now();
        let route = Route::parse(&request.target).ok_or_else(bad_request)?;
        let put = request.method == "PUT";
        let length = request.length.unwrap_or(0);
        let (scope, amount) = BOOKED.take().unwrap_or_default();
        let mut booked = Booked {
            scopes: &self.scopes,
            scope,
            amount,
        };
        if route == Route::Root {
            return Ok(json("{\"relay\":\"bilbo\",\"api\":1}".into()));
        }
        let signed = signature(request)?;
        let staged = if put {
            Some(
                self.data
                    .stage(length, body)
                    .map_err(|created| match created {
                        Created::Full(_) => Response::error(507, "quota"),
                        _ => internal(),
                    })?,
            )
        } else {
            None
        };
        let key = match &signed {
            Some(signed) => {
                let sha256 = staged
                    .as_ref()
                    .map_or_else(|| hash::sha256_hex(b""), |s| s.sha256_hex().to_string());
                sign::verify(signed, &request.method, &request.target, &sha256, now).map_err(
                    |bad| match bad {
                        sign::Bad::Clock => Response::error(401, "clock"),
                        sign::Bad::Signature => unsigned(),
                    },
                )?;
                match self.nonces.accept(&signed.key, &signed.nonce, now) {
                    Ok(()) => {}
                    Err(Reject::Replayed) => return Err(Response::error(401, "replay")),
                    Err(Reject::Full) => {
                        return Err(Response::error(503, "busy").with("Retry-After", BUSY));
                    }
                }
                *known = self.knows(&signed.key);
                Some(signed.key)
            }
            None => None,
        };
        let signer = || key.ok_or_else(unsigned);
        match route {
            Route::Root => unreachable!("answered above"),
            Route::Owned => {
                let ids = self.scopes.owner_scopes(&signer()?).map_err(refusal)?;
                let quoted: Vec<String> = ids.iter().map(|id| format!("\"{id}\"")).collect();
                Ok(json(format!("{{\"scopes\":[{}]}}", quoted.join(","))))
            }
            Route::Latest { scope } => {
                let (held, _) = self
                    .scopes
                    .access(scope, &signer()?, Action::ReadManifests)
                    .map_err(refusal)?;
                let held = held.lock().unwrap_or_else(PoisonError::into_inner);
                let latest = held.chain.latest().ok_or_else(not_found)?;
                Ok(object(latest.bytes.clone())
                    .with("Bilbo-Manifest", &held.chain.versions.len().to_string()))
            }
            Route::Manifest { scope, n } if put => {
                let staged = staged.ok_or_else(internal)?;
                let key = signer()?;
                let size = staged.length();
                let bytes = staged.bytes().map_err(|_| internal())?;
                let path = transport::manifest_path(scope, n);
                let cell = RefCell::new(Some(staged));
                let at = Put {
                    scope,
                    signer: &key,
                    number: n,
                    length: size,
                    reserved: booked.hand_over(),
                };
                let created = self
                    .scopes
                    .create_manifest(&self.flags, &at, &bytes, &mut || {
                        link(&cell, &self.data, &path)
                    })
                    .map_err(refusal)?;
                let by = self.signer_label(scope, &key);
                created_reply(created, || {
                    (self.log)(&format!("manifest {scope} {by} {n} {size}"));
                })
            }
            Route::Manifest { scope, n } => {
                let (held, _) = self
                    .scopes
                    .access(scope, &signer()?, Action::ReadManifests)
                    .map_err(refusal)?;
                let stored = held
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .chain
                    .versions
                    .len();
                if n > stored as u64 {
                    return Err(not_found());
                }
                self.stored(&transport::manifest_path(scope, n))
            }
            Route::Devices { scope } => {
                let (held, _) = self
                    .scopes
                    .access(scope, &signer()?, Action::ReadSegments)
                    .map_err(refusal)?;
                let held = held.lock().unwrap_or_else(PoisonError::into_inner);
                let devices: Vec<String> = held
                    .highest
                    .iter()
                    .filter(|(_, last)| **last > 0)
                    .map(|(id, last)| format!("{{\"id\":\"{id}\",\"last\":{last}}}"))
                    .collect();
                Ok(json(format!("{{\"devices\":[{}]}}", devices.join(","))))
            }
            Route::Folder {
                scope,
                device,
                after,
            } => {
                let (held, _) = self
                    .scopes
                    .access(scope, &signer()?, Action::ReadSegments)
                    .map_err(refusal)?;
                let last = held
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .highest
                    .get(device)
                    .copied()
                    .unwrap_or(0);
                let first = after.saturating_add(1);
                let page_end = after.saturating_add(PAGE).min(last);
                let seqs: Vec<String> = (first..=page_end).map(|seq| seq.to_string()).collect();
                let more = last > page_end;
                Ok(json(format!(
                    "{{\"seqs\":[{}],\"more\":{more}}}",
                    seqs.join(",")
                )))
            }
            Route::Segment { scope, device, seq } if put => {
                let staged = staged.ok_or_else(internal)?;
                let key = signer()?;
                let size = staged.length();
                let path = transport::segment_path(scope, device, seq);
                let cell = RefCell::new(Some(staged));
                let at = Put {
                    scope,
                    signer: &key,
                    number: seq,
                    length: size,
                    reserved: booked.hand_over(),
                };
                let probe = || match self.data.read(&path)? {
                    Some(stored) => {
                        let held = cell.borrow();
                        let staged = held.as_ref().ok_or("the body is gone")?;
                        let bytes = staged.bytes().map_err(|e| e.to_string())?;
                        Ok(Some(bytes == stored))
                    }
                    None => Ok(None),
                };
                let created = self
                    .scopes
                    .create_segment(&self.flags, &at, device, &probe, &mut || {
                        link(&cell, &self.data, &path)
                    })
                    .map_err(refusal)?;
                created_reply(created, || {
                    (self.log)(&format!("segment {scope} {device} {seq} {size}"));
                })
            }
            Route::Segment { scope, device, seq } => {
                let (held, _) = self
                    .scopes
                    .access(scope, &signer()?, Action::ReadSegments)
                    .map_err(refusal)?;
                let last = held
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .highest
                    .get(device)
                    .copied()
                    .unwrap_or(0);
                if seq > last {
                    return Err(not_found());
                }
                self.stored(&transport::segment_path(scope, device, seq))
            }
            Route::Message { nameplate, name } => {
                let at = Message { nameplate, name };
                match staged {
                    None => {
                        self.mailbox.head(
                            &self.scopes,
                            &at,
                            key.as_ref(),
                            request.peer,
                            None,
                            now,
                        )?;
                        Ok(self.mailbox.get(&self.data, &at, now))
                    }
                    Some(staged) => {
                        let size = staged.length();
                        let response = self.mailbox.put(&self.data, &at, key.as_ref(), staged, now);
                        if response.status == 201 {
                            (self.log)(&format!("mailbox {size}"));
                        }
                        Ok(response)
                    }
                }
            }
        }
    }

    /// What a manifest's log line names as its signer: `owner`, or the device's id.
    fn signer_label(&self, scope: &str, key: &[u8; 32]) -> String {
        let owner = self.scopes.get(scope).and_then(|held| {
            held.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .chain
                .owner()
        });
        if owner.as_ref() == Some(key) {
            "owner".to_string()
        } else {
            keys::device_id(key)
        }
    }

    /// The bytes stored at `path` of the layout, or 404.
    fn stored(&self, path: &str) -> Result<Response, Response> {
        match self.data.read(path) {
            Ok(Some(bytes)) => Ok(object(bytes)),
            Ok(None) => Err(not_found()),
            Err(_) => Err(internal()),
        }
    }
}

/// Links a staged body, once.
fn link(cell: &RefCell<Option<Staged>>, data: &Data, path: &str) -> Created {
    match cell.borrow_mut().take() {
        Some(staged) => staged.link(data, path),
        None => Created::Failed("the body was linked twice".into()),
    }
}

/// The answer to a create: 201 after `logged`, 200 for the same bytes, and a refusal otherwise.
fn created_reply(created: Created, logged: impl FnOnce()) -> Result<Response, Response> {
    match created {
        Created::New => {
            logged();
            Ok(Response::new(201, Vec::new()))
        }
        Created::Same => Ok(Response::new(200, Vec::new())),
        Created::Other => Err(Response::error(409, "exists")),
        Created::Full(_) => Err(Response::error(507, "quota")),
        Created::Failed(_) => Err(internal()),
    }
}

fn not_found() -> Response {
    Response::error(404, "not-found")
}

fn object(bytes: Vec<u8>) -> Response {
    Response::new(200, bytes).with("Content-Type", "application/octet-stream")
}

impl Handler for State<'_> {
    fn now(&self) -> u64 {
        (self.clock)()
    }

    fn head(&self, request: &Request) -> Head {
        match self.check_head(request, self.now()) {
            Ok(()) => Head::Read,
            Err(response) => Head::Refuse(self.settle(response, request, false)),
        }
    }

    fn answer(&self, request: &Request, body: &mut dyn Read) -> Response {
        let mut known = false;
        let response = self
            .respond(request, body, &mut known)
            .unwrap_or_else(|refusal| refusal);
        self.settle(response, request, known)
    }

    fn refused(&self, status: u16, reason: &str) {
        if status != 404 {
            self.refusals.count(reason, self.now());
        }
    }

    fn finish(&self) {
        if let Some((scope, amount)) = BOOKED.take() {
            self.scopes.release(&scope, amount);
        }
    }
}

/// Once a minute, logs the refusals counted since the last line, when there were any.
pub fn tick(state: &State, now: u64) {
    if let Some(line) = state.refusals.due(now) {
        (state.log)(&line);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::{self, Read, Write};
    use std::net::{IpAddr, TcpStream};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    use serde_json::{Value, json};

    use super::*;
    use crate::identity::keys::{Device, Identity, Owner, SignKey};
    use crate::identity::manifest::{self, Recipient};
    use crate::relay::admit::{Held, Scopes, Standing};
    use crate::relay::mailbox::Mailbox;
    use crate::relay::{self, Flags, Running};

    const T0: u64 = 1_800_000_000;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-route-{name}-{}", std::process::id()));
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

    fn print(owner: &Owner) -> String {
        keys::owner_fingerprint(&owner.sign.public())
    }

    /// A scope of three versions made as a device would: `rivendell` alone, then `bagend` added, then `bagend`
    /// revoked at epoch 2.
    struct World {
        owner: Owner,
        rivendell: Identity,
        bagend: Identity,
        id: String,
        files: Vec<Vec<u8>>,
        root: Scratch,
    }

    impl World {
        fn scope(&self) -> manifest::Scope {
            manifest::read_scope(&self.root.0, &self.id).unwrap()
        }

        fn forged(&self, base: usize, change: impl FnOnce(&mut manifest::Manifest)) -> Vec<u8> {
            let mut m = self.scope().versions[base].manifest.clone();
            change(&mut m);
            manifest::signed(m, &self.owner.sign).1
        }

        fn target(&self, n: u64) -> String {
            format!("/v1/{}", transport::manifest_path(&self.id, n))
        }

        fn segment(&self, who: &Identity, seq: u64) -> String {
            format!(
                "/v1/{}",
                transport::segment_path(&self.id, &who.device.id(), seq)
            )
        }

        fn held(&self, upto: usize, highest: &[(&Identity, u64)]) -> Held {
            Held {
                chain: manifest::verify_scope(&self.id, &self.files[..upto]),
                standing: Standing::Valid,
                bytes: 0,
                reserved: 0,
                highest: highest
                    .iter()
                    .map(|(who, last)| (who.device.id(), *last))
                    .collect(),
            }
        }
    }

    fn world(name: &str, owner_seed: u8, device_seed: u8) -> World {
        let root = scratch(name);
        let owner = Owner::derive(&[owner_seed; 16]);
        let rivendell = identity(&owner, "rivendell", device_seed + 1);
        let bagend = identity(&owner, "bagend", device_seed + 3);
        let lock = manifest::lock(&root.0).unwrap();
        let id = manifest::create(&lock, &rivendell, "personal", "file:///x", &[])
            .unwrap()
            .scope;
        let survey = |who: &Identity| {
            let owner_key = who.owner.sign.public();
            manifest::survey(
                &root.0,
                Some(&owner_key),
                Some(&Recipient::device(&who.device)),
            )
            .unwrap()
        };
        let known = survey(&bagend);
        let manifest::Outcome::Updated(_) =
            manifest::recover_step(&lock, &bagend, &owner.box_secret, &known[0])
        else {
            panic!("bagend was not added");
        };
        let known = survey(&rivendell);
        let manifest::Outcome::Updated(w) =
            manifest::revoke_step(&lock, &rivendell, &known[0], &bagend.device.id())
        else {
            panic!("bagend was not revoked");
        };
        assert_eq!((w.n, w.epoch), (3, 2));
        drop(lock);
        let files = manifest::read_scope(&root.0, &id)
            .unwrap()
            .versions
            .iter()
            .map(|v| v.bytes.clone())
            .collect();
        World {
            owner,
            rivendell,
            bagend,
            id,
            files,
            root,
        }
    }

    fn nobody() -> SignKey {
        SignKey::from_seed(&[90; 32])
    }

    fn flags(data: PathBuf, owners: &[&Owner]) -> Flags {
        Flags {
            data,
            owners: owners.iter().map(|o| print(o)).collect(),
            listen: "127.0.0.1:0".parse().unwrap(),
            max_scopes: 16,
            max_scope_mb: 1024,
            max_object_mb: 16,
        }
    }

    fn own(headers: Vec<(&'static str, String)>) -> Vec<(String, String)> {
        headers
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect()
    }

    /// What a client reads back.
    struct Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl Reply {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
        }

        fn error(&self) -> String {
            reason_of(&Response::new(self.status, self.body.clone()))
        }

        fn json(&self) -> Value {
            serde_json::from_slice(&self.body).unwrap()
        }
    }

    fn parse_reply(bytes: &[u8]) -> Reply {
        let at = bytes
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("a response head");
        let head = String::from_utf8(bytes[..at].to_vec()).unwrap();
        let mut lines = head.split("\r\n");
        let status = lines
            .next()
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers = lines
            .map(|line| {
                let (name, value) = line.split_once(": ").unwrap();
                (name.to_ascii_lowercase(), value.to_string())
            })
            .collect();
        Reply {
            status,
            headers,
            body: bytes[at + 4..].to_vec(),
        }
    }

    fn connect(port: u16) -> TcpStream {
        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        stream
    }

    fn read_reply(mut stream: TcpStream) -> Reply {
        let mut bytes = Vec::new();
        let _ = stream.read_to_end(&mut bytes);
        parse_reply(&bytes)
    }

    fn head_of(
        method: &str,
        target: &str,
        headers: &[(String, String)],
        length: Option<usize>,
    ) -> String {
        let mut head = format!("{method} {target} HTTP/1.1\r\nHost: relay\r\n");
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        if let Some(length) = length {
            head.push_str(&format!("Content-Length: {length}\r\n"));
        }
        head.push_str("\r\n");
        head
    }

    /// A relay on loopback with a clock the test moves and a log it can read.
    struct Rig {
        relay: Running,
        clock: Arc<AtomicU64>,
        lines: Arc<Mutex<Vec<String>>>,
        data: PathBuf,
        _scratch: Scratch,
    }

    impl Rig {
        fn new(name: &str, owners: &[&Owner]) -> Rig {
            Rig::with(name, owners, |_| {})
        }

        fn with(name: &str, owners: &[&Owner], tweak: impl FnOnce(&mut Flags)) -> Rig {
            let scratch = scratch(&format!("rig-{name}"));
            let data = scratch.0.join("data");
            let mut flags = flags(data.clone(), owners);
            tweak(&mut flags);
            let clock = Arc::new(AtomicU64::new(T0));
            let ticking = Arc::clone(&clock);
            let lines = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&lines);
            let relay = relay::start(
                flags,
                Arc::new(move || ticking.load(Ordering::SeqCst)),
                Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string())),
            )
            .unwrap();
            Rig {
                relay,
                clock,
                lines,
                data,
                _scratch: scratch,
            }
        }

        fn now(&self) -> u64 {
            self.clock.load(Ordering::SeqCst)
        }

        fn advance(&self, seconds: u64) {
            self.clock.fetch_add(seconds, Ordering::SeqCst);
        }

        /// The log lines after the startup lines.
        fn lines(&self) -> Vec<String> {
            self.lines
                .lock()
                .unwrap()
                .iter()
                .filter(|line| !line.starts_with("relay "))
                .cloned()
                .collect()
        }

        fn send(
            &self,
            method: &str,
            target: &str,
            headers: &[(String, String)],
            body: &[u8],
        ) -> Reply {
            let mut stream = connect(self.relay.port);
            let length = (method == "PUT").then_some(body.len());
            stream
                .write_all(head_of(method, target, headers, length).as_bytes())
                .unwrap();
            stream.write_all(body).unwrap();
            read_reply(stream)
        }

        fn raw(&self, bytes: &[u8]) -> Reply {
            let mut stream = connect(self.relay.port);
            stream.write_all(bytes).unwrap();
            read_reply(stream)
        }

        fn signed_at(
            &self,
            time: u64,
            who: &SignKey,
            method: &str,
            target: &str,
            body: &[u8],
        ) -> Reply {
            let headers = own(sign::headers(who, method, target, time, body).unwrap());
            self.send(method, target, &headers, body)
        }

        fn call(&self, who: &SignKey, method: &str, target: &str, body: &[u8]) -> Reply {
            self.signed_at(self.now(), who, method, target, body)
        }

        fn get(&self, who: &SignKey, target: &str) -> Reply {
            self.call(who, "GET", target, b"")
        }

        fn put(&self, who: &SignKey, target: &str, body: &[u8]) -> Reply {
            self.call(who, "PUT", target, body)
        }

        /// Creates manifests 1 to `upto` of `w` as `rivendell`.
        fn publish(&self, w: &World, upto: usize) {
            for n in 1..=upto {
                let reply = self.put(
                    &w.rivendell.device.sign,
                    &w.target(n as u64),
                    &w.files[n - 1],
                );
                assert_eq!(reply.status, 201, "manifest {n}: {}", reply.error());
            }
        }

        fn tmp_is_empty(&self) -> bool {
            fs::read_dir(self.data.join(".tmp"))
                .unwrap()
                .next()
                .is_none()
        }
    }

    fn url(w: &World, tail: &str) -> String {
        format!("/v1/scopes/{}/{tail}", w.id)
    }

    // The tree's root, framing and bodies.

    #[test]
    fn the_root_answers_without_a_signature() {
        let rig = Rig::new("root", &[]);
        let reply = rig.send("GET", "/v1/", &[], b"");
        assert_eq!(reply.status, 200);
        assert_eq!(reply.json(), json!({"relay": "bilbo", "api": 1}));
        assert_eq!(reply.header("bilbo-time"), Some(T0.to_string().as_str()));
    }

    #[test]
    fn only_get_and_put_are_methods_and_put_only_on_objects() {
        let w = world("methods", 10, 20);
        let rig = Rig::new("methods", &[&w.owner]);
        rig.publish(&w, 1);
        let key = &w.rivendell.device.sign;
        let seg = w.segment(&w.rivendell, 1);
        assert_eq!(rig.put(key, &seg, b"one").status, 201);
        let reply = rig.call(key, "DELETE", &seg, b"");
        assert_eq!(reply.status, 405);
        assert_eq!(rig.get(key, &seg).body, b"one");
        for (method, target) in [
            ("PUT", "/v1/".to_string()),
            ("PUT", "/v1/scopes/".to_string()),
            ("PUT", url(&w, "manifest/latest")),
            ("PUT", url(&w, "devices/")),
            ("POST", w.target(2)),
            ("HEAD", w.target(1)),
        ] {
            let reply = rig.call(key, method, &target, b"x");
            assert_eq!(reply.status, 405, "{method} {target}");
        }
    }

    #[test]
    fn a_create_answers_with_no_body() {
        let w = world("no_body", 10, 20);
        let rig = Rig::new("no_body", &[&w.owner]);
        rig.publish(&w, 1);
        let reply = rig.put(
            &w.rivendell.device.sign,
            &w.segment(&w.rivendell, 1),
            b"one",
        );
        assert_eq!(reply.status, 201);
        assert_eq!(reply.header("content-length"), Some("0"));
        assert!(reply.body.is_empty());
    }

    #[test]
    fn a_target_outside_the_grammar_is_a_bad_request() {
        let w = world("grammar", 10, 20);
        let rig = Rig::new("grammar", &[&w.owner]);
        for target in [
            "/v2/scopes/",
            "/",
            "http://relay/v1/",
            "/v1/scopes/%41/devices/",
        ] {
            let reply = rig.send("GET", target, &[], b"");
            assert_eq!(
                (reply.status, reply.error().as_str()),
                (400, "bad-request"),
                "{target}"
            );
        }
        let key = &w.rivendell.device.sign;
        let id = &w.id;
        let device = w.rivendell.device.id();
        for target in [
            "/v1/scopes/not-an-id/devices/".to_string(),
            "/v1/scopes".to_string(),
            "/v1/scopes//devices/".to_string(),
            format!("/v1/scopes/{id}/../scopes/{id}/devices/"),
            format!("/v1/scopes/{id}/./devices/"),
            format!("/v1/scopes/{id}/manifest/01.json"),
            format!("/v1/scopes/{id}/manifest/0.json"),
            format!("/v1/scopes/{id}/manifest/+1.json"),
            format!("/v1/scopes/{id}/manifest/latest?x=1"),
            format!("/v1/scopes/{id}/devices/?after=1"),
            format!("/v1/scopes/{id}/devices/{device}"),
            format!("/v1/scopes/{id}/devices/{device}/1.seg"),
            format!("/v1/scopes/{id}/devices/{device}/00000000000000000000.seg"),
            format!("/v1/scopes/{id}/devices/{device}/?after=1&x=2"),
            format!("/v1/scopes/{id}/devices/{device}/?x=1"),
            "/v1/pair/UPPER/a.msg".to_string(),
            "/v1/pair/42/a.txt".to_string(),
            format!("/v1/pair/42/{}.msg", "a".repeat(17)),
            "/v1/pair//a.msg".to_string(),
        ] {
            let reply = rig.get(key, &target);
            assert_eq!(
                (reply.status, reply.error().as_str()),
                (400, "bad-request"),
                "{target}"
            );
        }
    }

    #[test]
    fn a_bad_cursor_is_a_bad_request() {
        let w = world("cursor", 10, 20);
        let rig = Rig::new("cursor", &[&w.owner]);
        rig.publish(&w, 1);
        let key = &w.rivendell.device.sign;
        let device = w.rivendell.device.id();
        for after in ["-1", "x", "01", "+1", "", "18446744073709551616"] {
            let target = url(&w, &format!("devices/{device}/?after={after}"));
            let reply = rig.get(key, &target);
            assert_eq!(reply.status, 400, "after={after}");
        }
        let ok = rig.get(key, &url(&w, &format!("devices/{device}/?after=0")));
        assert_eq!(ok.json(), json!({"seqs": [], "more": false}));
    }

    #[test]
    fn framing_refusals_store_nothing() {
        let w = world("framing", 10, 20);
        let rig = Rig::new("framing", &[&w.owner]);
        let target = w.target(1);
        let smuggled = format!(
            "PUT {target} HTTP/1.1\r\nHost: relay\r\nTransfer-Encoding: chunked\r\nContent-Length: 10\r\n\r\n0123456789"
        );
        let twice = format!(
            "PUT {target} HTTP/1.1\r\nHost: relay\r\nContent-Length: 10\r\nContent-Length: 10\r\n\r\n0123456789"
        );
        let plus = format!(
            "PUT {target} HTTP/1.1\r\nHost: relay\r\nContent-Length: +10\r\n\r\n0123456789"
        );
        for bytes in [smuggled, twice, plus, "hello world\r\n\r\n".to_string()] {
            let reply = rig.raw(bytes.as_bytes());
            assert_eq!(
                (reply.status, reply.error().as_str()),
                (400, "bad-request"),
                "{bytes}"
            );
        }
        assert!(!rig.data.join("scopes").exists());
    }

    #[test]
    fn a_put_without_a_length_is_411() {
        let w = world("no_length", 10, 20);
        let rig = Rig::new("no_length", &[&w.owner]);
        let bytes = format!("PUT {} HTTP/1.1\r\nHost: relay\r\n\r\n", w.target(1));
        assert_eq!(rig.raw(bytes.as_bytes()).status, 411);
        assert!(!rig.data.join("scopes").exists());
    }

    #[test]
    fn a_proxy_that_expects_continue_gets_it() {
        let w = world("expect", 10, 20);
        let rig = Rig::new("expect", &[&w.owner]);
        let key = &w.rivendell.device.sign;
        rig.publish(&w, 1);
        let target = w.segment(&w.rivendell, 1);
        let mut headers = own(sign::headers(key, "PUT", &target, rig.now(), b"one").unwrap());
        headers.push(("Expect".into(), "100-continue".into()));
        let mut stream = connect(rig.relay.port);
        stream
            .write_all(head_of("PUT", &target, &headers, Some(3)).as_bytes())
            .unwrap();
        let mut interim = [0u8; 25];
        stream.read_exact(&mut interim).unwrap();
        assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
        stream.write_all(b"one").unwrap();
        assert_eq!(read_reply(stream).status, 201);
    }

    #[test]
    fn a_segment_over_the_cap_is_refused_before_its_body() {
        let w = world("too_large", 10, 20);
        let rig = Rig::new("too_large", &[&w.owner]);
        rig.publish(&w, 1);
        let key = &w.rivendell.device.sign;
        let target = w.segment(&w.rivendell, 1);
        let headers = own(sign::headers(key, "PUT", &target, rig.now(), b"x").unwrap());
        let mut stream = connect(rig.relay.port);
        let length = 17 * 1024 * 1024;
        stream
            .write_all(head_of("PUT", &target, &headers, Some(length)).as_bytes())
            .unwrap();
        let _ = stream.write_all(&vec![0u8; 256 * 1024]);
        let reply = read_reply(stream);
        assert_eq!((reply.status, reply.error().as_str()), (413, "too-large"));
        assert!(rig.tmp_is_empty());
        assert!(
            !rig.data
                .join(transport::segment_path(&w.id, &w.rivendell.device.id(), 1))
                .exists()
        );
    }

    // Signatures, the time window and replays.

    #[test]
    fn a_refusal_names_its_reason_and_the_time() {
        let w = world("reason", 10, 20);
        let rig = Rig::new("reason", &[&w.owner]);
        let mut headers =
            own(sign::headers(&nobody(), "GET", "/v1/scopes/", rig.now(), b"").unwrap());
        headers[3].1 = "0".repeat(128);
        let reply = rig.send("GET", "/v1/scopes/", &headers, b"");
        assert_eq!((reply.status, reply.error().as_str()), (401, "signature"));
        assert_eq!(reply.header("bilbo-time"), Some(T0.to_string().as_str()));
    }

    #[test]
    fn a_forged_request_does_not_burn_a_nonce() {
        let w = world("forged_nonce", 10, 20);
        let rig = Rig::new("forged_nonce", &[&w.owner]);
        rig.publish(&w, 1);
        let target = url(&w, "devices/");
        let key = &w.rivendell.device.sign;
        let good = own(sign::headers(key, "GET", &target, rig.now(), b"").unwrap());
        let mut forged = good.clone();
        forged[3].1 = "0".repeat(128);
        assert_eq!(rig.send("GET", &target, &forged, b"").status, 401);
        assert_eq!(rig.send("GET", &target, &good, b"").status, 200);
    }

    #[test]
    fn a_stranger_learns_nothing_of_a_scope() {
        let w = world("stranger", 10, 20);
        let rig = Rig::new("stranger", &[&w.owner]);
        rig.publish(&w, 1);
        let held = rig.get(&nobody(), &url(&w, "devices/"));
        let unheld = rig.get(&nobody(), "/v1/scopes/aaaaaaaaaaaaaaaaaaaaaaaaaa/devices/");
        assert_eq!((held.status, held.error().as_str()), (403, "not-admitted"));
        assert_eq!((unheld.status, unheld.error()), (403, held.error()));
    }

    #[test]
    fn a_listed_device_with_a_valid_signature_is_served() {
        let w = world("valid", 10, 20);
        let rig = Rig::new("valid", &[&w.owner]);
        rig.publish(&w, 1);
        let reply = rig.get(&w.rivendell.device.sign, &url(&w, "devices/"));
        assert_eq!(reply.status, 200);
        assert_eq!(reply.json(), json!({"devices": []}));
    }

    #[test]
    fn a_body_changed_in_transit_stores_nothing() {
        let w = world("changed", 10, 20);
        let rig = Rig::new("changed", &[&w.owner]);
        rig.publish(&w, 1);
        let key = &w.rivendell.device.sign;
        let target = w.segment(&w.rivendell, 1);
        let headers = own(sign::headers(key, "PUT", &target, rig.now(), b"aaaa").unwrap());
        let reply = rig.send("PUT", &target, &headers, b"aaab");
        assert_eq!((reply.status, reply.error().as_str()), (401, "signature"));
        assert!(
            !rig.data
                .join(transport::segment_path(&w.id, &w.rivendell.device.id(), 1))
                .exists()
        );
        assert!(rig.tmp_is_empty());
    }

    #[test]
    fn every_request_under_scopes_needs_a_signature() {
        let w = world("unsigned", 10, 20);
        let rig = Rig::new("unsigned", &[&w.owner]);
        for target in [
            "/v1/scopes/".to_string(),
            url(&w, "devices/"),
            url(&w, "manifest/latest"),
        ] {
            let reply = rig.send("GET", &target, &[], b"");
            assert_eq!(
                (reply.status, reply.error().as_str()),
                (401, "signature"),
                "{target}"
            );
        }
        let partial = own(sign::headers(&nobody(), "GET", "/v1/scopes/", rig.now(), b"").unwrap());
        let reply = rig.send("GET", "/v1/scopes/", &partial[..3], b"");
        assert_eq!((reply.status, reply.error().as_str()), (401, "signature"));
    }

    #[test]
    fn a_time_off_by_more_than_five_minutes_is_a_clock_refusal() {
        let w = world("clock", 10, 20);
        let rig = Rig::new("clock", &[&w.owner]);
        rig.publish(&w, 1);
        let key = &w.rivendell.device.sign;
        let target = url(&w, "devices/");
        let late = rig.signed_at(T0 - 301, key, "GET", &target, b"");
        assert_eq!((late.status, late.error().as_str()), (401, "clock"));
        assert_eq!(late.header("bilbo-time"), Some(T0.to_string().as_str()));
        let early = rig.signed_at(T0 + 301, key, "GET", &target, b"");
        assert_eq!(early.status, 401);
        assert_eq!(
            rig.signed_at(T0 - 300, key, "GET", &target, b"").status,
            200
        );
    }

    #[test]
    fn a_replayed_request_is_refused() {
        let w = world("replay", 10, 20);
        let rig = Rig::new("replay", &[&w.owner]);
        rig.publish(&w, 1);
        let target = url(&w, "devices/");
        let headers =
            own(sign::headers(&w.rivendell.device.sign, "GET", &target, rig.now(), b"").unwrap());
        assert_eq!(rig.send("GET", &target, &headers, b"").status, 200);
        rig.advance(10);
        let again = rig.send("GET", &target, &headers, b"");
        assert_eq!((again.status, again.error().as_str()), (401, "replay"));
    }

    #[test]
    fn a_replayed_nameplate_opener_opens_nothing() {
        let w = world("opener", 10, 20);
        let rig = Rig::new("opener", &[&w.owner]);
        rig.publish(&w, 1);
        let key = &w.rivendell.device.sign;
        let target = "/v1/pair/42/a.msg";
        let headers = own(sign::headers(key, "PUT", target, rig.now(), b"hello").unwrap());
        assert_eq!(rig.send("PUT", target, &headers, b"hello").status, 201);
        rig.advance(31 * 60);
        let again = rig.send("PUT", target, &headers, b"hello");
        assert_eq!((again.status, again.error().as_str()), (401, "clock"));
    }

    #[test]
    fn the_nonce_cache_remembers_ten_minutes_and_holds_100000() {
        let nonces = Nonces::default();
        let (key, nonce) = ([1; 32], [2; 16]);
        assert_eq!(nonces.accept(&key, &nonce, T0), Ok(()));
        assert_eq!(nonces.accept(&key, &nonce, T0 + 599), Err(Reject::Replayed));
        assert_eq!(nonces.accept(&[3; 32], &nonce, T0 + 599), Ok(()));
        assert_eq!(nonces.accept(&key, &nonce, T0 + 600), Ok(()));
        let full = Nonces::default();
        for i in 0..NONCES as u64 {
            let mut nonce = [0; 16];
            nonce[..8].copy_from_slice(&i.to_le_bytes());
            assert_eq!(full.accept(&key, &nonce, T0), Ok(()));
        }
        assert_eq!(full.accept(&key, &[9; 16], T0 + 1), Err(Reject::Full));
        assert_eq!(full.accept(&key, &[9; 16], T0 + 600), Ok(()));
    }

    // Who may use a scope.

    #[test]
    fn a_revoked_device_is_not_admitted() {
        let w = world("revoked", 10, 20);
        let rig = Rig::new("revoked", &[&w.owner]);
        rig.publish(&w, 3);
        let seg = w.segment(&w.rivendell, 1);
        assert_eq!(rig.put(&w.rivendell.device.sign, &seg, b"one").status, 201);
        let reply = rig.get(&w.bagend.device.sign, &seg);
        assert_eq!(
            (reply.status, reply.error().as_str()),
            (403, "not-admitted")
        );
    }

    #[test]
    fn a_device_writes_only_its_own_folder() {
        let w = world("own_folder", 10, 20);
        let rig = Rig::new("own_folder", &[&w.owner]);
        rig.publish(&w, 2);
        let reply = rig.put(&w.rivendell.device.sign, &w.segment(&w.bagend, 1), b"one");
        assert_eq!(
            (reply.status, reply.error().as_str()),
            (403, "not-admitted")
        );
        assert!(
            !rig.data
                .join(transport::segment_path(&w.id, &w.bagend.device.id(), 1))
                .exists()
        );
        assert!(rig.tmp_is_empty());
    }

    #[test]
    fn the_owner_reads_manifests_and_nothing_else() {
        let w = world("owner_reads", 10, 20);
        let rig = Rig::new("owner_reads", &[&w.owner]);
        rig.publish(&w, 3);
        let seg = w.segment(&w.rivendell, 1);
        assert_eq!(rig.put(&w.rivendell.device.sign, &seg, b"one").status, 201);
        let owner = &w.owner.sign;
        let latest = rig.get(owner, &url(&w, "manifest/latest"));
        assert_eq!(latest.status, 200);
        assert_eq!(latest.body, w.files[2]);
        assert_eq!(rig.get(owner, &w.target(1)).body, w.files[0]);
        let device = w.rivendell.device.id();
        for target in [
            url(&w, "devices/"),
            url(&w, &format!("devices/{device}/")),
            seg,
        ] {
            let reply = rig.get(owner, &target);
            assert_eq!(
                (reply.status, reply.error().as_str()),
                (403, "not-admitted"),
                "{target}"
            );
        }
    }

    #[test]
    fn a_key_that_is_nobody_is_not_admitted() {
        let w = world("nobody", 10, 20);
        let rig = Rig::new("nobody", &[&w.owner]);
        rig.publish(&w, 1);
        for target in [url(&w, "manifest/latest"), url(&w, "devices/"), w.target(1)] {
            let reply = rig.get(&nobody(), &target);
            assert_eq!(
                (reply.status, reply.error().as_str()),
                (403, "not-admitted"),
                "{target}"
            );
        }
    }

    // Admitting a scope and the manifest chain.

    #[test]
    fn a_new_scope_is_admitted_through_its_first_manifest() {
        let w = world("new_scope", 10, 20);
        let rig = Rig::new("new_scope", &[&w.owner]);
        rig.publish(&w, 1);
        assert!(rig.data.join(transport::manifest_path(&w.id, 1)).exists());
        let reply = rig.put(
            &w.rivendell.device.sign,
            &w.segment(&w.rivendell, 1),
            b"one",
        );
        assert_eq!(reply.status, 201);
    }

    #[test]
    fn another_owners_scope_keeps_nothing() {
        let w = world("other_owner", 10, 20);
        let other = Owner::derive(&[99; 16]);
        let rig = Rig::new("other_owner", &[&other]);
        let reply = rig.put(&w.rivendell.device.sign, &w.target(1), &w.files[0]);
        assert_eq!(
            (reply.status, reply.error().as_str()),
            (403, "not-admitted")
        );
        assert!(!rig.data.join("scopes").join(&w.id).exists());
        assert!(rig.tmp_is_empty());
    }

    #[test]
    fn a_forged_owner_signature_is_a_manifest_failure() {
        let w = world("forged_sig", 10, 20);
        let rig = Rig::new("forged_sig", &[&w.owner]);
        let mut m = w.scope().versions[0].manifest.clone();
        m.sig = "0".repeat(128);
        let mut bytes = serde_json::to_vec(&m).unwrap();
        bytes.push(b'\n');
        let reply = rig.put(&w.rivendell.device.sign, &w.target(1), &bytes);
        assert_eq!((reply.status, reply.error().as_str()), (422, "manifest"));
        assert!(!rig.data.join("scopes").join(&w.id).exists());
    }

    #[test]
    fn an_uploader_the_manifest_does_not_list_is_not_admitted() {
        let w = world("uploader", 10, 20);
        let rig = Rig::new("uploader", &[&w.owner]);
        let reply = rig.put(&nobody(), &w.target(1), &w.files[0]);
        assert_eq!(
            (reply.status, reply.error().as_str()),
            (403, "not-admitted")
        );
        assert!(!rig.data.join("scopes").join(&w.id).exists());
    }

    #[test]
    fn enrolling_a_device_admits_it_from_then_on() {
        let w = world("enroll", 10, 20);
        let rig = Rig::new("enroll", &[&w.owner]);
        rig.publish(&w, 1);
        let bagend = &w.bagend.device.sign;
        assert_eq!(rig.get(bagend, &url(&w, "devices/")).status, 403);
        let reply = rig.put(&w.rivendell.device.sign, &w.target(2), &w.files[1]);
        assert_eq!(reply.status, 201);
        assert_eq!(rig.get(bagend, &url(&w, "devices/")).status, 200);
    }

    #[test]
    fn a_skipped_version_is_not_next() {
        let w = world("skipped", 10, 20);
        let rig = Rig::new("skipped", &[&w.owner]);
        rig.publish(&w, 1);
        let reply = rig.put(&w.rivendell.device.sign, &w.target(3), &w.files[2]);
        assert_eq!((reply.status, reply.error().as_str()), (409, "not-next"));
        assert!(!rig.data.join(transport::manifest_path(&w.id, 3)).exists());
    }

    #[test]
    fn two_manifests_at_one_n_keep_the_first() {
        let w = world("race", 10, 20);
        let rig = Rig::new("race", &[&w.owner]);
        rig.publish(&w, 1);
        let key = &w.rivendell.device.sign;
        assert_eq!(rig.put(key, &w.target(2), &w.files[1]).status, 201);
        let other = w.forged(1, |m| m.name.push_str("00"));
        let reply = rig.put(key, &w.target(2), &other);
        assert_eq!((reply.status, reply.error().as_str()), (409, "exists"));
        let stored = fs::read(rig.data.join(transport::manifest_path(&w.id, 2))).unwrap();
        assert_eq!(stored, w.files[1]);
        assert_eq!(rig.put(key, &w.target(2), &w.files[1]).status, 200);
    }

    #[test]
    fn an_older_manifest_with_other_bytes_exists() {
        let w = world("older", 10, 20);
        let rig = Rig::new("older", &[&w.owner]);
        rig.publish(&w, 3);
        let other = w.forged(1, |m| m.name.push_str("00"));
        let reply = rig.put(&w.rivendell.device.sign, &w.target(2), &other);
        assert_eq!((reply.status, reply.error().as_str()), (409, "exists"));
    }

    #[test]
    fn a_manifest_that_fails_a_check_is_422() {
        let w = world("wrong_prev", 10, 20);
        let rig = Rig::new("wrong_prev", &[&w.owner]);
        rig.publish(&w, 1);
        let bytes = w.forged(1, |m| m.prev = Some("0".repeat(64)));
        let reply = rig.put(&w.rivendell.device.sign, &w.target(2), &bytes);
        assert_eq!((reply.status, reply.error().as_str()), (422, "manifest"));
        let reply = rig.put(&nobody(), &w.target(2), &bytes);
        assert_eq!(
            (reply.status, reply.error().as_str()),
            (403, "not-admitted")
        );
    }

    #[test]
    fn the_owner_recovers_by_signing_the_next_manifest() {
        let w = world("recover", 10, 20);
        let rig = Rig::new("recover", &[&w.owner]);
        rig.publish(&w, 1);
        let reply = rig.put(&w.owner.sign, &w.target(2), &w.files[1]);
        assert_eq!(reply.status, 201);
        assert_eq!(
            rig.get(&w.bagend.device.sign, &url(&w, "devices/")).status,
            200
        );
    }

    // Create-only objects and reads.

    #[test]
    fn a_segment_is_created_once_and_served() {
        let w = world("segments", 10, 20);
        let rig = Rig::new("segments", &[&w.owner]);
        rig.publish(&w, 1);
        let key = &w.rivendell.device.sign;
        let (one, two) = (w.segment(&w.rivendell, 1), w.segment(&w.rivendell, 2));
        assert_eq!(rig.put(key, &one, b"one").status, 201);
        assert_eq!(rig.get(key, &one).body, b"one");
        assert_eq!(rig.put(key, &one, b"one").status, 200);
        let overwrite = rig.put(key, &one, b"two");
        assert_eq!(
            (overwrite.status, overwrite.error().as_str()),
            (409, "exists")
        );
        assert_eq!(rig.get(key, &one).body, b"one");
        let gap = rig.put(key, &w.segment(&w.rivendell, 3), b"three");
        assert_eq!((gap.status, gap.error().as_str()), (409, "not-next"));
        assert_eq!(rig.put(key, &two, &[0xff, 0x00, b'{']).status, 201);
        assert_eq!(rig.get(key, &two).body, [0xff, 0x00, b'{']);
        assert!(rig.tmp_is_empty());
    }

    #[test]
    fn reads_answer_the_latest_manifest_and_missing_objects() {
        let w = world("reads", 10, 20);
        let rig = Rig::new("reads", &[&w.owner]);
        rig.publish(&w, 3);
        let key = &w.rivendell.device.sign;
        let latest = rig.get(key, &url(&w, "manifest/latest"));
        assert_eq!(latest.body, w.files[2]);
        assert_eq!(latest.header("bilbo-manifest"), Some("3"));
        assert_eq!(rig.get(key, &w.target(2)).body, w.files[1]);
        for target in [w.target(4), w.segment(&w.rivendell, 1)] {
            let reply = rig.get(key, &target);
            assert_eq!(
                (reply.status, reply.error().as_str()),
                (404, "not-found"),
                "{target}"
            );
        }
        assert_eq!(
            rig.put(key, &w.segment(&w.rivendell, 1), b"one").status,
            201
        );
        let past = rig.get(key, &w.segment(&w.rivendell, 2));
        assert_eq!(past.status, 404);
    }

    #[test]
    fn listings_name_every_folder_and_its_seqs() {
        let w = world("listings", 10, 20);
        let rig = Rig::new("listings", &[&w.owner]);
        rig.publish(&w, 2);
        let (a, b) = (&w.rivendell, &w.bagend);
        for seq in 1..=3 {
            assert_eq!(
                rig.put(&a.device.sign, &w.segment(a, seq), b"a").status,
                201
            );
        }
        for seq in 1..=12 {
            assert_eq!(
                rig.put(&b.device.sign, &w.segment(b, seq), b"b").status,
                201
            );
        }
        let mut expected = [(a.device.id(), 3), (b.device.id(), 12)];
        expected.sort();
        let want: Vec<Value> = expected
            .iter()
            .map(|(id, last)| json!({"id": id, "last": last}))
            .collect();
        let listing = rig.get(&a.device.sign, &url(&w, "devices/"));
        assert_eq!(listing.json(), json!({ "devices": want }));
        let folder = |after: Option<u64>| {
            let query = after.map_or(String::new(), |a| format!("?after={a}"));
            let target = url(&w, &format!("devices/{}/{query}", b.device.id()));
            rig.get(&a.device.sign, &target).json()
        };
        assert_eq!(folder(None)["seqs"].as_array().unwrap().len(), 12);
        assert_eq!(folder(Some(10)), json!({"seqs": [11, 12], "more": false}));
        assert_eq!(folder(Some(12)), json!({"seqs": [], "more": false}));
    }

    #[test]
    fn the_owner_lists_only_its_own_scopes() {
        let first = world("lists_first", 10, 20);
        let second = world("lists_second", 11, 30);
        let third = world("lists_third", 10, 40);
        let rig = Rig::new("lists", &[&first.owner, &second.owner]);
        for w in [&first, &second, &third] {
            rig.publish(w, 1);
        }
        let mut mine = vec![first.id.clone(), third.id.clone()];
        mine.sort();
        let reply = rig.get(&first.owner.sign, "/v1/scopes/");
        assert_eq!(reply.json(), json!({ "scopes": mine }));
        let theirs = rig.get(&second.owner.sign, "/v1/scopes/");
        assert_eq!(theirs.json(), json!({"scopes": [second.id]}));
        let device = rig.get(&first.rivendell.device.sign, "/v1/scopes/");
        assert_eq!(
            (device.status, device.error().as_str()),
            (403, "not-admitted")
        );
    }

    // Limits.

    #[test]
    fn a_full_scope_refuses_a_create_and_still_reads() {
        let w = world("full", 10, 20);
        let rig = Rig::with("full", &[&w.owner], |f| f.max_scope_mb = 1);
        rig.publish(&w, 1);
        let key = &w.rivendell.device.sign;
        let big = vec![7u8; 1024 * 1024];
        let reply = rig.put(key, &w.segment(&w.rivendell, 1), &big);
        assert_eq!((reply.status, reply.error().as_str()), (507, "quota"));
        assert!(rig.tmp_is_empty());
        assert_eq!(rig.get(key, &w.target(1)).body, w.files[0]);
        assert_eq!(
            rig.put(key, &w.segment(&w.rivendell, 1), &big[..1000])
                .status,
            201
        );
    }

    #[test]
    fn an_object_over_the_cap_is_too_large_without_its_body() {
        let w = world("object_cap", 10, 20);
        let rig = Rig::with("object_cap", &[&w.owner], |f| f.max_object_mb = 1);
        rig.publish(&w, 1);
        let key = &w.rivendell.device.sign;
        let target = w.segment(&w.rivendell, 1);
        let headers = own(sign::headers(key, "PUT", &target, rig.now(), b"x").unwrap());
        let mut stream = connect(rig.relay.port);
        stream
            .write_all(head_of("PUT", &target, &headers, Some(1024 * 1024 + 1)).as_bytes())
            .unwrap();
        let _ = stream.write_all(&[0u8; 1024]);
        let reply = read_reply(stream);
        assert_eq!((reply.status, reply.error().as_str()), (413, "too-large"));
        let big = vec![0u8; 1024 * 1024];
        assert_eq!(rig.put(key, &target, &big).status, 201);
    }

    #[test]
    fn a_manifest_above_one_mebibyte_is_too_large() {
        let w = world("manifest_cap", 10, 20);
        let rig = Rig::new("manifest_cap", &[&w.owner]);
        let big = vec![b' '; 1024 * 1024 + 1];
        let reply = rig.put(&w.rivendell.device.sign, &w.target(1), &big);
        assert_eq!((reply.status, reply.error().as_str()), (413, "too-large"));
    }

    #[test]
    fn scopes_are_counted_per_owner() {
        let first = world("count_first", 10, 20);
        let second = world("count_second", 10, 30);
        let rig = Rig::with("count", &[&first.owner], |f| f.max_scopes = 1);
        rig.publish(&first, 1);
        let reply = rig.put(
            &second.rivendell.device.sign,
            &second.target(1),
            &second.files[0],
        );
        assert_eq!((reply.status, reply.error().as_str()), (507, "quota"));
        assert!(!rig.data.join("scopes").join(&second.id).exists());
    }

    // The mailbox's place.

    #[test]
    fn the_mailbox_paths_reach_the_mailbox() {
        let w = world("mailbox", 10, 20);
        let rig = Rig::new("mailbox", &[&w.owner]);
        rig.publish(&w, 1);
        let key = &w.rivendell.device.sign;
        assert_eq!(rig.put(key, "/v1/pair/42/a.msg", b"hello").status, 201);
        assert_eq!(
            rig.send("PUT", "/v1/pair/42/b.msg", &[], b"reply").status,
            201
        );
        let read = rig.send("GET", "/v1/pair/42/b.msg", &[], b"");
        assert_eq!(read.body, b"reply");
        assert_eq!(rig.send("GET", "/v1/pair/42/c.msg", &[], b"").status, 404);
        let stranger = rig.send("PUT", "/v1/pair/91/a.msg", &[], b"x");
        assert_eq!(
            (stranger.status, stranger.error().as_str()),
            (403, "not-admitted")
        );
        let forged = rig.put(&nobody(), "/v1/pair/43/a.msg", b"x");
        assert_eq!(
            (forged.status, forged.error().as_str()),
            (403, "not-admitted")
        );
    }

    // The handler on its own, with peers and bodies the test controls.

    struct Direct {
        state: State<'static>,
        clock: Arc<AtomicU64>,
        lines: Arc<Mutex<Vec<String>>>,
        _scratch: Scratch,
    }

    impl Direct {
        fn new(name: &str, owners: &[&Owner], held: BTreeMap<String, Held>) -> Direct {
            Direct::with(name, owners, held, |_| {})
        }

        fn with(
            name: &str,
            owners: &[&Owner],
            held: BTreeMap<String, Held>,
            tweak: impl FnOnce(&mut Flags),
        ) -> Direct {
            let scratch = scratch(&format!("direct-{name}"));
            let mut flags = flags(scratch.0.join("data"), owners);
            tweak(&mut flags);
            let clock = Arc::new(AtomicU64::new(T0));
            let ticking = Arc::clone(&clock);
            let lines = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&lines);
            let log: &'static (dyn Fn(&str) + Sync) = Box::leak(Box::new(move |line: &str| {
                sink.lock().unwrap().push(line.to_string());
            }));
            let data = relay::store::Data::open(&flags.data).unwrap();
            let mailbox = Mailbox::open(&data, T0).unwrap();
            let state = State {
                scopes: Scopes::new(&flags.owners, held),
                flags,
                clock: Arc::new(move || ticking.load(Ordering::SeqCst)),
                log,
                data,
                mailbox,
                nonces: Nonces::default(),
                refusals: Refusals::default(),
            };
            Direct {
                state,
                clock,
                lines,
                _scratch: scratch,
            }
        }

        fn lines(&self) -> Vec<String> {
            self.lines.lock().unwrap().clone()
        }

        fn request(
            &self,
            method: &str,
            target: &str,
            who: Option<&SignKey>,
            body: &[u8],
            peer: &str,
        ) -> Request {
            let headers = who.map_or_else(Vec::new, |key| {
                sign::headers(key, method, target, self.clock.load(Ordering::SeqCst), body)
                    .unwrap()
                    .into_iter()
                    .map(|(name, value)| (name.to_string(), value))
                    .collect()
            });
            Request {
                method: method.to_string(),
                target: target.to_string(),
                headers,
                length: (method == "PUT").then_some(body.len() as u64),
                peer: peer.parse::<IpAddr>().unwrap(),
            }
        }

        fn send(&self, request: &Request, body: &[u8]) -> Response {
            match self.state.head(request) {
                Head::Refuse(response) => response,
                Head::Read => {
                    let response = self.state.answer(request, &mut &body[..]);
                    self.state.finish();
                    response
                }
            }
        }

        fn get(&self, who: Option<&SignKey>, target: &str, peer: &str) -> Response {
            self.send(&self.request("GET", target, who, b"", peer), b"")
        }
    }

    fn json_of(response: &Response) -> Value {
        serde_json::from_slice(&response.body).unwrap()
    }

    #[test]
    fn a_post_without_a_length_is_405() {
        let relay = Direct::new("post", &[], BTreeMap::new());
        let request = relay.request("POST", "/v1/scopes/", None, b"", "127.0.0.1");
        assert_eq!(request.length, None);
        let Head::Refuse(response) = relay.state.head(&request) else {
            panic!("a POST was let in");
        };
        assert_eq!(response.status, 405);
    }

    #[test]
    fn a_long_folder_is_listed_a_page_at_a_time() {
        let w = world("page", 10, 20);
        let held = BTreeMap::from([(w.id.clone(), w.held(1, &[(&w.rivendell, 2500)]))]);
        let relay = Direct::new("page", &[&w.owner], held);
        let key = &w.rivendell.device.sign;
        let device = w.rivendell.device.id();
        let page = |after: u64| {
            let target = url(&w, &format!("devices/{device}/?after={after}"));
            json_of(&relay.get(Some(key), &target, "127.0.0.1"))
        };
        let first = page(0);
        let seqs = first["seqs"].as_array().unwrap();
        assert_eq!(
            (seqs.len(), seqs[0].clone(), seqs[999].clone()),
            (1000, json!(1), json!(1000))
        );
        assert_eq!(first["more"], json!(true));
        let last = page(2000);
        assert_eq!(last["seqs"].as_array().unwrap().len(), 500);
        assert_eq!(last["more"], json!(false));
        assert_eq!(page(2499), json!({"seqs": [2500], "more": false}));
        assert_eq!(page(2500), json!({"seqs": [], "more": false}));
        assert_eq!(page(u64::MAX), json!({"seqs": [], "more": false}));
        let listing = relay.get(Some(key), &url(&w, "devices/"), "127.0.0.1");
        assert_eq!(
            json_of(&listing),
            json!({"devices": [{"id": device, "last": 2500}]})
        );
    }

    /// Reads nothing until released, so a request stays in flight between its head and its answer.
    struct Gate {
        release: mpsc::Receiver<()>,
        body: io::Cursor<Vec<u8>>,
        waited: bool,
    }

    impl Read for Gate {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if !self.waited {
                self.waited = true;
                let _ = self.release.recv();
            }
            self.body.read(out)
        }
    }

    #[test]
    fn uploads_in_flight_count_against_the_scope() {
        let w = world("in_flight", 10, 20);
        let held = BTreeMap::from([(w.id.clone(), w.held(2, &[]))]);
        let relay = Direct::with("in_flight", &[&w.owner], held, |f| f.max_scope_mb = 1);
        let body = vec![1u8; 600_000];
        let first = relay.request(
            "PUT",
            &w.segment(&w.rivendell, 1),
            Some(&w.rivendell.device.sign),
            &body,
            "127.0.0.1",
        );
        let second = relay.request(
            "PUT",
            &w.segment(&w.bagend, 1),
            Some(&w.bagend.device.sign),
            &body,
            "127.0.0.1",
        );
        let (started, wait) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        std::thread::scope(|scope| {
            let uploading = scope.spawn(|| {
                assert!(matches!(relay.state.head(&first), Head::Read));
                started.send(()).unwrap();
                let mut gate = Gate {
                    release: gate,
                    body: io::Cursor::new(body.clone()),
                    waited: false,
                };
                relay.state.answer(&first, &mut gate)
            });
            wait.recv().unwrap();
            let Head::Refuse(refused) = relay.state.head(&second) else {
                panic!("the second upload was let in");
            };
            assert_eq!(
                (refused.status, reason_of(&refused).as_str()),
                (507, "quota")
            );
            release.send(()).unwrap();
            assert_eq!(uploading.join().unwrap().status, 201);
        });
        let held = relay.state.scopes.get(&w.id).unwrap();
        let held = held.lock().unwrap();
        assert_eq!(held.reserved, 0);
        assert_eq!(held.bytes, 600_000);
    }

    #[test]
    fn a_refused_upload_gives_its_reservation_back() {
        let w = world("release", 10, 20);
        let held = BTreeMap::from([(w.id.clone(), w.held(1, &[]))]);
        let relay = Direct::new("release", &[&w.owner], held);
        let body = vec![1u8; 1000];
        let stranger = nobody();
        let request = relay.request(
            "PUT",
            &w.segment(&w.rivendell, 1),
            Some(&stranger),
            &body,
            "127.0.0.1",
        );
        let refused = relay.send(&request, &body);
        assert_eq!(
            (refused.status, reason_of(&refused).as_str()),
            (403, "not-admitted")
        );
        let mut forged = relay.request(
            "PUT",
            &w.segment(&w.rivendell, 1),
            Some(&stranger),
            &body,
            "127.0.0.1",
        );
        forged
            .headers
            .iter_mut()
            .find(|(n, _)| n == "bilbo-signature")
            .unwrap()
            .1 = "0".repeat(128);
        assert_eq!(relay.send(&forged, &body).status, 401);
        let held = relay.state.scopes.get(&w.id).unwrap();
        assert_eq!(held.lock().unwrap().reserved, 0);
    }

    #[test]
    fn a_full_nonce_cache_is_busy() {
        let w = world("busy", 10, 20);
        let held = BTreeMap::from([(w.id.clone(), w.held(1, &[]))]);
        let relay = Direct::new("busy", &[&w.owner], held);
        for i in 0..NONCES as u64 {
            let mut nonce = [0; 16];
            nonce[..8].copy_from_slice(&i.to_le_bytes());
            relay.state.nonces.accept(&[1; 32], &nonce, T0).unwrap();
        }
        let key = &w.rivendell.device.sign;
        let target = url(&w, "devices/");
        let busy = relay.get(Some(key), &target, "127.0.0.1");
        assert_eq!((busy.status, reason_of(&busy).as_str()), (503, "busy"));
        assert!(busy.headers.iter().any(|(n, _)| n == "Retry-After"));
        relay.clock.fetch_add(REPLAY, Ordering::SeqCst);
        assert_eq!(relay.get(Some(key), &target, "127.0.0.1").status, 200);
    }

    #[test]
    fn the_peer_address_reaches_the_mailbox() {
        let relay = Direct::new("peers", &[], BTreeMap::new());
        let target = "/v1/pair/1/a.msg";
        for _ in 0..60 {
            assert_eq!(relay.get(None, target, "10.0.0.1").status, 404);
        }
        let limited = relay.get(None, target, "10.0.0.1");
        assert_eq!(
            (limited.status, reason_of(&limited).as_str()),
            (429, "rate")
        );
        assert!(limited.headers.iter().any(|(n, _)| n == "Retry-After"));
        assert_eq!(relay.get(None, target, "10.0.0.2").status, 404);
    }

    // What the relay logs.

    #[test]
    fn a_create_is_one_line_of_ids_and_sizes() {
        let w = world("log_creates", 10, 20);
        let rig = Rig::new("log_creates", &[&w.owner]);
        rig.publish(&w, 1);
        let device = w.rivendell.device.id();
        let size = w.files[0].len();
        assert_eq!(
            rig.lines(),
            vec![format!("manifest {} {device} 1 {size}", w.id)]
        );
        let key = &w.rivendell.device.sign;
        assert_eq!(
            rig.put(key, &w.segment(&w.rivendell, 1), b"seg-bytes")
                .status,
            201
        );
        assert_eq!(
            rig.put(key, "/v1/pair/np-zzqx/a.msg", b"SECRETBYTES")
                .status,
            201
        );
        assert_eq!(
            rig.put(&w.owner.sign, &w.target(2), &w.files[1]).status,
            201
        );
        let lines = rig.lines();
        assert_eq!(
            lines[1..],
            [
                format!("segment {} {device} 1 9", w.id),
                "mailbox 11".to_string(),
                format!("manifest {} owner 2 {}", w.id, w.files[1].len()),
            ]
        );
    }

    #[test]
    fn a_retried_create_and_a_read_log_nothing() {
        let w = world("log_quiet", 10, 20);
        let rig = Rig::new("log_quiet", &[&w.owner]);
        rig.publish(&w, 1);
        let key = &w.rivendell.device.sign;
        let seg = w.segment(&w.rivendell, 1);
        assert_eq!(rig.put(key, &seg, b"one").status, 201);
        let before = rig.lines().len();
        assert_eq!(rig.put(key, &seg, b"one").status, 200);
        for _ in 0..20 {
            assert_eq!(rig.get(key, &seg).status, 200);
            assert_eq!(rig.get(key, &url(&w, "devices/")).status, 200);
            assert_eq!(rig.get(key, &url(&w, "manifest/latest")).status, 200);
        }
        assert_eq!(rig.get(key, &w.segment(&w.rivendell, 2)).status, 404);
        assert_eq!(rig.lines().len(), before);
    }

    #[test]
    fn a_refusal_to_a_known_key_is_one_line_except_a_404() {
        let w = world("log_refusals", 10, 20);
        let rig = Rig::new("log_refusals", &[&w.owner]);
        rig.publish(&w, 3);
        let before = rig.lines().len();
        let key = &w.rivendell.device.sign;
        let theirs = w.segment(&w.bagend, 1);
        assert_eq!(rig.put(key, &theirs, b"x").status, 403);
        assert_eq!(rig.get(&nobody(), &url(&w, "devices/")).status, 403);
        assert_eq!(
            rig.get(&w.bagend.device.sign, &url(&w, "devices/")).status,
            403
        );
        assert_eq!(rig.put(key, &w.target(5), &w.files[2]).status, 409);
        assert_eq!(rig.get(key, &w.segment(&w.rivendell, 5)).status, 404);
        assert_eq!(rig.get(&w.owner.sign, &url(&w, "devices/")).status, 403);
        let lines = rig.lines();
        assert_eq!(
            lines[before..],
            [
                format!(
                    "refused 403 not-admitted PUT segment {} {}",
                    w.id,
                    w.bagend.device.id()
                ),
                format!("refused 409 not-next PUT manifest {}", w.id),
                format!("refused 403 not-admitted GET devices {}", w.id),
            ]
        );
    }

    #[test]
    fn a_507_is_logged_without_its_cause() {
        let w = world("log_failed", 10, 20);
        let rig = Rig::with("log_failed", &[&w.owner], |f| f.max_scope_mb = 1);
        rig.publish(&w, 1);
        let big = vec![7u8; 1024 * 1024];
        let key = &w.rivendell.device.sign;
        assert_eq!(rig.put(key, &w.segment(&w.rivendell, 1), &big).status, 507);
        let lines = rig.lines();
        assert_eq!(
            lines.last().unwrap(),
            &format!(
                "failed 507 quota PUT segment {} {}",
                w.id,
                w.rivendell.device.id()
            )
        );
    }

    #[test]
    fn what_serve_refuses_itself_is_counted() {
        let relay = Direct::new("serve_refusals", &[], BTreeMap::new());
        relay.state.refused(400, "bad-request");
        relay.state.refused(431, "bad-request");
        relay.state.refused(404, "not-found");
        tick(&relay.state, T0 + 60);
        assert_eq!(
            relay.lines(),
            ["refused 2 other requests in the last minute: bad-request 2"]
        );
    }

    #[test]
    fn a_booking_is_given_back_when_the_body_never_comes() {
        let w = world("finish", 10, 20);
        let held = BTreeMap::from([(w.id.clone(), w.held(1, &[]))]);
        let relay = Direct::new("finish", &[&w.owner], held);
        let body = vec![1u8; 1000];
        let key = &w.rivendell.device.sign;
        let request = relay.request(
            "PUT",
            &w.segment(&w.rivendell, 1),
            Some(key),
            &body,
            "127.0.0.1",
        );
        assert!(matches!(relay.state.head(&request), Head::Read));
        let reserved = || {
            relay
                .state
                .scopes
                .get(&w.id)
                .unwrap()
                .lock()
                .unwrap()
                .reserved
        };
        assert_eq!(reserved(), 1000);
        relay.state.finish();
        assert_eq!(reserved(), 0);
        relay.state.finish();
        assert_eq!(reserved(), 0);
    }

    #[test]
    fn unsigned_and_forged_refusals_are_counted_not_logged() {
        let w = world("log_count", 10, 20);
        let relay = Direct::new("log_count", &[&w.owner], BTreeMap::new());
        for _ in 0..10_000 {
            assert_eq!(relay.get(None, "/v1/scopes/", "10.0.0.9").status, 401);
        }
        let mut forged = relay.request("GET", "/v1/scopes/", Some(&nobody()), b"", "10.0.0.9");
        forged
            .headers
            .iter_mut()
            .find(|(n, _)| n == "bilbo-signature")
            .unwrap()
            .1 = "0".repeat(128);
        assert_eq!(relay.send(&forged, b"").status, 401);
        assert_eq!(relay.get(None, "/v2/", "10.0.0.9").status, 400);
        assert!(relay.lines().is_empty());
        tick(&relay.state, T0 + 59);
        assert!(relay.lines().is_empty());
        tick(&relay.state, T0 + 60);
        assert_eq!(
            relay.lines(),
            ["refused 10002 other requests in the last minute: bad-request 1, signature 10001"]
        );
        tick(&relay.state, T0 + 400);
        assert_eq!(relay.lines().len(), 1);
        relay.clock.store(T0 + 400, Ordering::SeqCst);
        for _ in 0..3 {
            relay.get(None, "/v1/scopes/", "10.0.0.9");
        }
        tick(&relay.state, T0 + 459);
        assert_eq!(relay.lines().len(), 1);
        tick(&relay.state, T0 + 460);
        assert_eq!(
            relay.lines()[1],
            "refused 3 other requests in the last minute: signature 3"
        );
    }

    #[test]
    fn the_log_holds_no_nameplate_address_or_body() {
        let w = world("log_private", 10, 20);
        let rig = Rig::new("log_private", &[&w.owner]);
        rig.publish(&w, 1);
        let key = &w.rivendell.device.sign;
        assert_eq!(
            rig.put(key, "/v1/pair/np-zzqx/a.msg", b"SECRETBYTES")
                .status,
            201
        );
        assert_eq!(
            rig.send("PUT", "/v1/pair/np-zzqx/b.msg", &[], b"SECRETREPLY")
                .status,
            201
        );
        assert_eq!(
            rig.send("PUT", "/v1/pair/np-zzqx/c.msg", &[], b"SECRETMORE")
                .status,
            403
        );
        assert_eq!(
            rig.put(key, "/v1/pair/np-zzqx/a.msg", b"SECRETOTHER")
                .status,
            409
        );
        assert_eq!(
            rig.put(&nobody(), "/v1/pair/np-yyyy/a.msg", b"SECRETNOPE")
                .status,
            403
        );
        assert_eq!(
            rig.put(key, &w.segment(&w.rivendell, 1), b"SECRETSEGMENT")
                .status,
            201
        );
        assert_eq!(rig.get(&nobody(), &url(&w, "devices/")).status, 403);
        let lines = rig.lines();
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("refused 409 exists PUT mailbox")),
            "{lines:?}"
        );
        for line in &lines {
            for secret in [
                "np-zzqx",
                "np-yyyy",
                "SECRET",
                "127.0.0.1",
                "bilbo-",
                "Host",
            ] {
                assert!(!line.contains(secret), "{secret} in {line}");
            }
        }
    }
}
