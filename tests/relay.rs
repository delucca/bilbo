//! `bilbo relay` through the built binary: the flags, the data folder, durability across SIGKILL, the start-up walk,
//! the log, and sync through a relay. Requests are raw HTTP over loopback, signed with the golden keys of
//! `tests/fixtures/device/`. The relay's own rules are unit-tested in `src/relay/`; every wait is a poll with a
//! deadline, and a check that something did not happen waits on a barrier, a later line the relay must print.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use common::{Relay, Run, TempDir, Watcher, bilbo, config, poll_eq, sha256_hex};
use ed25519_dalek::{Signer, SigningKey};
use hpke::{Deserializable, OpModeR};

type Kem = hpke::kem::X25519HkdfSha256;
type Seal = hpke::aead::ChaCha20Poly1305;
type Kdf = hpke::kdf::HkdfSha256;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/device");
const RIVENDELL: &str = "gr2q7gf5lh6pzfdnurnkvputhp";
const BAGEND: &str = "wyxim75c6m5p4ywv22ywilqweh";
const ID: &str = "01M3YJ7R6HK6NQ30DCDB1P4DYB";
const OTHER: &str = "01M3YE296FMNXYZS89787DMY0A";
const FILE: &str = "decision-release.md";
const PREFIX: &str = "bilbo: ";

// Keys and fixtures

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn rivendell() -> SigningKey {
    key(1)
}

fn bagend() -> SigningKey {
    key(3)
}

/// The fixture owner's key, read from the fixture file.
fn owner() -> SigningKey {
    let text = fs::read_to_string(Path::new(FIXTURES).join("rivendell/owner.key")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let seed = unhex(value["sign"].as_str().unwrap());
    SigningKey::from_bytes(&seed.try_into().unwrap())
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

/// The fixture scope's id: the one folder of the fixture store.
fn scope() -> String {
    let mut ids: Vec<String> = fs::read_dir(Path::new(FIXTURES).join("store/.bilbo/scopes"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(ids.len(), 1, "{ids:?}");
    ids.remove(0)
}

/// The scope of another owner, in `fixtures/device/foreign`.
fn foreign_scope() -> String {
    let dir = Path::new(FIXTURES).join("foreign/.bilbo/scopes");
    let mut ids: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(ids.len(), 1, "{ids:?}");
    ids.remove(0)
}

fn manifest_bytes(n: u64) -> Vec<u8> {
    let path = format!("store/.bilbo/scopes/{}/manifest/{n}.json", scope());
    fs::read(Path::new(FIXTURES).join(path)).unwrap()
}

/// The fixture owner's fingerprint, as `bilbo device` prints it.
fn fingerprint() -> &'static str {
    static FINGERPRINT: OnceLock<String> = OnceLock::new();
    FINGERPRINT.get_or_init(|| {
        let dir = TempDir::new("relay-fingerprint");
        let keys = dir.path().join("state/bilbo/keys");
        copy_tree(&Path::new(FIXTURES).join("rivendell"), &keys);
        chmod(&keys, 0o700);
        for file in ["owner.key", "device.key"] {
            chmod(&keys.join(file), 0o600);
        }
        let state = dir.path().join("state");
        let home = dir.path().join("home");
        let run = bilbo(
            dir.path(),
            &[
                ("XDG_STATE_HOME", state.to_str().unwrap()),
                ("BILBO_HOME", home.to_str().unwrap()),
            ],
            &["device"],
        );
        assert_eq!(run.code, 0, "{}", run.stderr);
        run.stdout
            .lines()
            .find_map(|l| l.strip_prefix("owner\t"))
            .unwrap()
            .to_string()
    })
}

/// The fingerprint of the foreign owner: the lowercase base32 of the SHA-256 of its public key, in six groups.
fn foreign_fingerprint() -> String {
    let path = format!("foreign/.bilbo/scopes/{}/manifest/1.json", foreign_scope());
    let text = fs::read_to_string(Path::new(FIXTURES).join(path)).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let digest = unhex(&sha256_hex(&unhex(value["owner"].as_str().unwrap())));
    let alphabet = b"abcdefghijklmnopqrstuvwxyz234567";
    let (mut out, mut buffer, mut bits) = (String::new(), 0u32, 0);
    for byte in digest {
        buffer = buffer << 8 | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(alphabet[(buffer >> bits & 31) as usize] as char);
        }
    }
    out.as_bytes()[..24]
        .chunks(4)
        .map(|g| std::str::from_utf8(g).unwrap())
        .collect::<Vec<_>>()
        .join("-")
}

/// A data folder holding the fixture scope's manifests `versions` in the transport tree's layout.
fn preload(data: &Path, versions: &[u64]) {
    let into = data.join(format!("scopes/{}/manifest", scope()));
    fs::create_dir_all(&into).unwrap();
    for n in versions {
        fs::write(into.join(format!("{n}.json")), manifest_bytes(*n)).unwrap();
    }
}

fn preload_foreign(data: &Path) {
    let from = format!("foreign/.bilbo/scopes/{0}/manifest/1.json", foreign_scope());
    let into = data.join(format!("scopes/{}/manifest", foreign_scope()));
    fs::create_dir_all(&into).unwrap();
    fs::copy(Path::new(FIXTURES).join(from), into.join("1.json")).unwrap();
}

fn seg_name(seq: u64) -> String {
    format!("{seq:020}.seg")
}

fn segment_path(data: &Path, device: &str, seq: u64) -> PathBuf {
    data.join(format!(
        "scopes/{}/devices/{device}/{}",
        scope(),
        seg_name(seq)
    ))
}

/// Writes segment `seq` of `device` straight into the tree.
fn preload_segment(data: &Path, device: &str, seq: u64, bytes: &[u8]) {
    let path = segment_path(data, device, seq);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// Every file under `dir`, with its bytes.
fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(items) = fs::read_dir(dir) else {
            return;
        };
        for entry in items {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, files);
            } else {
                files.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(dir, &mut files);
    files
}

// Requests

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn connect(port: u16) -> TcpStream {
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(40)))
        .unwrap();
    stream
}

