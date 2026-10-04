use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::citation::{self, Document, Verdict};
use crate::corpus::{self, SourceFile};
use crate::markdown::{self, Resolved, Section};
use crate::source::{self, Frontmatter};
use crate::store::{self, EntryKind};
use crate::{Failure, frontmatter, hash, note, plan as reading};

const VALUE_OPTIONS: [&str; 9] = [
    "--depth",
    "--origin",
    "--fetched",
    "--keep",
    "--title",
    "--budget-tokens",
    "--slice-bytes",
    "--slice-lines",
    "--part",
];

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
    force: bool,
    html: bool,
}

impl Args {
    fn parse(args: &[String]) -> Result<Args, Failure> {
        let mut parsed = Args {
            positional: Vec::new(),
            options: Vec::new(),
            replace: false,
            force: false,
            html: false,
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
            } else if name == "--html" {
                if inline.is_some() {
                    return Err(usage("--html takes no value"));
                }
                parsed.html = true;
            } else if name == "--force" {
                if inline.is_some() {
                    return Err(usage("--force takes no value"));
                }
                parsed.force = true;
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
        let given = given
            .chain(self.replace.then_some("--replace"))
            .chain(self.force.then_some("--force"))
            .chain(self.html.then_some("--html"));
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
            args.only(&["--origin", "--fetched", "--html"])?;
            stage(&args, env)
        }
        Some("land") => {
            args.only(&["--keep", "--title", "--replace", "--force"])?;
            land(&args, env)
        }
        Some("plan") => {
            args.only(&["--budget-tokens", "--slice-bytes", "--slice-lines"])?;
            plan_picks(&args, env)
        }
        Some("read") => {
            args.only(&["--part"])?;
            read_slices(&args, env)
        }
        Some(name) => {
            args.only(&[])?;
            if let Some(extra) = args.positional.get(1) {
                return Err(usage(format!("unexpected argument '{extra}'")));
            }
            if !store::is_topic(name) {
                return Err(usage(format!(
                    "invalid corpus '{name}': use segments of a-z and 0-9 joined by single hyphens"
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
    markdown::outline(&markdown::lines(&file.text), file.source.body_start)
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
        markdown::tokens(bytes),
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
        for (i, line) in markdown::lines(text)
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
        None if frontmatter::is_ulid(reference) => Ok(Reference::Id(reference.into())),
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

/// The source with this id, found through the frontmatter scan, and read whole.
fn source_by_id(ids: &citation::Ids, id: &str) -> Result<SourceFile, Failure> {
    let target = ids
        .resolve(id)
        .map_err(|message| refused(format!("{id}: {message}")))?;
    if target.kind != citation::Kind::Source {
        return Err(refused(format!(
            "{id} is the id of {}, not of a source",
            target.path.display()
        )));
    }
    let name = target
        .name
        .rsplit_once('/')
        .map_or(&*target.name, |(_, n)| n);
    corpus::read_source(&target.path, name.to_string())
        .ok_or_else(|| refused(format!("cannot read {}", target.path.display())))
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

/// The reference and the anchor after its `#`, if any.
fn split_anchor(reference: &str) -> Result<(&str, Option<&str>), Failure> {
    match reference.split_once('#') {
        Some((_, "")) => Err(usage("the anchor after '#' is empty")),
        Some((reference, anchor)) => Ok((reference, Some(anchor))),
        None => Ok((reference, None)),
    }
}

/// The section the anchor names, or the refusal that lists the sections it matches or says it matches none.
fn section_for(sections: &[Section], reference: &str, anchor: &str) -> Result<usize, Failure> {
    match markdown::resolve(sections, anchor) {
        Resolved::One(i) => Ok(i),
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
            Err(refused(message))
        }
        Resolved::Missing => Err(refused(format!("no section '{anchor}' in {reference}"))),
    }
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
    let (reference, anchor) = split_anchor(reference)?;
    let parsed = parse_reference(reference)?;
    let root = root(env)?;
    let file = find(&root, &parsed)?;

    let s = &file.source;
    let lines = markdown::lines(&file.text);
    let sections = sections(&file);
    let bytes = file.body().len();
    let mut shown: Vec<&Section> = sections.iter().collect();
    if let Some(anchor) = anchor {
        let i = section_for(&sections, reference, anchor)?;
        let level = sections[i].level;
        let inside = sections[i + 1..]
            .iter()
            .take_while(|s| s.level > level)
            .count();
        shown = sections[i..=i + inside].iter().collect();
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
    out.push(format!("tokens: {}", markdown::tokens(bytes)));
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

fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A whole number of `min` or more, and of `max` or less when there is a limit.
fn whole(name: &str, value: &str, min: usize, max: Option<usize>) -> Result<usize, Failure> {
    let rule = match max {
        Some(max) => format!("from {} to {}", grouped(min), grouped(max)),
        None => format!("of {} or more", grouped(min)),
    };
    value
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| value.parse::<usize>().ok())
        .flatten()
        .filter(|n| *n >= min && max.is_none_or(|max| *n <= max))
        .ok_or_else(|| usage(format!("{name} '{value}' is not a whole number {rule}")))
}

fn plan_options(args: &Args) -> Result<reading::Options, Failure> {
    let mut options = reading::Options::default();
    if let Some(value) = args.one("--budget-tokens") {
        options.budget_tokens = whole("--budget-tokens", value, reading::MIN_BUDGET_TOKENS, None)?;
    }
    if let Some(value) = args.one("--slice-bytes") {
        options.slice_bytes = whole(
            "--slice-bytes",
            value,
            reading::MIN_SLICE_BYTES,
            Some(reading::MAX_SLICE_BYTES),
        )?;
    }
    if let Some(value) = args.one("--slice-lines") {
        options.slice_lines = Some(whole(
            "--slice-lines",
            value,
            reading::MIN_SLICE_LINES,
            None,
        )?);
    }
    Ok(options)
}

fn plan_picks(args: &Args, env: &store::Env) -> Result<Output, Failure> {
    let references = &args.positional[1..];
    if references.is_empty() {
        return Err(usage("missing <ref>"));
    }
    let options = plan_options(args)?;
    let root = root(env)?;
    let plans = store::plans_dir(env).ok_or_else(state_failure)?;

    let mut files: Vec<SourceFile> = Vec::new();
    let mut picks: Vec<reading::Pick> = Vec::new();
    for reference in references {
        let (name, anchor) = split_anchor(reference)?;
        let file = find(&root, &parse_reference(name)?)?;
        let label = label(&file);
        let lines = markdown::lines(&file.text);
        let sections = sections(&file);
        let (Some(id), Some(digest)) = (&file.source.id, &file.source.digest) else {
            return Err(refused(format!(
                "{label} has no valid id or digest; run bilbo check"
            )));
        };
        let (start, end) = match anchor {
            Some(anchor) => {
                let section = &sections[section_for(&sections, name, anchor)?];
                (section.start, section.end)
            }
            None if source::is_catalog(file.body().len(), &sections) => {
                return Err(refused(format!(
                    "{label} is a catalog, too big to read whole: pick a section as '{label}#<anchor>'; bilbo library show {label} lists them"
                )));
            }
            None => (
                file.source.body_start,
                lines.len().max(file.source.body_start),
            ),
        };
        picks.push(reading::Pick {
            reference: reference.clone(),
            id: id.clone(),
            corpus: label.split('/').next().unwrap_or_default().into(),
            name: file.name.clone(),
            start,
            end,
            digest: digest.clone(),
            body_start: file.source.body_start,
        });
        files.push(file);
    }
    if let Some((first, second)) = reading::overlap(&picks) {
        return Err(usage(format!(
            "'{}' and '{}' overlap: pick each line once",
            picks[first].reference, picks[second].reference
        )));
    }

    let lines: Vec<Vec<&str>> = files.iter().map(|f| markdown::lines(&f.text)).collect();
    let outlines: Vec<Vec<Section>> = files.iter().map(sections).collect();
    let materials: Vec<reading::Material> = lines
        .iter()
        .zip(&outlines)
        .map(|(lines, sections)| reading::Material { lines, sections })
        .collect();
    let id =
        frontmatter::mint_ulid().map_err(|e| refused(format!("cannot read /dev/urandom: {e}")))?;
    let plan = reading::build(
        id,
        root.display().to_string(),
        jiff::Timestamp::now().to_string(),
        options,
        picks,
        &materials,
    );

    reading::prune(&plans, std::time::SystemTime::now());
    reading::save(&plans, &plan).map_err(io_failure("write", &plans))?;

    let partitions = plan.partitions();
    let mut out = vec![
        format!("plan: {}", plan.id),
        format!("picks: {}", plan.picks.len()),
        format!("slices: {}", plan.slices.len()),
        format!("tokens: {}", plan.tokens()),
        format!("partitions: {}", partitions.len()),
    ];
    out.extend(partitions.iter().map(|p| {
        format!(
            "partition {}: slices {}-{}, {} tokens",
            p.number, p.first, p.last, p.tokens
        )
    }));
    out.push(String::new());
    for (i, slice) in plan.slices.iter().enumerate() {
        let pick = &plan.picks[slice.pick];
        let path = reading::heading_path(&outlines[slice.pick], slice.start);
        out.push(format!(
            "{}\t{}\t{}\t{}-{}\t{} tokens\t{}",
            i + 1,
            slice.partition,
            pick.label(),
            slice.start,
            slice.end,
            slice.tokens,
            path.as_deref().unwrap_or("-")
        ));
    }
    Ok(Output::lines(out))
}

/// `<k>/<n>` with `n` from 2 to the most parts and `k` from 1 to `n`.
fn parse_part(value: &str) -> Result<(usize, usize), Failure> {
    let number = |s: &str| {
        s.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| s.parse::<usize>().ok())
            .flatten()
    };
    value
        .split_once('/')
        .and_then(|(k, n)| Some((number(k)?, number(n)?)))
        .filter(|&(k, n)| (2..=reading::MAX_PARTS).contains(&n) && (1..=n).contains(&k))
        .ok_or_else(|| {
            usage(format!(
                "--part '{value}' is not <k>/<n> with n from 2 to {} and k from 1 to n",
                reading::MAX_PARTS
            ))
        })
}

fn read_slices(args: &Args, env: &store::Env) -> Result<Output, Failure> {
    let operands = &args.positional[1..];
    let Some(plan_id) = operands.first() else {
        return Err(usage("missing <plan>"));
    };
    if !frontmatter::is_ulid(plan_id) {
        return Err(usage(format!("'{plan_id}' is not a plan id")));
    }
    if operands.len() == 1 {
        return Err(usage("missing <slice>"));
    }
    let numbers = operands[1..]
        .iter()
        .map(|value| {
            value
                .bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| value.parse::<usize>().ok().filter(|n| *n >= 1))
                .flatten()
                .ok_or_else(|| usage(format!("'{value}' is not a slice number")))
        })
        .collect::<Result<Vec<usize>, Failure>>()?;
    let part = args.one("--part").map(parse_part).transpose()?;

    let root = root(env)?;
    let plans = store::plans_dir(env).ok_or_else(state_failure)?;
    let plan = reading::load(&plans, plan_id)
        .map_err(refused)?
        .ok_or_else(|| refused(format!("no plan '{plan_id}' in {}", plans.display())))?;
    if Path::new(&plan.root) != root {
        return Err(refused(format!(
            "plan {plan_id} was made for the store {}, not {}",
            plan.root,
            root.display()
        )));
    }
    if let Some(n) = numbers.iter().find(|n| **n > plan.slices.len()) {
        return Err(usage(format!(
            "slice {n} is not in plan {plan_id}, which has {} slices",
            plan.slices.len()
        )));
    }

    let ids = citation::Ids::scan(&root);
    let mut sources: Vec<(usize, SourceFile)> = Vec::new();
    let mut rendered = Vec::new();
    for &number in &numbers {
        let pick_index = plan.slices[number - 1].pick;
        let pick = &plan.picks[pick_index];
        if !sources.iter().any(|(i, _)| *i == pick_index) {
            let file = source_by_id(&ids, &pick.id).map_err(|failure| match failure {
                Failure::Refused(message) => refused(format!(
                    "{}: {message}; make a new plan with bilbo library plan",
                    pick.label()
                )),
                other => other,
            })?;
            if file.source.digest.as_deref() != Some(pick.digest.as_str())
                || file.source.body_start != pick.body_start
            {
                return Err(refused(format!(
                    "{} changed since plan {plan_id}; make a new plan with bilbo library plan",
                    pick.label()
                )));
            }
            sources.push((pick_index, file));
        }
        let file = &sources.iter().find(|(i, _)| *i == pick_index).unwrap().1;
        let lines = markdown::lines(&file.text);
        let outline = sections(file);
        let material = reading::Material {
            lines: &lines,
            sections: &outline,
        };
        rendered
            .push(reading::render(&plan, number, part, &label(file), &material).map_err(usage)?);
    }

    let bytes: usize = rendered.iter().map(reading::Rendered::bytes).sum();
    if numbers.len() > 1 && bytes > plan.options.slice_bytes {
        let named: Vec<String> = numbers.iter().map(usize::to_string).collect();
        return Err(usage(format!(
            "slices {} print {} bytes together, over the limit of {} bytes for one read; read fewer slices per call",
            named.join(", "),
            grouped(bytes),
            grouped(plan.options.slice_bytes)
        )));
    }

    let entries: Vec<reading::LogEntry> = numbers
        .iter()
        .zip(&rendered)
        .map(|(&number, run)| {
            reading::entry(
                number,
                &plan.picks[plan.slices[number - 1].pick],
                run.start,
                run.end,
            )
        })
        .collect();
    reading::append_log(&plans, plan_id, &entries)
        .map_err(io_failure("write", &reading::log_path(&plans, plan_id)))?;
    Ok(Output::lines(
        rendered.into_iter().flat_map(|run| run.lines).collect(),
    ))
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
    markdown::outside_fences(lines)
        .into_iter()
        .find_map(|i| match markdown::heading(lines[i]) {
            Some((1, text)) => Some((i + 1, text)),
            _ => None,
        })
}

fn is_url(arg: &str) -> bool {
    arg.starts_with("http://") || arg.starts_with("https://")
}

/// Usage errors of a URL argument, found before any request.
fn check_url(args: &Args, url: &str) -> Result<(), Failure> {
    for (given, name) in [
        (args.one("--origin").is_some(), "--origin"),
        (args.one("--fetched").is_some(), "--fetched"),
        (args.html, "--html"),
    ] {
        if given {
            return Err(usage(format!(
                "{name} does not apply to a URL: bilbo sets the origin and the date, and reads the media type"
            )));
        }
    }
    if let Some((_, fragment)) = url.split_once('#') {
        return Err(usage(format!(
            "the URL '{url}' holds the fragment '#{fragment}': stage the whole page, without it"
        )));
    }
    if url
        .chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '\\')
    {
        return Err(usage(format!(
            "the URL '{url}' holds whitespace, '\"' or '\\'"
        )));
    }
    Ok(())
}

/// `<letters>://` with a scheme bilbo does not fetch.
fn other_scheme(arg: &str) -> Option<&str> {
    let (scheme, _) = arg.split_once("://")?;
    (!scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphabetic())).then_some(scheme)
}

/// LF line endings and a final newline.
fn normalize(text: &str) -> String {
    let mut capture = text.replace("\r\n", "\n").replace('\r', "\n");
    if !capture.ends_with('\n') {
        capture.push('\n');
    }
    capture
}

/// What a stage holds before it is written.
struct Capture {
    text: String,
    origin: String,
    fetched: String,
    label: &'static str,
    raw: Option<Vec<u8>>,
    fetch: Option<serde_json::Value>,
    /// `media type:` and `final url:` lines, for a URL.
    fetch_lines: Vec<String>,
    page: Option<crate::html::Conversion>,
}

fn utf8_text(bytes: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(bytes).ok()?;
    Some(text.strip_prefix('\u{feff}').unwrap_or(text))
}

fn capture_url(url: &str) -> Result<Capture, Failure> {
    use crate::fetch::{self, Kind};
    let answer = fetch::get(url).map_err(refused)?;
    let fetched_at = jiff::Zoned::now();
    let media = answer.media_type.clone();
    let kind = fetch::kind(media.as_deref(), &answer.bytes);
    let (text, page) = match kind {
        Kind::Pdf => {
            return Err(refused(format!(
                "{url} is a PDF; extract its text with a PDF tool and stage that text file with --origin \"url: {url}\""
            )));
        }
        Kind::Other(media) => {
            return Err(refused(format!(
                "{url} answered {media}, which bilbo does not stage: it takes HTML and text"
            )));
        }
        Kind::Html | Kind::Text => {
            let text = utf8_text(&answer.bytes)
                .ok_or_else(|| refused(format!("{url} is not valid UTF-8, so it is not text")))?;
            if kind == Kind::Html {
                let page = crate::html::convert(text);
                if page.markdown.trim().is_empty() {
                    return Err(refused(format!("{url} converted to no text")));
                }
                (normalize(&page.markdown), Some(page))
            } else {
                if text.trim().is_empty() {
                    return Err(refused(format!("{url} holds only whitespace")));
                }
                (normalize(text), None)
            }
        }
    };
    let mut fetch_lines = vec![format!("media type: {}", media.as_deref().unwrap_or("-"))];
    if answer.redirected {
        fetch_lines.push(format!("final url: {}", answer.final_url));
    }
    let fetch = serde_json::json!({
        "url": url,
        "final_url": answer.final_url,
        "status": answer.status,
        "media_type": media,
        "fetched_at": fetched_at.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        "converter": page
            .as_ref()
            .map(|_| format!("bilbo {}", env!("CARGO_PKG_VERSION"))),
    });
    Ok(Capture {
        text,
        origin: format!("url: {url}"),
        fetched: today(),
        label: "fetched",
        raw: Some(answer.bytes),
        fetch: Some(fetch),
        fetch_lines,
        page,
    })
}

fn capture_file(args: &Args, file: &str) -> Result<Capture, Failure> {
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
    let path = Path::new(file);
    let bytes = fs::read(path).map_err(io_failure("read", path))?;
    let text = utf8_text(&bytes)
        .ok_or_else(|| refused(format!("{file} is not valid UTF-8, so it is not text")))?;
    if text.trim().is_empty() {
        return Err(refused(format!("{file} holds only whitespace")));
    }
    let (text, page) = if args.html {
        let page = crate::html::convert(text);
        if page.markdown.trim().is_empty() {
            return Err(refused(format!("{file} converted to no text")));
        }
        (normalize(&page.markdown), Some(page))
    } else {
        (normalize(text), None)
    };
    Ok(Capture {
        text,
        origin: origin.to_string(),
        fetched,
        label: "external",
        raw: args.html.then_some(bytes),
        fetch: None,
        fetch_lines: Vec::new(),
        page,
    })
}

/// The `origin` of every source in the library is read, and nothing is written.
fn existing_sources(root: &Path, origin: &str) -> Result<Vec<String>, Failure> {
    let library = store::library_dir(root);
    let mut found = Vec::new();
    for (corpus, dir) in corpus::corpus_dirs(root).map_err(io_failure("read", &library))? {
        for (name, found_origin) in corpus::read_origins(&dir).map_err(io_failure("read", &dir))? {
            if found_origin.as_deref() == Some(origin) {
                found.push(format!("existing: {corpus}/{name}"));
            }
        }
    }
    Ok(found)
}

const LOST_LINES: usize = 10;

fn lost_heading_warnings(page: &crate::html::Conversion) -> Vec<String> {
    let lost = crate::html::lost_headings(page);
    let mut out: Vec<String> = lost
        .iter()
        .take(LOST_LINES)
        .map(|(level, text)| {
            format!("heading lost: <h{level}> '{text}' is not a heading in the capture")
        })
        .collect();
    if lost.len() > LOST_LINES {
        out.push(format!("heading lost: {} more", lost.len() - LOST_LINES));
    }
    out
}

fn stage(args: &Args, env: &store::Env) -> Result<Output, Failure> {
    let [target] = args.operands(["<url>|<file>"])?;
    let url = is_url(target);
    if url {
        check_url(args, target)?;
    } else if let Some(scheme) = other_scheme(target) {
        return Err(usage(format!(
            "cannot fetch '{target}': bilbo fetches http and https, not {scheme}"
        )));
    }
    let staging = store::staging_dir(env).ok_or_else(state_failure)?;
    let captured = if url {
        capture_url(target)?
    } else {
        capture_file(args, target)?
    };
    let root = root(env)?;
    let existing = existing_sources(&root, &captured.origin)?;
    let capture = &captured.text;
    let lines = markdown::lines(capture);

    let content = captured
        .page
        .as_ref()
        .and_then(|p| p.content.as_deref())
        .and_then(|c| crate::html::content_lines(capture, c));
    let title = content
        .and_then(|(a, b)| {
            markdown::outside_fences(&lines)
                .into_iter()
                .filter(|i| (a..=b).contains(&(i + 1)))
                .find_map(|i| match markdown::heading(lines[i]) {
                    Some((1, text)) => Some((i + 1, text)),
                    _ => None,
                })
        })
        .or_else(|| capture_title(&lines));
    let blank = |l: &&str| l.trim().is_empty();
    let last = match content {
        Some((_, b)) => lines[..b]
            .iter()
            .rposition(|l| !blank(l))
            .map_or(0, |i| i + 1),
        None => lines.iter().rposition(|l| !blank(l)).map_or(0, |i| i + 1),
    };
    let first = match (&title, content) {
        (Some((line, _)), None) => line + 1,
        (Some((line, _)), Some((a, b))) if (a..=b).contains(line) => line + 1,
        (_, Some((a, _))) => a,
        (None, None) => lines.iter().position(|l| !blank(l)).map_or(1, |i| i + 1),
    };

    let id =
        frontmatter::mint_ulid().map_err(|e| refused(format!("cannot read /dev/urandom: {e}")))?;
    let folder = staging.join(&id);
    fs::create_dir_all(&staging).map_err(io_failure("create", &staging))?;
    fs::create_dir(&folder).map_err(io_failure("create", &folder))?;
    let record = serde_json::json!({
        "origin": captured.origin,
        "fetched": captured.fetched,
        "capture": captured.label,
        "sha256": hash::sha256_hex(capture.as_bytes()),
    });
    let capture_path = folder.join("capture.md");
    let written = (|| {
        fs::write(&capture_path, capture).map_err(io_failure("write", &capture_path))?;
        if let Some(raw) = &captured.raw {
            let raw_path = folder.join("raw");
            fs::write(&raw_path, raw).map_err(io_failure("write", &raw_path))?;
        }
        if let Some(fetch) = &captured.fetch {
            let fetch_path = folder.join("fetch.json");
            fs::write(&fetch_path, fetch.to_string()).map_err(io_failure("write", &fetch_path))?;
        }
        let record_path = folder.join("stage.json");
        fs::write(&record_path, record.to_string()).map_err(io_failure("write", &record_path))
    })();
    if let Err(failure) = written {
        let _ = fs::remove_dir_all(&folder);
        return Err(failure);
    }

    let mut out = vec![
        format!("stage: {id}"),
        format!("capture: {}", capture_path.display()),
    ];
    if captured.raw.is_some() {
        out.push(format!("raw: {}", folder.join("raw").display()));
    }
    out.extend(captured.fetch_lines.iter().cloned());
    if captured.page.is_some() {
        out.push(match content {
            Some((a, b)) => format!("content: {a}-{b}"),
            None => "content: -".into(),
        });
    }
    out.extend(existing);
    out.push(format!("lines: {}", lines.len()));
    out.push(format!("tokens: {}", markdown::tokens(capture.len())));
    out.push(format!(
        "title: {}",
        title.as_ref().map_or("-", |(_, text)| text)
    ));
    out.push(if first <= last {
        format!("keep: {first}-{last}")
    } else {
        "keep: -".into()
    });
    out.push(String::new());
    for i in markdown::outside_fences(&lines) {
        if markdown::heading(lines[i]).is_some_and(|(level, _)| level <= 2) {
            out.push(format!("{}\t{}", i + 1, lines[i]));
        }
    }
    let mut warnings = source::capture_warnings(&lines);
    if let Some(page) = &captured.page {
        warnings.extend(lost_heading_warnings(page));
    }
    Ok(Output {
        warnings,
        lines: out,
    })
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

fn land(args: &Args, env: &store::Env) -> Result<Output, Failure> {
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
