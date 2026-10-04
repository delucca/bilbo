use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::corpus::{self, SourceFile};
use crate::source::{self, Frontmatter, Resolved, Section};
use crate::store::{self, EntryKind};
use crate::{Failure, hash, note, rank};

const VALUE_OPTIONS: [&str; 5] = ["--depth", "--origin", "--fetched", "--keep", "--title"];

pub struct Output {
    /// stderr lines (without "bilbo: "), printed before stdout.
    pub warnings: Vec<String>,
    pub lines: Vec<String>,
}

impl Output {
    fn lines(lines: Vec<String>) -> Output {
        Output {
            warnings: Vec::new(),
            lines,
        }
    }
}

fn usage(message: impl Into<String>) -> Failure {
    Failure::Usage(message.into())
}

fn refused(message: impl Into<String>) -> Failure {
    Failure::Refused(message.into())
}

fn io_failure(what: &'static str, path: &Path) -> impl Fn(io::Error) -> Failure {
    let path = path.to_path_buf();
    move |e| refused(format!("cannot {what} {}: {e}", path.display()))
}

struct Args {
    positional: Vec<String>,
    options: Vec<(String, String)>,
    replace: bool,
}

impl Args {
    fn parse(args: &[String]) -> Result<Args, Failure> {
        let mut parsed = Args {
            positional: Vec::new(),
            options: Vec::new(),
            replace: false,
        };
        let mut options_ended = false;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            let is_option = arg.chars().nth(1).is_some_and(|c| !c.is_whitespace());
            if options_ended || !arg.starts_with('-') || !is_option {
                parsed.positional.push(arg.clone());
                continue;
            }
            if arg == "--" {
                options_ended = true;
                continue;
            }
            let (name, inline) = match arg.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (arg.as_str(), None),
            };
            if name == "--replace" {
                if inline.is_some() {
                    return Err(usage("--replace takes no value"));
                }
                parsed.replace = true;
            } else if VALUE_OPTIONS.contains(&name) {
                let value = match inline {
                    Some(value) => value,
                    None => iter
                        .next()
                        .ok_or_else(|| usage(format!("{name} needs a value")))?
                        .clone(),
                };
                if name != "--keep" && parsed.options.iter().any(|(n, _)| n == name) {
                    return Err(usage(format!("{name} given more than once")));
                }
                parsed.options.push((name.to_string(), value));
            } else {
                return Err(usage(format!("unknown option '{name}'")));
            }
        }
        Ok(parsed)
    }

    fn one(&self, name: &str) -> Option<&str> {
        self.all(name).into_iter().next()
    }

    fn all(&self, name: &str) -> Vec<&str> {
        self.options
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// Refuses every option that is not in `taken`.
    fn only(&self, taken: &[&str]) -> Result<(), Failure> {
        let given = self.options.iter().map(|(n, _)| n.as_str());
        let given = given.chain(self.replace.then_some("--replace"));
        match given.into_iter().find(|name| !taken.contains(name)) {
            Some(name) => Err(usage(format!("option '{name}' does not apply here"))),
            None => Ok(()),
        }
    }

    /// The operands after the subcommand word, exactly as many as `names`.
    fn operands<const N: usize>(&self, names: [&str; N]) -> Result<[&str; N], Failure> {
        let rest = &self.positional[1..];
        if let Some(extra) = rest.get(N) {
            return Err(usage(format!("unexpected argument '{extra}'")));
        }
        let mut out = [""; N];
        for (i, name) in names.iter().enumerate() {
            out[i] = rest
                .get(i)
                .ok_or_else(|| usage(format!("missing {name}")))?;
        }
        Ok(out)
    }
}

