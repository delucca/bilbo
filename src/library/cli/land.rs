//! `bilbo library land`.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::{Args, Output, capture_title, io_failure, refused, root, state_failure, today, usage};
use crate::Failure;
use crate::citation::{self, Document, Verdict};
use crate::library::corpus;
use crate::library::source::{self, Frontmatter};
use crate::shared::frontmatter;
use crate::shared::hash;
use crate::shared::markdown;
use crate::shared::store::{self, EntryKind};

struct Staged {
    dir: PathBuf,
    capture: String,
    origin: String,
    fetched: String,
    label: String,
    sha256: String,
}

fn read_stage(env: &store::Env, id: &str) -> Result<Staged, Failure> {
    let staging = store::staging_dir(env).ok_or_else(state_failure)?;
    let dir = staging.join(id);
    if !dir.is_dir() {
        return Err(refused(format!("no stage '{id}' in {}", staging.display())));
    }
    let capture_path = dir.join("capture.md");
    let capture = fs::read_to_string(&capture_path).map_err(io_failure("read", &capture_path))?;
    let record_path = dir.join("stage.json");
    let record = fs::read_to_string(&record_path).map_err(io_failure("read", &record_path))?;
    let record: serde_json::Value = serde_json::from_str(&record)
        .map_err(|e| refused(format!("cannot read {}: {e}", record_path.display())))?;
    let field = |key: &str| {
        record
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| refused(format!("{} has no '{key}'", record_path.display())))
    };
    let sha256 = field("sha256")?;
    if hash::sha256_hex(capture.as_bytes()) != sha256 {
        return Err(refused(format!(
            "the capture of stage {id} changed since it was staged; stage the file again"
        )));
    }
    Ok(Staged {
        dir,
        capture,
        origin: field("origin")?,
        fetched: field("fetched")?,
        label: field("capture")?,
        sha256,
    })
}

/// A `# ` line is a title to `check` even when it has no text, which is no heading to the outline.
fn is_heading(line: &str) -> bool {
    markdown::heading(line).is_some() || line.starts_with("# ")
}

struct Plan {
    corpus: String,
    name: String,
    ranges: Vec<(usize, usize)>,
    title: Option<String>,
    replace: bool,
    force: bool,
}

fn plan(args: &Args) -> Result<(String, Plan), Failure> {
    let [stage, target] = args.operands(["<stage>", "<corpus>/<name>"])?;
    if !frontmatter::is_ulid(stage) {
        return Err(usage(format!("'{stage}' is not a stage id")));
    }
    let (corpus, name) = match target.split_once('/') {
        Some((corpus, name)) if corpus::is_corpus_name(corpus) && corpus::is_source_name(name) => {
            (corpus, name)
        }
        _ => {
            return Err(usage(format!(
                "invalid target '{target}': use <corpus>/<name>, each with segments of a-z and 0-9 joined by single hyphens; 'guide' and the subcommand names are taken"
            )));
        }
    };
    let keep = args.all("--keep");
    if keep.is_empty() {
        return Err(usage("missing --keep <a>-<b>[,<c>-<d>]..."));
    }
    let joined = keep.join(",");
    let ranges =
        source::parse_kept(&joined).map_err(|rule| usage(format!("--keep '{joined}' {rule}")))?;
    let title = args.one("--title").map(str::to_string);
    if let Some(title) = &title {
        if title.trim().is_empty() {
            return Err(usage("--title must not be empty"));
        }
        if title.contains(['\n', '\r']) {
            return Err(usage("--title must be one line"));
        }
    }
    if args.force && !args.replace {
        return Err(usage("--force applies to --replace; pass both or neither"));
    }
    let plan = Plan {
        corpus: corpus.into(),
        name: name.into(),
        ranges,
        title,
        replace: args.replace,
        force: args.force,
    };
    Ok((stage.into(), plan))
}

