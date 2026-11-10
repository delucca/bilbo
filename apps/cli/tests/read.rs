//! The ignored test reads the store the environment names.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bilbo_cli::console::env::Env;
use bilbo_cli::console::outcome::Failure;
use bilbo_cli::console::settings::{self, Settings};
use bilbo_cli::console::view::View;
use bilbo_kinds::reader::{Passage, Reader};
use bilbo_store::entries::{Change, Cursor, Entries, RecordRange};
use bilbo_store::id::Id;
use bilbo_store::segments::{Options, Store};

const CLAUDE: &str = "8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b";
const OLDER: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";
const CODEX: &str = "019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b";
const SESSIONS: [(&str, &[u8]); 6] = [
    (
        "claude/p/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl",
        include_bytes!(
            "../../../libs/rust/kinds/session/fixtures/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl"
        ),
    ),
    (
        "claude/p/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl",
        include_bytes!(
            "../../../libs/rust/kinds/session/fixtures/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl"
        ),
    ),
    (
        "claude/p/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03/subagents/agent-ad63c2b3c9e7a4b3d.jsonl",
        include_bytes!("../../../libs/rust/kinds/session/fixtures/agent-ad63c2b3c9e7a4b3d.jsonl"),
    ),
    (
        "codex/2026/11/07/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl",
        include_bytes!(
            "../../../libs/rust/kinds/session/fixtures/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl"
        ),
    ),
    (
        "codex/2026/11/06/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl",
        include_bytes!(
            "../../../libs/rust/kinds/session/fixtures/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl"
        ),
    ),
    (
        "cursor/0e1d2c3b-4a59-4687-b6a5-a4b3c2d1e0f9.jsonl",
        include_bytes!("../../../libs/rust/kinds/session/fixtures/cursor-session.jsonl"),
    ),
];
/// One frontmatter id each: the other Markdown fixtures share `crlf.md`'s.
const NOTES: [(&str, &[u8]); 4] = [
    (
        "sections.md",
        include_bytes!("../../../libs/rust/kinds/markdown/fixtures/sections.md"),
    ),
    (
        "crlf.md",
        include_bytes!("../../../libs/rust/kinds/markdown/fixtures/crlf.md"),
    ),
    (
        "bare.md",
        include_bytes!("../../../libs/rust/kinds/markdown/fixtures/no-frontmatter.md"),
    ),
    (
        "open.md",
        b"Top\n===\n\ntext\n\n## Last\n\nno final newline",
    ),
];
const SECTIONS: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DG";

static COUNTER: AtomicU64 = AtomicU64::new(0);

#[expect(
    clippy::disallowed_methods,
    reason = "Bazel names a test's scratch folder, and the done check its store, in the environment"
)]
fn var(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

struct Run {
    code: u8,
    stdout: String,
    stderr: Vec<String>,
}

#[derive(Clone)]
struct Bilbo {
    vars: HashMap<String, OsString>,
    binary: Option<PathBuf>,
}

impl Bilbo {
    fn settings(&self) -> Settings {
        settings::load(
            &|name| self.vars.get(name).cloned(),
            &|path| fs::read(path),
            &bilbo_cli::kind_keys(),
        )
        .expect("settings")
    }

    fn run(&self, args: &[&str]) -> Run {
        if let Some(binary) = &self.binary {
            let output = Command::new(binary)
                .env_clear()
                .envs(&self.vars)
                .args(args)
                .output()
                .expect("bilbo runs");
            return Run {
                code: u8::try_from(output.status.code().expect("an exit code")).expect("a code"),
                stdout: String::from_utf8(output.stdout).expect("UTF-8 stdout"),
                stderr: String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .map(str::to_owned)
                    .collect(),
            };
        }
        let vars = self.vars.clone();
        let said = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&said);
        let mut env = Env {
            var: Box::new(move |name| vars.get(name).cloned()),
            stdin: Box::new(io::empty()),
            notice: Box::new(move |message| {
                sink.lock().expect("notices").extend(message.plain_lines());
            }),
        };
        let argv: Vec<OsString> = std::iter::once("bilbo")
            .chain(args.iter().copied())
            .map(OsString::from)
            .collect();
        let settings = settings::load(&*env.var, &|path| fs::read(path), &bilbo_cli::kind_keys())
            .map_err(Failure::from);
        let (code, stdout, lines) = match bilbo_cli::run(&argv, &mut env, settings) {
            Ok(outcome) => (
                outcome.exit_code(),
                outcome.stdout(&View::PLAIN),
                outcome.stderr_lines(&View::PLAIN),
            ),
            Err(failure) => (
                failure.exit_code(),
                String::new(),
                failure.stderr_lines(&View::PLAIN),
            ),
        };
        let mut stderr = said.lock().expect("notices").clone();
        stderr.extend(lines);
        Run {
            code,
            stdout,
            stderr,
        }
    }
}

struct Scratch {
    dir: PathBuf,
    bilbo: Bilbo,
}