pub fn run(args: &[String], env: &store::Env) -> Result<Output, Failure> {
    let args = Args::parse(args)?;
    match args.positional.first().map(String::as_str) {
        None => {
            args.only(&[])?;
            list(env)
        }
        Some("show") => {
            args.only(&["--depth"])?;
            show(&args, env)
        }
        Some("stage") => {
            args.only(&["--origin", "--fetched"])?;
            stage(&args, env)
        }
        Some("land") => {
            args.only(&["--keep", "--title", "--replace"])?;
            land(&args, env)
        }
        Some(name) => {
            args.only(&[])?;
            if let Some(extra) = args.positional.get(1) {
                return Err(usage(format!("unexpected argument '{extra}'")));
            }
            if !note::is_topic(name) {
                return Err(usage(format!(
                    "invalid corpus '{name}': use segments of a-z and 0-9 joined by single hyphens"
                )));
            }
            if !corpus::is_corpus_name(name) {
                return Err(usage(format!(
                    "'{name}' is reserved for a library subcommand"
                )));
            }
            show_corpus(name, env)
        }
    }
}

fn root(env: &store::Env) -> Result<PathBuf, Failure> {
    store::root(env).map_err(Failure::Config)
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "source" } else { "sources" }
}

fn list(env: &store::Env) -> Result<Output, Failure> {
    let root = root(env)?;
    let listing = corpus::listing(&root).map_err(io_failure("read", &store::library_dir(&root)))?;
    let lines = listing
        .iter()
        .map(|c| {
            format!(
                "{}\t{} {}\t{} KB\t{} tokens\t{}",
                c.name,
                c.sources,
                plural(c.sources),
                source::kb(c.bytes),
                c.tokens,
                c.title.as_deref().unwrap_or("-")
            )
        })
        .collect();
    Ok(Output::lines(lines))
}

fn sections(file: &SourceFile) -> Vec<Section> {
    source::outline(&note::lines(&file.text), file.source.body_start)
}

fn or_dash(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("-")
}

fn facts(file: &SourceFile) -> String {
    let s = &file.source;
    let bytes = file.body().len();
    let sections = sections(file);
    let mut line = format!(
        "`{}.md` · {} · {} KB · {} tokens · fetched {} · {} headings",
        file.name,
        or_dash(&s.id),
        source::kb(bytes),
        source::tokens(bytes),
        or_dash(&s.fetched),
        sections.len()
    );
    if source::is_catalog(bytes, &sections) {
        line.push_str(" · catalog");
    }
    if s.keys.iter().any(|k| k == "capture") {
        line.push_str(&format!(" · capture {}", or_dash(&s.capture)));
    }
    line
}

fn show_corpus(name: &str, env: &store::Env) -> Result<Output, Failure> {
    let root = root(env)?;
    let library = store::library_dir(&root);
    let dir = library.join(name);
    if !dir.is_dir() {
        return Err(refused(format!(
            "no corpus '{name}' in {}",
            library.display()
        )));
    }
    let sources = corpus::read_sources(&dir).map_err(io_failure("read", &dir))?;
    let guide_path = dir.join("guide.md");
    let mut lines = vec![guide_path.display().to_string()];
    let mut named: Vec<String> = Vec::new();
    let text = fs::read(&guide_path)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok());
    if let Some(text) = &text {
        let guide = corpus::read_guide(text);
        for (i, line) in note::lines(text)
            .iter()
            .enumerate()
            .skip(guide.body_start - 1)
        {
            lines.push(line.to_string());
            for entry in guide.entries.iter().filter(|e| e.line == i + 1) {
                lines.push(match sources.iter().find(|s| s.name == entry.name) {
                    Some(file) => facts(file),
                    None => format!("`{}.md` · missing", entry.name),
                });
            }
        }
        named = guide.entries.into_iter().map(|e| e.name).collect();
    }
    for file in sources.iter().filter(|s| !named.contains(&s.name)) {
        lines.push(format!("## {}", file.name));
        lines.push(facts(file));
        lines.push("(no entry in guide.md)".into());
    }
    Ok(Output::lines(lines))
}

enum Reference {
    Name { corpus: String, name: String },
    Id(String),
}

