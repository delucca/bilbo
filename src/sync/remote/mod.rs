//! The relay transport: `https://` URLs, and `http://` to a loopback host, reach a bilbo relay's API under
//! `<url>/v1/`, with every request signed as `sign` says.

pub mod sign;

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::identity::keys::{self, SignKey};
use crate::shared::config;
use crate::sync::transport::{self, Keys, OBJECT_MAX, Put, Transport};

/// How long a connection may take before the relay counts as unreachable.
const CONNECT: Duration = Duration::from_secs(10);

/// How long a whole request may take.
const REQUEST: Duration = Duration::from_secs(300);

/// Why `identify` found no relay at a URL.
#[derive(Debug, PartialEq)]
pub enum Probe {
    /// Something answered, but not as a bilbo relay does.
    NotRelay,
    /// Nothing answered, with the reason.
    Unreachable(String),
}

/// What went wrong before a relay's answer could be read as one.
#[derive(Debug)]
enum Fault {
    NotRelay,
    Redirect,
    /// A certificate that the bundled roots do not vouch for, with the reason.
    Certificate(String),
    Unreachable(String),
    /// The seconds the device's clock is off the relay's.
    Clock(i64),
}

impl Fault {
    fn text(&self, url: &str) -> String {
        match self {
            Fault::NotRelay => format!("{url} is not a bilbo relay"),
            Fault::Redirect => {
                format!("relay {url} redirects; set the scope's URL to the address it redirects to")
            }
            Fault::Certificate(why) => format!("relay {url}: certificate not trusted: {why}"),
            Fault::Unreachable(why) => format!("relay {url} unreachable: {why}"),
            Fault::Clock(n) => format!(
                "this device's clock is {} s off the relay's; fix the clock",
                n.abs()
            ),
        }
    }
}

/// A relay's answer: status, the headers the client reads, and the body.
struct Reply {
    status: u16,
    /// The relay's clock, from `Bilbo-Time`.
    time: u64,
    manifest: Option<u64>,
    body: Vec<u8>,
}

impl Reply {
    /// The `error` of a `{"error":"<reason>"}` body.
    fn reason(&self) -> &str {
        #[derive(Deserialize)]
        struct Error<'a> {
            error: &'a str,
        }
        serde_json::from_slice::<Error>(&self.body).map_or("", |e| e.error)
    }

    fn is(&self, status: u16, reason: &str) -> bool {
        self.status == status && self.reason() == reason
    }
}

/// Which key signs a request.
#[derive(Clone, Copy, PartialEq)]
enum Who {
    Nobody,
    Device,
    Owner,
}