impl Scratch {
    fn new(test: &str) -> Scratch {
        let base = var("TEST_TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let dir = base.canonicalize().expect("a scratch base").join(format!(
            "{test}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(dir.join("home")).expect("home");
        fs::create_dir_all(dir.join("config")).expect("config");
        fs::write(dir.join("config/config.toml"), "").expect("an empty config file");
        let vars = [
            ("HOME", dir.join("home")),
            ("BILBO_HOME", dir.join("store")),
            ("XDG_STATE_HOME", dir.join("state")),
            ("XDG_CACHE_HOME", dir.join("cache")),
            ("BILBO_CONFIG", dir.join("config/config.toml")),
        ]
        .into_iter()
        .map(|(name, path)| (name.to_owned(), path.into_os_string()))
        .collect();
        Scratch {
            dir,
            bilbo: Bilbo { vars, binary: None },
        }
    }

    fn remembered(test: &str) -> Scratch {
        let scratch = Scratch::new(test);
        for (name, bytes) in SESSIONS {
            scratch.write(&format!("sessions/{name}"), bytes);
        }
        for (name, bytes) in NOTES {
            scratch.write(&format!("notes/{name}"), bytes);
        }
        for folder in ["sessions", "notes"] {
            let run = scratch.bilbo.run(&["remember", &scratch.path(folder)]);
            assert_eq!(run.code, 0, "{folder}: {:?}", run.stderr);
        }
        scratch
    }

    fn path(&self, name: &str) -> String {
        self.dir.join(name).to_str().expect("UTF-8").to_owned()
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        let path = self.dir.join(name);
        fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
        fs::write(path, bytes).expect("write");
    }

    fn with(&self, name: &str, value: &str) -> Bilbo {
        let mut bilbo = self.bilbo.clone();
        bilbo.vars.insert(name.to_owned(), OsString::from(value));
        bilbo
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct Kept {
    id: Id,
    kind: String,
    generation: u32,
    total: u64,
    units: Vec<(u64, String)>,
    listed: Vec<(u64, u64)>,
}

fn units(passages: &[Passage]) -> Vec<(u64, String)> {
    let mut units = Vec::new();
    for passage in passages {
        let mut start = 0;
        for (at, end) in passage.ends.iter().enumerate() {
            units.push((
                passage.range.first + at as u64,
                passage.text[start..*end].to_owned(),
            ));
            start = *end;
        }
    }
    units
}

fn kept(settings: &Settings) -> Vec<Kept> {
    let store = Store::open_read_only(&settings.layout(), Options::default(), &mut |_| {})
        .expect("open the store");
    let readers = bilbo_cli::readers(settings).expect("the readers");
    let (changes, _) = store.changes(&Cursor::start()).expect("changes");
    let mut found = Vec::new();
    for change in changes {
        let Change::Meta(id) = change else {
            continue;
        };
        if found.iter().any(|kept: &Kept| kept.id == id) {
            continue;
        }
        let meta = store.meta(id).expect("meta").expect("an entry");
        let (generation, total) = store.extent(id, None).expect("extent").expect("an entry");
        let whole = RecordRange {
            first: 1,
            count: total,
        };
        let records = store.read(id, generation, whole).expect("read");
        let records: Vec<&[u8]> = records.iter().map(Vec::as_slice).collect();
        let reader: &dyn Reader = readers
            .iter()
            .find(|reader| reader.kind() == meta.kind)
            .expect("a reader for every kept kind")
            .as_ref();
        let passages = reader.passages(&records);
        found.push(Kept {
            id,
            kind: meta.kind,
            generation: generation.0,
            total,
            units: units(&passages),
            listed: passages
                .iter()
                .filter(|passage| passage.outline.is_some())
                .map(|passage| {
                    (
                        passage.range.first,
                        passage.range.first + passage.range.count - 1,
                    )
                })
                .collect(),
        });
    }
    found
}

#[derive(Debug, Default, Clone, Copy)]
struct Walked {
    pages: usize,
    records: usize,
    cuts: usize,
    longest: usize,
}

/// Describes a bad line without a byte of it: a walk over real sessions must never copy their text into a log.
fn shape(line: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    line.hash(&mut hasher);
    format!("{} bytes, hash {:016x}", line.len(), hasher.finish())
}

fn number(text: &str) -> Option<(u64, &str)> {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    Some((text[..digits].parse().ok()?, &text[digits..]))
}

fn locator(text: &str) -> Option<(u64, u64, &str)> {
    let (first, rest) = number(text.strip_prefix("#r")?)?;
    let (last, rest) = match rest.strip_prefix('-') {
        Some(rest) => number(rest)?,
        None => (first, rest),
    };
    Some((first, last, rest.strip_prefix(' ')?))
}

fn walk_pages(bilbo: &Bilbo, kept: &Kept, budget: Option<&str>) -> Result<Walked, String> {
    let id = kept.id.to_string();
    let pinned = format!("#g{}:", kept.generation);
    let mut walked = Walked::default();
    let mut next = 0;
    let mut reference = match kept.total {
        0 => {
            let run = bilbo.run(&["read", &id]);
            let empty = run.code == 0 && run.stdout.ends_with("outline:\nend: 0 records\n");
            return empty
                .then_some(walked)
                .ok_or("an entry with no record".to_owned());
        }
        1 => format!("{id}#r1"),
        total => format!("{id}#r1-{total}"),
    };
    loop {
        let before = next;
        let mut args = vec!["read", reference.as_str()];
        args.extend(budget.iter().flat_map(|budget| ["--budget", budget]));
        let run = bilbo.run(&args);
        if run.code != 0 {
            return Err(format!("exit {} for {reference}", run.code));
        }
        walked.pages += 1;
        walked.longest = walked.longest.max(run.stdout.chars().count());
        let body = run.stdout.strip_suffix('\n').ok_or("no final newline")?;
        let (mut body, last) = match body.rfind('\n') {
            Some(at) => (&run.stdout[..=at], &body[at + 1..]),
            None => ("", body),
        };
        let (mut cut, trailer) = match last.strip_prefix("cut: ") {
            Some(said) => {
                let (more, rest) = said
                    .split_once(" more characters; ")
                    .ok_or("a cut of another form")?;
                let (command, trailer) = rest.split_once("; ").ok_or("a cut with no trailer")?;
                (Some((more, command)), trailer)
            }
            None => (None, last),
        };
        while !body.is_empty() {
            let (first, last, rest) = locator(body).ok_or("a block without a locator")?;
            let unit = kept
                .units
                .get(next)
                .ok_or("a record past the last that renders")?;
            if unit.0 != first {
                return Err(format!("another record printed where {} is next", unit.0));
            }
            let count = usize::try_from(last - first).map_err(|_| "a long block")? + 1;
            let block = kept
                .units
                .get(next..next + count)
                .ok_or("a block past the end")?;
            if block.iter().map(|(record, _)| *record).ne(first..=last) {
                return Err(format!("the block at record {} skips a record", unit.0));
            }
            let text: String = block.iter().map(|(_, text)| text.as_str()).collect();
            let whole = if text.ends_with('\n') {
                text.clone()
            } else {
                format!("{text}\n")
            };
            if let Some(after) = rest.strip_prefix(whole.as_str()) {
                body = after;
            } else {
                let printed = rest.strip_suffix('\n').ok_or("a block without an end")?;
                let left = text
                    .strip_prefix(printed)
                    .ok_or(format!("record {first} is not printed as it reads"))?;
                let (more, command) = cut
                    .take()
                    .ok_or(format!("record {first} is not printed as it reads"))?;
                if count != 1 || more.parse().ok() != Some(left.chars().count()) || left.is_empty()
                {
                    return Err(format!("record {first}: the cut line miscounts"));
                }
                let alone = format!("{id}{pinned}r{first}");
                let whole_budget = command
                    .strip_prefix(&format!("bilbo read {alone} --budget "))
                    .ok_or("a cut command of another form")?;
                let again = bilbo.run(&["read", &alone, "--budget", whole_budget]);
                if again.code != 0 || !again.stdout.starts_with(&format!("#r{first} {whole}end: "))
                {
                    return Err(format!(
                        "record {first}: its cut's budget does not print it whole"
                    ));
                }
                walked.cuts += 1;
                body = "";
            }
            next += count;
        }
        if cut.is_some() {
            return Err("the last line says a cut, and no record of the page is cut".to_owned());
        }
        if let Some(command) = trailer.strip_prefix("more: ") {
            if next == before {
                return Err("a page names a next page and prints no record".to_owned());
            }
            let unit = kept
                .units
                .get(next)
                .ok_or("more: after the last record that renders")?;
            let mut expected = format!("bilbo read {id}{pinned}r{}", unit.0);
            if unit.0 != kept.total {
                expected += &format!("-{}", kept.total);
            }
            expected += &budget
                .map(|budget| format!(" --budget {budget}"))
                .unwrap_or_default();
            if command != expected {
                return Err(format!(
                    "more: is not {expected:?} but a line of {}",
                    shape(command)
                ));
            }
            reference = command
                .strip_prefix("bilbo read ")
                .and_then(|rest| rest.split(' ').next())
                .ok_or("a more: line of another form")?
                .to_owned();
        } else {
            let asked = reference.strip_prefix(id.as_str()).unwrap_or_default();
            let asked = asked.replacen(&pinned, "#", 1);
            let noun = if kept.total == 1 { "record" } else { "records" };
            let end = format!("end: {asked} of {} {noun}", kept.total);
            if trailer != end {
                return Err(format!(
                    "the last line is not {end:?} but a line of {}",
                    shape(trailer)
                ));
            }
            if next != kept.units.len() {
                return Err(format!(
                    "{} records that render were never printed",
                    kept.units.len() - next
                ));
            }
            walked.records = next;
            return Ok(walked);
        }
    }
}

fn walk_outline(
    bilbo: &Bilbo,
    kept: &Kept,
    budget: Option<&str>,
) -> Result<(usize, usize), String> {
    let id = kept.id.to_string();
    let mut reference = id.clone();
    let mut outline = false;
    let mut next = 0;
    let mut longest = 0;
    loop {
        let before = next;
        let mut args = vec!["read", reference.as_str()];
        if outline {
            args.push("--outline");
        }
        args.extend(budget.iter().flat_map(|budget| ["--budget", budget]));
        let run = bilbo.run(&args);
        longest = longest.max(run.stdout.chars().count());
        if run.code != 0 {
            return Err(format!("exit {} for the outline {reference}", run.code));
        }
        let mut lines: Vec<&str> = run.stdout.lines().collect();
        let trailer = lines.pop().ok_or("an empty outline")?;
        if !outline {
            let at = lines
                .iter()
                .position(|line| *line == "outline:")
                .ok_or("no outline: line")?;
            if lines[0] != format!("bilbo:{id}")
                || !lines.contains(&format!("kind: {}", kept.kind).as_str())
            {
                return Err("the metadata names another entry".to_owned());
            }
            lines.drain(..=at);
        }
        for line in lines {
            let (first, last, _) = locator(line).ok_or("an outline line without a locator")?;
            if kept.listed.get(next) != Some(&(first, last)) {
                return Err(format!(
                    "line {next} of the outline lists other records than the reader's"
                ));
            }
            next += 1;
        }
        match outline_trailer(kept, &reference, (before, next), budget, trailer)? {
            Some(target) => {
                reference = target;
                outline = true;
            }
            None => return Ok((next, longest)),
        }
    }
}

fn outline_trailer(
    kept: &Kept,
    reference: &str,
    (before, next): (usize, usize),
    budget: Option<&str>,
    trailer: &str,
) -> Result<Option<String>, String> {
    let id = kept.id.to_string();
    let pinned = format!("#g{}:", kept.generation);
    let range = |first: u64| match kept.total {
        0 => String::new(),
        total if first == total => format!("r{first}"),
        total => format!("r{first}-{total}"),
    };
    if let Some(&(first, _)) = kept.listed.get(next) {
        let target = format!("{id}{pinned}{}", range(first));
        let flag = budget
            .map(|budget| format!(" --budget {budget}"))
            .unwrap_or_default();
        let more = format!("more: bilbo read {target} --outline{flag}");
        if trailer == more && next == before {
            return Err("an outline page names a next page and lists nothing".to_owned());
        }
        if trailer == more {
            return Ok(Some(target));
        }
        if trailer.starts_with("more: ") {
            return Err(format!(
                "an outline's more: is not {more:?} but a line of {}",
                shape(trailer)
            ));
        }
    }
    let asked = match reference.strip_prefix(id.as_str()) {
        Some("") if kept.total > 0 => format!("#{}", range(1)),
        Some(asked) => asked.replacen(&pinned, "#", 1),
        None => return Err("a ref of another entry".to_owned()),
    };
    let of = if asked.is_empty() { "" } else { " of " };
    let noun = if kept.total == 1 { "record" } else { "records" };
    let end = format!("end: {asked}{of}{} {noun}", kept.total);
    if trailer != end {
        return Err(format!(
            "the last line of an outline is not {end:?} but a line of {}",
            shape(trailer)
        ));
    }
    if next != kept.listed.len() {
        return Err("listed passages are missing".to_owned());
    }
    Ok(None)
}

fn entry(scratch: &Scratch, name: &str) -> Id {
    let found = scratch.bilbo.run(&["read", name]);
    assert_eq!(found.code, 0, "{name}: {:?}", found.stderr);
    let id = found
        .stdout
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("bilbo:"));
    Id::parse(id.expect("the entry's id")).expect("an id")
}

#[test]
fn following_more_prints_each_record_of_every_fixture_exactly_once_at_any_budget() {
    let scratch = Scratch::remembered("read_chain");
    let kept = kept(&scratch.bilbo.settings());
    assert_eq!(kept.len(), SESSIONS.len() + NOTES.len());
    let mut cuts = 0;
    for budget in [None, Some("256"), Some("300"), Some("700"), Some("60000")] {
        for entry in &kept {
            let walked = walk_pages(&scratch.bilbo, entry, budget)
                .unwrap_or_else(|error| panic!("{} at {budget:?}: {error}", entry.id));
            assert_eq!(walked.records, entry.units.len());
            assert!(
                budget.is_some() || walked.pages == 1,
                "a fixture fits the default page"
            );
            cuts += walked.cuts;
            let (listed, longest) = walk_outline(&scratch.bilbo, entry, budget)
                .unwrap_or_else(|error| panic!("{} at {budget:?}: {error}", entry.id));
            assert_eq!(listed, entry.listed.len());
            if budget.is_none() {
                assert!(
                    walked.longest.max(longest) <= 30_000,
                    "a default page is over the cut"
                );
            }
        }
    }
    assert!(
        cuts >= 4,
        "a session's long result and a note's long line are cut at the small budgets: {cuts}"
    );
    let rendering: usize = kept.iter().map(|entry| entry.units.len()).sum();
    assert!(rendering > 50, "{rendering}");
    assert!(
        kept.iter()
            .any(|entry| (entry.units.len() as u64) < entry.total),
        "some records render nothing"
    );
}

type Spoil = fn(&mut Vec<(u64, String)>);

#[test]
fn the_walk_finds_a_record_printed_twice_left_out_or_changed() {
    let scratch = Scratch::remembered("read_chain_oracle");
    let all = kept(&scratch.bilbo.settings());
    let entry = all
        .iter()
        .find(|entry| entry.units.len() > 8)
        .expect("a session");
    let spoils: [(&str, Spoil); 4] = [
        (
            "a record the verb prints is unknown to the reader",
            |units| {
                units.remove(3);
            },
        ),
        ("a record the verb leaves out", |units| {
            let record = units[3].0;
            units.insert(4, (record, "user: again".to_owned()));
        }),
        ("another text", |units| units[3].1.push('!')),
        ("a record after the last", |units| {
            units.push((9999, "user: late".to_owned()));
        }),
    ];
    for (case, spoil) in spoils {
        let mut units = entry.units.clone();
        spoil(&mut units);
        let wrong = Kept {
            id: entry.id,
            kind: entry.kind.clone(),
            generation: entry.generation,
            total: entry.total,
            units,
            listed: entry.listed.clone(),
        };
        assert!(
            walk_pages(&scratch.bilbo, &wrong, Some("256")).is_err(),
            "{case}"
        );
    }
    let mut listed = entry.listed.clone();
    listed.pop();
    let wrong = Kept {
        id: entry.id,
        kind: entry.kind.clone(),
        generation: entry.generation,
        total: entry.total,
        units: entry.units.clone(),
        listed,
    };
    assert!(walk_outline(&scratch.bilbo, &wrong, Some("256")).is_err());
}

#[test]
fn the_walk_follows_only_the_more_line_it_expects_and_never_repeats_another() {
    const SECRET: &str = "synthetic-secret-7f3a91";
    let kept = Kept {
        id: Id::parse(SECTIONS).expect("id"),
        kind: "session".to_owned(),
        generation: 2,
        total: 40,
        units: Vec::new(),
        listed: vec![(3, 3), (17, 17), (40, 40)],
    };
    let first = SECTIONS.to_owned();
    let second = format!("{SECTIONS}#g2:r17-40");
    let more = format!("more: bilbo read {second} --outline");
    assert_eq!(
        outline_trailer(&kept, &first, (0, 1), None, &more),
        Ok(Some(second.clone()))
    );
    assert_eq!(
        outline_trailer(
            &kept,
            &second,
            (1, 2),
            Some("256"),
            &format!("more: bilbo read {SECTIONS}#g2:r40 --outline --budget 256")
        ),
        Ok(Some(format!("{SECTIONS}#g2:r40")))
    );
    assert_eq!(
        outline_trailer(&kept, &first, (2, 3), None, "end: #r1-40 of 40 records"),
        Ok(None)
    );
    assert_eq!(
        outline_trailer(&kept, &second, (2, 3), None, "end: #r17-40 of 40 records"),
        Ok(None)
    );
    for forged in [
        format!("more: bilbo read {SECRET} --outline"),
        format!("more: bilbo read {second} --outline {SECRET}"),
        format!("more: bilbo read {SECTIONS}#g2:r18-40 --outline"),
        format!("more: bilbo read {SECTIONS}#g1:r17-40 --outline"),
        format!("more: bilbo read {SECTIONS}#r17-40 --outline"),
        format!("more: bilbo read {second}"),
        format!("more: bilbo read {second} --outline --budget 256"),
        format!("end: {SECRET}"),
        "end: #r1-40 of 40 records".to_owned(),
    ] {
        let refused = outline_trailer(&kept, &first, (0, 1), None, &forged).expect_err(&forged);
        assert!(!refused.contains(SECRET), "{refused}");
    }
    assert!(outline_trailer(&kept, &first, (2, 3), None, &more).is_err());
    assert_eq!(
        outline_trailer(
            &kept,
            &first,
            (0, 0),
            None,
            &format!("more: bilbo read {SECTIONS}#g2:r3-40 --outline")
        ),
        Err("an outline page names a next page and lists nothing".to_owned())
    );
    assert_eq!(
        outline_trailer(&kept, &second, (1, 1), None, &more),
        Err("an outline page names a next page and lists nothing".to_owned())
    );
    assert!(outline_trailer(&kept, &second, (2, 3), None, "end: #r1-40 of 40 records").is_err());
}

#[test]
fn a_session_reads_by_its_uuid_or_eight_characters_of_it() {
    let scratch = Scratch::remembered("read_uuid");
    let id = entry(&scratch, CLAUDE);
    for reference in [
        CLAUDE.to_owned(),
        CLAUDE.to_uppercase(),
        CLAUDE[..8].to_owned(),
    ] {
        let run = scratch.bilbo.run(&["read", &format!("{reference}#r3")]);
        assert_eq!(run.code, 0, "{reference}: {:?}", run.stderr);
        assert_eq!(
            run.stdout,
            "#r3 user: Thin the carrot rows, then tell me how many seedlings are left.\n\n\
             Use the north bed only.\nend: #r3 of 23 records\n"
        );
    }
    let outline = scratch.bilbo.run(&["read", &CODEX[..13]]);
    assert_eq!(outline.code, 0);
    assert!(
        outline.stdout.contains("\nkind: session\n"),
        "{}",
        outline.stdout
    );
    assert!(
        outline.stdout.contains("\nstarted: 2026-11-07 09:02 UTC\n"),
        "{}",
        outline.stdout
    );
    assert!(
        outline.stdout.ends_with(
            "outline:\n#r5 user: Prune the roses and show me the result.\n\
             #r15 agent_message /root/pruner_1 to /root: The side bed is pruned too.\n\
             #r18 compact summary: The roses were pruned and beds/roses.csv was updated.\n\
             #r21 user: Now water them.\nend: #r1-25 of 25 records\n"
        ),
        "{}",
        outline.stdout
    );
    let page = scratch
        .bilbo
        .run(&["read", &format!("{id}#r5-6"), "--budget", "256"]);
    assert_eq!(
        page.stdout,
        "#r5 assistant: I'll count the rows first — então desbasto.\ntool_use Bash:\n\
         command: wc -l beds/north.csv\ndescription: Count the rows\ntimeout: 5000\n\
         run_in_background: false\n#r6 tool_result Bash:\n42 beds/north.csv\n\
         end: #r5-6 of 23 records\n"
    );
}

#[test]
fn a_subagent_names_its_parent_and_reads_as_its_own_thread() {
    let scratch = Scratch::remembered("read_subagent");
    let parent = entry(&scratch, OLDER);
    let all = kept(&scratch.bilbo.settings());
    let subagent = all
        .iter()
        .find(|kept| {
            kept.total == 2
                && kept
                    .units
                    .first()
                    .is_some_and(|unit| unit.1.contains("seed packets"))
        })
        .expect("the subagent");
    let run = scratch.bilbo.run(&["read", &subagent.id.to_string()]);
    assert_eq!(run.code, 0);
    assert!(
        run.stdout.contains(&format!("\nparent: bilbo:{parent}\n")),
        "{}",
        run.stdout
    );
    assert!(run.stdout.ends_with(
        "outline:\n#r1 user: Count the seed packets in the shed.\nend: #r1-2 of 2 records\n"
    ));
    let main = scratch.bilbo.run(&["read", &parent.to_string()]);
    assert!(!main.stdout.contains("parent:"), "{}", main.stdout);
}

#[test]
fn a_note_reads_back_by_heading_and_a_range_inside_a_section_prints_its_lines() {
    let scratch = Scratch::remembered("read_note");
    let outline = scratch.bilbo.run(&["read", SECTIONS]);
    assert_eq!(outline.code, 0, "{:?}", outline.stderr);
    assert!(
        outline
            .stdout
            .starts_with(&format!("bilbo:{SECTIONS}\nkind: markdown\norigin: file:"))
    );
    assert!(!outline.stdout.contains("started:"), "a note has no time");
    assert!(
        outline.stdout.ends_with(
            "records: 34\ngeneration: 1\noutline:\n#r1-7 frontmatter\n#r8-11 The garden log\n\
             #r12-21 The garden log > Beds\n#r22-27 The garden log > Watering\n\
             #r28-31 The garden log > Watering > Tools\n#r32-34 The garden log > Harvest\n\
             end: #r1-34 of 34 records\n"
        ),
        "{}",
        outline.stdout
    );
    let section = scratch
        .bilbo
        .run(&["read", &format!("bilbo:{SECTIONS}#r28-31")]);
    assert_eq!(
        section.stdout,
        "#r28-31 ### Tools\n\nA hoe and a rake.\n\nend: #r28-31 of 34 records\n"
    );
    let inside = scratch.bilbo.run(&["read", &format!("{SECTIONS}#r14-22")]);
    assert_eq!(
        inside.stdout,
        "#r14-21 - north: carrots\n- south: beans\n\n```sh\n# a comment in a code block, not a heading\n\
         ls beds\n```\n\n#r22 Watering\nend: #r14-22 of 34 records\n"
    );
    let whole = scratch
        .bilbo
        .run(&["read", &format!("{SECTIONS}#r1-34"), "--budget", "60000"]);
    let text: String = whole
        .stdout
        .lines()
        .filter(|line| !line.starts_with("end: "))
        .map(|line| match locator(line) {
            Some((_, _, rest)) => format!("{rest}\n"),
            None => format!("{line}\n"),
        })
        .collect();
    assert_eq!(
        text.as_bytes(),
        NOTES[0].1,
        "the pages join back to the note"
    );
}

#[test]
fn an_older_generation_reads_by_its_number() {
    let scratch = Scratch::remembered("read_generation");
    scratch.write("notes/sections.md", b"# Rewritten\n\nOne line.\n");
    let run = scratch
        .bilbo
        .run(&["remember", &scratch.path("notes/sections.md")]);
    assert_eq!(
        run.stdout.lines().next(),
        Some("new 0, appended 0, new generation 1, unchanged 0, refused 0")
    );
    let current = scratch.bilbo.run(&["read", SECTIONS]);
    assert!(
        current.stdout.ends_with(
            "records: 3\ngeneration: 2\noutline:\n#r1-3 Rewritten\nend: #r1-3 of 3 records\n"
        ),
        "{}",
        current.stdout
    );
    let older = scratch
        .bilbo
        .run(&["read", &format!("{SECTIONS}#g1"), "--budget", "256"]);
    assert!(
        older
            .stdout
            .contains("\nrecords: 34\ngeneration: 1 of 2\noutline:\n#g1:r1-7 frontmatter\n"),
        "{}",
        older.stdout
    );
    assert!(older.stdout.ends_with("end: #g1:r1-34 of 34 records\n"));
    let page = scratch
        .bilbo
        .run(&["read", &format!("{SECTIONS}#g1:r22-34"), "--budget", "256"]);
    assert!(
        page.stdout
            .starts_with("#g1:r22-25 Watering\n--------\n\nWater in the morning.\n"),
        "{}",
        page.stdout
    );
    assert!(
        page.stdout.ends_with(&format!(
            "more: bilbo read {SECTIONS}#g1:r26-34 --budget 256\n"
        )),
        "{}",
        page.stdout
    );
    let cut = scratch
        .bilbo
        .run(&["read", &format!("{SECTIONS}#g1:r26-34"), "--budget", "256"]);
    assert!(
        cut.stdout.contains(&format!(
            "\ncut: 622 more characters; bilbo read {SECTIONS}#g1:r26 --budget 1000; \
             more: bilbo read {SECTIONS}#g1:r27-34 --budget 256\n"
        )),
        "{}",
        cut.stdout
    );
    let named = scratch.bilbo.run(&["read", &format!("{SECTIONS}#g2:r1")]);
    assert_eq!(named.stdout, "#r1 # Rewritten\nend: #r1 of 3 records\n");
}

#[test]
fn what_cannot_be_read_exits_1_and_what_cannot_be_meant_exits_2() {
    let scratch = Scratch::remembered("read_exits");
    scratch.write(
        "copy/p/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl",
        SESSIONS[0].1,
    );
    assert_eq!(
        scratch.bilbo.run(&["remember", &scratch.path("copy")]).code,
        0
    );
    let unknown = "01JC5Q3V7XKM4P8R2T6W9YB0ZZ";
    let cases: [(&[&str], u8, &str); 12] = [
        (&[unknown], 1, "bilbo: no entry 01JC5Q3V7XKM4P8R2T6W9YB0ZZ"),
        (
            &["0e1d2c3b-0000"],
            1,
            "bilbo: no entry for the session 0e1d2c3b-0000",
        ),
        (
            &["01JC5Q3V7XKM4P8R2T6W9YB0DG#r35"],
            1,
            "bilbo: entry 01JC5Q3V7XKM4P8R2T6W9YB0DG has 34 records: #r35 is outside it",
        ),
        (
            &["01JC5Q3V7XKM4P8R2T6W9YB0DG#r30-40"],
            1,
            "bilbo: entry 01JC5Q3V7XKM4P8R2T6W9YB0DG has 34 records: #r30-40 is outside it",
        ),
        (
            &["01JC5Q3V7XKM4P8R2T6W9YB0DG#g2"],
            1,
            "bilbo: entry 01JC5Q3V7XKM4P8R2T6W9YB0DG has no generation 2",
        ),
        (
            &["nope"],
            2,
            "bilbo: 'nope' is not an entry id, a session id or 8 or more characters of one",
        ),
        (
            &["01JC5Q3V7XKM4P8R2T6W9YB0DG#r9-5"],
            2,
            "bilbo: #r9-5 ends before it starts",
        ),
        (
            &["01JC5Q3V7XKM4P8R2T6W9YB0DG#x"],
            2,
            "bilbo: '#x' is not a locator: use #r<a>, #r<a>-<b>, #g<n> or #g<n>:r<a>-<b>",
        ),
        (&[], 2, "bilbo: missing <ref>"),
        (
            &[SECTIONS, "--budget", "12"],
            2,
            "bilbo: --budget must be a whole number of 256 or more, got \"12\"",
        ),
        (&[SECTIONS, "--budget="], 2, "bilbo: --budget needs a value"),
        (&[SECTIONS, "--frob"], 2, "bilbo: unknown option '--frob'"),
    ];
    for (args, code, first) in cases {
        let mut list = vec!["read"];
        list.extend(args);
        let run = scratch.bilbo.run(&list);
        assert_eq!(
            (run.code, run.stdout.as_str()),
            (code, ""),
            "{args:?}: {:?}",
            run.stderr
        );
        assert_eq!(
            run.stderr.first().map(String::as_str),
            Some(first),
            "{args:?}"
        );
    }
    let several = scratch.bilbo.run(&["read", &CLAUDE[..8]]);
    assert_eq!(several.code, 2, "{:?}", several.stderr);
    assert!(
        several.stderr[0].starts_with("bilbo: 8f2b6d4e names 2 entries: "),
        "{:?}",
        several.stderr
    );
    assert!(
        several.stderr[0].ends_with("; give the id of one"),
        "{:?}",
        several.stderr
    );
    assert_eq!(
        several.stderr[1],
        "bilbo: usage: bilbo read <ref> [--budget <tokens>]"
    );
    assert_eq!(
        several.stderr.last().map(String::as_str),
        Some("bilbo: see 'bilbo read --help'")
    );
    let empty = scratch.bilbo.run(&[
        "read",
        &format!("{CLAUDE_ONLY}#r1-2", CLAUDE_ONLY = &OLDER[..8]),
    ]);
    assert_eq!(
        (empty.code, empty.stdout.as_str()),
        (
            0,
            "#r2 user: Plant the tomatoes in the north bed and log the commit.\nend: #r1-2 of 7 records\n"
        )
    );
    let nothing = scratch.bilbo.run(&["read", &format!("{}#r1", &OLDER[..8])]);
    assert_eq!(
        (nothing.code, nothing.stdout.as_str()),
        (0, "end: #r1 of 7 records\n")
    );
}

#[test]
fn a_read_with_no_store_says_so_and_creates_nothing() {
    let scratch = Scratch::new("read_no_store");
    let run = scratch.bilbo.run(&["read", SECTIONS]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        [format!("bilbo: no store at {}", scratch.path("store"))]
    );
    for folder in ["store", "state", "cache"] {
        assert!(!scratch.dir.join(folder).exists(), "{folder}");
    }
}

#[test]
fn the_budget_comes_from_the_flag_then_the_variable_then_the_file() {
    let scratch = Scratch::remembered("read_budget");
    let reference = format!("{SECTIONS}#r22-34");
    let by_default = scratch.bilbo.run(&["read", &reference]);
    assert!(by_default.stdout.ends_with("end: #r22-34 of 34 records\n"));
    fs::write(
        scratch.dir.join("config/config.toml"),
        "[read]\nbudget = 256\n",
    )
    .expect("config");
    let by_file = scratch.bilbo.run(&["read", &reference]);
    assert!(
        by_file
            .stdout
            .ends_with(&format!("more: bilbo read {SECTIONS}#g1:r26-34\n")),
        "a budget the file gives is not repeated: {}",
        by_file.stdout
    );
    let by_variable = scratch
        .with("BILBO_READ_BUDGET", "60000")
        .run(&["read", &reference]);
    assert_eq!(by_variable.stdout, by_default.stdout);
    let by_flag = scratch
        .with("BILBO_READ_BUDGET", "60000")
        .run(&["read", &reference, "--budget", "256"]);
    assert!(by_flag.stdout.ends_with(&format!(
        "more: bilbo read {SECTIONS}#g1:r26-34 --budget 256\n"
    )));
    let config = scratch.bilbo.run(&["config", "read.budget"]);
    assert_eq!(config.stdout, "256\n");
    fs::write(
        scratch.dir.join("config/config.toml"),
        "[read]\nbudget = 12\n",
    )
    .expect("config");
    let refused = scratch.bilbo.run(&["read", &reference]);
    assert_eq!(refused.code, 2);
    assert!(
        refused.stderr[0]
            .ends_with("config.toml:2: read.budget must be a whole number of 256 or more, got 12"),
        "{:?}",
        refused.stderr
    );
}

fn outline_texts(run: &Run) -> Vec<String> {
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    run.stdout
        .lines()
        .skip_while(|line| *line != "outline:")
        .skip(1)
        .filter_map(|line| line.strip_prefix("#r")?.split_once(' '))
        .map(|(_, text)| text.to_owned())
        .collect()
}

#[test]
fn the_outline_width_comes_from_the_variable_then_the_file_and_is_checked() {
    let scratch = Scratch::remembered("read_outline_width");
    let longest = |run: &Run| {
        outline_texts(run)
            .iter()
            .map(|text| text.chars().count())
            .max()
            .unwrap_or(0)
    };
    let by_default = scratch.bilbo.run(&["read", &CODEX[..13]]);
    assert!(longest(&by_default) > 40, "{}", by_default.stdout);
    assert!(!by_default.stdout.contains('…'));
    fs::write(
        scratch.dir.join("config/config.toml"),
        "[read]\noutline_width = 30\n",
    )
    .expect("config");
    let by_file = scratch.bilbo.run(&["read", &CODEX[..13]]);
    assert_eq!(longest(&by_file), 30, "{}", by_file.stdout);
    assert!(by_file.stdout.contains("…\n"), "{}", by_file.stdout);
    let by_variable = scratch
        .with("BILBO_READ_OUTLINE_WIDTH", "25")
        .run(&["read", &CODEX[..13]]);
    assert_eq!(longest(&by_variable), 25, "{}", by_variable.stdout);
    let config = scratch.bilbo.run(&["config", "read.outline_width"]);
    assert_eq!(config.stdout, "30\n");
    for text in ["19", "1001", "wide"] {
        let refused = scratch
            .with("BILBO_READ_OUTLINE_WIDTH", text)
            .run(&["read", &CODEX[..13]]);
        assert_eq!(refused.code, 2, "{text}");
        assert_eq!(
            refused.stderr[0],
            format!(
                "bilbo: BILBO_READ_OUTLINE_WIDTH must be a whole number from 20 to 1000, got \"{text}\""
            )
        );
    }
    fs::write(
        scratch.dir.join("config/config.toml"),
        "[read]\noutline_width = 19\n",
    )
    .expect("config");
    let refused = scratch.bilbo.run(&["read", &CODEX[..13]]);
    assert_eq!(refused.code, 2);
    assert!(
        refused.stderr[0].ends_with(
            "config.toml:2: read.outline_width must be a whole number from 20 to 1000, got 19"
        ),
        "{:?}",
        refused.stderr
    );
}

#[test]
fn the_matches_listed_come_from_the_variable_then_the_file_and_are_checked() {
    let scratch = Scratch::remembered("read_matches_listed");
    for copy in ["copy", "again"] {
        scratch.write(
            &format!("{copy}/p/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl"),
            SESSIONS[0].1,
        );
        assert_eq!(
            scratch.bilbo.run(&["remember", &scratch.path(copy)]).code,
            0
        );
    }
    let entries = |run: &Run| {
        assert_eq!(run.code, 2, "{:?}", run.stderr);
        run.stderr[0].matches("(file:").count()
    };
    let by_default = scratch.bilbo.run(&["read", &CLAUDE[..8]]);
    assert_eq!(entries(&by_default), 3);
    assert!(!by_default.stderr[0].contains(" more"));
    fs::write(
        scratch.dir.join("config/config.toml"),
        "[read]\nmatches_listed = 2\n",
    )
    .expect("config");
    let by_file = scratch.bilbo.run(&["read", &CLAUDE[..8]]);
    assert_eq!(entries(&by_file), 2);
    assert!(
        by_file.stderr[0].ends_with(" and 1 more; give the id of one"),
        "{:?}",
        by_file.stderr
    );
    let by_variable = scratch
        .with("BILBO_READ_MATCHES_LISTED", "1")
        .run(&["read", &CLAUDE[..8]]);
    assert_eq!(entries(&by_variable), 1);
    assert!(by_variable.stderr[0].ends_with(" and 2 more; give the id of one"));
    for text in ["0", "101", "many"] {
        let refused = scratch
            .with("BILBO_READ_MATCHES_LISTED", text)
            .run(&["read", &CLAUDE[..8]]);
        assert_eq!(refused.code, 2, "{text}");
        assert_eq!(
            refused.stderr[0],
            format!(
                "bilbo: BILBO_READ_MATCHES_LISTED must be a whole number from 1 to 100, got \"{text}\""
            )
        );
    }
}

#[test]
fn the_heading_width_comes_from_the_variable_then_the_file_and_is_checked() {
    let scratch = Scratch::remembered("read_heading_width");
    scratch.write(
        "wide/wide.md",
        format!(
            "---\nid: 01JC5Q3V7XKM4P8R2T6W9YB0DH\n---\n# {}\n## Inner\ntext\n",
            "h".repeat(40)
        )
        .as_bytes(),
    );
    assert_eq!(
        scratch.bilbo.run(&["remember", &scratch.path("wide")]).code,
        0
    );
    let page = |bilbo: &Bilbo| bilbo.run(&["read", "01JC5Q3V7XKM4P8R2T6W9YB0DH", "--outline"]);
    let by_default = page(&scratch.bilbo);
    assert!(
        by_default
            .stdout
            .contains(&format!("{} > Inner", "h".repeat(40))),
        "{}",
        by_default.stdout
    );
    fs::write(
        scratch.dir.join("config/config.toml"),
        "[markdown]\nheading_width = 30\n",
    )
    .expect("config");
    let by_file = page(&scratch.bilbo);
    assert!(
        by_file
            .stdout
            .contains(&format!("{}… > Inner", "h".repeat(29))),
        "{}",
        by_file.stdout
    );
    let by_variable = page(&scratch.with("BILBO_MARKDOWN_HEADING_WIDTH", "20"));
    assert!(
        by_variable
            .stdout
            .contains(&format!("{}… > Inner", "h".repeat(19))),
        "{}",
        by_variable.stdout
    );
    for text in ["19", "1001", "wide"] {
        let refused = page(&scratch.with("BILBO_MARKDOWN_HEADING_WIDTH", text));
        assert_eq!(refused.code, 2, "{text}");
        assert_eq!(
            refused.stderr[0],
            format!(
                "bilbo: BILBO_MARKDOWN_HEADING_WIDTH must be a whole number from 20 to 1000, got \"{text}\""
            )
        );
    }
}

#[test]
fn a_command_of_a_page_reads_the_generation_the_page_read_also_after_the_file_was_rewritten() {
    let scratch = Scratch::remembered("read_pinned");
    let first = scratch
        .bilbo
        .run(&["read", &format!("{SECTIONS}#r22-34"), "--budget", "256"]);
    let more = first.stdout.lines().last().expect("a last line");
    assert_eq!(
        more,
        format!("more: bilbo read {SECTIONS}#g1:r26-34 --budget 256")
    );
    let cut_page = scratch
        .bilbo
        .run(&["read", &format!("{SECTIONS}#g1:r26"), "--budget", "256"]);
    let cut = cut_page
        .stdout
        .lines()
        .last()
        .and_then(|line| line.strip_prefix("cut: "))
        .and_then(|said| said.strip_suffix("; end: #r26 of 34 records"))
        .expect("a cut on the last line")
        .to_owned();
    assert!(
        cut.ends_with(&format!("; bilbo read {SECTIONS}#g1:r26 --budget 1000")),
        "{cut}"
    );

    scratch.write("notes/sections.md", b"# Rewritten\n\nOne line.\n");
    assert_eq!(
        scratch
            .bilbo
            .run(&["remember", &scratch.path("notes")])
            .code,
        0
    );
    let args: Vec<&str> = more["more: bilbo ".len()..].split(' ').collect();
    let next = scratch.bilbo.run(&args);
    assert_eq!(
        next.code, 0,
        "the old generation is still read: {:?}",
        next.stderr
    );
    assert!(
        next.stdout
            .starts_with("#g1:r26 A line longer than a small page: "),
        "{}",
        next.stdout
    );
    let args: Vec<&str> = cut
        .split("; bilbo ")
        .nth(1)
        .expect("a command")
        .split(' ')
        .collect();
    let whole = scratch.bilbo.run(&args);
    assert!(
        whole.stdout.starts_with("#g1:r26 A line longer")
            && whole.stdout.ends_with("end: #g1:r26 of 34 records\n"),
        "{}",
        whole.stdout
    );
    assert!(!whole.stdout.contains("\ncut: "));
}

fn headings(origin: String, line: usize) -> bilbo_kinds::read::Fetched {
    use bilbo_store::entries::{Generation, Meta, Origin};
    bilbo_kinds::read::Fetched {
        meta: Meta {
            id: Id::parse(SECTIONS).expect("id"),
            kind: "markdown".to_owned(),
            origin: Origin::new(origin),
            captured: std::time::UNIX_EPOCH,
            parent: None,
        },
        generation: Generation(1),
        current: Generation(1),
        records: (0..600)
            .map(|n| format!("## {n:03} {}\n", "x".repeat(line)).into_bytes())
            .collect(),
    }
}

fn default_pages(fetched: &bilbo_kinds::read::Fetched, first: &str, outline: bool) -> usize {
    use bilbo_kinds::read::Request;
    use bilbo_kinds::read::refs::Ref;
    let budget = Scratch::new("read_default_budget")
        .bilbo
        .settings()
        .read_budget
        .value;
    let mut reference = first.to_owned();
    let mut listed = 0;
    for page in 0.. {
        assert!(page < 20, "the chain does not end");
        let request = Request {
            reference: Ref::parse(&reference).expect("a ref"),
            outline,
            budget,
            outline_width: 100,
            matches_listed: 5,
        };
        let output = bilbo_cli::read::rendered(
            fetched,
            &request,
            &bilbo_cli::readers(&Scratch::new("read_readers").bilbo.settings())
                .expect("the readers"),
            None,
        )
        .expect("a page");
        let text = bilbo_cli::read::view::plain(&output, None);
        let characters = text.chars().count();
        assert!(characters <= 30_000, "page {page}: {characters}");
        assert!(
            page > 0 || characters > 20_000,
            "the first page is nearly full: {characters}"
        );
        listed += text.lines().filter(|line| line.starts_with("#r")).count();
        match output.more {
            Some(more) => reference = format!("{SECTIONS}{}", output.pinned(more)),
            None => break,
        }
    }
    listed
}

#[test]
fn an_outline_page_stays_under_the_cut_whatever_its_origin_holds() {
    // A path may hold escape characters, and each prints as six.
    let origin = format!("file:/{}", "\u{1b}".repeat(900));
    let fetched = headings(origin, 110);
    assert_eq!(default_pages(&fetched, SECTIONS, true), 600);
}

#[test]
fn a_page_of_the_default_budget_stays_under_the_cut_off_a_terminal() {
    let fetched = headings("file:/notes/long.md".to_owned(), 110);
    let all = format!("{SECTIONS}#r1-600");
    assert_eq!(default_pages(&fetched, &all, false), 600);
}

struct Probe {
    lock: PathBuf,
    free: Arc<Mutex<Vec<bool>>>,
}

impl Reader for Probe {
    fn kind(&self) -> &'static str {
        "markdown"
    }

    fn passages(&self, _records: &[&[u8]]) -> Vec<Passage> {
        let lock = fs::File::open(&self.lock).expect("the lock file");
        self.free
            .lock()
            .expect("free")
            .push(lock.try_lock().is_ok());
        Vec::new()
    }
}

#[test]
fn the_store_is_let_go_before_a_page_is_rendered() {
    let scratch = Scratch::remembered("read_lock");
    let settings = scratch.bilbo.settings();
    let free = Arc::new(Mutex::new(Vec::new()));
    let readers: Vec<Box<dyn Reader>> = vec![Box::new(Probe {
        lock: settings.layout().state.join("store.lock"),
        free: Arc::clone(&free),
    })];
    let args = bilbo_cli::read::Args {
        reference: SECTIONS.to_owned(),
        outline: false,
        budget: None,
    };
    let outcome = bilbo_cli::read::run(
        &bilbo_cli::bilbo(),
        &args,
        &mut Env::quiet(),
        &settings,
        &readers,
    )
    .expect("an outline");
    assert_eq!(outcome.exit_code(), 0);
    assert_eq!(
        *free.lock().expect("free"),
        [true],
        "a writer could take the store while read rendered"
    );
}

#[test]
fn a_read_that_finds_the_store_held_by_a_writer_waits_as_long_as_it_is_told_then_gives_up() {
    let scratch = Scratch::remembered("read_busy");
    let lock = fs::File::options()
        .read(true)
        .write(true)
        .open(scratch.bilbo.settings().layout().state.join("store.lock"))
        .expect("the lock file");
    lock.lock().expect("hold the store as a writer does");
    let at_once = scratch
        .with("BILBO_STORE_WAIT", "0s")
        .run(&["read", SECTIONS]);
    assert_eq!((at_once.code, at_once.stdout.as_str()), (1, ""));
    assert_eq!(
        at_once.stderr,
        ["bilbo: another bilbo is writing the store"]
    );
    let waited = scratch
        .with("BILBO_STORE_WAIT", "1s")
        .run(&["read", SECTIONS]);
    assert_eq!(waited.code, 1);
    assert_eq!(
        waited.stderr,
        [
            "bilbo: waiting for another bilbo to finish writing the store",
            "bilbo: another bilbo is writing the store"
        ]
    );
    drop(lock);
    assert_eq!(
        scratch
            .with("BILBO_STORE_WAIT", "0s")
            .run(&["read", SECTIONS])
            .code,
        0
    );
}

#[test]
fn a_read_changes_nothing_in_the_store_the_state_or_the_cache() {
    let scratch = Scratch::remembered("read_only");
    let snapshot = |dir: &Path| {
        let mut files = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(dir).expect("list") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let bytes = fs::read(&path).expect("read");
                    files.push((path, bytes));
                }
            }
        }
        files.sort();
        files
    };
    let folders = ["store", "state", "cache", "config"].map(|name| scratch.dir.join(name));
    let before: Vec<_> = folders.iter().map(|dir| snapshot(dir)).collect();
    for args in [
        vec!["read", SECTIONS],
        vec!["read", CLAUDE, "--budget", "256"],
        vec!["read", "nope"],
    ] {
        scratch.bilbo.run(&args);
    }
    let read: Vec<_> = folders.iter().map(|dir| snapshot(dir)).collect();
    assert!(
        read == before,
        "a read changed the store, the state, the cache or the config"
    );
    let catalog = scratch.dir.join("cache/bilbo/catalog.sqlite");
    fs::remove_file(&catalog).expect("delete the catalog");
    let slow = scratch.bilbo.run(&["read", SECTIONS]);
    assert_eq!(
        slow.code, 0,
        "the segments are read when the catalog is gone: {:?}",
        slow.stderr
    );
    let mut without = before;
    without[2].retain(|(path, _)| *path != catalog);
    let after: Vec<_> = folders.iter().map(|dir| snapshot(dir)).collect();
    assert!(
        after == without,
        "a read with no catalog wrote one, or changed another file"
    );
}

