use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::note;
use crate::note::conflicts::{self, Entry};
use crate::search::rank::{self, Document, Hit};
use crate::search::{documents, embed, vectors};
use crate::shared::{config, store, text};

const QUERY_BYTES: usize = 2000;
const EMBED_BYTES: usize = 1000;
const BUDGET: Duration = Duration::from_millis(1500);
/// Kept back from the embedder for the gate, the block, session memory and the log.
const RESERVE: Duration = Duration::from_millis(100);
const EMBED_LIMIT: Duration = Duration::from_millis(1200);
const FIRST: usize = 6;
const LATER: usize = 3;
const BLOCK_BYTES: usize = 9000;
const LOG_PROMPT_CHARS: usize = 500;
const SESSION_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const LONG_WORD_CHARS: usize = 4;
const KEYWORD_GATE: usize = 3;
const SESSION_MAX: usize = 128;
const LISTED_WAITING: usize = 3;
/// The line of a session's memory that says the sync conflicts were raised; a path never reads like it.
const RAISED: &str = "sync conflicts raised";

pub struct Outcome {
    /// stdout lines; empty means no digest.
    pub lines: Vec<String>,
    /// The run's error as one stderr line, without "bilbo: ".
    pub diagnostic: Option<String>,
}

/// What a run saw, for the digest log.
struct Record {
    session: Option<String>,
    prompt: Option<String>,
    ranking: &'static str,
    passed: usize,
    shown: Vec<String>,
    error: Option<String>,
}

/// A note that passed the gate, as its line in the block needs it.
struct Passed<'a> {
    path: &'a Path,
    kind: &'a str,
    created: Option<&'a str>,
    /// `conflict` or `auto-merged`, from the open-conflict summary.
    label: Option<&'static str>,
    hit: Hit,
}

/// Never fails: every error ends as an empty `lines` and a `diagnostic`.
pub fn run(args: &[String], input: &mut dyn Read, env: &store::Env) -> Outcome {
    let start = Instant::now();
    let settings = match config::load(env) {
        Ok(settings) => settings,
        Err(e) => {
            return Outcome {
                lines: Vec::new(),
                diagnostic: Some(one_line(&e)),
            };
        }
    };
    if !settings.digest.enable {
        let _ = std::io::copy(input, &mut std::io::sink());
        return Outcome {
            lines: Vec::new(),
            diagnostic: None,
        };
    }
    let mut record = Record {
        session: None,
        prompt: None,
        ranking: "none",
        passed: 0,
        shown: Vec::new(),
        error: None,
    };
    let lines = match digest(args, input, env, &settings, start, &mut record) {
        Ok(lines) => lines,
        Err(e) => {
            record.shown.clear();
            record.error = Some(e);
            Vec::new()
        }
    };
    if let Some(dir) = store::cache_dir(env) {
        sweep(&dir.join("sessions"), SystemTime::now());
    }
    if settings.digest.log
        && let Err(e) = append_log(env, &record, start.elapsed())
    {
        record.error.get_or_insert(e);
    }
    Outcome {
        lines,
        diagnostic: record.error.as_deref().map(one_line),
    }
}