/// The seconds each relay's clock is ahead of this process's, by URL, once a relay has refused the time.
fn offsets() -> &'static Mutex<BTreeMap<String, i64>> {
    static OFFSETS: OnceLock<Mutex<BTreeMap<String, i64>>> = OnceLock::new();
    OFFSETS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn offset(url: &str) -> i64 {
    offsets()
        .lock()
        .ok()
        .and_then(|table| table.get(url).copied())
        .unwrap_or(0)
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The agent for `url`: no redirects, statuses as answers, and no proxy for a loopback URL.
fn agent(url: &str) -> ureq::Agent {
    let builder = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_connect(Some(CONNECT))
        .timeout_global(Some(REQUEST))
        .accept_encoding(ureq::config::AutoHeaderValue::None);
    let builder = if config::is_local(url) {
        builder.proxy(None)
    } else {
        builder
    };
    builder.build().into()
}

/// The URL as the config spells it, without a trailing slash, when a relay may be reached at it.
fn base(url: &str) -> Result<String, String> {
    if url.starts_with("https://") || (url.starts_with("http://") && config::is_local(url)) {
        Ok(url.trim_end_matches('/').to_string())
    } else {
        let scheme = url.split("://").next().unwrap_or(url);
        Err(format!(
            "{scheme} transports are not supported yet; use a file:// folder"
        ))
    }
}

/// The fault a failed send is: a certificate the roots do not vouch for, or an unreachable relay.
fn fault(error: &ureq::Error) -> Fault {
    let text = error.to_string();
    if let Some((_, why)) = text.split_once("invalid peer certificate: ") {
        return Fault::Certificate(why.to_string());
    }
    Fault::Unreachable(match error {
        ureq::Error::Io(e) => e.to_string(),
        ureq::Error::Timeout(_) => "timed out".to_string(),
        ureq::Error::HostNotFound => "host not found".to_string(),
        _ => text,
    })
}

/// One request, sent as given: the answer when it came from a relay, a fault otherwise.
fn exchange(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    headers: &[(&str, String)],
    body: Option<&[u8]>,
) -> Result<Reply, Fault> {
    let sent = if method == "PUT" {
        let mut request = agent.put(url);
        for (name, value) in headers {
            request = request.header(*name, value.as_str());
        }
        request.send(body.unwrap_or(&[]))
    } else {
        let mut request = agent.get(url);
        for (name, value) in headers {
            request = request.header(*name, value.as_str());
        }
        request.call()
    };
    let mut response = sent.map_err(|e| fault(&e))?;
    let status = response.status().as_u16();
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let time = header(sign::TIME).and_then(|v| v.parse::<u64>().ok());
    let manifest = header("bilbo-manifest").and_then(|v| v.parse::<u64>().ok());
    if (300..400).contains(&status) {
        return Err(Fault::Redirect);
    }
    let Some(time) = time else {
        return Err(if matches!(status, 502..=504) {
            Fault::Unreachable(format!("the proxy answered {status}"))
        } else {
            Fault::NotRelay
        });
    };
    let body = response
        .body_mut()
        .with_config()
        .limit(OBJECT_MAX)
        .read_to_vec()
        .map_err(|e| fault(&e))?;
    Ok(Reply {
        status,
        time,
        manifest,
        body,
    })
}

/// Whether the unsigned `GET <url>/v1/` answers as a bilbo relay does.
fn root(agent: &ureq::Agent, url: &str) -> Result<(), Fault> {
    let reply = exchange(agent, "GET", &format!("{url}/v1/"), &[], None)?;
    #[derive(Deserialize)]
    struct Root {
        relay: String,
        api: u64,
    }
    match serde_json::from_slice::<Root>(&reply.body) {
        Ok(root) if reply.status == 200 && root.relay == "bilbo" && root.api == 1 => Ok(()),
        _ => Err(Fault::NotRelay),
    }
}

/// Whether a bilbo relay answers at `url`, asking it nothing that needs a key.
pub fn identify(url: &str) -> Result<(), Probe> {
    let url = base(url).map_err(Probe::Unreachable)?;
    root(&agent(&url), &url).map_err(|fault| match fault {
        Fault::NotRelay | Fault::Redirect => Probe::NotRelay,
        Fault::Certificate(why) => Probe::Unreachable(format!("certificate not trusted: {why}")),
        Fault::Unreachable(why) => Probe::Unreachable(why),
        Fault::Clock(n) => Probe::Unreachable(Fault::Clock(n).text(&url)),
    })
}

/// The `Transport` of a relay, signing as `Keys` says.
pub struct Relay {
    url: String,
    agent: ureq::Agent,
    device: SignKey,
    device_id: String,
    owner: Option<SignKey>,
    opener: bool,
}

/// The relay at `url`, acting for `keys`.
pub fn open(url: &str, keys: &Keys) -> Result<Relay, String> {
    let url = base(url)?;
    Ok(Relay {
        agent: agent(&url),
        device: SignKey::from_seed(&keys.device.sign.seed()),
        device_id: keys.device.id(),
        owner: keys.owner.map(|owner| SignKey::from_seed(&owner.seed())),
        opener: keys.opener,
        url,
    })
}

/// Whether the manifest in `body` lists the device `id`; one that cannot be read lists none.
fn lists(body: &[u8], id: &str) -> bool {
    #[derive(Deserialize)]
    struct Entry {
        id: String,
    }
    #[derive(Deserialize)]
    struct Listed {
        devices: Vec<Entry>,
    }
    serde_json::from_slice::<Listed>(body).is_ok_and(|m| m.devices.iter().any(|d| d.id == id))
}

fn is_manifest(path: &str) -> bool {
    path.contains("/manifest/")
}

impl Relay {
    /// The key that signs a request for `path`: the owner's for the scope listing, manifest reads and the create of a
    /// manifest version that does not list this device, when held; the device's for the rest of `scopes/`, and the device's on a mailbox only for the device that shows the code.
    fn who(&self, method: &str, path: &str, body: &[u8]) -> Who {
        if path.starts_with("pair/") {
            return if self.opener {
                Who::Device
            } else {
                Who::Nobody
            };
        }
        let owned = self.owner.is_some()
            && match method {
                "GET" => path == "scopes/" || is_manifest(path),
                _ => is_manifest(path) && !lists(body, &self.device_id),
            };
        if owned { Who::Owner } else { Who::Device }
    }

    fn key(&self, who: Who) -> Option<&SignKey> {
        match who {
            Who::Nobody => None,
            Who::Device => Some(&self.device),
            Who::Owner => self.owner.as_ref(),
        }
    }

    /// Sends the request for `path` and returns the relay's answer; a 401 `clock` is retried once at the relay's
    /// time, which later requests keep.
    fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        who: Who,
    ) -> Result<Reply, String> {
        let target = format!("/v1/{path}");
        let url = format!("{}{target}", self.url);
        let mut retried = false;
        loop {
            let headers = match self.key(who) {
                None => Vec::new(),
                Some(key) => {
                    let time = (unix_now() + offset(&self.url)).max(0) as u64;
                    sign::headers(key, method, &target, time, body.unwrap_or(&[]))?
                }
            };
            let reply = exchange(&self.agent, method, &url, &headers, body)
                .map_err(|fault| fault.text(&self.url))?;
            if who != Who::Nobody && reply.is(401, "clock") {
                let skew = reply.time as i64 - unix_now();
                if let Ok(mut table) = offsets().lock() {
                    table.insert(self.url.clone(), skew);
                }
                if retried {
                    return Err(Fault::Clock(skew).text(&self.url));
                }
                retried = true;
                continue;
            }
            return Ok(reply);
        }
    }

    /// The message for a refusal of a request for `path`.
    fn refusal(&self, reply: &Reply, who: Who, path: &str) -> String {
        let url = &self.url;
        match (reply.status, reply.reason()) {
            (403, "not-admitted") => {
                let owner = self.owner.as_ref().filter(|_| who == Who::Owner);
                match (who, owner) {
                    (_, Some(owner)) => format!(
                        "relay {url} does not admit this owner; start it with --owner {}",
                        keys::owner_fingerprint(&owner.public())
                    ),
                    (Who::Nobody, _) => {
                        format!("relay {url} does not accept this pairing message")
                    }
                    _ => format!("relay {url} does not admit this device"),
                }
            }
            (403, "invalid") => format!(
                "relay {url} holds an invalid copy of this scope; repair the relay's data folder"
            ),
            (507, "quota") if path.starts_with("pair/") => {
                format!("relay {url} has no room for a pairing now; try again later")
            }
            (507, "quota") => format!("relay {url} is full"),
            (status, "") => format!("relay {url} answered {status}"),
            (status, reason) => format!("relay {url} answered {status}: {reason}"),
        }
    }

    /// Whether a refusal of a manifest read by the owner key means the relay holds no such scope of this owner.
    fn absent(&self, reply: &Reply, who: Who, path: &str) -> bool {
        reply.is(403, "not-admitted") && is_manifest(path) && who == Who::Owner
    }

    fn page<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, String> {
        let who = self.who("GET", path, &[]);
        let reply = self.call("GET", path, None, who)?;
        if reply.status != 200 {
            return Err(self.refusal(&reply, who, path));
        }
        serde_json::from_slice(&reply.body).map_err(|_| {
            format!(
                "relay {} answered a listing this bilbo cannot read",
                self.url
            )
        })
    }

    /// The seqs above `cursor` of a device folder, a page at a time; with `contiguous`, only the run that starts at
    /// `cursor + 1` and stops reading at its end.
    fn seqs(
        &self,
        scope: &str,
        device: &str,
        cursor: u64,
        contiguous: bool,
    ) -> Result<Vec<u64>, String> {
        check_ids(&[scope, device])?;
        #[derive(Deserialize)]
        struct Page {
            seqs: Vec<u64>,
            more: bool,
        }
        let mut found: Vec<u64> = Vec::new();
        let mut after = cursor;
        loop {
            let page: Page =
                self.page(&format!("scopes/{scope}/devices/{device}/?after={after}"))?;
            let mut next = after;
            for seq in &page.seqs {
                if contiguous && *seq != next + 1 {
                    return Ok(found);
                }
                found.push(*seq);
                next = *seq;
            }
            if !page.more || next == after {
                return Ok(found);
            }
            after = next;
        }
    }
}

