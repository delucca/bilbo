use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use bilbo_store::entries::{Capture, CaptureError, Captured, Entries, Origin, StoreError};
use bilbo_store::id::Id;

use crate::files::Files;
use crate::kind::Kind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub target: PathBuf,
    pub kind: Option<String>,
    pub project: Option<String>,
}

pub struct Plan<'k> {
    /// Canonical.
    pub target: PathBuf,
    /// The kind that runs; `None` when no kind claimed files.
    pub kind: Option<&'k dyn Kind>,
    /// The files it claimed.
    pub files: Vec<PathBuf>,
    /// What the walk could not read.
    pub refused: Vec<(PathBuf, String)>,
    /// The project, when given.
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    New(Id),
    Appended(Id),
    NewGeneration(Id),
    Unchanged(Id),
    Refused(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub path: PathBuf,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub target: PathBuf,
    pub kind: Option<String>,
    /// 0 means nothing to keep.
    pub claimed: u64,
    /// The walk's refusals first, then the claimed files in claim order.
    pub items: Vec<Item>,
    pub records: u64,
    pub raw_bytes: u64,
    pub stored_bytes: u64,
}

impl Output {
    /// For a plan with no file: its refusals, `claimed` 0, zeros. Opens no store.
    #[must_use]
    pub fn unkept(plan: &Plan<'_>) -> Output {
        Output {
            target: plan.target.clone(),
            kind: plan.kind.map(|kind| kind.name().to_string()),
            claimed: 0,
            items: plan
                .refused
                .iter()
                .map(|(path, reason)| Item {
                    path: path.clone(),
                    outcome: Outcome::Refused(reason.clone()),
                })
                .collect(),
            records: 0,
            raw_bytes: 0,
            stored_bytes: 0,
        }
    }

    #[must_use]
    pub fn tally(&self) -> Tally {
        let mut tally = Tally::default();
        for item in &self.items {
            match item.outcome {
                Outcome::New(_) => tally.new += 1,
                Outcome::Appended(_) => tally.appended += 1,
                Outcome::NewGeneration(_) => tally.new_generation += 1,
                Outcome::Unchanged(_) => tally.unchanged += 1,
                Outcome::Refused(_) => tally.refused += 1,
            }
        }
        tally
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub new: u64,
    pub appended: u64,
    pub new_generation: u64,
    pub unchanged: u64,
    pub refused: u64,
}

#[derive(Debug)]
pub enum Error {
    UnknownKind { name: String, known: Vec<String> },
    SeveralKinds { target: PathBuf, names: Vec<String> },
    Target { path: PathBuf, error: io::Error },
    Store(StoreError),
}

pub fn plan<'k>(
    request: &Request,
    kinds: &'k [Box<dyn Kind>],
    files: &dyn Files,
) -> Result<Plan<'k>, Error> {
    let named = match &request.kind {
        Some(name) => {
            let kind = kinds
                .iter()
                .find(|kind| kind.name() == name)
                .ok_or_else(|| Error::UnknownKind {
                    name: name.clone(),
                    known: kinds.iter().map(|kind| kind.name().to_string()).collect(),
                })?;
            Some(kind)
        }
        None => None,
    };
    let target = files
        .canonical(&request.target)
        .map_err(|error| Error::Target {
            path: request.target.clone(),
            error,
        })?;
    let labels: Vec<String> = request.project.iter().cloned().collect();

    if let Some(kind) = named {
        let claims = kind.claims(files, &target);
        return Ok(Plan {
            target,
            kind: Some(&**kind),
            files: claims.files,
            refused: claims.refused,
            labels,
        });
    }

    let mut claiming = Vec::new();
    let mut all_refused: Vec<(PathBuf, String)> = Vec::new();
    for kind in kinds {
        let claims = kind.claims(files, &target);
        for refusal in &claims.refused {
            if !all_refused.contains(refusal) {
                all_refused.push(refusal.clone());
            }
        }
        if !claims.files.is_empty() {
            claiming.push((kind, claims));
        }
    }
    if claiming.len() > 1 {
        return Err(Error::SeveralKinds {
            target,
            names: claiming
                .iter()
                .map(|(kind, _)| kind.name().to_string())
                .collect(),
        });
    }
    let (kind, claimed, refused) = match claiming.pop() {
        Some((kind, claims)) => (Some(&**kind), claims.files, claims.refused),
        None => (None, Vec::new(), all_refused),
    };
    Ok(Plan {
        target,
        kind,
        files: claimed,
        refused,
        labels,
    })
}