/// The block's lines, empty when nothing is shown. Errors that stop the digest are `Err`; an
/// embedder that fails is only noted in `record.error`.
fn digest(
    args: &[String],
    input: &mut dyn Read,
    env: &store::Env,
    settings: &config::Settings,
    start: Instant,
    record: &mut Record,
) -> Result<Vec<String>, String> {
    if let Some(arg) = args.first() {
        return Err(if arg.starts_with('-') && arg != "-" {
            format!("unknown option '{arg}'")
        } else {
            format!("unexpected argument '{arg}'")
        });
    }
    let mut text = String::new();
    input
        .read_to_string(&mut text)
        .map_err(|e| format!("cannot read the hook input: {e}"))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .ok()
        .filter(serde_json::Value::is_object)
        .ok_or("the hook input is not a JSON object")?;
    let field = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
    };
    let prompt = field("prompt").ok_or("the hook input has no prompt")?;
    record.prompt = Some(prompt.trim().chars().take(LOG_PROMPT_CHARS).collect());
    let session = field("session_id").ok_or("the hook input has no session_id")?;
    if !is_session(session) {
        return Err(
            "the hook input's session_id is not 1 to 128 of A-Z, a-z, 0-9, '.', '_' and '-'".into(),
        );
    }
    record.session = Some(session.to_string());
    let ranked = match query(prompt) {
        Some(query) => ranked(&query, env, settings, start, record)?,
        None => {
            let Ok(root) = store::root(env) else {
                return Ok(Vec::new());
            };
            Ranked {
                root,
                documents: Vec::new(),
                found: Vec::new(),
                order: Vec::new(),
            }
        }
    };
    let Ranked {
        root,
        documents,
        found,
        order,
    } = ranked;
    record.passed = order.len();
    let by_path = match sync_state(&root, &found) {
        Ok(state) => state,
        Err(e) => {
            record.error.get_or_insert(e);
            Default::default()
        }
    };
    let mut waiting: Vec<&Path> = by_path
        .iter()
        .filter(|(_, entry)| entry.waits())
        .map(|(path, _)| path.as_path())
        .collect();
    waiting.sort();
    if order.is_empty() && waiting.is_empty() {
        return Ok(Vec::new());
    }

    let sessions = store::cache_dir(env)
        .ok_or("cannot find the cache folder: set XDG_CACHE_HOME, or HOME, to an absolute path")?
        .join("sessions");
    let memory = sessions.join(session);
    let (first, before) = match std::fs::read_to_string(&memory) {
        Ok(text) => (false, text.lines().map(str::to_string).collect()),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            (true, Vec::<String>::new())
        }
        Err(e) => return Err(format!("cannot read {}: {e}", memory.display())),
    };
    let fresh: Vec<Passed> = order
        .into_iter()
        .map(|hit| {
            let (path, kind, created) = &found[hit.document];
            Passed {
                path,
                kind,
                created: created.as_deref(),
                label: by_path.get(path.as_path()).and_then(Entry::label),
                hit,
            }
        })
        .filter(|p| !before.iter().any(|b| Path::new(b) == p.path))
        .collect();
    let cap = if first { FIRST } else { LATER };
    let raise = first && !waiting.is_empty();
    let (lines, shown) = block(&fresh, cap, &documents, raise.then_some(&waiting[..]));
    if shown.is_empty() && !raise {
        return Ok(Vec::new());
    }
    let mut text: String = before.iter().map(|line| format!("{line}\n")).collect();
    for path in &shown {
        text.push_str(path);
        text.push('\n');
    }
    if raise {
        text.push_str(RAISED);
        text.push('\n');
    }
    remember(&sessions, &memory, &text)?;
    record.shown = shown;
    Ok(lines)
}

/// What ranking found: the store, its notes and the ones that passed the gate, best first.
struct Ranked {
    root: PathBuf,
    documents: Vec<Document>,
    found: Vec<(PathBuf, String, Option<String>)>,
    order: Vec<Hit>,
}

/// Ranks the notes for `query`. An embedder that fails is only noted in `record.error`.
fn ranked(
    query: &str,
    env: &store::Env,
    settings: &config::Settings,
    start: Instant,
    record: &mut Record,
) -> Result<Ranked, String> {
    let words = text::words(query);

    let root = store::root(env)?;
    let notes = root.join("notes");
    if !notes.is_dir() {
        return Err(format!("no store at {}", root.display()));
    }
    let stored = documents::read_notes(&notes)
        .map_err(|e| format!("cannot read {}: {e}", notes.display()))?;
    let withheld = vectors::withheld(&stored, settings);
    let mut documents = Vec::with_capacity(stored.len());
    let mut found = Vec::with_capacity(stored.len());
    for n in stored {
        documents.push(n.document);
        found.push((n.path, n.kind, n.created));
    }

    let keyword = rank::keyword(&words, &documents);
    let mut scored = Vec::new();
    record.ranking = "keywords";
    let embeddable = documents
        .iter()
        .flat_map(|d| &d.passages)
        .filter_map(rank::input)
        .any(|input| !withheld.contains(&vectors::key(&input)));
    if let Some(embedder) = settings.embedder.as_ref().filter(|_| embeddable) {
        let cache = store::cache_dir(env)
            .map(|dir| vectors::load(&vectors::path(&dir, &root)))
            .unwrap_or_default();
        match meaning(
            embedder,
            &cache,
            query,
            &documents,
            &withheld,
            settings.digest.min_similarity,
            start,
        ) {
            Ok(hits) => {
                scored = hits;
                record.ranking = "meaning";
            }
            Err(e) => record.error = Some(e),
        }
    }
    let mut long: Vec<String> = Vec::new();
    for word in &words {
        if word.chars().count() >= LONG_WORD_CHARS && !long.contains(word) {
            long.push(word.clone());
        }
    }
    let by_keywords =
        |p: &rank::Passage| long.len() >= KEYWORD_GATE && rank::shared(p, &long) >= KEYWORD_GATE;
    let gated: Vec<bool> = if record.ranking == "meaning" {
        let mut gated = vec![false; documents.len()];
        for (_, hit) in &scored {
            gated[hit.document] = true;
        }
        if !withheld.is_empty() {
            for (gate, d) in gated.iter_mut().zip(&documents) {
                *gate = *gate
                    || d.passages.iter().any(|p| {
                        rank::input(p).is_some_and(|input| withheld.contains(&vectors::key(&input)))
                            && by_keywords(p)
                    });
            }
        }
        gated
    } else {
        documents
            .iter()
            .map(|d| d.passages.iter().any(by_keywords))
            .collect()
    };
    let keyword: Vec<Hit> = keyword.into_iter().filter(|h| gated[h.document]).collect();
    let meaning: Vec<Hit> = scored.iter().map(|(_, hit)| *hit).collect();
    let mut order = rank::fuse(&keyword, &meaning);
    let mut seen: HashSet<usize> = order.iter().map(|hit| hit.document).collect();
    for hit in &meaning {
        if seen.insert(hit.document) {
            order.push(*hit);
        }
    }
    Ok(Ranked {
        root,
        documents,
        found,
        order,
    })
}