/// An error for an id that is not 26 base32 characters.
fn check_ids(ids: &[&str]) -> Result<(), String> {
    match ids.iter().find(|id| !keys::is_id(id)) {
        Some(id) => Err(format!("{id} is not an id of the transport layout")),
        None => Ok(()),
    }
}

impl Transport for Relay {
    fn reachable(&self) -> Result<(), String> {
        root(&self.agent, &self.url).map_err(|fault| fault.text(&self.url))
    }

    fn keeps(&self) -> bool {
        true
    }

    fn scopes(&self) -> Result<Vec<String>, String> {
        #[derive(Deserialize)]
        struct Scopes {
            scopes: Vec<String>,
        }
        let listed: Scopes = self.page("scopes/")?;
        Ok(listed.scopes)
    }

    fn devices(&self, scope: &str) -> Result<Vec<String>, String> {
        check_ids(&[scope])?;
        #[derive(Deserialize)]
        struct Device {
            id: String,
        }
        #[derive(Deserialize)]
        struct Devices {
            devices: Vec<Device>,
        }
        let listed: Devices = self.page(&format!("scopes/{scope}/devices/"))?;
        Ok(listed.devices.into_iter().map(|d| d.id).collect())
    }

    fn list_after(&self, scope: &str, device: &str, cursor: u64) -> Result<Vec<u64>, String> {
        self.seqs(scope, device, cursor, false)
    }

    fn probe(&self, scope: &str, device: &str, cursor: u64) -> Result<Vec<u64>, String> {
        self.seqs(scope, device, cursor, true)
    }

    fn get(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        if !transport::is_object_path(path) {
            return Err(format!("{path} is not in the transport layout"));
        }
        let who = self.who("GET", path, &[]);
        let reply = self.call("GET", path, None, who)?;
        match reply.status {
            200 => Ok(Some(reply.body)),
            404 => Ok(None),
            _ if self.absent(&reply, who, path) => Ok(None),
            _ => Err(self.refusal(&reply, who, path)),
        }
    }

    fn create(&self, path: &str, bytes: &[u8]) -> Put {
        if !transport::is_object_path(path) {
            return Put::Unreachable(format!("{path} is not in the transport layout"));
        }
        let who = self.who("PUT", path, bytes);
        let reply = match self.call("PUT", path, Some(bytes), who) {
            Ok(reply) => reply,
            Err(why) => return Put::Unreachable(why),
        };
        match (reply.status, reply.reason()) {
            (200 | 201, _) => Put::Created,
            (409, "exists") => Put::Exists,
            (507, "quota") => Put::Full(self.refusal(&reply, who, path)),
            _ => Put::Unreachable(self.refusal(&reply, who, path)),
        }
    }

