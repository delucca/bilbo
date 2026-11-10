pub mod outline;
pub mod page;
pub mod refs;

use std::error::Error as StdError;
use std::fmt;
use std::time::SystemTime;

use bilbo_settings::key::{About as KeyAbout, Default, Key};
use bilbo_store::entries::{Entries, Generation, Meta, Origin, RecordRange, StoreError};
use bilbo_store::id::Id;

use crate::reader::Reader;
use crate::tokens::Tokens;
use outline::Line;
use page::{Block, Locators};
use refs::{Locator, Ref, Target, last};

const LEAST: usize = 256;
/// The most a budget can be: what a TOML integer holds.
const MOST: usize = usize::MAX >> 1;
const ROUND: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Budget(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfRange;

impl Budget {
    pub fn tokens(self) -> usize {
        self.0
    }

    pub(crate) fn holding(tokens: usize) -> Budget {
        let round = tokens.max(1).checked_next_multiple_of(ROUND);
        Budget(round.map_or(MOST, |round| round.min(MOST)))
    }
}

impl TryFrom<usize> for Budget {
    type Error = OutOfRange;

    fn try_from(tokens: usize) -> Result<Self, OutOfRange> {
        if (LEAST..=MOST).contains(&tokens) {
            Ok(Budget(tokens))
        } else {
            Err(OutOfRange)
        }
    }
}

impl fmt::Display for Budget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub const BUDGET: Key<Budget> = Key {
    about: KeyAbout {
        name: "read.budget",
        variable: Some("BILBO_READ_BUDGET"),
        default: Default::Toml("7000"),
        allowed: "a whole number of 256 or more",
        doc: "The most tokens a page of bilbo read prints, as bilbo counts them. The default keeps a page written to a pipe or a file under the 30,000 characters at which Claude Code cuts a command's output.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|tokens| usize::try_from(tokens).ok())
            .and_then(|tokens| Budget::try_from(tokens).ok())
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const OUTLINE_WIDTH: Key<usize> = Key {
    about: KeyAbout {
        name: "read.outline_width",
        variable: Some("BILBO_READ_OUTLINE_WIDTH"),
        default: Default::Toml("100"),
        allowed: "a whole number from 20 to 1000",
        doc: "The most characters an outline line of bilbo read may have, an ellipsis included: a longer one is cut and ends in one.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|width| usize::try_from(width).ok())
            .filter(|width| (20..=1000).contains(width))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const MATCHES_LISTED: Key<usize> = Key {
    about: KeyAbout {
        name: "read.matches_listed",
        variable: Some("BILBO_READ_MATCHES_LISTED"),
        default: Default::Toml("5"),
        allowed: "a whole number from 1 to 100",
        doc: "How many entries bilbo read names when a session id matches several, before it says how many more there are.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|listed| usize::try_from(listed).ok())
            .filter(|listed| (1..=100).contains(listed))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub reference: Ref,
    pub outline: bool,
    pub budget: Budget,
    pub outline_width: usize,
    pub matches_listed: usize,
}

/// One generation of an entry, read whole: a reader's state runs from its first record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub meta: Meta,
    pub generation: Generation,
    pub current: Generation,
    pub records: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct About {
    pub kind: String,
    pub origin: Option<Origin>,
    pub captured: SystemTime,
    pub parent: Option<Id>,
    pub started: Option<SystemTime>,
    pub current: Generation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Outline {
        about: Option<About>,
        lines: Vec<Line>,
    },
    Page(Vec<Block>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub id: Id,
    pub generation: Generation,
    pub older: bool,
    pub total: u64,
    pub asked: RecordRange,
    pub body: Body,
    pub more: Option<RecordRange>,
}

impl Output {
    pub fn locator(&self, records: RecordRange) -> Locator {
        Locator {
            generation: self.older.then_some(self.generation),
            records: Some(records),
        }
    }

    /// As a command to run later carries it: always with the generation that was read, so a
    /// file rewritten in between cannot make the command read other records.
    pub fn pinned(&self, records: RecordRange) -> Locator {
        Locator {
            generation: Some(self.generation),
            records: Some(records),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    NoEntry(Target),
    Ambiguous {
        session: String,
        entries: Vec<(Id, Origin)>,
        listed: usize,
    },
    NoGeneration {
        id: Id,
        generation: Generation,
    },
    OutOfRange {
        id: Id,
        total: u64,
        asked: RecordRange,
    },
    UnreadKind {
        id: Id,
        kind: String,
    },
    Store(StoreError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoEntry(Target::Entry(id)) => write!(f, "no entry {id}"),
            Self::NoEntry(Target::Session(session)) => {
                write!(f, "no entry for the session {session}")
            }
            Self::Ambiguous {
                session,
                entries,
                listed,
            } => {
                write!(f, "{session} names {} entries: ", entries.len())?;
                for (at, (id, origin)) in entries.iter().take(*listed).enumerate() {
                    let comma = if at == 0 { "" } else { ", " };
                    write!(f, "{comma}{id} ({origin})")?;
                }
                if entries.len() > *listed {
                    write!(f, " and {} more", entries.len() - listed)?;
                }
                f.write_str("; give the id of one")
            }
            Self::NoGeneration { id, generation } => {
                write!(f, "entry {id} has no generation {}", generation.0)
            }
            Self::OutOfRange { id, total, asked } => {
                let locator = Locator {
                    generation: None,
                    records: Some(*asked),
                };
                write!(f, "entry {id} has {total} records: {locator} is outside it")
            }
            Self::UnreadKind { id, kind } => {
                write!(
                    f,
                    "entry {id} is of the kind {kind}, which this bilbo cannot read"
                )
            }
            Self::Store(error) => write!(f, "{error}"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StoreError> for Error {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// A note named for a UUID is no session.
fn names(origin: &Origin, session: &str) -> bool {
    let Some(name) = origin
        .path()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
    else {
        return false;
    };
    let Some(stem) = name.strip_suffix(".jsonl") else {
        return false;
    };
    stem.len()
        .checked_sub(refs::UUID)
        .and_then(|start| stem.get(start..))
        .is_some_and(|uuid| {
            refs::is_session(uuid) && uuid.to_ascii_lowercase().starts_with(session)
        })
}

fn resolve(target: &Target, listed: usize, entries: &dyn Entries) -> Result<Id, Error> {
    let session = match target {
        Target::Entry(id) => return Ok(*id),
        Target::Session(session) => session,
    };
    let mut found: Vec<(Id, Origin)> = entries
        .origins_with(session)?
        .into_iter()
        .filter(|(_, origin)| names(origin, session))
        .collect();
    found.sort_by_key(|(id, _)| *id);
    match found.len() {
        0 => Err(Error::NoEntry(target.clone())),
        1 => Ok(found.remove(0).0),
        _ => Err(Error::Ambiguous {
            session: session.clone(),
            entries: found,
            listed,
        }),
    }
}

pub fn fetch(request: &Request, entries: &dyn Entries) -> Result<Fetched, Error> {
    let reference = &request.reference;
    let id = resolve(&reference.target, request.matches_listed, entries)?;
    let no_entry = || Error::NoEntry(Target::Entry(id));
    let meta = entries.meta(id)?.ok_or_else(no_entry)?;
    let (current, _) = entries.extent(id, None)?.ok_or_else(no_entry)?;
    let asked = reference.locator.generation;
    let (generation, total) = entries.extent(id, asked)?.ok_or(Error::NoGeneration {
        id,
        generation: asked.unwrap_or(current),
    })?;
    if let Some(asked) = reference.locator.records
        && last(asked) > total
    {
        return Err(Error::OutOfRange { id, total, asked });
    }
    let whole = RecordRange {
        first: 1,
        count: total,
    };
    // An entry kept with its meta and no generation yet has nothing to read.
    let records = if total == 0 {
        Vec::new()
    } else {
        entries.read(id, generation, whole)?
    };
    Ok(Fetched {
        meta,
        generation,
        current,
        records,
    })
}

pub struct Shown<'s> {
    pub tokens: &'s dyn Tokens,
    /// What the caller prints around the lines of an outline, at its longest, given the output
    /// with no line yet: it is counted before the lines are chosen.
    pub beside: &'s dyn Fn(&Output) -> String,
}

/// Pure. The request need not be the one that was fetched: a range outside the records is refused here too.
pub fn render(
    fetched: &Fetched,
    request: &Request,
    readers: &[Box<dyn Reader>],
    shown: &Shown<'_>,
) -> Result<Output, Error> {
    let tokens = shown.tokens;
    let id = fetched.meta.id;
    let reader = readers
        .iter()
        .find(|reader| reader.kind() == fetched.meta.kind)
        .ok_or_else(|| Error::UnreadKind {
            id,
            kind: fetched.meta.kind.clone(),
        })?;
    let records: Vec<&[u8]> = fetched.records.iter().map(Vec::as_slice).collect();
    let total = u64::try_from(records.len()).unwrap_or(u64::MAX);
    let ranged = request.reference.locator.records;
    let asked = ranged.unwrap_or(RecordRange {
        first: 1,
        count: total,
    });
    if asked.count > 0 && last(asked) > total {
        return Err(Error::OutOfRange { id, total, asked });
    }
    let passages = reader.passages(&records);
    let older = fetched.generation != fetched.current;
    let mut output = Output {
        id,
        generation: fetched.generation,
        older,
        total,
        asked,
        body: Body::Page(Vec::new()),
        more: None,
    };
    let locator = |records: RecordRange| output.locator(records).to_string();
    let budget = request.budget.tokens();
    let (body, more) = if request.outline || ranged.is_none() {
        let about = ranged.is_none().then(|| About {
            kind: fetched.meta.kind.clone(),
            origin: fetched.meta.origin.clone(),
            captured: fetched.meta.captured,
            parent: fetched.meta.parent,
            started: passages.iter().find_map(|passage| passage.at),
            current: fetched.current,
        });
        let frame = Output {
            body: Body::Outline {
                about: about.clone(),
                lines: Vec::new(),
            },
            ..output.clone()
        };
        let budget = budget.saturating_sub(tokens.count(&(shown.beside)(&frame)));
        let listing = outline::outline(
            &passages,
            asked,
            &locator,
            budget,
            request.outline_width,
            tokens,
        );
        (
            Body::Outline {
                about,
                lines: listing.lines,
            },
            listing.more,
        )
    } else {
        let pinned = |records: RecordRange| output.pinned(records).to_string();
        let locators = Locators {
            shown: &locator,
            pinned: &pinned,
        };
        let page = page::page(&passages, asked, &locators, budget, tokens);
        (Body::Page(page.blocks), page.more)
    };
    output.body = body;
    output.more = more;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use bilbo_settings::file::File;
    use bilbo_settings::key::{Flag, Origin as From, Source};
    use bilbo_store::entries::{Generation, Origin, RecordRange};
    use bilbo_store::id::Id;

    use super::refs::Target;
    use super::{BUDGET, Budget, Error, MATCHES_LISTED, MOST, OUTLINE_WIDTH, OutOfRange, names};
    use bilbo_settings::key::Key;

    fn resolved(variable: Option<&str>) -> Result<(usize, From), String> {
        let file = File::default();
        let var = |name: &str| {
            (name == "BILBO_READ_BUDGET")
                .then(|| variable.map(Into::into))
                .flatten()
        };
        BUDGET
            .resolve(&Source {
                var: &var,
                file: &file,
            })
            .map(|resolved| (resolved.value.tokens(), resolved.origin))
            .map_err(|error| error.to_string())
    }

    #[test]
    fn the_budget_defaults_to_7000_tokens() {
        assert_eq!(resolved(None), Ok((7000, From::Default)));
        let budget = Budget::try_from(7000).expect("a budget");
        assert_eq!((BUDGET.show)(&budget), "7000");
    }

    #[test]
    fn a_budget_is_made_only_of_a_number_the_key_accepts() {
        for tokens in [256, 7000, MOST] {
            assert_eq!(Budget::try_from(tokens).map(Budget::tokens), Ok(tokens));
        }
        for tokens in [0, 255, MOST + 1, usize::MAX] {
            assert_eq!(Budget::try_from(tokens), Err(OutOfRange), "{tokens}");
        }
        for (tokens, held) in [
            (0, 1000),
            (1, 1000),
            (1000, 1000),
            (1001, 2000),
            (MOST - 1, MOST),
            (usize::MAX, MOST),
        ] {
            let budget = Budget::holding(tokens);
            assert_eq!(budget.tokens(), held, "{tokens}");
            assert_eq!(Budget::try_from(held), Ok(budget), "the key accepts {held}");
        }
    }

    #[test]
    fn the_budget_is_a_whole_number_from_256_up_to_what_a_toml_integer_holds() {
        let most = "9223372036854775807";
        for (text, budget) in [("256", 256), ("60000", 60_000), (most, MOST)] {
            assert_eq!(
                resolved(Some(text)),
                Ok((budget, From::Variable("BILBO_READ_BUDGET"))),
                "{text}"
            );
        }
        for text in [
            "255",
            "0",
            "-1",
            "9223372036854775808",
            "7e3",
            "7000.0",
            "many",
            " 7000",
        ] {
            assert_eq!(
                resolved(Some(text)),
                Err(format!(
                    "BILBO_READ_BUDGET must be a whole number of 256 or more, got \"{text}\""
                )),
                "{text}"
            );
        }
    }

    #[test]
    fn a_budget_flag_is_read_by_the_key_and_a_refusal_names_the_flag() {
        let file = File::default();
        let source = Source {
            var: &|_| None,
            file: &file,
        };
        let refused = Flag::parse(&BUDGET, &source, "--budget", "12")
            .map(|_| ())
            .map_err(|error| error.to_string());
        assert_eq!(
            refused,
            Err("--budget must be a whole number of 256 or more, got \"12\"".to_owned())
        );
        assert!(Flag::parse(&BUDGET, &source, "--budget", "60000").is_ok());
    }

    fn count(
        key: &Key<usize>,
        variable: &str,
        env: Option<&str>,
        file: &str,
    ) -> Result<(usize, From), String> {
        let file = File::parse(Path::new("/c/config.toml"), file.as_bytes()).expect("toml");
        let var = |name: &str| (name == variable).then(|| env.map(Into::into)).flatten();
        key.resolve(&Source {
            var: &var,
            file: &file,
        })
        .map(|resolved| (resolved.value, resolved.origin))
        .map_err(|error| error.to_string())
    }

    #[test]
    fn the_outline_width_is_20_to_1000_characters_and_defaults_to_100() {
        let var = "BILBO_READ_OUTLINE_WIDTH";
        let at = |env, file| count(&OUTLINE_WIDTH, var, env, file);
        assert_eq!(at(None, ""), Ok((100, From::Default)));
        assert_eq!(
            at(None, "read.outline_width = 60"),
            Ok((
                60,
                From::File {
                    path: "/c/config.toml".into(),
                    line: 1
                }
            ))
        );
        assert_eq!(
            at(Some("1000"), "read.outline_width = 60"),
            Ok((1000, From::Variable("BILBO_READ_OUTLINE_WIDTH")))
        );
        assert_eq!(at(Some("20"), "").map(|(width, _)| width), Ok(20));
        for text in ["19", "1001", "0", "-100", "wide", "100.5"] {
            assert_eq!(
                at(Some(text), ""),
                Err(format!(
                    "{var} must be a whole number from 20 to 1000, got \"{text}\""
                )),
                "{text}"
            );
        }
        assert_eq!(
            at(None, "read.outline_width = 19"),
            Err(
                "/c/config.toml:1: read.outline_width must be a whole number from 20 to 1000, got 19"
                    .to_owned()
            )
        );
    }

    #[test]
    fn the_matches_listed_are_1_to_100_and_default_to_5() {
        let var = "BILBO_READ_MATCHES_LISTED";
        let at = |env, file| count(&MATCHES_LISTED, var, env, file);
        assert_eq!(at(None, ""), Ok((5, From::Default)));
        assert_eq!(
            at(None, "read.matches_listed = 12").map(|(listed, _)| listed),
            Ok(12)
        );
        assert_eq!(
            at(Some("100"), "read.matches_listed = 12"),
            Ok((100, From::Variable("BILBO_READ_MATCHES_LISTED")))
        );
        assert_eq!(at(Some("1"), "").map(|(listed, _)| listed), Ok(1));
        for text in ["0", "101", "-1", "some", "2.5"] {
            assert_eq!(
                at(Some(text), ""),
                Err(format!(
                    "{var} must be a whole number from 1 to 100, got \"{text}\""
                )),
                "{text}"
            );
        }
        assert_eq!(
            at(None, "read.matches_listed = 0"),
            Err(
                "/c/config.toml:1: read.matches_listed must be a whole number from 1 to 100, got 0"
                    .to_owned()
            )
        );
    }

    #[test]
    fn an_ambiguous_session_names_as_many_entries_as_were_asked() {
        let id = bilbo_store::id::Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("id");
        let origin = Origin::file(Path::new("/a")).expect("an origin");
        let error = |listed| Error::Ambiguous {
            session: "5c3a9e1f".to_owned(),
            entries: vec![(id, origin.clone()); 4],
            listed,
        };
        assert!(
            error(2)
                .to_string()
                .ends_with("(file:/a) and 2 more; give the id of one")
        );
        assert!(!error(4).to_string().contains(" more"));
        assert_eq!(error(1).to_string().matches("(file:/a)").count(), 1);
    }

    #[test]
    fn an_origin_names_a_session_when_its_file_name_ends_in_that_uuid() {
        const UUID: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";
        let origin = |path: &str| Origin::file(Path::new(path)).expect("an origin");
        for path in [
            format!("/p/proj/{UUID}.jsonl"),
            format!("/s/2026/11/06/rollout-2026-11-06T09-10-00-{UUID}.jsonl"),
            format!("/c/repo/2026-02-14T233343Z--{UUID}.jsonl"),
            format!("/p/proj/{}.jsonl", UUID.to_uppercase()),
        ] {
            assert!(names(&origin(&path), UUID), "{path}");
            assert!(names(&origin(&path), &UUID[..8]), "{path}");
            assert!(!names(&origin(&path), "5c3a9e1e"), "{path}");
        }
        for path in [
            format!("/p/{UUID}/subagents/agent-ad63c2b3c9e7a4b3d.jsonl"),
            format!("/p/proj/{UUID}x.jsonl"),
            format!("/notes/{UUID}.md"),
            format!("/p/proj/{UUID}"),
            format!("/p/proj/{UUID}.jsonl.bak"),
            format!("/p/proj/{}.jsonl", &UUID[1..]),
            format!("/p/proj/é{}.jsonl", &UUID[2..]),
            "/p/proj/notes.jsonl".to_owned(),
        ] {
            assert!(!names(&origin(&path), &UUID[..8]), "{path}");
        }
        let stdin = Origin::new("stdin".to_owned()).expect("an origin");
        assert!(!names(&stdin, &UUID[..8]));
    }

    #[test]
    fn every_error_says_what_was_asked_and_what_the_entry_holds() {
        let id = Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("id");
        let other = Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DF").expect("id");
        let origin = |name: &str| Origin::file(Path::new(name)).expect("an origin");
        let cases = [
            (
                Error::NoEntry(Target::Entry(id)),
                "no entry 01JC5Q3V7XKM4P8R2T6W9YB0DE".to_owned(),
            ),
            (
                Error::NoEntry(Target::Session("5c3a9e1f".to_owned())),
                "no entry for the session 5c3a9e1f".to_owned(),
            ),
            (
                Error::Ambiguous {
                    session: "5c3a9e1f".to_owned(),
                    entries: vec![(id, origin("/a/x.jsonl")), (other, origin("/b/x.jsonl"))],
                    listed: 5,
                },
                "5c3a9e1f names 2 entries: 01JC5Q3V7XKM4P8R2T6W9YB0DE (file:/a/x.jsonl), \
                 01JC5Q3V7XKM4P8R2T6W9YB0DF (file:/b/x.jsonl); give the id of one"
                    .to_owned(),
            ),
            (
                Error::Ambiguous {
                    session: "5c3a9e1f".to_owned(),
                    entries: vec![(id, origin("/a")); 7],
                    listed: 5,
                },
                format!(
                    "5c3a9e1f names 7 entries: {} and 2 more; give the id of one",
                    ["01JC5Q3V7XKM4P8R2T6W9YB0DE (file:/a)"; 5].join(", ")
                ),
            ),
            (
                Error::NoGeneration {
                    id,
                    generation: Generation(3),
                },
                "entry 01JC5Q3V7XKM4P8R2T6W9YB0DE has no generation 3".to_owned(),
            ),
            (
                Error::OutOfRange {
                    id,
                    total: 7,
                    asked: RecordRange { first: 5, count: 5 },
                },
                "entry 01JC5Q3V7XKM4P8R2T6W9YB0DE has 7 records: #r5-9 is outside it".to_owned(),
            ),
            (
                Error::UnreadKind {
                    id,
                    kind: "page".to_owned(),
                },
                "entry 01JC5Q3V7XKM4P8R2T6W9YB0DE is of the kind page, which this bilbo cannot read"
                    .to_owned(),
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
        }
    }
}
