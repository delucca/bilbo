use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::Failure;
use crate::citation::{self, Citation, Document, Ids, Kind, Outcome, Verdict};
use crate::library::reading::{self, LogEntry, Plan};
use crate::library::{corpus, source};
use crate::shared::{frontmatter, store};

pub struct Output {
    /// stderr lines (without "bilbo: "), printed before stdout.
    pub warnings: Vec<String>,
    pub lines: Vec<String>,
    /// Whether any verdict makes `cite` exit 1.
    pub failed: bool,
}

struct Args {
    plans: Vec<String>,
    draft: Option<String>,
}

fn usage(message: impl Into<String>) -> Failure {
    Failure::Usage(message.into())
}

fn refused(message: impl Into<String>) -> Failure {
    Failure::Refused(message.into())
}

impl Args {
    fn parse(args: &[String]) -> Result<Args, Failure> {
        let mut parsed = Args {
            plans: Vec::new(),
            draft: None,
        };
        let mut options_ended = false;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            let is_option = arg.chars().nth(1).is_some_and(|c| !c.is_whitespace());
            if options_ended || !arg.starts_with('-') || !is_option {
                if parsed.draft.is_some() {
                    return Err(usage(format!("unexpected argument '{arg}'")));
                }
                parsed.draft = Some(arg.clone());
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
            if name != "--plan" {
                return Err(usage(format!("unknown option '{name}'")));
            }
            let value = match inline {
                Some(value) => value,
                None => iter
                    .next()
                    .ok_or_else(|| usage("--plan needs a value"))?
                    .clone(),
            };
            if value.is_empty() {
                return Err(usage("--plan needs a value"));
            }
            if !parsed.plans.contains(&value) {
                parsed.plans.push(value);
            }
        }
        Ok(parsed)
    }
}

/// A plan with its read log.
struct Loaded {
    plan: Plan,
    log: Vec<LogEntry>,
}

/// A cited file's body, ready for any number of checks, and its digest when it has one.
struct Prepared {
    document: Document,
    digest: Option<String>,
    body_start: usize,
}

pub fn run(args: &[String], stdin: &mut impl Read, env: &store::Env) -> Result<Output, Failure> {
    let args = Args::parse(args)?;
    let root = store::root(env).map_err(Failure::Config)?;
    let plans_dir = match args.plans.is_empty() {
        true => None,
        false => Some(store::plans_dir(env).ok_or_else(|| {
            Failure::Config(
                "cannot find the state folder: set XDG_STATE_HOME, or HOME, to an absolute path"
                    .into(),
            )
        })?),
    };
    if !root.join("notes").is_dir() && !store::library_dir(&root).is_dir() {
        return Err(refused(format!("no store at {}", root.display())));
    }
    let draft = read_draft(args.draft.as_deref(), stdin)?;
    let plans = match &plans_dir {
        Some(dir) => load_plans(&args.plans, dir, &root)?,
        None => Vec::new(),
    };

    let (citations, notices) = citation::parse(&draft);
    let mut warnings: Vec<String> = notices.iter().map(ToString::to_string).collect();
    if citations.is_empty() {
        warnings.push("no citations found".into());
    }

    let ids = Ids::scan(&root);
    let mut cache: HashMap<PathBuf, Result<Prepared, String>> = HashMap::new();
    let mut failed = false;
    let mut ok = 0;
    let mut lines = Vec::new();
    for c in &citations {
        let (outcome, path) = check_one(c, &ids, &mut cache, &plans);
        failed |= outcome.verdict.fails();
        ok += usize::from(outcome.verdict == Verdict::Ok);
        let anchor = c
            .anchor
            .as_deref()
            .map(|a| format!("#{}", a.replace(['\t', '\n'], " ")))
            .unwrap_or_default();
        lines.push(format!(
            "{}\t{}\t{}{anchor}\t{path}\t{}",
            c.line,
            outcome.verdict.name(),
            c.id,
            outcome.detail
        ));
    }
    lines.push(format!("citations: {} checked, {ok} ok", citations.len()));
    for loaded in &plans {
        lines.extend(coverage_lines(loaded, &ids, &root));
    }
    Ok(Output {
        warnings,
        lines,
        failed,
    })
}

fn read_draft(path: Option<&str>, stdin: &mut impl Read) -> Result<String, Failure> {
    let bytes = match path {
        None | Some("-") => {
            let mut bytes = Vec::new();
            stdin
                .read_to_end(&mut bytes)
                .map_err(|e| refused(format!("cannot read the draft from stdin: {e}")))?;
            bytes
        }
        Some(path) => {
            std::fs::read(path).map_err(|e| refused(format!("cannot read {path}: {e}")))?
        }
    };
    String::from_utf8(bytes).map_err(|_| refused("the draft is not valid UTF-8"))
}