enum Done {
    Kept(Captured),
    Refused(String),
}

pub fn keep(
    plan: &Plan<'_>,
    files: &dyn Files,
    entries: &mut dyn Entries,
) -> Result<Output, Error> {
    let mut output = Output::unkept(plan);
    let Some(kind) = plan.kind else {
        return Ok(output);
    };
    output.claimed = u64::try_from(plan.files.len()).unwrap_or(u64::MAX);
    for path in &plan.files {
        let outcome = match keep_file(kind, &plan.labels, path, files, entries)? {
            Done::Refused(reason) => Outcome::Refused(reason),
            Done::Kept(captured) => {
                let (outcome, kept) = match captured {
                    Captured::New(kept) => (Outcome::New(kept.id), Some(kept)),
                    Captured::Appended(kept) => (Outcome::Appended(kept.id), Some(kept)),
                    Captured::NewGeneration(kept) => (Outcome::NewGeneration(kept.id), Some(kept)),
                    Captured::Unchanged(id) => (Outcome::Unchanged(id), None),
                };
                if let Some(kept) = kept {
                    output.records += kept.records.count;
                    output.raw_bytes += kept.raw_bytes;
                    output.stored_bytes += kept.stored_bytes;
                }
                outcome
            }
        };
        output.items.push(Item {
            path: path.clone(),
            outcome,
        });
    }
    Ok(output)
}

fn keep_file(
    kind: &dyn Kind,
    labels: &[String],
    path: &Path,
    files: &dyn Files,
    entries: &mut dyn Entries,
) -> Result<Done, Error> {
    let Some(origin) = Origin::file(path) else {
        return Ok(Done::Refused("its path is not UTF-8".into()));
    };
    let bytes = match files.read(path) {
        Ok(bytes) => bytes,
        Err(error) => return Ok(Done::Refused(format!("cannot read: {error}"))),
    };
    let ends = match kind.split(&bytes) {
        Ok(ends) => ends,
        Err(refusal) => return Ok(Done::Refused(refusal.to_string())),
    };
    let proposal = kind.propose(&bytes);
    let input = Capture {
        origin: &origin,
        kind: kind.name(),
        id: proposal.id,
        labels,
        bytes: &bytes,
        ends: &ends,
    };
    match entries.capture(&input) {
        Ok(captured) => Ok(Done::Kept(captured)),
        Err(CaptureError::Store(error)) => Err(error.into()),
        Err(
            error @ (CaptureError::IdHeld { .. }
            | CaptureError::Records(_)
            | CaptureError::TooLarge),
        ) => Ok(Done::Refused(error.to_string())),
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKind { name, known } => {
                write!(f, "unknown kind '{name}'; kinds: {}", known.join(", "))
            }
            Self::SeveralKinds { target, names } => write!(
                f,
                "several kinds claim files under {}: {}; choose one with --kind",
                target.display(),
                list(names)
            ),
            Self::Target { path, error } => {
                write!(f, "cannot read {}: {error}", path.display())
            }
            Self::Store(error) => write!(f, "{error}"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Target { error, .. } => Some(error),
            Self::Store(error) => Some(error),
            Self::UnknownKind { .. } | Self::SeveralKinds { .. } => None,
        }
    }
}

impl From<StoreError> for Error {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

fn list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::{Path, PathBuf};