fn parse_reference(reference: &str) -> Result<Reference, Failure> {
    let malformed = || {
        usage(format!(
            "malformed source reference '{reference}': use <corpus>/<name> or a source id"
        ))
    };
    match reference.split_once('/') {
        Some((corpus, name)) if corpus::is_corpus_name(corpus) && corpus::is_source_name(name) => {
            Ok(Reference::Name {
                corpus: corpus.into(),
                name: name.into(),
            })
        }
        None if note::is_ulid(reference) => Ok(Reference::Id(reference.into())),
        _ => Err(malformed()),
    }
}

/// `<corpus>/<name>` of a source file.
fn label(file: &SourceFile) -> String {
    let corpus = file
        .path
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{corpus}/{}", file.name)
}

fn find(root: &Path, reference: &Reference) -> Result<SourceFile, Failure> {
    let library = store::library_dir(root);
    match reference {
        Reference::Name { corpus, name } => {
            let path = library.join(corpus).join(format!("{name}.md"));
            path.is_file()
                .then(|| corpus::read_source(&path, name.clone()))
                .flatten()
                .ok_or_else(|| {
                    refused(format!(
                        "no source '{corpus}/{name}' in {}",
                        library.display()
                    ))
                })
        }
        Reference::Id(id) => {
            let mut found = Vec::new();
            for (_, dir) in corpus::corpus_dirs(root).map_err(io_failure("read", &library))? {
                let sources = corpus::read_sources(&dir).map_err(io_failure("read", &dir))?;
                found.extend(
                    sources
                        .into_iter()
                        .filter(|s| s.source.id.as_deref() == Some(id)),
                );
            }
            match found.len() {
                0 => Err(refused(note_with_id(root, id).map_or_else(
                    || format!("no source with id {id} in {}", library.display()),
                    |path| {
                        format!(
                            "{id} is the id of the note {}, not of a source",
                            path.display()
                        )
                    },
                ))),
                1 => Ok(found.remove(0)),
                _ => Err(refused(format!(
                    "id {id} belongs to {}",
                    found.iter().map(label).collect::<Vec<_>>().join(" and ")
                ))),
            }
        }
    }
}

fn note_with_id(root: &Path, id: &str) -> Option<PathBuf> {
    store::entries(&root.join("notes"))
        .ok()?
        .into_iter()
        .filter(|e| e.kind == EntryKind::File)
        .find(|e| {
            fs::read(&e.path)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .is_some_and(|text| note::read(&text).id.as_deref() == Some(id))
        })
        .map(|e| e.path)
}

/// The capture folder whose `landed` file records this id with this digest.
fn capture_folder(root: &Path, id: &str, digest: &str) -> Option<PathBuf> {
    store::entries(&store::captures_dir(root))
        .ok()?
        .into_iter()
        .filter(|e| e.kind == EntryKind::Folder)
        .find(|e| {
            fs::read_to_string(e.path.join("landed")).is_ok_and(|landed| {
                landed.lines().any(|line| {
                    let mut parts = line.split('\t');
                    parts.next() == Some(id) && parts.next() == Some(digest)
                })
            })
        })
        .map(|e| e.path)
}