/// The passages that pass the meaning gate, best first, or why the embedder gave no answer.
fn meaning(
    embedder: &config::Embedder,
    cache: &vectors::Cache,
    query: &str,
    documents: &[Document],
    withheld: &HashSet<u64>,
    min_similarity: f64,
    start: Instant,
) -> Result<Vec<(f32, Hit)>, String> {
    let (found, _) = vectors::lookup(cache, &embedder.model, documents, withheld);
    if !found.iter().flatten().any(Option::is_some) {
        return Err("no passage is indexed; run bilbo index".into());
    }
    let left = (BUDGET - RESERVE)
        .saturating_sub(start.elapsed())
        .min(EMBED_LIMIT);
    if left.is_zero() {
        return Err("no time left to ask the embedder".into());
    }
    let text = format!(
        "{}{}",
        embedder.query_prefix,
        &query[..query.floor_char_boundary(EMBED_BYTES)]
    );
    let q = embed::query(embedder, &text, left, cache.dims)?;
    Ok(rank::meaning(&q, &found, min_similarity))
}

/// The prompt trimmed and cut to `QUERY_BYTES`, a leading `/name` or `$name` without its sigil;
/// `None` when the first word starts with a sigil but is not a name.
fn query(prompt: &str) -> Option<String> {
    let trimmed = prompt.trim();
    let mut query = trimmed[..trimmed.floor_char_boundary(QUERY_BYTES)].to_string();
    if let Some(rest) = query.strip_prefix(['/', '$']) {
        let name = rest.split(char::is_whitespace).next().unwrap_or("");
        let is_name = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'));
        if !is_name {
            return None;
        }
        query = rest.to_string();
    }
    Some(query)
}

/// 1 to 128 of `A-Z`, `a-z`, `0-9`, `.`, `_` and `-`, and not `.` or `..`.
fn is_session(id: &str) -> bool {
    (1..=SESSION_MAX).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && id != "."
        && id != ".."
}

/// The block for at most `cap` of `fresh`, dropping notes from the end until it fits
/// `BLOCK_BYTES`, and the paths it lists. `waiting` adds the `Sync conflicts` line; with no note
/// shown the block is then the header and that line.
fn block(
    fresh: &[Passed],
    cap: usize,
    documents: &[Document],
    waiting: Option<&[&Path]>,
) -> (Vec<String>, Vec<String>) {
    let left = fresh.len();
    let mut shown = cap.min(left);
    loop {
        let mut lines = vec![format!("<!-- bilbo digest: {shown} of {left} notes -->")];
        if shown > 0 {
            lines.push(
                "Notes that may bear on this prompt (open the file to read more):".to_string(),
            );
        }
        for note in &fresh[..shown] {
            let passage = &documents[note.hit.document].passages[note.hit.passage];
            let label = note.label.map(|l| format!(", {l}")).unwrap_or_default();
            lines.push(format!(
                "- {}:{} ({}, {}{}) {}: {}",
                note.path.display(),
                passage.line,
                note.kind,
                note.created.unwrap_or("-"),
                label,
                passage.path.join(" > "),
                rank::snippet(passage)
            ));
        }
        if left > shown {
            lines.push(format!(
                "({} more passed; run bilbo recall for them)",
                left - shown
            ));
        }
        if let Some(waiting) = waiting {
            lines.push(waiting_line(waiting));
        }
        if lines.iter().map(|line| line.len() + 1).sum::<usize>() <= BLOCK_BYTES {
            let paths = fresh[..shown]
                .iter()
                .map(|note| note.path.display().to_string())
                .collect();
            return (lines, paths);
        }
        if shown == 0 {
            return (Vec::new(), Vec::new());
        }
        shown -= 1;
    }
}