    use bilbo_store::entries::StoreError;

    use super::{Error, Output, Plan, Request, plan};
    use crate::fake::Suffix;
    use crate::files::{Files, Memory};
    use crate::kind::Kind;

    fn registry(kinds: Vec<Suffix>) -> Vec<Box<dyn Kind>> {
        kinds
            .into_iter()
            .map(|kind| Box::new(kind) as Box<dyn Kind>)
            .collect()
    }

    fn request(target: &str, kind: Option<&str>) -> Request {
        Request {
            target: PathBuf::from(target),
            kind: kind.map(str::to_string),
            project: None,
        }
    }

    fn tree() -> Memory {
        Memory::new()
            .file("/t/a.one", "a")
            .file("/t/sub/b.one", "b")
            .file("/t/c.two", "c")
            .file("/t/d.txt", "d")
    }

    fn running<'a>(plan: &'a Plan<'_>) -> Option<&'a str> {
        plan.kind.map(Kind::name)
    }

    #[test]
    fn detection_runs_the_one_kind_that_claims_files() {
        let kinds = registry(vec![Suffix::new("one", ".one"), Suffix::new("two", ".two")]);
        let plan = plan(&request("/t/sub", None), &kinds, &tree()).expect("plan");
        assert_eq!(running(&plan), Some("one"));
        assert_eq!(plan.files, vec![PathBuf::from("/t/sub/b.one")]);
        assert_eq!(plan.target, PathBuf::from("/t/sub"));
    }

    #[test]
    fn detection_with_no_claiming_kind_has_nothing_to_keep() {
        let mut one = Suffix::new("one", ".one");
        one.refused
            .push((PathBuf::from("/t/x"), "cannot read".into()));
        let mut two = Suffix::new("two", ".two");
        two.refused
            .push((PathBuf::from("/t/x"), "cannot read".into()));
        two.refused
            .push((PathBuf::from("/t/y"), "cannot read".into()));
        let kinds = registry(vec![one, two]);
        let plan = plan(&request("/t/d.txt", None), &kinds, &tree()).expect("plan");
        assert!(plan.kind.is_none());
        assert!(plan.files.is_empty());
        assert_eq!(
            plan.refused,
            vec![
                (PathBuf::from("/t/x"), "cannot read".to_string()),
                (PathBuf::from("/t/y"), "cannot read".to_string()),
            ]
        );
        let output = Output::unkept(&plan);
        assert_eq!(output.claimed, 0);
        assert_eq!(output.kind, None);
    }

    #[test]
    fn detection_with_several_claiming_kinds_names_them() {
        let kinds = registry(vec![
            Suffix::new("one", ".one"),
            Suffix::new("none", ".none"),
            Suffix::new("two", ".two"),
        ]);
        let error = plan(&request("/t", None), &kinds, &tree())
            .err()
            .expect("several kinds claim files");
        assert_eq!(
            error.to_string(),
            "several kinds claim files under /t: one and two; choose one with --kind"
        );
    }

    #[test]
    fn only_the_running_kinds_refusals_are_kept_when_one_kind_claims() {
        let mut quiet = Suffix::new("two", ".two");
        quiet.refused = vec![(PathBuf::from("/t/locked"), "denied".to_owned())];
        let mut running_kind = Suffix::new("one", ".one");
        running_kind.refused = vec![(PathBuf::from("/t/other"), "unreadable".to_owned())];
        let kinds = registry(vec![running_kind, quiet]);
        let files = Memory::new().file("/t/a.one", "x");
        let plan = plan(&request("/t", None), &kinds, &files).expect("plan");
        assert_eq!(running(&plan), Some("one"));
        assert_eq!(
            plan.refused,
            vec![(PathBuf::from("/t/other"), "unreadable".to_owned())]
        );

        let none = Memory::new().file("/t/a.txt", "x");
        let plan = self::plan(&request("/t", None), &kinds, &none).expect("plan");
        assert_eq!(plan.refused.len(), 2);
    }