/// The body for the kept lines, and whether their headings were demoted.
fn build_body(title: &str, kept: &[&str]) -> (String, bool) {
    let outside = markdown::outside_fences(kept);
    let demote = outside.iter().any(|&i| kept[i].starts_with("# "));
    let mut body = format!("# {title}\n\n");
    for (i, line) in kept.iter().enumerate() {
        if demote && outside.binary_search(&i).is_ok() && is_heading(line) {
            body.push('#');
        }
        body.push_str(line);
        body.push('\n');
    }
    (body, demote)
}

/// The source being replaced, or `None` for a new one; refuses a taken name or nothing to replace.
fn check_target(path: &Path, replace: bool) -> Result<Option<(String, Option<String>)>, Failure> {
    if !replace {
        return match path.symlink_metadata() {
            Ok(_) => Err(refused(format!(
                "{} already exists; pass --replace to replace it",
                path.display()
            ))),
            Err(_) => Ok(None),
        };
    }
    let text = fs::read(path)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| {
            refused(format!(
                "{} is not a source, so there is nothing to replace",
                path.display()
            ))
        })?;
    let old = source::read(&text);
    match old.id {
        Some(id) => Ok(Some((id, old.digest))),
        None => Err(refused(format!(
            "{} has no valid id; fix it before replacing it",
            path.display()
        ))),
    }
}

pub fn run(args: &Args, env: &store::Env) -> Result<Output, Failure> {
    let (stage_id, plan) = plan(args)?;
    let root = root(env)?;
    let staged = read_stage(env, &stage_id)?;

    let lines = markdown::lines(&staged.capture);
    if let Some(&(_, end)) = plan.ranges.last()
        && end > lines.len()
    {
        return Err(usage(format!(
            "--keep goes to line {end}, past the {} lines of the capture",
            lines.len()
        )));
    }
    let title = match plan
        .title
        .clone()
        .or_else(|| capture_title(&lines).map(|(_, t)| t))
    {
        Some(title) => title,
        None => {
            return Err(usage(
                "the capture has no level-1 heading to use as the title; pass --title",
            ));
        }
    };
    let kept: Vec<&str> = plan
        .ranges
        .iter()
        .flat_map(|&(a, b)| &lines[a - 1..b])
        .copied()
        .collect();
    let (body, demoted) = build_body(&title, &kept);
    let digest = source::digest(&body);
    let whole = source::format_kept(&plan.ranges) == format!("1-{}", lines.len());

    let library = store::library_dir(&root);
    let dir = library.join(&plan.corpus);
    let target = dir.join(format!("{}.md", plan.name));
    check_target(&target, plan.replace)?;

    fs::create_dir_all(&library).map_err(io_failure("create", &library))?;
    let lock_path = library.join(".lock");
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(io_failure("open", &lock_path))?;
    lock.lock().map_err(io_failure("lock", &lock_path))?;

    let old = check_target(&target, plan.replace)?;
    let id = match &old {
        Some((id, _)) => id.clone(),
        None => frontmatter::mint_ulid()
            .map_err(|e| refused(format!("cannot read /dev/urandom: {e}")))?,
    };
    let mut warnings = Vec::new();
    if demoted {
        warnings.push(
            "the kept lines hold a level-1 heading, so every heading in them moved one level down"
                .into(),
        );
    }
    warnings.extend(duplicate_origins(&root, &staged.origin, &target)?);

    let text = source::render(
        &Frontmatter {
            id: id.clone(),
            fetched: staged.fetched.clone(),
            origin: staged.origin.clone(),
            digest: digest.clone(),
            kept: (!whole).then(|| source::format_kept(&plan.ranges)),
            capture: (staged.label != "fetched").then(|| staged.label.clone()),
        },
        &body,
    );
    if let Some(problem) = source::read(&text).problems.first() {
        return Err(refused(format!(
            "internal error: the rendered source has problems: {problem}"
        )));
    }

    if let Some((_, old_digest)) = &old
        && old_digest.as_deref() != Some(&digest)
        && let Ok(old_text) = fs::read_to_string(&target)
    {
        let degraded = degraded_citations(&root, &id, &old_text, &text);
        if !degraded.is_empty() && !plan.force {
            return Err(refused(format!(
                "{} would degrade; nothing was written and the stage is kept; pass --force to replace anyway\n{}",
                match degraded.len() {
                    1 => "1 citation".to_string(),
                    n => format!("{n} citations"),
                },
                degraded.join("\n")
            )));
        }
        warnings.extend(degraded);
    }

    let today = today();
    let folder = keep_capture(&root, &staged)?;
    fs::create_dir_all(&dir).map_err(io_failure("create", &dir))?;
    write_source(&dir, &target, &text, plan.replace)?;
    record_landing(&folder, &id, &digest, &today)?;

    let guide = dir.join("guide.md");
    let changed = old
        .as_ref()
        .is_none_or(|(_, old_digest)| old_digest.as_deref() != Some(&digest));
    if changed && let Err(failure) = update_guide(&guide, &plan, &today) {
        let Failure::Refused(message) = failure else {
            return Err(failure);
        };
        return Err(refused(format!(
            "wrote {} but not its guide entry: {message}; bilbo check reports the missing entry, and landing again needs --replace",
            target.display()
        )));
    }
    if let Err(e) = fs::remove_dir_all(&staged.dir) {
        warnings.push(format!(
            "cannot remove the stage {}: {e}",
            staged.dir.display()
        ));
    }
    drop(lock);
    Ok(Output {
        warnings,
        lines: vec![
            format!("source: {}", target.display()),
            format!("id: {id}"),
            format!("guide: {}", guide.display()),
            format!("capture folder: {}", folder.display()),
        ],
    })
}

