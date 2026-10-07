//! The whole exchange in one process: both sides on two threads, two folders joined by a delayed copier.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::{self, BufRead, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use super::*;
use crate::identity::keys::{self, Device, Owner};
use crate::identity::manifest::{self, Recipient};
use crate::identity::phrase;
use crate::shared::store;
use crate::sync::transport::{self, Put, Transport};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/device");

/// How long a test waits for a side to end, or for a line, before it fails.
const DEADLINE: Duration = Duration::from_secs(30);

type Files = BTreeMap<String, Vec<u8>>;

/// Every version of every `pair/` file the copier saw, by path.
type Seen = BTreeMap<String, Vec<Vec<u8>>>;

/// The lines a side printed, in order.
#[derive(Default)]
struct Log(Mutex<Vec<String>>);

impl Log {
    fn push(&self, line: &str) {
        self.0.lock().unwrap().push(line.to_string());
    }

    fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    /// The first line starting with `prefix`, waiting for it.
    fn wait_for(&self, prefix: &str) -> Option<String> {
        let until = Instant::now() + DEADLINE;
        loop {
            if let Some(line) = self.lines().into_iter().find(|l| l.starts_with(prefix)) {
                return Some(line);
            }
            if Instant::now() >= until {
                return None;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let limit = Instant::now() + DEADLINE;
    while !done() {
        assert!(Instant::now() < limit, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Every regular file under `root` by its relative path; hidden files only when `hidden`.
fn files(root: &Path, hidden: bool) -> Files {
    fn walk(dir: &Path, rel: &str, hidden: bool, out: &mut Files) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !hidden && name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let rel = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => walk(&path, &rel, hidden, out),
                Ok(kind) if kind.is_file() => {
                    if let Ok(bytes) = fs::read(&path) {
                        out.insert(rel, bytes);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Files::new();
    walk(root, "", hidden, &mut out);
    out
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

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// A temporary folder holding the machines of one test.
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
        std::env::temp_dir().join(format!("bilbo-exchange-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    World(dir)
}

impl World {
    fn machine(&self, name: &str) -> Machine {
        let base = self.0.join(name);
        for sub in ["root/notes", "state", "xdg", "home", "sync"] {
            fs::create_dir_all(base.join(sub)).unwrap();
        }
        Machine { base }
    }

    /// A, the device that shows the code: the fixture `rhosgobel` with the fixture `personal` scope, syncing through
    /// its own folder.
    fn a(&self) -> Machine {
        let a = self.machine("a");
        a.enroll("rhosgobel");
        a.config(&format!("scope.personal.sync = {}\n", a.url()));
        a
    }

    /// B, a device with no keys, through its own folder.
    fn b(&self) -> Machine {
        self.machine("b")
    }
}

/// One device's disk.
struct Machine {
    base: PathBuf,
}

impl Machine {
    fn root(&self) -> PathBuf {
        self.base.join("root")
    }

    fn sync(&self) -> PathBuf {
        self.base.join("sync")
    }

    fn url(&self) -> String {
        format!("file://{}", self.sync().display())
    }

    fn keys(&self) -> PathBuf {
        self.base.join("state/bilbo/keys")
    }

    fn pending(&self) -> PathBuf {
        keys::pending_path(&self.keys())
    }

    fn config_path(&self) -> PathBuf {
        self.base.join("xdg/bilbo/config")
    }

    fn config(&self, text: &str) {
        fs::create_dir_all(self.config_path().parent().unwrap()).unwrap();
        fs::write(self.config_path(), text).unwrap();
    }

    fn config_text(&self) -> Option<String> {
        fs::read_to_string(self.config_path()).ok()
    }

    /// The fixture keys of `who` in the keys folder, with the modes `bilbo device` demands.
    fn put_keys(&self, who: &str) {
        let keys = self.keys();
        copy_tree(&Path::new(FIXTURES).join(who), &keys);
        chmod(&keys, 0o700);
        for file in ["owner.key", "device.key"] {
            chmod(&keys.join(file), 0o600);
        }
    }

    /// The manifests of a fixture store (`store` or `foreign`).
    fn put_store(&self, which: &str) {
        copy_tree(
            &Path::new(FIXTURES).join(which).join(".bilbo"),
            &self.root().join(".bilbo"),
        );
    }

    fn enroll(&self, who: &str) {
        self.put_keys(who);
        self.put_store("store");
    }

    fn identity(&self) -> keys::Identity {
        keys::read_identity(&self.keys()).unwrap().unwrap()
    }

    fn scope(&self, id: &str) -> manifest::Scope {
        manifest::read_scope(&self.root(), id).unwrap()
    }

    /// A scope `name` that this device owns, syncing through its folder, and its config line.
    fn add_scope(&self, name: &str) -> String {
        let lock = manifest::lock(&self.root()).unwrap();
        let id = manifest::create(&lock, &self.identity(), name, &self.url(), &[])
            .unwrap()
            .scope;
        let mut config = self.config_text().unwrap_or_default();
        config.push_str(&format!("scope.{name}.sync = {}\n", self.url()));
        self.config(&config);
        id
    }

    /// Every file of this machine.
    fn tree(&self) -> Files {
        files(&self.base, true)
    }

    fn key_files(&self) -> Files {
        files(&self.keys(), true)
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

    /// Plants `pair/<nameplate>/a.msg`, last modified `age` ago.
    fn plant(&self, nameplate: &str, age: Duration) {
        let dir = self.sync().join("pair").join(nameplate);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.msg");
        fs::write(&file, b"{}").unwrap();
        fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(SystemTime::now() - age)
            .unwrap();
    }
}

fn env_at(base: &Path) -> store::Env {
    store::Env {
        bilbo_home: Some(base.join("root").into()),
        home: Some(base.join("home").into()),
        xdg_config_home: Some(base.join("xdg").into()),
        xdg_state_home: Some(base.join("state").into()),
        ..store::Env::from_vars(|_| None)
    }
}

/// The fixture scope's id: the one folder of the fixture store.
fn personal() -> String {
    let mut ids = manifest::scope_ids(&Path::new(FIXTURES).join("store")).unwrap();
    assert_eq!(ids.len(), 1);
    ids.remove(0)
}

fn limits() -> Limits {
    Limits {
        window: Duration::from_secs(10),
        appear: Duration::from_secs(2),
        manifests: Duration::from_secs(3),
        poll_file: Duration::from_millis(5),
        poll_https: Duration::from_millis(5),
        sweep: Duration::from_secs(30 * 60),
    }
}

type Delay = Box<dyn Fn(&str) -> Option<Duration> + Send>;

type Tamper = Box<dyn Fn(&str, &[u8]) -> Vec<u8> + Send>;

/// How the copier treats the files it moves.
struct Rules {
    /// How long a file waits before it crosses; `None` never lets it.
    delay: Delay,
    /// What arrives for the bytes that left.
    tamper: Tamper,
}

impl Rules {
    fn prompt() -> Rules {
        Rules {
            delay: Box::new(|_| Some(Duration::from_millis(10))),
            tamper: Box::new(|_, bytes| bytes.to_vec()),
        }
    }

    /// Everything under `scopes/` crosses after `delay`, or never.
    fn manifests(delay: Option<Duration>) -> Rules {
        Rules {
            delay: Box::new(move |rel| {
                if rel.starts_with("scopes/") {
                    delay
                } else {
                    Some(Duration::from_millis(10))
                }
            }),
            ..Rules::prompt()
        }
    }

    /// Replaces the bytes of every file whose path ends with `suffix`.
    fn replacing(suffix: &'static str, with: &'static [u8]) -> Rules {
        Rules {
            tamper: Box::new(move |rel, bytes| {
                if rel.ends_with(suffix) {
                    with.to_vec()
                } else {
                    bytes.to_vec()
                }
            }),
            ..Rules::prompt()
        }
    }
}

/// Two folders kept alike by a thread: a new, changed or removed file crosses after its delay.
struct Copier {
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<Seen>>,
    thread: Option<JoinHandle<()>>,
}

impl Copier {
    fn start(a: PathBuf, b: PathBuf, rules: Rules) -> Copier {
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Seen::new()));
        let thread = {
            let (stop, seen) = (stop.clone(), seen.clone());
            std::thread::spawn(move || copy_loop(&a, &b, &rules, &stop, &seen))
        };
        Copier {
            stop,
            seen,
            thread: Some(thread),
        }
    }

    fn seen(&self) -> Seen {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for Copier {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The bytes a path is waiting to carry across, and since when.
type Waiting = (Option<Vec<u8>>, Instant);

/// What each side held the last time they agreed on a path.
type Known = (Option<Vec<u8>>, Option<Vec<u8>>);

fn copy_loop(a: &Path, b: &Path, rules: &Rules, stop: &AtomicBool, seen: &Mutex<Seen>) {
    let mut known: HashMap<String, Known> = HashMap::new();
    let mut waiting: HashMap<(String, bool), Waiting> = HashMap::new();
    while !stop.load(Ordering::Relaxed) {
        let (on_a, on_b) = (files(a, false), files(b, false));
        {
            let mut seen = seen.lock().unwrap();
            for (rel, bytes) in on_a.iter().chain(&on_b) {
                if rel.starts_with("pair/") {
                    let versions = seen.entry(rel.clone()).or_default();
                    if versions.last() != Some(bytes) {
                        versions.push(bytes.clone());
                    }
                }
            }
        }
        let paths: BTreeSet<String> = on_a
            .keys()
            .chain(on_b.keys())
            .chain(known.keys())
            .cloned()
            .collect();
        for rel in paths {
            let (was_a, was_b) = known.get(&rel).cloned().unwrap_or((None, None));
            let (now_a, now_b) = (on_a.get(&rel).cloned(), on_b.get(&rel).cloned());
            let to_b = match (now_a != was_a, now_b != was_b) {
                (true, false) => true,
                (false, true) => false,
                (true, true) => {
                    let sent = now_a.as_ref().map(|bytes| (rules.tamper)(&rel, bytes));
                    if sent == now_b || now_a == now_b {
                        known.insert(rel.clone(), (now_a, now_b));
                    }
                    waiting.remove(&(rel.clone(), true));
                    waiting.remove(&(rel, false));
                    continue;
                }
                (false, false) => {
                    if known.get(&rel) == Some(&(None, None)) {
                        known.remove(&rel);
                    }
                    continue;
                }
            };
            let source = if to_b { now_a.clone() } else { now_b.clone() };
            let key = (rel.clone(), to_b);
            let entry = waiting
                .entry(key.clone())
                .or_insert_with(|| (source.clone(), Instant::now()));
            if entry.0 != source {
                *entry = (source.clone(), Instant::now());
            }
            let ready = (rules.delay)(&rel).is_some_and(|delay| entry.1.elapsed() >= delay);
            if !ready {
                continue;
            }
            waiting.remove(&key);
            let delivered = source.as_ref().map(|bytes| (rules.tamper)(&rel, bytes));
            deliver(if to_b { b } else { a }, &rel, delivered.as_deref());
            known.insert(
                rel,
                if to_b {
                    (source, delivered)
                } else {
                    (delivered, source)
                },
            );
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Writes `bytes` at `rel` under `root` whole, or removes the file and the mailbox folder it leaves empty.
fn deliver(root: &Path, rel: &str, bytes: Option<&[u8]>) {
    let path = root.join(rel);
    match bytes {
        Some(bytes) => {
            let dir = path.parent().unwrap();
            fs::create_dir_all(dir).unwrap();
            let tmp = dir.join(".copier.tmp");
            fs::write(&tmp, bytes).unwrap();
            fs::rename(&tmp, &path).unwrap();
        }
        None => {
            let _ = fs::remove_file(&path);
            if let Some(dir) = path.parent()
                && dir.parent().is_some_and(|up| up != root)
            {
                let _ = fs::remove_dir(dir);
            }
        }
    }
}

/// What the user types at A's question, once B has printed its fingerprint.
#[derive(Clone, Copy, PartialEq)]
enum Confirm {
    Yes,
    No,
    /// Ends input without a line.
    Eof,
    /// Types `y` once A's window has passed.
    Late,
}

/// A's stdin: blocks on its first read until `gate` returns, then yields `text`.
struct Answer {
    text: Vec<u8>,
    pos: usize,
    gate: Option<Box<dyn FnOnce() + Send>>,
}

/// When `a` printed its code.
type Shown = Arc<Mutex<Option<Instant>>>;

impl Answer {
    fn none() -> Answer {
        Answer {
            text: Vec::new(),
            pos: 0,
            gate: None,
        }
    }

    fn after(text: &str, gate: impl FnOnce() + Send + 'static) -> Answer {
        Answer {
            text: text.as_bytes().to_vec(),
            pos: 0,
            gate: Some(Box::new(gate)),
        }
    }

    /// `confirm`, typed once `b` has printed its fingerprint. A late `y` waits until `late` is past
    /// the instant `a` showed its code, by the window it holds.
    fn typed(confirm: Confirm, b: Arc<Log>, late: Option<(Shown, Duration)>) -> Answer {
        let text = match confirm {
            Confirm::Yes | Confirm::Late => "y\n",
            Confirm::No => "n\n",
            Confirm::Eof => "",
        };
        Answer::after(text, move || {
            b.wait_for("fingerprint ");
            if let Some((shown, window)) = late {
                let since = shown.lock().unwrap().expect("the code was shown");
                while since.elapsed() < window + Duration::from_millis(100) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        })
    }
}

impl Read for Answer {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = {
            let have = self.fill_buf()?;
            let n = have.len().min(buf.len());
            buf[..n].copy_from_slice(&have[..n]);
            n
        };
        self.consume(n);
        Ok(n)
    }
}

impl BufRead for Answer {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if let Some(gate) = self.gate.take() {
            gate();
        }
        Ok(&self.text[self.pos..])
    }

    fn consume(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.text.len());
    }
}

type Ended = Result<(), (u8, String)>;

/// What a side did.
struct Run {
    result: Ended,
    out: Vec<String>,
    err: Vec<String>,
}

impl Run {
    fn ok(&self) {
        assert!(self.result.is_ok(), "{:?} {:?}", self.result, self.err);
    }

    /// The message of an exit with `code`.
    fn ended(&self, code: u8) -> &str {
        match &self.result {
            Err((c, message)) if *c == code => message,
            other => panic!("not an exit {code}: {other:?} {:?}", self.err),
        }
    }

    fn has_err(&self, line: &str) -> bool {
        self.err.iter().any(|l| l == line)
    }
}

/// One side, running `pair::run` on a thread of its own.
struct Side {
    out: Arc<Log>,
    err: Arc<Log>,
    rx: Option<mpsc::Receiver<Ended>>,
    ended: Option<Ended>,
}

impl Side {
    fn new() -> Side {
        Side {
            out: Arc::default(),
            err: Arc::default(),
            rx: None,
            ended: None,
        }
    }

    fn start(
        &mut self,
        machine: &Machine,
        args: &[String],
        terminal: bool,
        limits: Limits,
        mut answer: Answer,
    ) {
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let (out, err) = (self.out.clone(), self.err.clone());
        let (base, args) = (machine.base.clone(), args.to_vec());
        std::thread::spawn(move || {
            let env = env_at(&base);
            let mut put_out = |line: &str| out.push(line);
            let mut put_err = |line: &str| err.push(line);
            let result = run(
                &args,
                &env,
                terminal,
                &mut answer,
                &limits,
                &mut put_out,
                &mut put_err,
            );
            let _ = tx.send(result.map_err(|failure| match failure {
                Failure::Usage(message) | Failure::Config(message) => (2, message),
                Failure::Refused(message) | Failure::Unmatched { message, .. } => (1, message),
            }));
        });
    }

    fn poll(&mut self) -> bool {
        if self.ended.is_none()
            && let Some(Ok(ended)) = self.rx.as_ref().map(|rx| rx.try_recv())
        {
            self.ended = Some(ended);
        }
        self.ended.is_some()
    }

    /// The code A shows, waiting for it.
    fn code(&mut self) -> String {
        let limit = Instant::now() + DEADLINE;
        loop {
            if let Some(line) = self
                .err
                .lines()
                .iter()
                .find_map(|l| l.strip_prefix("pairing code ").map(String::from))
            {
                return line;
            }
            assert!(
                !self.poll() && Instant::now() < limit,
                "no code was shown: {:?} {:?}",
                self.ended,
                self.err.lines()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Waits for the side to end, up to the deadline.
    fn finish(mut self) -> Run {
        let limit = Instant::now() + DEADLINE;
        while !self.poll() {
            assert!(
                Instant::now() < limit,
                "a side did not end: {:?}",
                self.err.lines()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        Run {
            result: self.ended.take().unwrap(),
            out: self.out.lines(),
            err: self.err.lines(),
        }
    }
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

/// The words of a code as the user would type them in shorthand: upper case, each word cut to four letters.
fn loose(code: &str) -> String {
    let mut parts = code.split('-');
    let nameplate = parts.next().unwrap();
    let words: Vec<String> = parts
        .map(|w| w.chars().take(4).collect::<String>().to_uppercase())
        .collect();
    format!("{nameplate} {}", words.join(" "))
}

/// `code` with its last word changed to another word of the list.
fn wrong(code: &str) -> String {
    let (head, last) = code.rsplit_once('-').unwrap();
    let other = if last == "abandon" {
        "ability"
    } else {
        "abandon"
    };
    format!("{head}-{other}")
}

fn nameplate(code: &str) -> &str {
    code.split('-').next().unwrap()
}

/// How a pairing is set up: A's arguments and limits, what the user types, what B types, and the copier's rules.
struct Plan {
    a_args: Vec<String>,
    a: Limits,
    b: Limits,
    confirm: Confirm,
    rules: Rules,
    /// B's `--name`; none for a device that holds keys.
    name: Option<&'static str>,
    /// What B types for the code A shows.
    typed: Box<dyn Fn(&str) -> String>,
}

impl Plan {
    fn new() -> Plan {
        Plan {
            a_args: Vec::new(),
            a: limits(),
            b: limits(),
            confirm: Confirm::Yes,
            rules: Rules::prompt(),
            name: Some("mirkwood"),
            typed: Box::new(|code| code.to_string()),
        }
    }
}

/// A pairing that ran to its end, with the copier still running.
struct Done {
    code: String,
    a: Run,
    b: Run,
    copier: Copier,
}

/// Runs A and B, each with its own machine of `w`, to their ends.
fn exchange(w: &World, plan: Plan) -> Done {
    let (a, b) = (w.machine("a"), w.machine("b"));
    let copier = Copier::start(a.sync(), b.sync(), plan.rules);
    let mut a_side = Side::new();
    let mut b_side = Side::new();
    let shown = Shown::default();
    let late = (plan.confirm == Confirm::Late).then(|| (shown.clone(), plan.a.window));
    let answer = Answer::typed(plan.confirm, b_side.err.clone(), late);
    a_side.start(&a, &plan.a_args, true, plan.a, answer);
    let code = a_side.code();
    *shown.lock().unwrap() = Some(Instant::now());
    let mut args = vec![(plan.typed)(&code), "--via".into(), b.url()];
    if let Some(name) = plan.name {
        args.extend(["--name".into(), name.into()]);
    }
    b_side.start(&b, &args, false, plan.b, Answer::none());
    Done {
        code,
        a: a_side.finish(),
        b: b_side.finish(),
        copier,
    }
}

impl Done {
    fn nameplate(&self) -> &str {
        nameplate(&self.code)
    }

    fn fingerprint(run: &Run) -> String {
        let line = run
            .err
            .iter()
            .find(|l| l.starts_with("fingerprint "))
            .unwrap();
        let words = line.strip_prefix("fingerprint ").unwrap();
        words
            .split(';')
            .next()
            .unwrap()
            .split(" for ")
            .next()
            .unwrap()
            .to_string()
    }
}

fn paired_ok(done: &Done) {
    done.b.ok();
    done.a.ok();
}

fn b_id(b: &Machine) -> String {
    b.identity().device.id()
}

fn owner_of(m: &Machine) -> String {
    keys::owner_fingerprint(&m.identity().owner.sign.public())
}

#[test]
fn joining() {
    let w = world("joining");
    let (a, b) = (w.a(), w.b());
    let done = exchange(&w, Plan::new());
    paired_ok(&done);
    let id = b_id(&b);
    assert_eq!(done.a.out, [format!("paired mirkwood {id}: personal")]);
    assert_eq!(
        done.b.out,
        [
            "paired with rhosgobel: personal",
            "bilbo watch starts syncing them within one cycle"
        ]
    );
    assert_eq!(b.identity().device.name, "mirkwood");
    assert_eq!(owner_of(&b), owner_of(&a));
    assert!(done.a.has_err(&format!("pairing code {}", done.code)));
    assert!(done.a.has_err(&format!(
        "on the new device, run: bilbo pair {} --via {}",
        done.code,
        a.url()
    )));
    assert!(done.a.has_err("the code works once, for 10 minutes"));
}

#[test]
fn a_loosely_typed_code() {
    let w = world("loose");
    w.a();
    let b = w.b();
    let mut plan = Plan::new();
    plan.typed = Box::new(loose);
    let done = exchange(&w, plan);
    paired_ok(&done);
    assert!(b.keys().join("device.key").exists());
}

#[test]
fn matching_fingerprints() {
    let w = world("fingerprints");
    w.a();
    let b = w.b();
    let done = exchange(&w, Plan::new());
    paired_ok(&done);
    let fp = Done::fingerprint(&done.a);
    assert_eq!(fp, Done::fingerprint(&done.b));
    let digits: Vec<&str> = fp.split(' ').collect();
    assert!(
        digits.len() == 3
            && digits
                .iter()
                .all(|g| g.len() == 4 && g.bytes().all(|d| d.is_ascii_digit()))
    );
    let id = b_id(&b);
    assert!(done.a.has_err(&format!(
        "pair mirkwood {id} into personal? compare the fingerprint on that device, then type y to confirm"
    )));
    assert!(done.b.has_err(&format!(
        "fingerprint {fp} for mirkwood {id}; confirm on the device that showed the code"
    )));
}

/// A refusal after the answer: both exit 1, A's versions unchanged, B holds no keys.
fn nothing_changed(w: &World, done: &Done, a_says: &str, b_says: &str) {
    assert_eq!(done.a.ended(1), a_says);
    assert_eq!(done.b.ended(1), b_says);
    assert!(done.a.out.is_empty() && done.b.out.is_empty());
    assert_eq!(w.machine("a").scope(&personal()).versions.len(), 2);
    assert!(!w.machine("b").keys().exists());
}

#[test]
fn declined() {
    let w = world("declined");
    w.a();
    w.b();
    let mut plan = Plan::new();
    plan.confirm = Confirm::No;
    let done = exchange(&w, plan);
    nothing_changed(
        &w,
        &done,
        "not confirmed; nothing was sent",
        "the other device declined; nothing was received",
    );
}

#[test]
fn end_of_input() {
    let w = world("eof");
    w.a();
    w.b();
    let mut plan = Plan::new();
    plan.confirm = Confirm::Eof;
    let done = exchange(&w, plan);
    nothing_changed(
        &w,
        &done,
        "not confirmed; nothing was sent",
        "the other device declined; nothing was received",
    );
}

#[test]
fn a_wrong_word() {
    let w = world("wrong");
    w.a();
    w.b();
    let mut plan = Plan::new();
    plan.typed = Box::new(wrong);
    let done = exchange(&w, plan);
    nothing_changed(
        &w,
        &done,
        "the other device used a wrong code; this code is used up, run bilbo pair again",
        "wrong code; run bilbo pair again on the other device for a new one",
    );
}

#[test]
fn retrying_the_right_code_after_a_wrong_one() {
    let w = world("retry");
    w.a();
    let b = w.b();
    let mut plan = Plan::new();
    plan.typed = Box::new(wrong);
    let done = exchange(&w, plan);
    assert_eq!(done.b.ended(1).split(';').next(), Some("wrong code"));
    let mut again = Side::new();
    let args = [
        done.code.clone(),
        "--via".into(),
        b.url(),
        "--name".into(),
        "mirkwood".into(),
    ];
    again.start(&b, &args, false, limits(), Answer::none());
    let run = again.finish();
    assert_eq!(
        run.ended(1),
        format!(
            "code {} was already used; run bilbo pair again on the other device",
            done.nameplate()
        )
    );
    assert!(!b.keys().exists());
}

#[test]
fn a_second_answer_while_the_first_is_pending() {
    let w = world("second");
    let (a, b) = (w.a(), w.b());
    let other = w.machine("b2");
    let copier = Copier::start(a.sync(), b.sync(), Rules::prompt());
    let (mut a_side, mut b_side) = (Side::new(), Side::new());
    let second_done = Arc::new(AtomicBool::new(false));
    let answer = {
        let (b_err, flag) = (b_side.err.clone(), second_done.clone());
        Answer::after("y\n", move || {
            b_err.wait_for("fingerprint ");
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(2));
            }
        })
    };
    a_side.start(&a, &[], true, limits(), answer);
    let code = a_side.code();
    let args = strings(&[&code, "--via", &b.url(), "--name", "mirkwood"]);
    b_side.start(&b, &args, false, limits(), Answer::none());
    b_side.err.wait_for("fingerprint ").unwrap();
    let mut second = Side::new();
    let args = strings(&[&code, "--via", &b.url(), "--name", "dunharrow"]);
    second.start(&other, &args, false, limits(), Answer::none());
    let second = second.finish();
    second_done.store(true, Ordering::Relaxed);
    assert_eq!(
        second.ended(1),
        format!(
            "code {} was already used; run bilbo pair again on the other device",
            nameplate(&code)
        )
    );
    let (a_run, b_run) = (a_side.finish(), b_side.finish());
    a_run.ok();
    b_run.ok();
    assert_eq!(a_run.out.len(), 1);
    assert!(a_run.out[0].starts_with(&format!("paired mirkwood {}", b_id(&b))));
    assert!(!other.keys().exists());
    drop(copier);
}

#[test]
fn no_such_mailbox() {
    let w = world("nomailbox");
    let (a, b) = (w.a(), w.b());
    let copier = Copier::start(a.sync(), b.sync(), Rules::prompt());
    let (mut a_side, mut b_side) = (Side::new(), Side::new());
    let answer = Answer::typed(Confirm::Yes, b_side.err.clone(), None);
    a_side.start(&a, &[], true, limits(), answer);
    let code = a_side.code();
    let other = if nameplate(&code) == "43" { "44" } else { "43" };
    let mut lost = Side::new();
    let args = strings(&[
        &format!("{other}-orbit-tunnel-velvet"),
        "--via",
        &b.url(),
        "--name",
        "mirkwood",
    ]);
    lost.start(
        &b,
        &args,
        false,
        Limits {
            appear: Duration::from_millis(300),
            ..limits()
        },
        Answer::none(),
    );
    assert_eq!(
        lost.finish().ended(1),
        format!("no pairing {other} at {}", b.url())
    );
    let args = strings(&[&code, "--via", &b.url(), "--name", "mirkwood"]);
    b_side.start(&b, &args, false, limits(), Answer::none());
    a_side.finish().ok();
    b_side.finish().ok();
    drop(copier);
}

#[test]
fn nobody_answers() {
    let w = world("nobody");
    let a = w.a();
    let mut side = Side::new();
    let a_limits = Limits {
        window: Duration::from_millis(400),
        ..limits()
    };
    side.start(&a, &[], true, a_limits, Answer::none());
    let code = side.code();
    assert!(
        a.sync()
            .join("pair")
            .join(nameplate(&code))
            .join("a.msg")
            .exists()
    );
    assert_eq!(side.finish().ended(1), "the code expired; nothing was sent");
    assert!(a.mailboxes().is_empty());
}

#[test]
fn too_late() {
    let w = world("toolate");
    let (a, b) = (w.a(), w.b());
    let mut side = Side::new();
    let a_limits = Limits {
        window: Duration::from_millis(300),
        ..limits()
    };
    side.start(&a, &[], true, a_limits, Answer::none());
    let code = side.code();
    side.finish().ended(1);
    let mut late = Side::new();
    let args = strings(&[&code, "--via", &a.url(), "--name", "mirkwood"]);
    let b_limits = Limits {
        appear: Duration::from_millis(200),
        ..limits()
    };
    late.start(&b, &args, false, b_limits, Answer::none());
    assert_eq!(
        late.finish().ended(1),
        format!("no pairing {} at {}", nameplate(&code), a.url())
    );
}

#[test]
fn confirmed_too_late() {
    let w = world("confirmlate");
    w.a();
    w.b();
    let mut plan = Plan::new();
    plan.confirm = Confirm::Late;
    plan.a.window = Duration::from_millis(800);
    let done = exchange(&w, plan);
    nothing_changed(
        &w,
        &done,
        "the code expired; nothing was sent",
        "the code expired on the other device; nothing was received",
    );
}

#[test]
fn a_late_manifest() {
    let w = world("latemanifest");
    w.a();
    let b = w.b();
    let mut plan = Plan::new();
    plan.rules = Rules::manifests(Some(Duration::from_millis(800)));
    plan.b.manifests = Duration::from_secs(5);
    let done = exchange(&w, plan);
    paired_ok(&done);
    assert!(b.keys().join("device.key").exists());
    assert_eq!(b.scope(&personal()).versions.len(), 3);
}

#[test]
fn a_manifest_that_never_arrives() {
    let w = world("nevermanifest");
    w.a();
    let b = w.b();
    let mut plan = Plan::new();
    plan.rules = Rules::manifests(None);
    plan.b.manifests = Duration::from_millis(400);
    let done = exchange(&w, plan);
    done.a.ok();
    assert_eq!(
        done.b.ended(1),
        format!(
            "the personal manifest did not reach {} in time; run bilbo pair again",
            b.url()
        )
    );
    assert!(done.b.out.is_empty());
    assert!(!b.keys().exists() && b.config_text().is_none());
    assert!(manifest::scope_ids(&b.root()).unwrap().is_empty());
}

#[test]
fn after_pairing() {
    let w = world("after");
    let (a, b) = (w.a(), w.b());
    let done = exchange(&w, Plan::new());
    paired_ok(&done);
    let held = b.scope(&personal());
    assert_eq!(held.versions.len(), 3);
    assert!(held.pending.is_empty() && held.invalid.is_none());
    let id = b.identity();
    let latest = held.latest().unwrap();
    assert!(latest.manifest.lists(&id.device.id()));
    assert!(
        manifest::open(&held, &Recipient::device(&id.device))
            .unwrap()
            .is_some_and(|opened| opened.name == "personal" && opened.n == 3)
    );
    assert_eq!(owner_of(&b), owner_of(&a));
    let on_a = a.scope(&personal());
    assert_eq!(on_a.versions.len(), 3);
    assert_eq!(on_a.pending, BTreeSet::from([3]));
    assert!(
        a.sync()
            .join(format!("scopes/{}/manifest/3.json", personal()))
            .exists()
    );
}

#[test]
fn a_manifest_that_does_not_verify() {
    let w = world("verify");
    w.a();
    let b = w.b();
    let mut plan = Plan::new();
    plan.rules = Rules::replacing("/manifest/3.json", b"{\"format\":1}");
    let done = exchange(&w, plan);
    done.a.ok();
    assert_eq!(
        done.b.ended(1),
        "the personal manifests on the transport do not match what the other device sent; nothing was written"
    );
    assert!(!b.keys().exists() && b.config_text().is_none());
    assert!(manifest::scope_ids(&b.root()).unwrap().is_empty());
}

#[test]
fn an_earlier_version_that_is_missing() {
    let w = world("missing");
    w.a();
    let b = w.b();
    let mut plan = Plan::new();
    plan.rules = Rules {
        delay: Box::new(|rel| {
            (!rel.ends_with("/manifest/1.json")).then_some(Duration::from_millis(10))
        }),
        ..Rules::prompt()
    };
    plan.b.manifests = Duration::from_millis(600);
    let done = exchange(&w, plan);
    done.a.ok();
    assert_eq!(
        done.b.ended(1),
        "the personal manifests on the transport do not match what the other device sent; nothing was written"
    );
    assert!(!b.keys().exists() && b.config_text().is_none());
}

#[test]
fn pairing_again_after_an_interrupted_pairing() {
    let w = world("interrupted");
    let a = w.a();
    let b = w.b();
    let mut plan = Plan::new();
    plan.rules = Rules::manifests(None);
    plan.b.manifests = Duration::from_millis(300);
    let first = exchange(&w, plan);
    first.a.ok();
    first.b.ended(1);
    let id = keys::pending_device(&b.pending(), "mirkwood").unwrap().id();
    assert_eq!(a.scope(&personal()).versions.len(), 3);
    drop(first);
    let again = exchange(&w, Plan::new());
    paired_ok(&again);
    assert_eq!(a.scope(&personal()).versions.len(), 3);
    assert_eq!(b_id(&b), id);
    assert_eq!(b.scope(&personal()).versions.len(), 3);
    assert!(again.a.has_err(&format!(
        "pair mirkwood {id} into personal? compare the fingerprint on that device, then type y to confirm"
    )));
}

#[test]
fn the_config_cannot_be_written() {
    let w = world("unwritable");
    let a = w.a();
    let b = w.b();
    let blocker = b.config_path().parent().unwrap().to_path_buf();
    fs::write(&blocker, "a file").unwrap();
    let first = exchange(&w, Plan::new());
    first.a.ok();
    assert!(first.b.ended(1).starts_with("cannot write "));
    assert!(first.b.out.is_empty());
    assert!(!b.keys().exists());
    let id = keys::pending_device(&b.pending(), "mirkwood").unwrap().id();
    drop(first);
    fs::remove_file(&blocker).unwrap();
    let again = exchange(&w, Plan::new());
    paired_ok(&again);
    assert_eq!(b_id(&b), id);
    assert_eq!(a.scope(&personal()).versions.len(), 3);
}

#[test]
fn the_config_after_pairing() {
    let w = world("config");
    let a = w.a();
    let b = w.b();
    a.config(&format!(
        "scope.personal.sync = {}\nscope.personal.embedder = local\n",
        a.url()
    ));
    b.config("embedder.url = http://127.0.0.1:8081\nembedder.model = m\n");
    let done = exchange(&w, Plan::new());
    paired_ok(&done);
    let text = b.config_text().unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    assert!(lines.contains(&"embedder.url = http://127.0.0.1:8081"));
    assert!(lines.contains(&"embedder.model = m"));
    assert!(lines.contains(&format!("scope.personal.sync = {}", b.url()).as_str()));
    assert!(lines.contains(&"scope.personal.embedder = local"));
    assert!(!text.contains("scope.personal.paths"));
    assert!(b.config_path().with_file_name("config.bak").exists());
}

#[test]
fn bs_stricter_embedder_stays() {
    let w = world("stricter");
    w.a();
    let b = w.b();
    b.config("scope.personal.embedder = local\n");
    let done = exchange(&w, Plan::new());
    paired_ok(&done);
    let text = b.config_text().unwrap();
    assert_eq!(text.matches("scope.personal.embedder = local").count(), 1);
    assert!(text.contains(&format!("scope.personal.sync = {}", b.url())));
}

#[test]
fn notes_that_already_carry_the_scope() {
    let w = world("notes");
    w.a();
    let b = w.b();
    b.config("scope.personal.sync = off\n");
    let head = "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4D01\ncreated: 2026-10-02T14:23-03:00\n";
    for (file, scope) in [
        ("plan-a.md", "scope: personal\n"),
        ("plan-b.md", "scope: personal\n"),
        ("plan-c.md", "scope: personal\n"),
        ("plan-d.md", "scope: other\n"),
    ] {
        let text = format!("{head}{scope}---\n\n# A\n\nbody\n");
        fs::write(b.root().join("notes").join(file), text).unwrap();
    }
    let done = exchange(&w, Plan::new());
    paired_ok(&done);
    assert!(
        done.b
            .has_err("3 notes already carry scope: personal and sync from now on")
    );
    assert_eq!(done.b.out.len(), 2);
    let text = b.config_text().unwrap();
    assert!(text.contains(&format!("scope.personal.sync = {}", b.url())));
    assert!(!text.contains("= off"));
}

/// Needles that mark a leak of what `a` keeps secret: the owner seed and the epoch key as bytes, hex and base64.
fn secrets(a: &Machine, urls: &[String]) -> Vec<Vec<u8>> {
    let id = a.identity();
    let held = a.scope(&personal());
    let opened = manifest::open(&held, &Recipient::device(&id.device))
        .unwrap()
        .unwrap();
    let mut needles = vec![b"personal".to_vec()];
    for raw in [*id.owner.sign.seed(), *opened.keys[&opened.epoch]] {
        needles.push(raw.to_vec());
        needles.push(keys::hex(&raw).into_bytes());
        needles.push(base64(&raw).into_bytes());
    }
    for url in urls {
        needles.push(url.clone().into_bytes());
        needles.push(url.trim_start_matches("file://").as_bytes().to_vec());
    }
    needles
}

/// `bytes` in standard padded base64, as the messages carry a box.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    bytes
        .chunks(3)
        .flat_map(|chunk| {
            let n =
                chunk.iter().fold(0u32, |n, b| n << 8 | u32::from(*b)) << (8 * (3 - chunk.len()));
            (0..4).map(move |i| {
                if i <= chunk.len() {
                    ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char
                } else {
                    '='
                }
            })
        })
        .collect()
}

fn leaks(files: &Seen, needles: &[Vec<u8>]) -> Vec<String> {
    let mut found = Vec::new();
    for (rel, versions) in files {
        for bytes in versions {
            for (i, needle) in needles.iter().enumerate() {
                if bytes.windows(needle.len()).any(|w| w == needle.as_slice()) {
                    found.push(format!("{rel} holds needle {i}"));
                }
            }
        }
    }
    found
}

#[test]
fn the_mailbox_is_opaque() {
    let w = world("opaque");
    let (a, b) = (w.a(), w.b());
    let done = exchange(&w, Plan::new());
    paired_ok(&done);
    let seen = done.copier.seen();
    let mut names: Vec<&str> = seen.keys().map(|k| k.rsplit('/').next().unwrap()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names, ["a.msg", "b.msg", "c.msg"]);
    let needles = secrets(&a, &[a.url(), b.url()]);
    assert_eq!(leaks(&seen, &needles), Vec::<String>::new());
    let planted = Seen::from([(
        "pair/1/c.msg".to_string(),
        vec![
            format!(
                "{{\"box\":\"{}\"}}",
                keys::hex(&*a.identity().owner.sign.seed())
            )
            .into_bytes(),
        ],
    )]);
    assert_eq!(leaks(&planted, &needles).len(), 1);
}

#[test]
fn no_phrase_anywhere() {
    let w = world("phrase");
    let (a, b) = (w.a(), w.b());
    let done = exchange(&w, Plan::new());
    paired_ok(&done);
    let words: Vec<&str> = phrase::encode(&[0; 16])
        .iter()
        .map(|&i| phrase::word(i))
        .collect();
    let needles: Vec<Vec<u8>> = words
        .windows(3)
        .map(|run| run.join(" ").into_bytes())
        .collect();
    let mut held = Seen::new();
    for machine in [&a, &b] {
        for (rel, bytes) in machine.tree() {
            held.entry(rel).or_default().push(bytes);
        }
    }
    for run in [&done.a, &done.b] {
        held.entry("output".into()).or_default().push(
            run.out
                .iter()
                .chain(&run.err)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
                .to_lowercase()
                .into_bytes(),
        );
    }
    for (rel, versions) in done.copier.seen() {
        held.entry(rel).or_default().extend(versions);
    }
    assert!(held.len() > 10);
    assert_eq!(leaks(&held, &needles), Vec::<String>::new());
    let planted = Seen::from([("output".to_string(), vec![words.join(" ").into_bytes()])]);
    assert!(!leaks(&planted, &needles).is_empty());
}

#[test]
fn narrowing() {
    let w = world("narrowing");
    let a = w.a();
    let shared = a.add_scope("shared");
    let b = w.b();
    let mut plan = Plan::new();
    plan.a_args = strings(&["--scope", "shared"]);
    let done = exchange(&w, plan);
    paired_ok(&done);
    let id = b_id(&b);
    assert!(done.a.has_err(&format!(
        "pair mirkwood {id} into shared? compare the fingerprint on that device, then type y to confirm"
    )));
    assert_eq!(done.a.out, [format!("paired mirkwood {id}: shared")]);
    let text = b.config_text().unwrap();
    assert!(text.contains("scope.shared.sync") && !text.contains("scope.personal"));
    assert_eq!(a.scope(&personal()).versions.len(), 2);
    assert!(a.scope(&shared).latest().unwrap().manifest.lists(&id));
    assert!(manifest::scope_ids(&b.root()).unwrap() == [shared]);
}

/// `name` answers as a new device, and A and B refuse with `a_says` and `b_says`; no manifest changes.
fn refused_name(w: &World, plan: Plan, a_says: &str, b_says: &str) {
    let done = exchange(w, plan);
    assert_eq!(done.a.ended(1), a_says);
    assert_eq!(done.b.ended(1), b_says);
    assert!(done.a.out.is_empty() && done.b.out.is_empty());
    let (a, b) = (w.machine("a"), w.machine("b"));
    assert_eq!(a.scope(&personal()).versions.len(), 2);
    assert!(!b.keys().exists() && b.config_text().is_none());
}

#[test]
fn a_taken_name() {
    let w = world("taken");
    w.a();
    w.b();
    let mut plan = Plan::new();
    plan.name = Some("bywater");
    refused_name(
        &w,
        plan,
        "a device named bywater is already enrolled; pair again with --name on the new device",
        "the name bywater is taken; run bilbo pair again with --name",
    );
}

#[test]
fn a_name_taken_in_a_scope_not_being_paired() {
    let w = world("takenelsewhere");
    let a = w.a();
    let shared = a.add_scope("shared");
    w.b();
    let mut plan = Plan::new();
    plan.a_args = strings(&["--scope", "shared"]);
    plan.name = Some("bywater");
    refused_name(
        &w,
        plan,
        "a device named bywater is already enrolled; pair again with --name on the new device",
        "the name bywater is taken; run bilbo pair again with --name",
    );
    assert_eq!(a.scope(&shared).versions.len(), 1);
}

#[test]
fn the_showing_devices_own_name() {
    let w = world("ownname");
    w.a();
    w.b();
    let mut plan = Plan::new();
    plan.name = Some("rhosgobel");
    refused_name(
        &w,
        plan,
        "a device named rhosgobel is already enrolled; pair again with --name on the new device",
        "the name rhosgobel is taken; run bilbo pair again with --name",
    );
}

#[test]
fn a_store_of_another_owner() {
    let w = world("foreignstore");
    let a = w.a();
    let b = w.b();
    b.put_store("foreign");
    let done = exchange(&w, Plan::new());
    done.a.ok();
    let theirs = keys::owner_fingerprint(&Owner::derive(&[1; 16]).sign.public());
    let ours = owner_of(&a);
    let said = done.b.ended(1);
    assert!(said.contains(&theirs) && said.contains(&ours), "{said}");
    assert!(said.ends_with("nothing was written"));
    assert!(!b.keys().exists() && b.config_text().is_none());
    assert_eq!(manifest::scope_ids(&b.root()).unwrap().len(), 1);
}

#[test]
fn a_newer_device_shows_the_code() {
    let w = world("newershow");
    w.a();
    let b = w.b();
    let mut plan = Plan::new();
    plan.rules = Rules::replacing("/a.msg", b"{\"format\":2}");
    plan.a.window = Duration::from_millis(600);
    let done = exchange(&w, plan);
    assert_eq!(
        done.b.ended(1),
        "the other device runs a newer bilbo; update this one and pair again"
    );
    assert!(
        !b.sync()
            .join("pair")
            .join(done.nameplate())
            .join("b.msg")
            .exists()
    );
    assert_eq!(done.a.ended(1), "the code expired; nothing was sent");
}

#[test]
fn a_newer_device_answers() {
    let w = world("neweranswer");
    let a = w.a();
    w.b();
    let mut plan = Plan::new();
    plan.rules = Rules::replacing("/b.msg", b"{\"format\":2}");
    plan.b.window = Duration::from_millis(600);
    let done = exchange(&w, plan);
    assert_eq!(
        done.a.ended(1),
        "the other device runs a newer bilbo; update this one and pair again"
    );
    assert_eq!(done.b.ended(1), "no answer from the other device");
    assert!(
        !a.sync()
            .join("pair")
            .join(done.nameplate())
            .join("c.msg")
            .exists()
    );
}

/// B is the enrolled `bywater`, with the fixture store and `personal` in its config.
fn bywater(w: &World, store: bool) -> Machine {
    let b = w.b();
    b.put_keys("bywater");
    if store {
        b.put_store("store");
    }
    b.config(&format!("scope.personal.sync = {}\n", b.url()));
    b
}

#[test]
fn an_enrolled_device_joins_another_scope() {
    let w = world("enrolledother");
    let a = w.a();
    let shared = a.add_scope("shared");
    let b = bywater(&w, true);
    let before = b.key_files();
    let mut plan = Plan::new();
    plan.a_args = strings(&["--scope", "shared"]);
    plan.name = None;
    let done = exchange(&w, plan);
    paired_ok(&done);
    let id = b_id(&b);
    assert_eq!(done.a.out, [format!("paired bywater {id}: shared")]);
    assert_eq!(done.b.out[0], "paired with rhosgobel: shared");
    assert_eq!(b.key_files(), before);
    assert!(a.scope(&shared).latest().unwrap().manifest.lists(&id));
    assert_eq!(a.scope(&personal()).versions.len(), 2);
    let text = b.config_text().unwrap();
    assert!(text.contains(&format!("scope.personal.sync = {}", b.url())));
    assert!(text.contains(&format!("scope.shared.sync = {}", b.url())));
    let seed = *a.identity().owner.sign.seed();
    let seen = done.copier.seen();
    let needles = vec![
        seed.to_vec(),
        keys::hex(&seed).into_bytes(),
        base64(&seed).into_bytes(),
    ];
    assert_eq!(leaks(&seen, &needles), Vec::<String>::new());
    assert_eq!(b.scope(&shared).versions.len(), 2);
}

#[test]
fn enrolled_with_another_owner() {
    let w = world("otherowner");
    let a = w.a();
    let b = w.b();
    let theirs = Owner::derive(&[1; 16]);
    let device = Device::from_seeds("grey", &[5; 32], &[6; 32]);
    keys::write_identity(&b.keys(), &theirs.file(), &device).unwrap();
    let before = b.key_files();
    let mut plan = Plan::new();
    plan.name = None;
    let done = exchange(&w, plan);
    let (ours, other) = (owner_of(&a), keys::owner_fingerprint(&theirs.sign.public()));
    assert_eq!(
        done.a.ended(1),
        format!("grey belongs to another owner ({other})")
    );
    assert_eq!(
        done.b.ended(1),
        format!("this device belongs to owner {other}, the other device to {ours}")
    );
    assert!(done.a.out.is_empty() && done.b.out.is_empty());
    assert_eq!(a.scope(&personal()).versions.len(), 2);
    assert_eq!(b.key_files(), before);
}

#[test]
fn the_folder_has_another_path() {
    let w = world("otherpath");
    let (a, b) = (w.a(), w.b());
    assert_ne!(a.url(), b.url());
    let done = exchange(&w, Plan::new());
    paired_ok(&done);
    assert_eq!(
        b.config_text().unwrap(),
        format!("scope.personal.sync = {}\n", b.url())
    );
    let held = a.scope(&personal());
    assert_eq!(held.versions.len(), 3);
    assert!(
        held.versions
            .iter()
            .all(|v| v.manifest.transport == "file://")
    );
    assert_eq!(b.scope(&personal()).versions.len(), 3);
}

#[test]
fn paired_into_the_scope_afterwards() {
    let w = world("afterwards");
    let a = w.a();
    let b = bywater(&w, false);
    let before = b.key_files();
    let mut plan = Plan::new();
    plan.name = None;
    let done = exchange(&w, plan);
    paired_ok(&done);
    let id = b_id(&b);
    assert_eq!(b.key_files(), before);
    let held = b.scope(&personal());
    assert!(held.latest().unwrap().manifest.lists(&id));
    assert_eq!(held.versions.len(), a.scope(&personal()).versions.len());
    assert_eq!(
        b.config_text().unwrap(),
        format!("scope.personal.sync = {}\n", b.url())
    );
}

#[test]
fn cleaned_up_after_success() {
    let w = world("cleanup");
    let (a, b) = (w.a(), w.b());
    let done = exchange(&w, Plan::new());
    paired_ok(&done);
    until("both mailboxes to go", || {
        a.mailboxes().is_empty() && b.mailboxes().is_empty()
    });
}

#[test]
fn kept_after_a_wrong_code() {
    let w = world("keptwrong");
    let (a, b) = (w.a(), w.b());
    let mut plan = Plan::new();
    plan.typed = Box::new(wrong);
    let done = exchange(&w, plan);
    done.b.ended(1);
    let np = done.nameplate().to_string();
    until("the mailbox to reach both folders", || {
        a.mailboxes() == [np.clone()] && b.mailboxes() == [np.clone()]
    });
    drop(done);
    let old = b.sync().join("pair").join(&np).join("a.msg");
    fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(31 * 60))
        .unwrap();
    let mut later = Side::new();
    let other = if np == "7" { "8" } else { "7" };
    let args = strings(&[
        &format!("{other}-orbit-tunnel-velvet"),
        "--via",
        &b.url(),
        "--name",
        "mirkwood",
    ]);
    let b_limits = Limits {
        appear: Duration::from_millis(100),
        ..limits()
    };
    later.start(&b, &args, false, b_limits, Answer::none());
    later.finish().ended(1);
    assert!(b.mailboxes().is_empty());
}

#[test]
fn a_stale_mailbox() {
    let w = world("stale");
    let a = w.a();
    let b = w.b();
    b.plant("7", Duration::from_secs(31 * 60));
    let mut lost = Side::new();
    let args = strings(&[
        "43-orbit-tunnel-velvet",
        "--via",
        &b.url(),
        "--name",
        "mirkwood",
    ]);
    let b_limits = Limits {
        appear: Duration::from_millis(100),
        ..limits()
    };
    lost.start(&b, &args, false, b_limits, Answer::none());
    lost.finish().ended(1);
    assert!(b.mailboxes().is_empty());
    a.plant("7", Duration::from_secs(31 * 60));
    let mut side = Side::new();
    let a_limits = Limits {
        window: Duration::from_millis(200),
        ..limits()
    };
    side.start(&a, &[], true, a_limits, Answer::none());
    let code = side.code();
    side.finish().ended(1);
    assert!(!a.mailboxes().contains(&"7".to_string()), "{code}");
}

#[test]
fn a_fresh_mailbox_stays() {
    let w = world("fresh");
    let b = w.b();
    b.plant("8", Duration::from_secs(5 * 60));
    let mut lost = Side::new();
    let args = strings(&[
        "43-orbit-tunnel-velvet",
        "--via",
        &b.url(),
        "--name",
        "mirkwood",
    ]);
    let b_limits = Limits {
        appear: Duration::from_millis(100),
        ..limits()
    };
    lost.start(&b, &args, false, b_limits, Answer::none());
    lost.finish().ended(1);
    assert_eq!(b.mailboxes(), ["8"]);
}

#[test]
fn the_base64_needle_matches_the_standard_alphabet() {
    assert_eq!(base64(b"Man"), "TWFu");
    assert_eq!(base64(b"Ma"), "TWE=");
    assert_eq!(base64(b"M"), "TQ==");
    assert_eq!(base64(&[0xfb, 0xff, 0xbf]), "+/+/");
}

/// The exchange through relays.
#[cfg(test)]
mod tests {
    use super::*;

    /// A relay on loopback admitting `owner`, its data folder, and what it logged.
    struct Site {
        relay: crate::relay::Running,
        data: PathBuf,
        lines: Arc<Mutex<Vec<String>>>,
    }

    impl Site {
        fn start(w: &World, name: &str, owner: &str, seed: &[(&str, &[Vec<u8>])]) -> Site {
            let data = w.0.join(name);
            fs::create_dir_all(&data).unwrap();
            let held = transport::Folder::new(data.clone(), "relay");
            for (scope, versions) in seed {
                for (i, bytes) in versions.iter().enumerate() {
                    let path = transport::manifest_path(scope, i as u64 + 1);
                    assert!(matches!(held.create(&path, bytes), Put::Created));
                }
            }
            let lines = Arc::new(Mutex::new(Vec::<String>::new()));
            let sink = lines.clone();
            let relay = crate::relay::start(
                crate::relay::Flags {
                    data: data.clone(),
                    owners: vec![owner.to_string()],
                    listen: "127.0.0.1:0".parse().unwrap(),
                    max_scopes: 16,
                    max_scope_mb: 1024,
                    max_object_mb: 16,
                },
                crate::relay::system_clock(),
                Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string())),
            )
            .unwrap();
            Site { relay, data, lines }
        }

        fn url(&self) -> String {
            self.relay.url()
        }

        fn folder(&self) -> transport::Folder {
            transport::Folder::new(self.data.clone(), "reader")
        }

        fn logged(&self) -> Vec<String> {
            self.lines.lock().unwrap().clone()
        }

        fn mailbox_lines(&self) -> usize {
            self.logged()
                .iter()
                .filter(|l| l.starts_with("mailbox "))
                .count()
        }
    }

    fn versions_of(m: &Machine, id: &str) -> Vec<Vec<u8>> {
        m.scope(id)
            .versions
            .iter()
            .map(|v| v.bytes.clone())
            .collect()
    }

    fn owner_print(m: &Machine) -> String {
        keys::owner_fingerprint(&m.identity().owner.sign.public())
    }

    /// A shows a code, B answers it with `via`, A confirms; both ran to their ends.
    fn run_pair(a: &Machine, b: &Machine, a_via: &[&str], via: &str) -> (String, Run, Run) {
        let mut a_side = Side::new();
        let mut b_side = Side::new();
        let answer = Answer::typed(Confirm::Yes, b_side.err.clone(), None);
        a_side.start(a, &strings(a_via), true, limits(), answer);
        let code = a_side.code();
        let args = strings(&[&code, "--via", via, "--name", "mirkwood"]);
        b_side.start(b, &args, false, limits(), Answer::none());
        let (a_run, b_run) = (a_side.finish(), b_side.finish());
        a_run.ok();
        b_run.ok();
        (code, a_run, b_run)
    }

    #[test]
    fn pair_through_relay() {
        let w = world("relay");
        let (a, b) = (w.a(), w.b());
        let seed = versions_of(&a, &personal());
        let site = Site::start(&w, "relay", &owner_print(&a), &[(&personal(), &seed)]);
        let url = site.url();
        a.config(&format!("scope.personal.sync = {url}\n"));
        let (code, a_run, b_run) = run_pair(&a, &b, &[], &url);
        assert!(a_run.has_err(&format!(
            "on the new device, run: bilbo pair {code} --via {url}"
        )));
        assert_eq!(
            b_run.out,
            [
                "paired with rhosgobel: personal",
                "bilbo watch starts syncing them within one cycle"
            ]
        );
        assert_eq!(b.identity().device.name, "mirkwood");
        assert_eq!(owner_of(&b), owner_of(&a));
        let config = b.config_text().unwrap();
        assert!(config.contains(&format!("scope.personal.sync = {url}\n")));
        assert_eq!(b.scope(&personal()).versions.len(), 3);
        assert!(
            site.folder()
                .get(&transport::manifest_path(&personal(), 3))
                .unwrap()
                .is_some()
        );
        let logged = site.logged();
        let plate = nameplate(&code);
        assert!(
            logged.iter().all(|l| !l.contains(&format!("pair/{plate}"))),
            "{logged:?}"
        );
        assert!(
            logged.iter().all(|l| !l.starts_with("refused")),
            "{logged:?}"
        );
        assert_eq!(site.mailbox_lines(), 3, "{logged:?}");
    }

    #[test]
    fn a_via_with_a_trailing_slash_is_written_without_it() {
        let w = world("slash");
        let (a, b) = (w.a(), w.b());
        let seed = versions_of(&a, &personal());
        let site = Site::start(&w, "relay", &owner_print(&a), &[(&personal(), &seed)]);
        let url = site.url();
        a.config(&format!("scope.personal.sync = {url}\n"));
        run_pair(&a, &b, &[], &format!("{url}/"));
        let config = b.config_text().unwrap();
        assert!(
            config.contains(&format!("scope.personal.sync = {url}\n")),
            "{config}"
        );
    }

    #[test]
    fn pair_through_two_relays() {
        let w = world("two-relays");
        let (a, b) = (w.a(), w.b());
        let owner = owner_print(&a);
        let seed = versions_of(&a, &personal());
        let first = Site::start(&w, "relay-1", &owner, &[(&personal(), &seed)]);
        let second = Site::start(&w, "relay-2", &owner, &[]);
        let (one, two) = (first.url(), second.url());
        let identity = a.identity();
        let work = {
            let lock = manifest::lock(&a.root()).unwrap();
            manifest::create(&lock, &identity, "work", &two, &[])
                .unwrap()
                .scope
        };
        let on_two = transport::open(&two, &transport::Keys::of(&identity)).unwrap();
        for version in &a.scope(&work).versions {
            let path = transport::manifest_path(&work, version.manifest.n);
            let put = on_two.create(&path, &version.bytes);
            assert!(matches!(put, Put::Created), "{put:?}");
        }
        a.config(&format!(
            "scope.personal.sync = {one}\nscope.work.sync = {two}\n"
        ));
        run_pair(&a, &b, &["--via", &one], &one);
        let config = b.config_text().unwrap();
        assert!(
            config.contains(&format!("scope.personal.sync = {one}\n")),
            "{config}"
        );
        assert!(
            config.contains(&format!("scope.work.sync = {two}\n")),
            "{config}"
        );
        assert_eq!(b.scope(&personal()).versions.len(), 3);
        assert_eq!(b.scope(&work).versions.len(), 2);
        assert!(
            second
                .folder()
                .get(&transport::manifest_path(&work, 2))
                .unwrap()
                .is_some()
        );
        assert_eq!(first.mailbox_lines(), 3, "{:?}", first.logged());
        assert_eq!(second.mailbox_lines(), 0, "{:?}", second.logged());
    }
}