    #[test]
    fn a_named_kind_runs_alone_even_when_others_claim() {
        let kinds = registry(vec![Suffix::new("one", ".one"), Suffix::new("two", ".two")]);
        let plan = plan(&request("/t", Some("two")), &kinds, &tree()).expect("plan");
        assert_eq!(running(&plan), Some("two"));
        assert_eq!(plan.files, vec![PathBuf::from("/t/c.two")]);

        let empty = self::plan(&request("/t/d.txt", Some("two")), &kinds, &tree()).expect("plan");
        assert_eq!(running(&empty), Some("two"));
        assert!(empty.files.is_empty());
    }

    #[test]
    fn an_unknown_kind_names_the_known_ones() {
        let kinds = registry(vec![Suffix::new("one", ".one"), Suffix::new("two", ".two")]);
        let error = plan(&request("/nowhere", Some("pdf")), &kinds, &tree())
            .err()
            .expect("unknown kind");
        assert_eq!(error.to_string(), "unknown kind 'pdf'; kinds: one, two");
    }

    #[test]
    fn a_target_that_cannot_be_read_is_an_error() {
        let kinds = registry(vec![Suffix::new("one", ".one")]);
        let error = plan(&request("/nowhere", None), &kinds, &tree())
            .err()
            .expect("missing target");
        assert!(matches!(&error, Error::Target { path, .. } if path == Path::new("/nowhere")));
    }

    #[test]
    fn kinds_see_the_canonical_target_and_the_project_becomes_the_label() {
        struct Links(Memory);
        impl Files for Links {
            fn canonical(&self, path: &Path) -> io::Result<PathBuf> {
                if path == Path::new("/link") {
                    Ok(PathBuf::from("/t/sub"))
                } else {
                    self.0.canonical(path)
                }
            }
            fn node(&self, path: &Path) -> io::Result<crate::files::Node> {
                self.0.node(path)
            }
            fn list(&self, folder: &Path) -> io::Result<Vec<std::ffi::OsString>> {
                self.0.list(folder)
            }
            fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
                self.0.read(path)
            }
        }
        let kinds = registry(vec![Suffix::new("one", ".one")]);
        let mut named = request("/link", None);
        named.project = Some("p".into());
        let plan = plan(&named, &kinds, &Links(tree())).expect("plan");
        assert_eq!(plan.target, PathBuf::from("/t/sub"));
        assert_eq!(plan.files, vec![PathBuf::from("/t/sub/b.one")]);
        assert_eq!(plan.labels, vec!["p".to_string()]);
    }

    #[test]
    fn the_use_case_errors_say_what_went_wrong() {
        let names = |names: &[&str]| names.iter().map(|n| (*n).to_string()).collect::<Vec<_>>();
        let several = |list: &[&str]| {
            Error::SeveralKinds {
                target: PathBuf::from("/t"),
                names: names(list),
            }
            .to_string()
        };
        assert_eq!(
            Error::UnknownKind {
                name: "pdf".into(),
                known: names(&["markdown", "jsonl"]),
            }
            .to_string(),
            "unknown kind 'pdf'; kinds: markdown, jsonl"
        );
        assert_eq!(
            several(&["a", "b"]),
            "several kinds claim files under /t: a and b; choose one with --kind"
        );
        assert_eq!(
            several(&["a", "b", "c"]),
            "several kinds claim files under /t: a, b and c; choose one with --kind"
        );
        assert_eq!(
            Error::Target {
                path: PathBuf::from("/t/x"),
                error: io::Error::from(io::ErrorKind::NotFound),
            }
            .to_string(),
            format!(
                "cannot read /t/x: {}",
                io::Error::from(io::ErrorKind::NotFound)
            )
        );
        assert_eq!(
            Error::from(StoreError::ReadOnly).to_string(),
            "the store is open read-only"
        );
    }
}