/// `Sync conflicts wait in: <path>, ... (run bilbo check)`, naming `LISTED_WAITING` paths.
fn waiting_line(waiting: &[&Path]) -> String {
    let mut names: Vec<String> = waiting
        .iter()
        .take(LISTED_WAITING)
        .map(|path| path.display().to_string())
        .collect();
    if waiting.len() > LISTED_WAITING {
        names.push(format!("and {} more", waiting.len() - LISTED_WAITING));
    }
    format!(
        "Sync conflicts wait in: {} (run bilbo check)",
        names.join(", ")
    )
}

/// The notes of the store that the open-conflict summary has an entry for, by their path now. Only
/// the entries that label or raise are kept, and the summary is read only when it exists. A note
/// is matched by its id, because an entry's file name is stale after a rename.
fn sync_state(
    root: &Path,
    found: &[(PathBuf, String, Option<String>)],
) -> Result<HashMap<PathBuf, Entry>, String> {
    let path = conflicts::path(root);
    let mut summary =
        conflicts::read(root).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    summary
        .notes
        .retain(|_, entry| entry.waits() || entry.merged);
    let mut by_path = HashMap::new();
    if summary.notes.is_empty() {
        return Ok(by_path);
    }
    for (note, _, _) in found {
        let Ok(bytes) = std::fs::read(note) else {
            continue;
        };
        let id = note::read(&String::from_utf8_lossy(&bytes)).id;
        if let Some(entry) = id.and_then(|id| summary.notes.remove(&id)) {
            by_path.insert(note.clone(), entry);
        }
    }
    Ok(by_path)
}

/// Writes the session's memory through a temporary file and a rename.
fn remember(sessions: &Path, memory: &Path, text: &str) -> Result<(), String> {
    std::fs::create_dir_all(sessions)
        .map_err(|e| format!("cannot create {}: {e}", sessions.display()))?;
    let mut temp = memory.as_os_str().to_owned();
    temp.push(format!("~{}", std::process::id()));
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, text)
        .and_then(|()| std::fs::rename(&temp, memory))
        .map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            format!("cannot write {}: {e}", memory.display())
        })
}

/// Deletes the files in `sessions` not modified for `SESSION_AGE`; errors are ignored.
fn sweep(sessions: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(sessions) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let old = meta
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > SESSION_AGE);
        if meta.is_file() && old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Appends the run's line to `<state>/bilbo/digest.jsonl`, created with mode 0600.