fn show(args: &Args, env: &store::Env) -> Result<Output, Failure> {
    let [reference] = args.operands(["<reference>"])?;
    let depth = args
        .one("--depth")
        .map(|value| {
            value
                .bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| value.parse::<usize>().ok().filter(|n| *n >= 1))
                .flatten()
                .ok_or_else(|| {
                    usage(format!(
                        "--depth '{value}' is not a whole number of 1 or more"
                    ))
                })
        })
        .transpose()?;
    let (reference, anchor) = match reference.split_once('#') {
        Some((_, "")) => return Err(usage("the anchor after '#' is empty")),
        Some((reference, anchor)) => (reference, Some(anchor)),
        None => (reference, None),
    };
    let parsed = parse_reference(reference)?;
    let root = root(env)?;
    let file = find(&root, &parsed)?;

    let s = &file.source;
    let lines = note::lines(&file.text);
    let sections = sections(&file);
    let bytes = file.body().len();
    let mut shown: Vec<&Section> = sections.iter().collect();
    if let Some(anchor) = anchor {
        match source::resolve(&sections, anchor) {
            Resolved::One(i) => {
                let level = sections[i].level;
                let inside = sections[i + 1..]
                    .iter()
                    .take_while(|s| s.level > level)
                    .count();
                shown = sections[i..=i + inside].iter().collect();
            }
            Resolved::Ambiguous(found) => {
                let mut message = format!("'{anchor}' matches several sections of {reference}:");
                for i in found {
                    let section = &sections[i];
                    message.push_str(&format!(
                        "\n{} (line {})",
                        section.path_text(),
                        section.start
                    ));
                }
                return Err(refused(message));
            }
            Resolved::Missing => {
                return Err(refused(format!("no section '{anchor}' in {reference}")));
            }
        }
    }
    if let Some(depth) = depth {
        shown.retain(|section| section.path.len() <= depth);
    }

    let title = lines
        .get(s.body_start - 1)
        .and_then(|line| line.strip_prefix("# "))
        .unwrap_or("-");
    let mut out = vec![
        format!("path: {}", file.path.display()),
        format!("id: {}", or_dash(&s.id)),
        format!("title: {title}"),
        format!("origin: {}", or_dash(&s.origin)),
        format!("fetched: {}", or_dash(&s.fetched)),
    ];
    for (key, value) in [("kept", &s.kept), ("capture", &s.capture)] {
        if s.keys.iter().any(|k| k == key) {
            out.push(format!("{key}: {}", or_dash(value)));
        }
    }
    out.push(format!(
        "lines: {}-{}",
        s.body_start,
        lines.len().max(s.body_start)
    ));
    out.push(format!("tokens: {}", source::tokens(bytes)));
    out.push(format!("headings: {}", sections.len()));
    let catalog = source::is_catalog(bytes, &sections);
    out.push(format!("catalog: {}", if catalog { "yes" } else { "no" }));
    if let (Some(id), Some(digest)) = (&s.id, &s.digest)
        && let Some(folder) = capture_folder(&root, id, digest)
    {
        out.push(format!("capture folder: {}", folder.display()));
    }
    out.push(String::new());
    out.extend(shown.iter().map(|section| {
        format!(
            "{}-{}\t{} tokens\t{}",
            section.start,
            section.end,
            section.tokens,
            section.path_text()
        )
    }));
    Ok(Output::lines(out))
}

fn today() -> String {
    jiff::Zoned::now().date().to_string()
}

fn state_failure() -> Failure {
    Failure::Config(
        "cannot find the state folder: set XDG_STATE_HOME, or HOME, to an absolute path".into(),
    )
}

/// The line of the first level-1 heading outside fences and its text.
fn capture_title(lines: &[&str]) -> Option<(usize, String)> {
    source::outside_fences(lines)
        .into_iter()
        .find_map(|i| match rank::heading(lines[i]) {
            Some((1, text)) => Some((i + 1, text)),
            _ => None,
        })
}