fn load_plans(ids: &[String], dir: &Path, root: &Path) -> Result<Vec<Loaded>, Failure> {
    ids.iter()
        .map(|id| {
            let missing = || refused(format!("no plan '{id}' in {}", dir.display()));
            if !frontmatter::is_ulid(id) {
                return Err(missing());
            }
            let plan = reading::load(dir, id)
                .map_err(refused)?
                .ok_or_else(missing)?;
            if Path::new(&plan.root) != root {
                return Err(refused(format!(
                    "plan {id} was made for the store {}, not {}",
                    plan.root,
                    root.display()
                )));
            }
            let log = reading::load_log(dir, id).map_err(|e| {
                refused(format!(
                    "cannot read {}: {e}",
                    reading::log_path(dir, id).display()
                ))
            })?;
            Ok(Loaded { plan, log })
        })
        .collect()
}

fn prepare(path: &Path) -> Result<Prepared, String> {
    let text = std::fs::read(path)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| format!("cannot read {}", path.display()))?;
    let (body, first_line) = citation::body_of(&text);
    let front = source::read(&text);
    Ok(Prepared {
        document: Document::new(body, first_line),
        digest: front.digest,
        body_start: front.body_start,
    })
}

/// The outcome of one citation, and the path of the cited file or `-`.
fn check_one(
    c: &Citation,
    ids: &Ids,
    cache: &mut HashMap<PathBuf, Result<Prepared, String>>,
    plans: &[Loaded],
) -> (Outcome, String) {
    let target = match ids.resolve(&c.id) {
        Ok(target) => target,
        Err(message) => return (Outcome::id_missing(message), "-".into()),
    };
    let path = target.path.display().to_string();
    let prepared = cache
        .entry(target.path.clone())
        .or_insert_with(|| prepare(&target.path));
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(message) => return (Outcome::id_missing(message.clone()), "-".into()),
    };
    let mut outcome = prepared.document.check(c);
    let reads = matches!(
        outcome.verdict,
        Verdict::Ok | Verdict::TooShort | Verdict::QuoteElsewhere | Verdict::Ambiguous
    );
    if target.kind == Kind::Source
        && reads
        && !plans.is_empty()
        && let Some(detail) = unread(&c.id, prepared, &outcome.spans, plans)
    {
        outcome.verdict = Verdict::Unread;
        outcome.detail = detail;
    }
    (outcome, path)
}

/// Why no match of the quote lies in read lines, `None` when one does.
fn unread(id: &str, now: &Prepared, spans: &[(usize, usize)], plans: &[Loaded]) -> Option<String> {
    let holding: Vec<&Loaded> = plans.iter().filter(|p| p.plan.has_source(id)).collect();
    if holding.is_empty() {
        return Some("the source is in no plan".into());
    }
    let digest = now.digest.as_deref();
    let current: Vec<&Loaded> = holding
        .into_iter()
        .filter(|p| p.plan.is_current(id, digest, now.body_start))
        .collect();
    let Some(digest) = digest.filter(|_| !current.is_empty()) else {
        return Some("the source changed since its plan".into());
    };
    let log: Vec<LogEntry> = current.iter().flat_map(|p| p.log.clone()).collect();
    let ranges = reading::read_ranges(&log, id, digest);
    if spans.iter().any(|&(a, b)| reading::covers(&ranges, a, b)) {
        return None;
    }
    let Some(&(a, b)) = spans.first() else {
        return Some("no line of the quote was read".into());
    };
    let slice = current
        .iter()
        .find_map(|p| p.plan.slice_at(id, a).map(|i| (&p.plan.id, i)));
    Some(match slice {
        Some((plan, i)) if plans.len() > 1 => {
            format!("lines {a}-{b} not read (slice {i} of plan {plan})")
        }
        Some((_, i)) => format!("lines {a}-{b} not read (slice {i})"),
        None => format!("lines {a}-{b} not read, and outside the picked lines"),
    })
}

/// The `coverage:` and `picked:` lines of one plan, sources named as they are now.
fn coverage_lines(loaded: &Loaded, ids: &Ids, root: &Path) -> [String; 2] {
    let plan = &loaded.plan;
    let now = |id: &str| {
        ids.resolve(id)
            .ok()
            .filter(|t| t.kind == Kind::Source)
            .map(|t| t.name.clone())
    };
    let coverage = plan.coverage(&loaded.log).line_with(&plan.id, |run| {
        now(&run.id).unwrap_or_else(|| run.label.clone())
    });
    let place = |pick: &reading::Pick| {
        now(&pick.id)
            .and_then(|name| {
                name.split_once('/')
                    .map(|(corpus, name)| (corpus.to_string(), name.to_string()))
            })
            .unwrap_or_else(|| pick.place())
    };
    let sources_in = |name: &str| {
        corpus::read_sources(&store::library_dir(root).join(name)).map_or(0, |found| found.len())
    };
    [coverage, plan.picked_line(place, sources_in)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, Failure> {
        Args::parse(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn plans_repeat_and_one_file_is_allowed() {
        let parsed = args(&["--plan", "a", "--plan=b", "--plan", "a", "draft.md"])
            .ok()
            .unwrap();
        assert_eq!(parsed.plans, ["a", "b"]);
        assert_eq!(parsed.draft.as_deref(), Some("draft.md"));
        assert_eq!(args(&["-"]).ok().unwrap().draft.as_deref(), Some("-"));
        assert!(args(&["a", "b"]).is_err());
        assert!(args(&["--force"]).is_err());
        assert!(args(&["--plan"]).is_err());
    }
}