fn append_log(env: &store::Env, record: &Record, elapsed: Duration) -> Result<(), String> {
    let dir = store::state_dir(env)
        .ok_or("cannot find the state folder: set XDG_STATE_HOME, or HOME, to an absolute path")?
        .join("bilbo");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = dir.join("digest.jsonl");
    let mut line = serde_json::json!({
        "time": jiff::Zoned::now().strftime("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        "session": record.session,
        "prompt": record.prompt,
        "ranking": record.ranking,
        "passed": record.passed,
        "shown": record.shown,
        "elapsed_ms": elapsed.as_millis() as u64,
    });
    if let Some(error) = &record.error {
        line["error"] = error.as_str().into();
    }
    let mut bytes = line.to_string().into_bytes();
    bytes.push(b'\n');
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(&path)
        .and_then(|mut file| file.write_all(&bytes))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn one_line(message: &str) -> String {
    message.replace('\n', "; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_prompt_is_trimmed() {
        assert_eq!(
            query("  how do embeddings work?\n").as_deref(),
            Some("how do embeddings work?")
        );
    }

    #[test]
    fn command_loses_its_sigil() {
        assert_eq!(
            query("/review:pr fix-login-timeout").as_deref(),
            Some("review:pr fix-login-timeout")
        );
        assert_eq!(
            query("$recall the store").as_deref(),
            Some("recall the store")
        );
    }

    #[test]
    fn path_is_not_a_command() {
        assert_eq!(query("/Users/a/notes/plan.md what is this?"), None);
        assert_eq!(query("/ hello"), None);
    }

    #[test]
    fn query_is_cut_on_a_char_boundary() {
        let q = query(&"ã".repeat(1500)).unwrap();
        assert!(q.len() <= QUERY_BYTES && q.len() >= QUERY_BYTES - 1);
    }

    #[test]
    fn session_ids() {
        assert!(is_session("719375aa-d6ab-43fd-83b1-5f71041eb527"));
        assert!(is_session("a.b_c-1"));
        for bad in ["", ".", "..", "../../etc", "a/b", "a b", &"x".repeat(129)] {
            assert!(!is_session(bad), "{bad:?}");
        }
    }

    fn fixture(count: usize, heading: &str) -> (Vec<Document>, Vec<PathBuf>) {
        let documents = (0..count)
            .map(|_| Document {
                passages: vec![rank::Passage {
                    path: vec![heading.to_string()],
                    line: 5,
                    text: "body".to_string(),
                }],
            })
            .collect();
        let paths = (0..count)
            .map(|i| PathBuf::from(format!("/notes/note-{i}.md")))
            .collect();
        (documents, paths)
    }

    fn passed(paths: &[PathBuf]) -> Vec<Passed<'_>> {
        paths
            .iter()
            .enumerate()
            .map(|(document, path)| Passed {
                path,
                kind: "note",
                created: None,
                label: None,
                hit: Hit {
                    document,
                    passage: 0,
                },
            })
            .collect()
    }

    #[test]
    fn waiting_line_names_three_paths_and_counts_the_rest() {
        let paths: Vec<PathBuf> = (1..=5)
            .map(|n| PathBuf::from(format!("/n/{n}.md")))
            .collect();
        let all: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
        assert_eq!(
            waiting_line(&all[..3]),
            "Sync conflicts wait in: /n/1.md, /n/2.md, /n/3.md (run bilbo check)"
        );
        assert_eq!(
            waiting_line(&all),
            "Sync conflicts wait in: /n/1.md, /n/2.md, /n/3.md, and 2 more (run bilbo check)"
        );
    }

    #[test]
    fn labelled_note_and_waiting_line_stay_within_the_block_limit() {
        let (documents, paths) = fixture(6, &"x".repeat(2000));
        let waiting = [Path::new("/n/1.md")];
        let (lines, shown) = block(&passed(&paths), FIRST, &documents, Some(&waiting));
        assert_eq!(shown.len(), 4);
        assert_eq!(
            lines.last().unwrap(),
            "Sync conflicts wait in: /n/1.md (run bilbo check)"
        );
        assert!(lines.iter().map(|l| l.len() + 1).sum::<usize>() <= BLOCK_BYTES);
        let (lines, shown) = block(&[], FIRST, &documents, Some(&waiting));
        assert!(shown.is_empty());
        assert_eq!(lines[0], "<!-- bilbo digest: 0 of 0 notes -->");
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn block_drops_notes_from_the_end_to_fit() {
        let (documents, paths) = fixture(6, &"x".repeat(2000));
        let (lines, shown) = block(&passed(&paths), FIRST, &documents, None);
        assert_eq!(
            shown,
            [
                "/notes/note-0.md",
                "/notes/note-1.md",
                "/notes/note-2.md",
                "/notes/note-3.md"
            ]
        );
        assert_eq!(lines[0], "<!-- bilbo digest: 4 of 6 notes -->");
        assert_eq!(
            lines.last().unwrap(),
            "(2 more passed; run bilbo recall for them)"
        );
        assert!(lines.iter().map(|l| l.len() + 1).sum::<usize>() <= BLOCK_BYTES);
    }

    #[test]
    fn block_has_no_overflow_line_when_all_fit() {
        let (documents, paths) = fixture(2, "Title");
        let (lines, shown) = block(&passed(&paths), FIRST, &documents, None);
        assert_eq!(shown.len(), 2);
        assert_eq!(lines.len(), 4);
        assert!(!lines.iter().any(|l| l.contains("more passed")));
    }
}