/// `notes/<file>:<line>: <old> -> <new>` for each citation of source `id` in the notes whose verdict changes to
/// anything but `ok` between the old text of the source and the new one.
fn degraded_citations(root: &Path, id: &str, old_text: &str, new_text: &str) -> Vec<String> {
    let mut degraded = Vec::new();
    let Ok(entries) = store::entries(&root.join("notes")) else {
        return degraded;
    };
    let needle = format!("bilbo:{id}");
    let mut documents: Option<(Document, Document)> = None;
    for entry in entries {
        if entry.kind != EntryKind::File || !entry.utf8 || !entry.name.ends_with(".md") {
            continue;
        }
        let Some(text) = fs::read(&entry.path)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .filter(|text| text.contains(&needle))
        else {
            continue;
        };
        for c in citation::parse(&text).0.iter().filter(|c| c.id == id) {
            let (old, new) = documents.get_or_insert_with(|| {
                let (old_body, old_first) = citation::body_of(old_text);
                let (new_body, new_first) = citation::body_of(new_text);
                (
                    Document::new(old_body, old_first),
                    Document::new(new_body, new_first),
                )
            });
            let (before, after) = (old.check(c).verdict, new.check(c).verdict);
            if before != after && after != Verdict::Ok {
                degraded.push(format!(
                    "notes/{}:{}: {} -> {}",
                    entry.name,
                    c.line,
                    before.name(),
                    after.name()
                ));
            }
        }
    }
    degraded
}

/// A warning for each other source with the same origin.
fn duplicate_origins(root: &Path, origin: &str, target: &Path) -> Result<Vec<String>, Failure> {
    let mut warnings = Vec::new();
    for (corpus, dir) in corpus::corpus_dirs(root).map_err(io_failure("read", root))? {
        for file in corpus::read_sources(&dir).map_err(io_failure("read", &dir))? {
            if file.source.origin.as_deref() == Some(origin) && file.path != target {
                warnings.push(format!(
                    "{origin} is also the origin of {corpus}/{}",
                    file.name
                ));
            }
        }
    }
    Ok(warnings)
}

/// Keeps the staged files under `<root>/.bilbo/captures/<sha256>/` unless that folder exists, and records the
/// landing in its `landed` file.
fn keep_capture(root: &Path, staged: &Staged) -> Result<PathBuf, Failure> {
    let captures = store::captures_dir(root);
    let folder = captures.join(&staged.sha256);
    if !folder.exists() {
        fs::create_dir_all(&captures).map_err(io_failure("create", &captures))?;
        let random = frontmatter::mint_ulid()
            .map_err(|e| refused(format!("cannot read /dev/urandom: {e}")))?;
        let tmp = captures.join(format!(".tmp-{random}"));
        let copied = copy_stage(&staged.dir, &tmp)
            .map_err(io_failure("write", &tmp))
            .and_then(|()| fs::rename(&tmp, &folder).map_err(io_failure("write", &folder)));
        if let Err(failure) = copied {
            let _ = fs::remove_dir_all(&tmp);
            return Err(failure);
        }
    }
    Ok(folder)
}