/// `READ_BINARY`, when set, is the binary to run in place of this process. Prints counts only.
#[test]
#[ignore = "the done check: the lead runs it against a scratch store filled from the real folders"]
#[expect(
    clippy::disallowed_macros,
    reason = "the done check prints what it counted"
)]
fn every_entry_of_a_store_reads_to_its_end_with_each_record_once() {
    let names = [
        "HOME",
        "BILBO_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
        "BILBO_CONFIG",
    ];
    let vars: HashMap<String, OsString> = names
        .into_iter()
        .map(|name| {
            let value = var(name)
                .unwrap_or_else(|| panic!("the done check needs {names:?}; {name} is not set"));
            (name.to_owned(), value)
        })
        .collect();
    let bilbo = Bilbo {
        vars,
        binary: var("READ_BINARY")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from),
    };
    let kept = kept(&bilbo.settings());
    let mut total = Walked::default();
    let (mut rendering, mut listed, mut failed) = (0, 0, 0);
    for entry in &kept {
        rendering += entry.units.len();
        for budget in [None, Some("256")] {
            match walk_pages(&bilbo, entry, budget) {
                Ok(walked) => {
                    total.pages += walked.pages;
                    total.cuts += walked.cuts;
                    total.records += walked.records;
                    if budget.is_none() {
                        total.longest = total.longest.max(walked.longest);
                    }
                }
                Err(error) => {
                    failed += 1;
                    println!("entry {} at {budget:?}: {error}", entry.id);
                }
            }
        }
        match walk_outline(&bilbo, entry, None) {
            Ok((lines, longest)) => {
                listed += lines;
                total.longest = total.longest.max(longest);
            }
            Err(error) => {
                failed += 1;
                println!("entry {}: {error}", entry.id);
            }
        }
    }
    let records: u64 = kept.iter().map(|entry| entry.total).sum();
    println!(
        "entries {}, records {records}, rendering {rendering}, outline lines {listed}, pages {}, cuts {}, \
         longest default page {} characters, failed {failed}",
        kept.len(),
        total.pages,
        total.cuts,
        total.longest
    );
    assert!(!kept.is_empty(), "the store is empty");
    assert_eq!(failed, 0);
    assert_eq!(
        total.records,
        2 * rendering,
        "each record that renders, once per budget"
    );
    assert!(
        total.longest <= 30_000,
        "a default page is over Claude Code's cut"
    );
}