fn read_reply(stream: &mut TcpStream) -> Reply {
    let mut bytes = Vec::new();
    let _ = stream.read_to_end(&mut bytes);
    let split = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("no response head in {:?}", String::from_utf8_lossy(&bytes)));
    let head = String::from_utf8_lossy(&bytes[..split]).into_owned();
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
        .filter_map(|l| l.split_once(": "))
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect();
    Reply {
        status,
        headers,
        body: bytes[split + 4..].to_vec(),
    }
}

/// The request head for `method target` with `headers` and a `Content-Length` of `length` when there is one.
fn head(method: &str, target: &str, headers: &[(String, String)], length: Option<usize>) -> String {
    let mut head = format!("{method} {target} HTTP/1.1\r\nHost: relay\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some(length) = length {
        head.push_str(&format!("Content-Length: {length}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    head
}

fn send(port: u16, method: &str, target: &str, headers: &[(String, String)], body: &[u8]) -> Reply {
    let mut stream = connect(port);
    let length = (method == "PUT" || !body.is_empty()).then_some(body.len());
    stream
        .write_all(head(method, target, headers, length).as_bytes())
        .unwrap();
    stream.write_all(body).unwrap();
    read_reply(&mut stream)
}

static NONCES: AtomicU64 = AtomicU64::new(0);

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// The four signature headers of `key` over a request, with a nonce no other request of this process uses.
fn signature(key: &SigningKey, method: &str, target: &str, body: &[u8]) -> Vec<(String, String)> {
    let n = NONCES.fetch_add(1, Ordering::SeqCst);
    let unique = format!("{}-{n}-{:?}", std::process::id(), Instant::now());
    let nonce = hex(&unhex(&sha256_hex(unique.as_bytes()))[..16]);
    let time = now();
    let text = format!(
        "bilbo-relay-1\n{method}\n{target}\n{time}\n{nonce}\n{}",
        sha256_hex(body)
    );
    vec![
        (
            "Bilbo-Key".into(),
            hex(key.verifying_key().as_bytes().as_slice()),
        ),
        ("Bilbo-Time".into(), time.to_string()),
        ("Bilbo-Nonce".into(), nonce),
        (
            "Bilbo-Signature".into(),
            hex(&key.sign(text.as_bytes()).to_bytes()),
        ),
    ]
}

fn signed(port: u16, key: &SigningKey, method: &str, target: &str, body: &[u8]) -> Reply {
    send(
        port,
        method,
        target,
        &signature(key, method, target, body),
        body,
    )
}

fn get(port: u16, key: &SigningKey, target: &str) -> Reply {
    signed(port, key, "GET", target, b"")
}

fn put(port: u16, key: &SigningKey, target: &str, body: &[u8]) -> Reply {
    signed(port, key, "PUT", target, body)
}

fn manifest_target(n: u64) -> String {
    format!("/v1/scopes/{}/manifest/{n}.json", scope())
}

fn segment_target(device: &str, seq: u64) -> String {
    format!("/v1/scopes/{}/devices/{device}/{}", scope(), seg_name(seq))
}

/// Creates manifests 1 and `up_to` through the API, as `bagend`, which both list.
fn publish(port: u16, up_to: u64) {
    for n in 1..=up_to {
        let reply = put(port, &bagend(), &manifest_target(n), &manifest_bytes(n));
        assert_eq!(reply.status, 201, "manifest {n}: {}", reply.text());
    }
}

fn start(data: &Path) -> Relay {
    Relay::start(data, &[fingerprint()], &[])
}

/// Every line after the startup line, without the prefix.
fn after_startup(relay: &Relay) -> Vec<String> {
    relay
        .lines()
        .iter()
        .skip(1)
        .map(|l| l.strip_prefix(PREFIX).unwrap_or(l).to_string())
        .collect()
}

fn tmp_entries(data: &Path) -> Vec<PathBuf> {
    match fs::read_dir(data.join(".tmp")) {
        Ok(items) => items.map(|e| e.unwrap().path()).collect(),
        Err(_) => Vec::new(),
    }
}

fn data_in(dir: &TempDir) -> PathBuf {
    dir.path().join("data")
}

fn usage_error(run: &Run, needle: &str) {
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.contains(needle),
        "stderr lacks {needle:?}: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("bilbo: usage: bilbo "),
        "{}",
        run.stderr
    );
}

// Start the relay

#[test]
fn a_relay_starts_on_a_free_port_and_answers_the_root() {
    let dir = TempDir::new("relay-start");
    let data = data_in(&dir);
    let relay = start(&data);
    assert_eq!(
        relay.lines(),
        [format!(
            "bilbo: relay listening on http://127.0.0.1:{}",
            relay.port
        )]
    );
    let reply = send(relay.port, "GET", "/v1/", &[], b"");
    assert_eq!(reply.status, 200);
    assert_eq!(reply.text(), r#"{"relay":"bilbo","api":1}"#);
    assert!(reply.header("Bilbo-Time").is_some());
}

#[test]
fn no_owner_is_a_usage_error_and_creates_nothing() {
    let dir = TempDir::new("relay-no-owner");
    let data = data_in(&dir);
    let run = bilbo(
        dir.path(),
        &[],
        &["relay", "--data", data.to_str().unwrap()],
    );
    usage_error(&run, "bilbo: missing --owner");
    assert!(!data.exists());
    let run = bilbo(dir.path(), &[], &["relay", "--owner", fingerprint()]);
    usage_error(&run, "bilbo: missing --data");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn a_mistyped_fingerprint_is_a_usage_error() {
    let dir = TempDir::new("relay-mistyped");
    let data = data_in(&dir);
    for typed in [
        "abc",
        "yb4b-5aju-v6zb-x2nm-nc5x",
        "yb4b-5aju-v6zb-x2nm-nc5x-ompf-aaaa",
        "11111-1111-1111-1111-1111-1111",
    ] {
        let run = bilbo(
            dir.path(),
            &[],
            &["relay", "--data", data.to_str().unwrap(), "--owner", typed],
        );
        usage_error(&run, typed);
        assert!(
            run.stderr.contains("not an owner fingerprint"),
            "{}",
            run.stderr
        );
    }
    assert!(!data.exists());
}

#[test]
fn an_owner_is_read_without_regard_to_case_or_hyphens() {
    let dir = TempDir::new("relay-owner-spelling");
    let data = data_in(&dir);
    let plain = fingerprint().replace('-', "").to_uppercase();
    let relay = Relay::start(&data, &[&plain], &[]);
    publish(relay.port, 2);
    assert_eq!(after_startup(&relay).len(), 2, "{:?}", relay.lines());
}

#[test]
fn a_bad_limit_or_another_argument_is_a_usage_error() {
    let dir = TempDir::new("relay-bad-flags");
    let data = data_in(&dir);
    let base = [
        "relay",
        "--data",
        data.to_str().unwrap(),
        "--owner",
        fingerprint(),
    ];
    for (flag, value) in [
        ("--max-scope-mb", "0"),
        ("--max-scope-mb", "1048577"),
        ("--max-scopes", "x"),
        ("--max-object-mb", "-1"),
        ("--max-object-mb", "1.5"),
    ] {
        let mut args = base.to_vec();
        args.extend([flag, value]);
        let run = bilbo(dir.path(), &[], &args);
        usage_error(&run, flag);
    }
    for extra in [vec!["--frob"], vec!["stray"], vec!["--listen", "nope"]] {
        let mut args = base.to_vec();
        args.extend(extra);
        let run = bilbo(dir.path(), &[], &args);
        assert_eq!(run.code, 2, "{}", run.stderr);
        assert!(run.stdout.is_empty());
    }
    assert!(!data.exists());
}

#[test]
fn a_relay_ignores_the_config() {
    let dir = TempDir::new("relay-config");
    let data = data_in(&dir);
    let missing = dir.path().join("nope");
    let env = [("BILBO_CONFIG", missing.to_str().unwrap())];
    let relay = Relay::start_with(&data, &[fingerprint()], "127.0.0.1:0", &[], &env);
    assert_eq!(relay.lines().len(), 1, "{:?}", relay.lines());
    assert_eq!(send(relay.port, "GET", "/v1/", &[], b"").status, 200);
}

// The listen address

#[test]
fn a_public_address_warns_and_serves() {
    let dir = TempDir::new("relay-public");
    let data = data_in(&dir);
    let relay = Relay::start_with(&data, &[fingerprint()], "0.0.0.0:0", &[], &[]);
    relay.wait_for(&format!(
        "bilbo: relay serves plain HTTP on 0.0.0.0:{}; put a TLS proxy in front of it",
        relay.port
    ));
    assert_eq!(send(relay.port, "GET", "/v1/", &[], b"").status, 200);
}

#[test]
fn a_port_in_use_exits_1_and_leaves_the_folder_unlocked() {
    let dir = TempDir::new("relay-port");
    let data = data_in(&dir);
    let holder = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = holder.local_addr().unwrap().port();
    let listen = format!("127.0.0.1:{port}");
    let run = bilbo(
        dir.path(),
        &[],
        &[
            "relay",
            "--data",
            data.to_str().unwrap(),
            "--owner",
            fingerprint(),
            "--listen",
            &listen,
        ],
    );
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr
            .contains(&format!("bilbo: cannot listen on {listen}")),
        "{}",
        run.stderr
    );
    let relay = start(&data);
    assert_eq!(send(relay.port, "GET", "/v1/", &[], b"").status, 200);
}

// The data folder

#[test]
fn the_data_folder_is_created_private_and_the_temporaries_are_cleared() {
    let dir = TempDir::new("relay-folder");
    let data = data_in(&dir);
    drop(start(&data));
    let mode = fs::metadata(&data).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
    assert!(data.join(".relay.lock").exists());
    fs::write(data.join(".tmp/0123456789abcdef"), b"half a body").unwrap();
    assert_eq!(tmp_entries(&data).len(), 1);
    let relay = start(&data);
    assert_eq!(tmp_entries(&data), Vec::<PathBuf>::new());
    assert_eq!(send(relay.port, "GET", "/v1/", &[], b"").status, 200);
}

#[test]
fn objects_are_kept_at_the_paths_of_the_transport_tree() {
    let dir = TempDir::new("relay-tree");
    let data = data_in(&dir);
    let relay = start(&data);
    publish(relay.port, 2);
    let body = b"segment one";
    let reply = put(
        relay.port,
        &rivendell(),
        &segment_target(RIVENDELL, 1),
        body,
    );
    assert_eq!(reply.status, 201, "{}", reply.text());
    assert_eq!(reply.header("Content-Length"), Some("0"));
    assert_eq!(fs::read(segment_path(&data, RIVENDELL, 1)).unwrap(), body);
    for n in [1, 2] {
        let path = data.join(format!("scopes/{}/manifest/{n}.json", scope()));
        assert_eq!(fs::read(path).unwrap(), manifest_bytes(n));
    }
    let tree = tree(&data.join("scopes"));
    assert_eq!(tree.len(), 3, "{:?}", tree.keys().collect::<Vec<_>>());
}

#[test]
fn a_second_relay_on_one_folder_stops_and_the_first_keeps_serving() {
    let dir = TempDir::new("relay-lock");
    let data = data_in(&dir);
    let first = start(&data);
    let run = bilbo(
        dir.path(),
        &[],
        &[
            "relay",
            "--data",
            data.to_str().unwrap(),
            "--owner",
            fingerprint(),
            "--listen",
            "127.0.0.1:0",
        ],
    );
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        run.stderr,
        format!("bilbo: another relay serves {}\n", data.display())
    );
    assert_eq!(send(first.port, "GET", "/v1/", &[], b"").status, 200);
}

#[test]
fn data_that_is_a_file_is_refused_and_left_alone() {
    let dir = TempDir::new("relay-file");
    let data = data_in(&dir);
    fs::write(&data, b"not a folder").unwrap();
    let run = bilbo(
        dir.path(),
        &[],
        &[
            "relay",
            "--data",
            data.to_str().unwrap(),
            "--owner",
            fingerprint(),
        ],
    );
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(
        run.stderr.contains(data.to_str().unwrap()),
        "{}",
        run.stderr
    );
    assert_eq!(fs::read(&data).unwrap(), b"not a folder");
}

// Durable writes

/// Sends the head of a `PUT` of `body` and the first `sent` bytes, then waits until the relay has a file under
/// `.tmp/`: the write has started.
fn start_a_put(port: u16, data: &Path, target: &str, body: &[u8], sent: usize) -> TcpStream {
    let headers = signature(&rivendell(), "PUT", target, body);
    let mut stream = connect(port);
    stream
        .write_all(head("PUT", target, &headers, Some(body.len())).as_bytes())
        .unwrap();
    stream.write_all(&body[..sent]).unwrap();
    stream.flush().unwrap();
    poll_eq("a file under .tmp", || !tmp_entries(data).is_empty(), true);
    stream
}

#[test]
fn a_relay_killed_mid_body_keeps_nothing_and_the_retry_is_created() {
    let dir = TempDir::new("relay-kill-body");
    let data = data_in(&dir);
    let body = vec![7u8; 256 * 1024];
    let target = segment_target(RIVENDELL, 1);
    let mut relay = start(&data);
    publish(relay.port, 2);
    let stream = start_a_put(relay.port, &data, &target, &body, body.len() / 2);
    relay.kill();
    drop(stream);
    let relay = start(&data);
    assert!(tmp_entries(&data).is_empty());
    assert!(!segment_path(&data, RIVENDELL, 1).exists());
    assert_eq!(get(relay.port, &rivendell(), &target).status, 404);
    assert_eq!(put(relay.port, &rivendell(), &target, &body).status, 201);
    assert_eq!(fs::read(segment_path(&data, RIVENDELL, 1)).unwrap(), body);
}

#[test]
fn a_created_object_survives_a_kill_after_the_answer() {
    let dir = TempDir::new("relay-kill-after");
    let data = data_in(&dir);
    let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let target = segment_target(RIVENDELL, 1);
    let mut relay = start(&data);
    publish(relay.port, 2);
    let mut stream = start_a_put(relay.port, &data, &target, &body, body.len() / 2);
    stream.write_all(&body[body.len() / 2..]).unwrap();
    let reply = read_reply(&mut stream);
    assert_eq!(reply.status, 201, "{}", reply.text());
    relay.kill();
    let relay = start(&data);
    let reply = get(relay.port, &rivendell(), &target);
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, body);
    assert!(tmp_entries(&data).is_empty());
}

/// Removes the file it names when dropped.
struct Filler(PathBuf);

impl Drop for Filler {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn a_full_disk_answers_507_and_keeps_serving_reads() {
    let Some(tmpfs) = std::env::var_os("BILBO_TEST_TMPFS") else {
        eprintln!("skipped: BILBO_TEST_TMPFS names no size-limited tmpfs");
        return;
    };
    let data = PathBuf::from(&tmpfs).join(format!("bilbo-relay-full-{}", std::process::id()));
    let _clean = Filler(data.clone());
    let relay = start(&data);
    publish(relay.port, 2);
    let filler = Filler(PathBuf::from(&tmpfs).join(format!("bilbo-filler-{}", std::process::id())));
    {
        let mut file = fs::File::create(&filler.0).unwrap();
        let chunk = vec![0u8; 64 * 1024];
        while file.write_all(&chunk).is_ok() {}
    }
    let target = segment_target(RIVENDELL, 1);
    let reply = put(relay.port, &rivendell(), &target, &vec![9u8; 512 * 1024]);
    assert_eq!(reply.status, 507, "{}", reply.text());
    assert_eq!(reply.text(), r#"{"error":"quota"}"#);
    assert!(!segment_path(&data, RIVENDELL, 1).exists());
    assert!(tmp_entries(&data).is_empty());
    let latest = format!("/v1/scopes/{}/manifest/latest", scope());
    assert_eq!(get(relay.port, &rivendell(), &latest).status, 200);
    relay.wait_for(&format!(
        "failed 507 quota PUT segment {} {RIVENDELL}",
        scope()
    ));
    let _ = fs::remove_dir_all(&data);
}

// Admitted and valid scopes

/// The lines of `relay` that name `needle`.
fn mentioning(relay: &Relay, needle: &str) -> Vec<String> {
    relay
        .lines()
        .into_iter()
        .filter(|l| l.contains(needle))
        .collect()
}

#[test]
fn a_hand_edited_manifest_marks_its_scope_invalid() {
    let dir = TempDir::new("relay-tampered");
    let data = data_in(&dir);
    preload(&data, &[1, 2]);
    let path = data.join(format!("scopes/{}/manifest/2.json", scope()));
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"name\":\"bagend\""));
    fs::write(
        &path,
        text.replacen("\"name\":\"bagend\"", "\"name\":\"bagenx\"", 1),
    )
    .unwrap();
    let before = tree(&data.join("scopes"));
    let relay = start(&data);
    let id = scope();
    // Every request below is answered after the walk, so the one line it prints is already there.
    let devices = format!("/v1/scopes/{id}/devices/");
    for key in [rivendell(), bagend()] {
        let reply = get(relay.port, &key, &devices);
        assert_eq!(reply.status, 403, "{}", reply.text());
        assert_eq!(reply.text(), r#"{"error":"invalid"}"#);
    }
    let reply = put(relay.port, &rivendell(), "/v1/pair/42/a.msg", b"hello");
    assert_eq!(reply.status, 403, "{}", reply.text());
    assert_eq!(
        mentioning(&relay, &format!("scope {id} is invalid")).len(),
        1
    );
    assert_eq!(tree(&data.join("scopes")), before);
}

#[test]
fn a_gap_in_a_devices_seqs_marks_its_scope_invalid() {
    let dir = TempDir::new("relay-gap");
    let data = data_in(&dir);
    preload(&data, &[1, 2]);
    preload_segment(&data, RIVENDELL, 1, b"one");
    preload_segment(&data, RIVENDELL, 3, b"three");
    let before = tree(&data.join("scopes"));
    let relay = start(&data);
    let reply = get(
        relay.port,
        &rivendell(),
        &format!("/v1/scopes/{}/devices/", scope()),
    );
    assert_eq!(reply.status, 403, "{}", reply.text());
    assert_eq!(reply.text(), r#"{"error":"invalid"}"#);
    assert_eq!(
        mentioning(&relay, &format!("scope {} is invalid", scope())).len(),
        1
    );
    assert_eq!(tree(&data.join("scopes")), before);
}

#[test]
fn an_owner_dropped_from_the_flags_is_not_admitted_and_its_folder_is_unchanged() {
    let dir = TempDir::new("relay-dropped");
    let data = data_in(&dir);
    preload(&data, &[1, 2]);
    preload_segment(&data, RIVENDELL, 1, b"one");
    let before = tree(&data.join("scopes"));
    let other = foreign_fingerprint();
    let relay = Relay::start(&data, &[&other], &[]);
    let id = scope();
    for target in [
        format!("/v1/scopes/{id}/devices/"),
        format!("/v1/scopes/{id}/manifest/latest"),
    ] {
        let reply = get(relay.port, &rivendell(), &target);
        assert_eq!(reply.status, 403, "{}", reply.text());
        assert_eq!(reply.text(), r#"{"error":"not-admitted"}"#);
    }
    let owner_read = get(
        relay.port,
        &owner(),
        &format!("/v1/scopes/{id}/manifest/latest"),
    );
    assert_eq!(owner_read.status, 403);
    assert_eq!(
        mentioning(&relay, &format!("scope {id} is not admitted")).len(),
        1
    );
    assert_eq!(tree(&data.join("scopes")), before);
    drop(relay);
    let relay = start(&data);
    let reply = get(
        relay.port,
        &rivendell(),
        &format!("/v1/scopes/{id}/devices/"),
    );
    assert_eq!(reply.status, 200, "{}", reply.text());
}

#[test]
fn two_owners_are_admitted_and_each_lists_only_its_own() {
    let dir = TempDir::new("relay-two-owners");
    let data = data_in(&dir);
    preload(&data, &[1, 2]);
    preload_foreign(&data);
    let other = foreign_fingerprint();
    let relay = Relay::start(&data, &[fingerprint(), &other], &[]);
    // Neither scope is logged as invalid or not admitted.
    assert_eq!(relay.lines().len(), 1, "{:?}", relay.lines());
    let listing = get(relay.port, &owner(), "/v1/scopes/");
    assert_eq!(listing.status, 200, "{}", listing.text());
    assert_eq!(listing.text(), format!("{{\"scopes\":[\"{}\"]}}", scope()));
    let theirs = get(
        relay.port,
        &owner(),
        &format!("/v1/scopes/{}/manifest/latest", foreign_scope()),
    );
    assert_eq!(theirs.status, 403, "{}", theirs.text());
    let own = get(
        relay.port,
        &owner(),
        &format!("/v1/scopes/{}/manifest/latest", scope()),
    );
    assert_eq!(own.status, 200);
    assert_eq!(own.header("Bilbo-Manifest"), Some("2"));
}

#[test]
fn a_copied_file_transport_is_served_as_if_created_through_the_relay() {
    let dir = TempDir::new("relay-copied");
    let transport = dir.path().join("transport");
    preload(&transport, &[1, 2]);
    preload_segment(&transport, RIVENDELL, 1, b"one");
    preload_segment(&transport, RIVENDELL, 2, b"two");
    let data = data_in(&dir);
    copy_tree(&transport, &data);
    chmod(&data, 0o700);
    let relay = start(&data);
    let id = scope();
    let listing = get(
        relay.port,
        &rivendell(),
        &format!("/v1/scopes/{id}/devices/"),
    );
    assert_eq!(listing.status, 200, "{}", listing.text());
    assert_eq!(
        listing.text(),
        format!("{{\"devices\":[{{\"id\":\"{RIVENDELL}\",\"last\":2}}]}}")
    );
    let segment = get(relay.port, &bagend(), &segment_target(RIVENDELL, 2));
    assert_eq!(segment.body, b"two");
    let latest = get(
        relay.port,
        &bagend(),
        &format!("/v1/scopes/{id}/manifest/latest"),
    );
    assert_eq!(latest.header("Bilbo-Manifest"), Some("2"));
    assert_eq!(latest.body, manifest_bytes(2));
    // A device continues the folder where it left off, as on a relay that created it.
    let next = put(
        relay.port,
        &rivendell(),
        &segment_target(RIVENDELL, 3),
        b"three",
    );
    assert_eq!(next.status, 201, "{}", next.text());
    assert_eq!(relay.lines().len() - 1, 1);
}

// What the relay logs

#[test]
fn a_create_is_logged_and_reads_are_not() {
    let dir = TempDir::new("relay-log-create");
    let data = data_in(&dir);
    let relay = start(&data);
    publish(relay.port, 2);
    let mut last = Vec::new();
    for seq in 1..=12u64 {
        last = vec![seq as u8; 100 + seq as usize];
        let reply = put(
            relay.port,
            &rivendell(),
            &segment_target(RIVENDELL, seq),
            &last,
        );
        assert_eq!(reply.status, 201, "{}", reply.text());
    }
    let line = format!("segment {} {RIVENDELL} 12 {}", scope(), last.len());
    assert!(after_startup(&relay).contains(&line), "{:?}", relay.lines());
    let before = relay.lines().len();
    for i in 0..500u64 {
        let seq = i % 12 + 1;
        let reply = get(relay.port, &bagend(), &segment_target(RIVENDELL, seq));
        assert_eq!(reply.status, 200);
    }
    // A create is logged, so its line is the barrier behind which any line for the reads would stand.
    let reply = put(
        relay.port,
        &rivendell(),
        &segment_target(RIVENDELL, 13),
        b"x",
    );
    assert_eq!(reply.status, 201);
    relay.wait_for(&format!("segment {} {RIVENDELL} 13 1", scope()));
    assert_eq!(relay.lines().len(), before + 1, "{:?}", relay.lines());
}

#[test]
fn nothing_personal_reaches_the_log() {
    let dir = TempDir::new("relay-log-private");
    let data = data_in(&dir);
    let relay = start(&data);
    publish(relay.port, 2);
    let plate = "plate-xyzzy-77";
    let message = b"payload-plover-secret";
    let opener = put(
        relay.port,
        &rivendell(),
        &format!("/v1/pair/{plate}/a.msg"),
        message,
    );
    assert_eq!(opener.status, 201, "{}", opener.text());
    let answer = send(
        relay.port,
        "PUT",
        &format!("/v1/pair/{plate}/b.msg"),
        &[],
        b"answer-plover-secret",
    );
    assert_eq!(answer.status, 201, "{}", answer.text());
    // A listed device that did not open the nameplate is refused before its body is read, which counts the refusal.
    let refused = put(
        relay.port,
        &bagend(),
        &format!("/v1/pair/{plate}/c.msg"),
        b"intruder",
    );
    assert_eq!(refused.status, 403, "{}", refused.text());
    // A refusal of a verified key is logged: the owner may not list devices.
    let devices = format!("/v1/scopes/{}/devices/", scope());
    assert_eq!(get(relay.port, &owner(), &devices).status, 403);
    relay.wait_for("refused 403 not-admitted GET devices");
    let lines = after_startup(&relay);
    assert!(
        lines.contains(&format!("mailbox {}", message.len())),
        "{lines:?}"
    );
    let key = hex(rivendell().verifying_key().as_bytes().as_slice());
    for line in &lines {
        for secret in [
            plate,
            "plover",
            "intruder",
            "127.0.0.1",
            key.as_str(),
            "Bilbo-",
        ] {
            assert!(!line.contains(secret), "{line:?} holds {secret:?}");
        }
    }
}

#[test]
fn a_flood_of_refusals_is_one_counting_line() {
    let dir = TempDir::new("relay-log-flood");
    let data = data_in(&dir);
    let relay = start(&data);
    let port = relay.port;
    let answered: u64 = std::thread::scope(|s| {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                s.spawn(move || {
                    let mut refused = 0;
                    while refused < 1250 {
                        let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        };
                        stream
                            .set_read_timeout(Some(Duration::from_secs(40)))
                            .unwrap();
                        let request = head("GET", "/v1/scopes/", &[], None);
                        if stream.write_all(request.as_bytes()).is_err() {
                            continue;
                        }
                        let mut bytes = Vec::new();
                        let _ = stream.read_to_end(&mut bytes);
                        if bytes.starts_with(b"HTTP/1.1 401 ") {
                            refused += 1;
                        }
                    }
                    refused
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).sum()
    });
    assert_eq!(answered, 10_000);
    // The count is written by the sweeper's tick, at most once a minute.
    let deadline = Instant::now() + Duration::from_secs(150);
    while mentioning(&relay, "other requests in the last minute").is_empty() {
        assert!(
            Instant::now() < deadline,
            "no counting line: {:?}",
            relay.lines()
        );
        std::thread::sleep(Duration::from_millis(250));
    }
    // A create is logged at once, so its line is the barrier behind which any further refusal line would stand.
    publish(relay.port, 1);
    relay.wait_for(&format!("manifest {} {BAGEND} 1", scope()));
    let lines = after_startup(&relay);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(
        lines[0],
        "refused 10000 other requests in the last minute: signature 10000"
    );
}

// Sync through a relay

/// One device: a store, its own state folder and config, and a watcher while it runs.
struct Site {
    dir: TempDir,
    name: &'static str,
    env: Vec<(String, String)>,
    watcher: Option<Watcher>,
}

impl Site {
    /// `who`'s keys and the manifest files `manifests` (versions 1, 2, ...) in a new store, syncing `personal`
    /// through `url` every second.
    fn new(who: &'static str, url: &str, manifests: &[Vec<u8>]) -> Site {
        let dir = TempDir::new(who);
        let root = dir.path().join("store");
        fs::create_dir_all(root.join("notes")).unwrap();
        let into = root.join(format!(".bilbo/scopes/{}/manifest", scope()));
        fs::create_dir_all(&into).unwrap();
        for (i, bytes) in manifests.iter().enumerate() {
            fs::write(into.join(format!("{}.json", i + 1)), bytes).unwrap();
        }
        let keys = dir.path().join("state/bilbo/keys");
        copy_tree(&Path::new(FIXTURES).join(who), &keys);
        chmod(&keys, 0o700);
        for file in ["owner.key", "device.key"] {
            chmod(&keys.join(file), 0o600);
        }
        config(
            &dir,
            &[
                &format!("scope.personal.sync = {url}"),
                "sync.poll_seconds = 1",
            ],
        );
        Site {
            env: vec![
                ("BILBO_HOME".into(), root.to_str().unwrap().into()),
                (
                    "BILBO_CONFIG".into(),
                    dir.path().join("config").to_str().unwrap().into(),
                ),
                (
                    "XDG_STATE_HOME".into(),
                    dir.path().join("state").to_str().unwrap().into(),
                ),
                ("HOME".into(), "/home/tester".into()),
            ],
            dir,
            name: who,
            watcher: None,
        }
    }

    fn notes(&self) -> PathBuf {
        self.dir.path().join("store/notes")
    }

    fn pairs(&self) -> Vec<(&str, &str)> {
        self.env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }

    fn start(&mut self) {
        let watcher = Watcher::start(&self.pairs());
        watcher.wait_for(&format!("bilbo: watching {}", self.notes().display()));
        self.watcher = Some(watcher);
    }

    fn stop(&mut self) {
        self.watcher = None;
    }

    fn wait_for(&self, needle: &str) {
        self.watcher.as_ref().unwrap().wait_for(needle);
    }

    fn lines(&self) -> Vec<String> {
        self.watcher.as_ref().map_or_else(Vec::new, Watcher::lines)
    }

    fn write(&self, name: &str, text: &str) {
        fs::write(self.notes().join(name), text).unwrap();
    }

    fn read(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.notes().join(name)).ok()
    }

    fn wait_text(&self, name: &str, text: &str) {
        poll_eq(
            &format!("{} holds {name}", self.name),
            || self.read(name),
            Some(text.to_string()),
        );
    }
}

fn note(id: &str, setup: &str, rollout: &str) -> String {
    format!(
        "---\nid: {id}\ncreated: 2026-10-02T14:23-03:00\nscope: personal\n---\n\n# Release\n\n## Setup\n\n{setup}\n\n## Rollout\n\n{rollout}\n"
    )
}

/// The epoch key of a fixture manifest, opened with rivendell's box key.
fn epoch_key(m: &serde_json::Value) -> [u8; 32] {
    let sealed = unhex(m["sealed"][RIVENDELL].as_str().unwrap());
    let secret = <Kem as hpke::Kem>::PrivateKey::from_bytes(&[2u8; 32]).unwrap();
    let enc = <Kem as hpke::Kem>::EncappedKey::from_bytes(&sealed[..32]).unwrap();
    let aad = format!(
        "{}\n{}\n{RIVENDELL}",
        m["scope"].as_str().unwrap(),
        m["epoch"]
    );
    let plain = hpke::single_shot_open::<Seal, Kdf, Kem>(
        &OpModeR::Base,
        &secret,
        &enc,
        b"bilbo-epoch-1",
        &sealed[32..],
        aad.as_bytes(),
    )
    .unwrap();
    plain.try_into().unwrap()
}

/// The scope's sealed name for version `n`, from the one of `m` (version `m.n`).
fn resealed_name(m: &serde_json::Value, n: u64) -> String {
    let key = epoch_key(m);
    let (scope, epoch) = (m["scope"].as_str().unwrap(), &m["epoch"]);
    let cipher = XChaCha20Poly1305::new_from_slice(&key).unwrap();
    let sealed = unhex(m["name"].as_str().unwrap());
    let old = format!("bilbo-name-1\n{scope}\n{epoch}\n{}", m["n"]);
    let (nonce, text) = sealed.split_at(24);
    let nonce: [u8; 24] = nonce.try_into().unwrap();
    let name = cipher
        .decrypt(
            (&nonce).into(),
            Payload {
                msg: text,
                aad: old.as_bytes(),
            },
        )
        .unwrap();
    let fresh = unhex(&sha256_hex(format!("{n}{}", hex(&name)).as_bytes()));
    let nonce: [u8; 24] = fresh[..24].try_into().unwrap();
    let new = format!("bilbo-name-1\n{scope}\n{epoch}\n{n}");
    let text = cipher
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: &name,
                aad: new.as_bytes(),
            },
        )
        .unwrap();
    hex(&[&fresh[..24], &text[..]].concat())
}

/// The version after `prev` that pins `transport`: `prev` with `n`, `prev`, `transport` and the sealed name replaced,
/// signed by the owner. The fixture manifests pin `file://`, and a relay's port is not known until it runs.
fn repinned(prev: &[u8], n: u64, transport: &str) -> Vec<u8> {
    let text = String::from_utf8(prev.to_vec()).unwrap();
    let before: serde_json::Value = serde_json::from_str(&text).unwrap();
    let cut = text.find(",\"sig\":\"").unwrap();
    let mut body = format!("{}}}", &text[..cut]);
    let swap = |body: &mut String, from: &str, to: &str, value: String| {
        let a = body.find(from).unwrap() + from.len();
        let b = a + body[a..].find(to).unwrap();
        body.replace_range(a..b, &value);
    };
    swap(&mut body, "\"n\":", ",\"prev\"", n.to_string());
    swap(
        &mut body,
        "\"prev\":",
        ",\"owner\"",
        format!("\"{}\"", sha256_hex(prev)),
    );
    swap(
        &mut body,
        "\"transport\":",
        ",\"epoch\"",
        format!("\"{transport}\""),
    );
    // The scope's name is the last member; device entries have names too.
    let at = body.rfind("\"name\":").unwrap() + "\"name\":".len();
    body.replace_range(
        at..body.len() - 1,
        &format!("\"{}\"", resealed_name(&before, n)),
    );
    let mut message = b"bilbo-manifest-1\n".to_vec();
    message.extend(body.as_bytes());
    let sig = hex(&owner().sign(&message).to_bytes());
    body.pop();
    format!("{body},\"sig\":\"{sig}\"}}\n").into_bytes()
}

/// The fixture's manifests 1 and 2 and a version 3 that pins `url`, and the same pinned to a folder as version 4.
fn pinned(url: &str) -> Vec<Vec<u8>> {
    let three = repinned(&manifest_bytes(2), 3, url);
    vec![manifest_bytes(1), manifest_bytes(2), three]
}

#[test]
fn sync_through_relay() {
    let dir = TempDir::new("relay-sync");
    let data = data_in(&dir);
    preload(&data, &[1, 2]);
    let relay = start(&data);
    let url = relay.url();
    let manifests = pinned(&url);
    let reply = put(relay.port, &rivendell(), &manifest_target(3), &manifests[2]);
    assert_eq!(reply.status, 201, "{}", reply.text());
    let mut a = Site::new("rivendell", &url, &manifests);
    let mut b = Site::new("bagend", &url, &manifests);
    a.start();
    b.start();
    for site in [&a, &b] {
        site.wait_for(&format!("bilbo: syncing personal through {url}"));
    }
    let text = note(ID, "Install it.", "Ship on Monday.");
    a.write(FILE, &text);
    b.wait_text(FILE, &text);
    let edited = format!("{text}\nA paragraph from rivendell.\n");
    a.write(FILE, &edited);
    b.wait_text(FILE, &edited);
    // A note of the other device travels the other way.
    let reply = note(OTHER, "Open it.", "Ship on Tuesday.").replace("# Release", "# Other");
    b.write("plan-other.md", &reply);
    a.wait_text("plan-other.md", &reply);
    for site in [&a, &b] {
        for line in site.lines() {
            assert!(!line.contains("not reachable"), "{line}");
            assert!(!line.contains("does not admit"), "{line}");
        }
    }
    // Every object the devices made went through the relay.
    assert!(segment_path(&data, RIVENDELL, 1).exists());
    assert!(segment_path(&data, BAGEND, 1).exists());
    assert_eq!(
        mentioning(&relay, &format!("segment {} {RIVENDELL} 1 ", scope())).len(),
        1,
        "{:?}",
        relay.lines()
    );
}

#[test]
fn data_folder_is_a_file_transport() {
    let dir = TempDir::new("relay-folder-transport");
    let data = data_in(&dir);
    preload(&data, &[1, 2]);
    let mut relay = start(&data);
    let url = relay.url();
    let manifests = pinned(&url);
    let reply = put(relay.port, &rivendell(), &manifest_target(3), &manifests[2]);
    assert_eq!(reply.status, 201, "{}", reply.text());
    let mut a = Site::new("rivendell", &url, &manifests);
    a.start();
    a.wait_for(&format!("bilbo: syncing personal through {url}"));
    let text = note(ID, "Install it.", "Ship on Monday.");
    a.write(FILE, &text);
    relay.wait_for(&format!("segment {} {RIVENDELL} 1 ", scope()));
    a.stop();
    relay.kill();
    // Only objects of the tree's grammar are left under scopes/.
    let files = tree(&data.join("scopes"));
    for path in files.keys() {
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(
            name.ends_with(".json") || name.ends_with(".seg"),
            "{}",
            path.display()
        );
    }
    assert_eq!(files.len(), 4, "{:?}", files.keys().collect::<Vec<_>>());
    // The owner moves the scope back to the folder: a version 4 that pins `file://`.
    let folder = format!("file://{}", data.display());
    let four = repinned(&manifests[2], 4, "file://");
    let into = data.join(format!("scopes/{}/manifest/4.json", scope()));
    fs::write(into, &four).unwrap();
    let mut all = manifests.clone();
    all.push(four);
    let mut b = Site::new("bagend", &folder, &all);
    b.start();
    b.wait_text(FILE, &text);
}

#[test]
fn a_relay_that_refuses_the_owner_reaches_the_watch_log_and_sync() {
    let dir = TempDir::new("relay-refusal");
    let data = data_in(&dir);
    let other = foreign_fingerprint();
    let relay = Relay::start(&data, &[&other], &[]);
    let url = relay.url();
    let mut a = Site::new("rivendell", &url, &pinned(&url));
    a.start();
    let refusal = format!(
        "bilbo: sync personal: relay {url} does not admit this owner; start it with --owner {}",
        fingerprint()
    );
    a.wait_for(&refusal);
    let run = bilbo(a.dir.path(), &a.pairs(), &["sync"]);
    assert!(
        run.stdout.contains(&refusal[PREFIX.len()..]) || run.stderr.contains(&refusal),
        "stdout: {}\nstderr: {}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn a_relay_that_goes_down_reaches_the_watch_log() {
    let dir = TempDir::new("relay-down");
    let data = data_in(&dir);
    preload(&data, &[1, 2]);
    let mut relay = start(&data);
    let url = relay.url();
    let manifests = pinned(&url);
    let reply = put(relay.port, &rivendell(), &manifest_target(3), &manifests[2]);
    assert_eq!(reply.status, 201, "{}", reply.text());
    let mut a = Site::new("rivendell", &url, &manifests);
    a.start();
    a.wait_for(&format!("bilbo: syncing personal through {url}"));
    relay.kill();
    a.wait_for(&format!("bilbo: sync personal: relay {url} unreachable: "));
}