/// Appends the source's id, digest and date to the capture folder's `landed` file, unless a line with that id
/// and digest is there already.
fn record_landing(folder: &Path, id: &str, digest: &str, today: &str) -> Result<(), Failure> {
    let landed = folder.join("landed");
    let known = fs::read_to_string(&landed).is_ok_and(|text| {
        text.lines()
            .any(|line| line.starts_with(&format!("{id}\t{digest}\t")))
    });
    if known {
        return Ok(());
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&landed)
        .and_then(|mut file| writeln!(file, "{id}\t{digest}\t{today}"))
        .map_err(io_failure("write", &landed))
}

/// Every file of the stage folder but bilbo's own `stage.json`.
fn copy_stage(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_name() != "stage.json" && entry.file_type()?.is_file() {
            fs::copy(entry.path(), to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Writes `text` to a hidden `.<prefix>-<random>.tmp` file in `dir` and fsyncs it. A temp file that already exists
/// belongs to another run and is never touched.
fn write_temp(dir: &Path, prefix: &str, text: &str) -> Result<PathBuf, Failure> {
    let random =
        frontmatter::mint_ulid().map_err(|e| refused(format!("cannot read /dev/urandom: {e}")))?;
    let tmp = dir.join(format!(".{prefix}-{random}.tmp"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(io_failure("write", &tmp))?;
    match file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
    {
        Ok(()) => Ok(tmp),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(refused(format!("cannot write {}: {e}", tmp.display())))
        }
    }
}

/// A new source is hard-linked into place, as `bilbo new` does, so a name taken meanwhile fails; a replaced one
/// is renamed over the old file.
fn write_source(dir: &Path, target: &Path, text: &str, replace: bool) -> Result<(), Failure> {
    let tmp = write_temp(dir, "land", text)?;
    let placed = if replace {
        fs::rename(&tmp, target)
    } else {
        let linked = fs::hard_link(&tmp, target);
        // A failed unlink leaves a hidden temp file; the source itself already exists.
        let _ = fs::remove_file(&tmp);
        linked
    };
    placed.map_err(|e| match e.kind() {
        io::ErrorKind::AlreadyExists => refused(format!(
            "{} already exists; pass --replace to replace it",
            target.display()
        )),
        _ => {
            let _ = fs::remove_file(&tmp);
            refused(format!("cannot write {}: {e}", target.display()))
        }
    })
}

/// Creates the guide when missing, then adds the entry or marks it stale.
fn update_guide(guide: &Path, plan: &Plan, today: &str) -> Result<(), Failure> {
    let current = match fs::read(guide) {
        Ok(bytes) => String::from_utf8(bytes)
            .map_err(|_| refused(format!("{} is not valid UTF-8", guide.display())))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let guide_id = frontmatter::mint_ulid()
                .map_err(|e| refused(format!("cannot read /dev/urandom: {e}")))?;
            corpus::new_guide(&guide_id, &frontmatter::now_created(), &plan.corpus)
        }
        Err(e) => return Err(refused(format!("cannot read {}: {e}", guide.display()))),
    };
    let has_entry = corpus::read_guide(&current)
        .entries
        .iter()
        .any(|e| e.name == plan.name);
    let updated = if has_entry {
        corpus::mark_stale(&current, &plan.name, today)
    } else {
        corpus::add_entry(&current, &plan.name)
    };
    let dir = guide.parent().unwrap_or(Path::new("."));
    let tmp = write_temp(dir, "land-guide", &updated)?;
    fs::rename(&tmp, guide).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        refused(format!("cannot write {}: {e}", guide.display()))
    })
}