    fn highest_manifest(&self, scope: &str) -> Result<Option<u64>, String> {
        check_ids(&[scope])?;
        let path = format!("scopes/{scope}/manifest/latest");
        let who = self.who("GET", &path, &[]);
        let reply = self.call("GET", &path, None, who)?;
        match reply.status {
            200 => reply
                .manifest
                .map(Some)
                .ok_or_else(|| format!("{} is not a bilbo relay", self.url)),
            404 => Ok(None),
            _ if self.absent(&reply, who, &path) => Ok(None),
            _ => Err(self.refusal(&reply, who, &path)),
        }
    }

    fn replace(&self, path: &str, _bytes: &[u8]) -> Result<(), String> {
        Err(format!(
            "relay {} keeps what it stores; {path} cannot be replaced",
            self.url
        ))
    }

    fn sweep(&self, _now: SystemTime) -> Result<(), String> {
        Ok(())
    }

    fn remove_mailbox(&self, _nameplate: &str) -> Result<(), String> {
        Ok(())
    }

    fn sweep_mailboxes(&self, _now: SystemTime, _age: Duration) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;
    use crate::identity::keys::Device;
    use crate::shared::hash;

    const SCOPE: &str = "abcdefghijklmnopqrstuvwxyz";
    const DEVICE: &str = "bcdefghijklmnopqrstuvwxyz2";

    #[derive(Clone)]
    struct Seen {
        method: String,
        target: String,
        headers: BTreeMap<String, String>,
        body: Vec<u8>,
    }

    impl Seen {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers.get(name).map(String::as_str)
        }