fn stage(args: &Args, env: &store::Env) -> Result<Output, Failure> {
    let [file] = args.operands(["<file>"])?;
    let origin = args
        .one("--origin")
        .ok_or_else(|| usage("missing --origin \"<url|doc>: <value>\""))?;
    if !source::valid_origin(origin) {
        return Err(usage(format!(
            "invalid --origin '{origin}': write it as \"<url or doc>: <value>\""
        )));
    }
    let fetched = match args.one("--fetched") {
        Some(date) if source::is_date(date) => date.to_string(),
        Some(date) => {
            return Err(usage(format!(
                "--fetched '{date}' is not YYYY-MM-DD, a real date"
            )));
        }
        None => today(),
    };
    let staging = store::staging_dir(env).ok_or_else(state_failure)?;

    let path = Path::new(file);
    let bytes = fs::read(path).map_err(io_failure("read", path))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| refused(format!("{file} is not valid UTF-8, so it is not text")))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    if text.trim().is_empty() {
        return Err(refused(format!("{file} holds only whitespace")));
    }
    let mut capture = text.replace("\r\n", "\n").replace('\r', "\n");
    if !capture.ends_with('\n') {
        capture.push('\n');
    }

    let id = note::mint_ulid().map_err(|e| refused(format!("cannot read /dev/urandom: {e}")))?;
    let folder = staging.join(&id);
    fs::create_dir_all(&staging).map_err(io_failure("create", &staging))?;
    fs::create_dir(&folder).map_err(io_failure("create", &folder))?;
    let record = serde_json::json!({
        "origin": origin,
        "fetched": fetched,
        "capture": "external",
        "sha256": hash::sha256_hex(capture.as_bytes()),
    });
    let capture_path = folder.join("capture.md");
    let written = fs::write(&capture_path, &capture)
        .map_err(io_failure("write", &capture_path))
        .and_then(|()| {
            let record_path = folder.join("stage.json");
            fs::write(&record_path, record.to_string()).map_err(io_failure("write", &record_path))
        });
    if let Err(failure) = written {
        let _ = fs::remove_dir_all(&folder);
        return Err(failure);
    }

    let lines = note::lines(&capture);
    let title = capture_title(&lines);
    let blank = |l: &&str| l.trim().is_empty();
    let last = lines.iter().rposition(|l| !blank(l)).map_or(0, |i| i + 1);
    let first = match &title {
        Some((line, _)) => line + 1,
        None => lines.iter().position(|l| !blank(l)).map_or(1, |i| i + 1),
    };
    let mut out = vec![
        format!("stage: {id}"),
        format!("capture: {}", capture_path.display()),
        format!("lines: {}", lines.len()),
        format!("tokens: {}", source::tokens(capture.len())),
        format!("title: {}", title.as_ref().map_or("-", |(_, text)| text)),
        if first <= last {
            format!("keep: {first}-{last}")
        } else {
            "keep: -".into()
        },
        String::new(),
    ];
    for i in source::outside_fences(&lines) {
        if rank::heading(lines[i]).is_some_and(|(level, _)| level <= 2) {
            out.push(format!("{}\t{}", i + 1, lines[i]));
        }
    }
    Ok(Output::lines(out))
}

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
    rank::heading(line).is_some() || line.starts_with("# ")
}

struct Plan {
    corpus: String,
    name: String,
    ranges: Vec<(usize, usize)>,
    title: Option<String>,
    replace: bool,
}

fn plan(args: &Args) -> Result<(String, Plan), Failure> {
    let [stage, target] = args.operands(["<stage>", "<corpus>/<name>"])?;
    if !note::is_ulid(stage) {
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
    let plan = Plan {
        corpus: corpus.into(),
        name: name.into(),
        ranges,
        title,
        replace: args.replace,
    };
    Ok((stage.into(), plan))
}

/// The body for the kept lines, and whether their headings were demoted.
fn build_body(title: &str, kept: &[&str]) -> (String, bool) {
    let outside = source::outside_fences(kept);
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

fn land(args: &Args, env: &store::Env) -> Result<Output, Failure> {
    let (stage_id, plan) = plan(args)?;
    let root = root(env)?;
    let staged = read_stage(env, &stage_id)?;

    let lines = note::lines(&staged.capture);
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
        None => note::mint_ulid().map_err(|e| refused(format!("cannot read /dev/urandom: {e}")))?,
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
            capture: Some(staged.label.clone()),
        },
        &body,
    );
    if let Some(problem) = source::read(&text).problems.first() {
        return Err(refused(format!(
            "internal error: the rendered source has problems: {problem}"
        )));
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
        let random =
            note::mint_ulid().map_err(|e| refused(format!("cannot read /dev/urandom: {e}")))?;
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
        note::mint_ulid().map_err(|e| refused(format!("cannot read /dev/urandom: {e}")))?;
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
            let guide_id =
                note::mint_ulid().map_err(|e| refused(format!("cannot read /dev/urandom: {e}")))?;
            corpus::new_guide(&guide_id, &note::now_created(), &plan.corpus)
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