        /// The key that signed the request, `None` when it carries no signature.
        fn signer(&self) -> Option<String> {
            self.header(sign::SIGNATURE)?;
            self.header(sign::KEY).map(str::to_string)
        }
    }

    /// A loopback server that answers each request with the text its closure gives and records what it saw.
    struct Fake {
        port: u16,
        seen: Arc<Mutex<Vec<Seen>>>,
        stop: Arc<AtomicBool>,
    }

    impl Fake {
        fn start(answer: impl Fn(&Seen) -> String + Send + 'static) -> Fake {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let seen = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let (record, halt) = (seen.clone(), stop.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if halt.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(mut stream) = stream else { continue };
                    let Some(request) = read_request(&stream) else {
                        continue;
                    };
                    let response = answer(&request);
                    record.lock().unwrap().push(request);
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                }
            });
            Fake { port, seen, stop }
        }

        fn url(&self) -> String {
            format!("http://127.0.0.1:{}", self.port)
        }

        fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect(("127.0.0.1", self.port));
        }
    }

    fn read_request(stream: &TcpStream) -> Option<Seen> {
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let mut parts = line.split_whitespace();
        let (method, target) = (parts.next()?.to_string(), parts.next()?.to_string());
        let mut headers = BTreeMap::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).ok()?;
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            let (name, value) = line.split_once(':')?;
            headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
        }
        let length: usize = headers
            .get("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut body = vec![0; length];
        reader.read_exact(&mut body).ok()?;
        Some(Seen {
            method,
            target,
            headers,
            body,
        })
    }

    fn reply(status: u16, time: Option<u64>, extra: &[(&str, &str)], body: &str) -> String {
        let mut text = format!(
            "HTTP/1.1 {status} Status\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        if let Some(time) = time {
            text.push_str(&format!("Bilbo-Time: {time}\r\n"));
        }
        for (name, value) in extra {
            text.push_str(&format!("{name}: {value}\r\n"));
        }
        text.push_str("\r\n");
        text.push_str(body);
        text
    }

    fn now() -> u64 {
        unix_now() as u64
    }

    fn answer(status: u16, body: &str) -> String {
        reply(status, Some(now()), &[], body)
    }

    fn refuse(status: u16, reason: &str) -> String {
        answer(status, &format!("{{\"error\":\"{reason}\"}}"))
    }

    fn device() -> Device {
        Device::from_seeds("a", &[1; 32], &[2; 32])
    }

    fn owner() -> SignKey {
        SignKey::from_seed(&[9; 32])
    }

    fn hex_of(key: &SignKey) -> String {
        keys::hex(&key.public())
    }

    /// A relay client of the fake, signing as the device and, when `with_owner`, the owner.
    fn client(fake: &Fake, with_owner: bool, opener: bool) -> Relay {
        let (device, owner) = (device(), owner());
        let keys = Keys {
            device: &device,
            owner: with_owner.then_some(&owner),
            opener,
        };
        open(&fake.url(), &keys).unwrap()
    }

    /// A manifest body that lists the test device, or lists no one.
    fn listing(lists: bool) -> Vec<u8> {
        let devices = if lists {
            format!("[{{\"id\":\"{}\"}}]", device().id())
        } else {
            "[]".to_string()
        };
        format!("{{\"devices\":{devices}}}").into_bytes()
    }

    fn manifest(n: u64) -> String {
        transport::manifest_path(SCOPE, n)
    }

    fn segment(seq: u64) -> String {
        transport::segment_path(SCOPE, DEVICE, seq)
    }

    #[test]
    fn a_relay_url_picks_the_relay_transport() {
        let device = device();
        let keys = Keys {
            device: &device,
            owner: None,
            opener: false,
        };
        for url in [
            "https://relay.example",
            "https://relay.example:8738/bilbo",
            "http://127.0.0.1:8738",
            "http://localhost:8738",
            "http://[::1]:8738",
        ] {
            assert!(transport::open(url, &keys).unwrap().keeps(), "{url}");
        }
    }

    #[test]
    fn plain_http_to_another_host_sends_nothing() {
        let fake = Fake::start(|_| answer(200, "{}"));
        let device = device();
        let keys = Keys {
            device: &device,
            owner: None,
            opener: false,
        };
        let other = format!("http://127.0.0.2:{}", fake.port);
        assert!(transport::open(&other, &keys).is_err());
        assert!(open(&other, &keys).is_err());
        assert_eq!(
            identify(&other),
            Err(Probe::Unreachable(open(&other, &keys).err().unwrap()))
        );
        assert!(fake.seen().is_empty());
    }

    #[test]
    fn every_request_is_signed_over_the_target_the_relay_receives() {
        let fake = Fake::start(|request| match request.method.as_str() {
            "PUT" => answer(201, ""),
            _ => answer(200, "{\"devices\":[]}"),
        });
        let device = device();
        let keys = Keys {
            device: &device,
            owner: None,
            opener: false,
        };
        let url = format!("{}/bilbo/", fake.url());
        let relay = open(&url, &keys).unwrap();
        assert_eq!(relay.create(&segment(1), b"bytes"), Put::Created);
        assert_eq!(relay.devices(SCOPE).unwrap(), Vec::<String>::new());
        assert_eq!(relay.devices(SCOPE).unwrap(), Vec::<String>::new());
        let seen = fake.seen();
        assert_eq!(seen.len(), 3);
        assert_eq!(
            seen[0].target,
            format!("/bilbo/v1/scopes/{SCOPE}/devices/{DEVICE}/00000000000000000001.seg")
        );
        for request in &seen {
            let target = request.target.strip_prefix("/bilbo").unwrap();
            let signed = sign::read(&|name| request.header(name).map(str::to_string))
                .unwrap()
                .unwrap();
            assert_eq!(signed.key, device.sign.public());
            sign::verify(
                &signed,
                &request.method,
                target,
                &hash::sha256_hex(&request.body),
                now(),
            )
            .unwrap();
        }
        assert_ne!(seen[1].header(sign::NONCE), seen[2].header(sign::NONCE));
    }

    #[test]
    fn the_owner_signs_listings_and_manifest_reads_and_the_device_the_rest() {
        let fake = Fake::start(|request| {
            if request.target.ends_with("/devices/") && request.target.contains("scopes/a") {
                answer(200, "{\"devices\":[]}")
            } else if request.target == "/v1/scopes/" {
                answer(200, "{\"scopes\":[]}")
            } else if request.method == "PUT" {
                answer(201, "")
            } else if request.target.contains("?after=") {
                answer(200, "{\"seqs\":[],\"more\":false}")
            } else if request.target.ends_with("/latest") {
                reply(200, Some(now()), &[("Bilbo-Manifest", "3")], "m")
            } else {
                answer(200, "x")
            }
        });
        let relay = client(&fake, true, false);
        relay.scopes().unwrap();
        relay.get(&manifest(1)).unwrap();
        assert_eq!(relay.highest_manifest(SCOPE).unwrap(), Some(3));
        relay.devices(SCOPE).unwrap();
        relay.list_after(SCOPE, DEVICE, 0).unwrap();
        relay.get(&segment(1)).unwrap();
        assert_eq!(relay.create(&manifest(1), &listing(true)), Put::Created);
        assert_eq!(relay.create(&manifest(2), &listing(false)), Put::Created);
        assert_eq!(relay.create(&segment(1), b"s"), Put::Created);
        let signers: Vec<Option<String>> = fake.seen().iter().map(Seen::signer).collect();
        let (owner, device) = (
            Some(hex_of(&owner())),
            Some(keys::hex(&device().sign.public())),
        );
        assert_eq!(
            signers,
            vec![
                owner.clone(),
                owner.clone(),
                owner.clone(),
                device.clone(),
                device.clone(),
                device.clone(),
                device.clone(),
                owner,
                device
            ]
        );
    }

    #[test]
    fn a_device_without_the_owner_key_signs_manifest_reads_itself() {
        let fake = Fake::start(|_| answer(200, "m"));
        let relay = client(&fake, false, false);
        relay.get(&manifest(1)).unwrap();
        assert_eq!(
            fake.seen()[0].signer(),
            Some(keys::hex(&device().sign.public()))
        );
    }

    #[test]
    fn the_device_that_shows_a_code_signs_its_mailbox_and_the_one_that_answers_signs_nothing() {
        let fake = Fake::start(|request| match request.method.as_str() {
            "PUT" => answer(201, ""),
            _ => answer(404, "{\"error\":\"not-found\"}"),
        });
        let path = transport::message_path("42", "a");
        let shows = client(&fake, true, true);
        assert_eq!(shows.create(&path, b"a"), Put::Created);
        assert_eq!(
            shows.get(&transport::message_path("42", "c")).unwrap(),
            None
        );
        let answers = client(&fake, true, false);
        assert_eq!(
            answers.create(&transport::message_path("42", "b"), b"b"),
            Put::Created
        );
        assert_eq!(
            answers.get(&transport::message_path("42", "c")).unwrap(),
            None
        );
        let signers: Vec<Option<String>> = fake.seen().iter().map(Seen::signer).collect();
        let device = Some(keys::hex(&device().sign.public()));
        assert_eq!(signers, vec![device.clone(), device, None, None]);
    }

    #[test]
    fn a_redirect_is_reported_and_not_followed() {
        let fake = Fake::start(|_| reply(301, None, &[("Location", "https://other.example/")], ""));
        let relay = client(&fake, false, false);
        let expected = format!(
            "relay {} redirects; set the scope's URL to the address it redirects to",
            fake.url()
        );
        assert_eq!(relay.scopes().unwrap_err(), expected);
        assert_eq!(relay.reachable().unwrap_err(), expected);
        assert_eq!(fake.seen().len(), 2);
        assert_eq!(identify(&fake.url()), Err(Probe::NotRelay));
    }

    #[test]
    fn an_answer_without_the_relay_time_is_not_a_relay_or_a_proxy_error() {
        let fake = Fake::start(|request| {
            if request.target.contains("devices") {
                reply(502, None, &[], "")
            } else {
                reply(404, None, &[], "not found")
            }
        });
        let relay = client(&fake, false, false);
        assert_eq!(
            relay.get(&manifest(1)).unwrap_err(),
            format!("{} is not a bilbo relay", fake.url())
        );
        assert_eq!(
            relay.devices(SCOPE).unwrap_err(),
            format!("relay {} unreachable: the proxy answered 502", fake.url())
        );
        assert_eq!(identify(&fake.url()), Err(Probe::NotRelay));
    }

    #[test]
    fn identify_asks_for_the_root_unsigned() {
        let fake = Fake::start(|_| answer(200, "{\"relay\":\"bilbo\",\"api\":1}"));
        assert_eq!(identify(&fake.url()), Ok(()));
        assert_eq!(identify(&format!("{}/", fake.url())), Ok(()));
        let seen = fake.seen();
        assert_eq!(seen[0].target, "/v1/");
        assert_eq!(seen[0].signer(), None);
        let other = Fake::start(|_| answer(200, "{\"relay\":\"other\",\"api\":1}"));
        assert_eq!(identify(&other.url()), Err(Probe::NotRelay));
        let proxy = Fake::start(|_| reply(503, None, &[], ""));
        assert_eq!(
            identify(&proxy.url()),
            Err(Probe::Unreachable("the proxy answered 503".into()))
        );
    }

    #[test]
    fn nothing_listening_is_unreachable() {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let url = format!("http://127.0.0.1:{port}");
        let device = device();
        let keys = Keys {
            device: &device,
            owner: None,
            opener: false,
        };
        let relay = open(&url, &keys).unwrap();
        let message = relay.reachable().unwrap_err();
        assert!(
            message.starts_with(&format!("relay {url} unreachable: ")),
            "{message}"
        );
        assert!(
            matches!(relay.create(&segment(1), b"x"), Put::Unreachable(m) if m.starts_with("relay "))
        );
        assert!(matches!(identify(&url), Err(Probe::Unreachable(_))));
    }

    #[test]
    fn a_certificate_the_roots_do_not_vouch_for_is_named() {
        let error = ureq::Error::Io(std::io::Error::other(
            "invalid peer certificate: UnknownIssuer",
        ));
        let text = fault(&error).text("https://relay.example");
        assert_eq!(
            text,
            "relay https://relay.example: certificate not trusted: UnknownIssuer"
        );
        let other = fault(&ureq::Error::HostNotFound).text("https://relay.example");
        assert_eq!(
            other,
            "relay https://relay.example unreachable: host not found"
        );
    }

    #[test]
    fn a_clock_off_by_ten_minutes_is_retried_once_and_remembered() {
        let ahead = 600;
        let fake = Fake::start(move |request| {
            let relay_now = now() + ahead;
            let time: u64 = request.header(sign::TIME).unwrap().parse().unwrap();
            if time.abs_diff(relay_now) > sign::WINDOW {
                reply(401, Some(relay_now), &[], "{\"error\":\"clock\"}")
            } else {
                reply(200, Some(relay_now), &[], "{\"scopes\":[]}")
            }
        });
        let relay = client(&fake, true, false);
        assert!(relay.scopes().unwrap().is_empty());
        assert_eq!(fake.seen().len(), 2);
        assert_ne!(
            fake.seen()[0].header(sign::NONCE),
            fake.seen()[1].header(sign::NONCE)
        );
        relay.scopes().unwrap();
        assert_eq!(fake.seen().len(), 3);
        let other = client(&fake, true, false);
        other.scopes().unwrap();
        assert_eq!(fake.seen().len(), 4);
    }

    #[test]
    fn a_relay_that_keeps_refusing_the_time_is_reported_with_the_offset() {
        let fake = Fake::start(|_| reply(401, Some(now() + 600), &[], "{\"error\":\"clock\"}"));
        let relay = client(&fake, true, false);
        let message = relay.scopes().unwrap_err();
        assert_eq!(fake.seen().len(), 2);
        let n: i64 = message
            .strip_prefix("this device's clock is ")
            .and_then(|rest| rest.strip_suffix(" s off the relay's; fix the clock"))
            .unwrap_or_else(|| panic!("{message}"))
            .parse()
            .unwrap();
        assert!((599..=601).contains(&n), "{n}");
    }

    #[test]
    fn a_create_maps_the_answers_of_the_relay() {
        let status = Arc::new(AtomicUsize::new(201));
        let reason = Arc::new(Mutex::new(String::new()));
        let (s, r) = (status.clone(), reason.clone());
        let fake = Fake::start(move |_| {
            let status = s.load(Ordering::SeqCst) as u16;
            let reason = r.lock().unwrap().clone();
            if reason.is_empty() {
                answer(status, "")
            } else {
                refuse(status, &reason)
            }
        });
        let relay = client(&fake, true, false);
        let url = fake.url();
        let put = |code: usize, why: &str, path: &str| {
            status.store(code, Ordering::SeqCst);
            *reason.lock().unwrap() = why.to_string();
            relay.create(path, &listing(path != manifest(1)))
        };
        assert_eq!(put(201, "", &segment(1)), Put::Created);
        assert_eq!(put(200, "", &segment(1)), Put::Created);
        assert_eq!(put(409, "exists", &segment(1)), Put::Exists);
        assert_eq!(
            put(507, "quota", &segment(1)),
            Put::Full(format!("relay {url} is full"))
        );
        assert_eq!(
            put(507, "quota", &transport::message_path("42", "a")),
            Put::Full(format!(
                "relay {url} has no room for a pairing now; try again later"
            ))
        );
        assert_eq!(
            put(403, "not-admitted", &manifest(1)),
            Put::Unreachable(format!(
                "relay {url} does not admit this owner; start it with --owner {}",
                keys::owner_fingerprint(&owner().public())
            ))
        );
        assert_eq!(
            put(403, "not-admitted", &segment(1)),
            Put::Unreachable(format!("relay {url} does not admit this device"))
        );
        assert_eq!(
            put(403, "not-admitted", &manifest(4)),
            Put::Unreachable(format!("relay {url} does not admit this device"))
        );
        assert_eq!(
            put(403, "invalid", &segment(1)),
            Put::Unreachable(format!(
                "relay {url} holds an invalid copy of this scope; repair the relay's data folder"
            ))
        );
        assert_eq!(
            put(409, "not-next", &segment(1)),
            Put::Unreachable(format!("relay {url} answered 409: not-next"))
        );
        assert_eq!(
            put(429, "rate", &segment(1)),
            Put::Unreachable(format!("relay {url} answered 429: rate"))
        );
    }

    #[test]
    fn an_owner_read_the_relay_does_not_admit_reads_as_absent() {
        let fake = Fake::start(|_| refuse(403, "not-admitted"));
        let url = fake.url();
        let owner_client = client(&fake, true, false);
        assert_eq!(owner_client.get(&manifest(1)).unwrap(), None);
        assert_eq!(owner_client.highest_manifest(SCOPE).unwrap(), None);
        assert_eq!(
            owner_client.get(&segment(1)).unwrap_err(),
            format!("relay {url} does not admit this device")
        );
        assert_eq!(
            owner_client.scopes().unwrap_err(),
            format!(
                "relay {url} does not admit this owner; start it with --owner {}",
                keys::owner_fingerprint(&owner().public())
            )
        );
        let device_client = client(&fake, false, false);
        assert_eq!(
            device_client.get(&manifest(1)).unwrap_err(),
            format!("relay {url} does not admit this device")
        );
        assert!(device_client.highest_manifest(SCOPE).is_err());
    }

    #[test]
    fn a_scope_the_relay_found_invalid_names_the_repair() {
        let fake = Fake::start(|_| refuse(403, "invalid"));
        let relay = client(&fake, false, false);
        assert_eq!(
            relay.devices(SCOPE).unwrap_err(),
            format!(
                "relay {} holds an invalid copy of this scope; repair the relay's data folder",
                fake.url()
            )
        );
    }

    #[test]
    fn objects_and_the_latest_manifest_are_read_as_stored() {
        let fake = Fake::start(|request| {
            if request.target.ends_with("/latest") {
                reply(200, Some(now()), &[("Bilbo-Manifest", "4")], "four")
            } else if request.target.contains("00000000000000000007") {
                refuse(404, "not-found")
            } else {
                answer(200, "bytes")
            }
        });
        let relay = client(&fake, false, false);
        assert_eq!(relay.get(&manifest(2)).unwrap(), Some(b"bytes".to_vec()));
        assert_eq!(relay.get(&segment(7)).unwrap(), None);
        assert_eq!(relay.highest_manifest(SCOPE).unwrap(), Some(4));
        assert!(relay.get("scopes/x").is_err());
        assert!(relay.devices("nope").is_err());
        assert!(relay.list_after(SCOPE, "nope", 0).is_err());
        assert_eq!(fake.seen().len(), 3);
    }

    #[test]
    fn a_listing_follows_the_pages_of_the_relay() {
        let fake = Fake::start(|request| {
            let after: u64 = request.target.rsplit('=').next().unwrap().parse().unwrap();
            match after {
                0 => answer(200, "{\"seqs\":[1,2,3],\"more\":true}"),
                3 => answer(200, "{\"seqs\":[4,6],\"more\":true}"),
                _ => answer(200, "{\"seqs\":[7],\"more\":false}"),
            }
        });
        let relay = client(&fake, false, false);
        assert_eq!(
            relay.list_after(SCOPE, DEVICE, 0).unwrap(),
            vec![1, 2, 3, 4, 6, 7]
        );
        let targets: Vec<String> = fake.seen().iter().map(|s| s.target.clone()).collect();
        assert!(targets[1].ends_with("/?after=3"), "{targets:?}");
        assert!(targets[2].ends_with("/?after=6"), "{targets:?}");
        assert_eq!(relay.probe(SCOPE, DEVICE, 0).unwrap(), vec![1, 2, 3, 4]);
        assert_eq!(fake.seen().len(), 5);
        assert_eq!(relay.probe(SCOPE, DEVICE, 3).unwrap(), vec![4]);
    }

    #[test]
    fn a_listing_that_names_the_scopes_and_devices() {
        let fake = Fake::start(|request| {
            if request.target == "/v1/scopes/" {
                answer(200, &format!("{{\"scopes\":[\"{SCOPE}\"]}}"))
            } else {
                answer(
                    200,
                    &format!("{{\"devices\":[{{\"id\":\"{DEVICE}\",\"last\":3}}]}}"),
                )
            }
        });
        let relay = client(&fake, true, false);
        assert_eq!(relay.scopes().unwrap(), vec![SCOPE.to_string()]);
        assert_eq!(relay.devices(SCOPE).unwrap(), vec![DEVICE.to_string()]);
    }

    #[test]
    fn a_relay_keeps_what_it_stores() {
        let fake = Fake::start(|_| answer(200, ""));
        let relay = client(&fake, false, false);
        assert!(relay.keeps());
        assert!(relay.replace(&segment(1), b"x").is_err());
        assert!(relay.sweep(SystemTime::now()).is_ok());
        assert!(relay.remove_mailbox("42").is_ok());
        assert!(
            relay
                .sweep_mailboxes(SystemTime::now(), Duration::ZERO)
                .is_ok()
        );
        assert!(fake.seen().is_empty());
    }

    #[test]
    fn a_later_device_copies_the_whole_chain_to_an_empty_relay() {
        use crate::identity::keys::{Identity, Owner};
        use crate::identity::manifest::{self, Member, Recipient};

        let dir = std::env::temp_dir().join(format!("bilbo-remote-chain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let owner = Owner::derive(&[0; 16]);
        let who = |name: &str, seed: u8| Identity {
            owner: owner.file(),
            device: Device::from_seeds(name, &[seed; 32], &[seed + 1; 32]),
        };
        let (bagend, rivendell, carol) = (who("bagend", 3), who("rivendell", 1), who("carol", 5));
        let lock = manifest::lock(&dir).unwrap();
        let id = manifest::create(&lock, &bagend, "personal", "file:///x", &[])
            .unwrap()
            .scope;
        for joining in [&rivendell, &carol] {
            let scope = manifest::read_scope(&dir, &id).unwrap();
            let opened = manifest::open(&scope, &Recipient::Owner(&owner.box_secret))
                .unwrap()
                .unwrap();
            let member = Member::of(&joining.device);
            manifest::add_device(&lock, &scope, &opened.keys[&opened.epoch], &member, &bagend)
                .unwrap();
        }
        let versions = manifest::read_scope(&dir, &id).unwrap().versions;
        assert_eq!(versions.len(), 3);
        assert!(!versions[0].manifest.lists(&rivendell.device.id()));

        let log = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = log.clone();
        let flags = crate::relay::Flags {
            data: dir.join("relay"),
            owners: vec![keys::owner_fingerprint(&owner.sign.public())],
            listen: "127.0.0.1:0".parse().unwrap(),
            max_scopes: 16,
            max_scope_mb: 1024,
            max_object_mb: 16,
        };
        let relay = crate::relay::start(
            flags,
            crate::relay::system_clock(),
            Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string())),
        )
        .unwrap();
        let keys = Keys::of(&rivendell);
        let t = transport::open(&relay.url(), &keys).unwrap();
        for v in &versions {
            let path = transport::manifest_path(&id, v.manifest.n);
            assert_eq!(
                t.create(&path, &v.bytes),
                Put::Created,
                "version {}",
                v.manifest.n
            );
        }
        assert_eq!(t.highest_manifest(&id).unwrap(), Some(3));
        for v in &versions {
            let path = transport::manifest_path(&id, v.manifest.n);
            assert_eq!(t.get(&path).unwrap(), Some(v.bytes.clone()));
        }
        let this = rivendell.device.id();
        let lines: Vec<String> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with("manifest "))
            .map(|l| l.split(' ').skip(2).take(2).collect::<Vec<_>>().join(" "))
            .collect();
        assert_eq!(
            lines,
            vec![
                "owner 1".to_string(),
                format!("{this} 2"),
                format!("{this} 3")
            ]
        );
        drop(relay);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
