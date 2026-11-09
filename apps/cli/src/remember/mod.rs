pub mod disk;
pub mod registry;
pub mod view;

use std::path::PathBuf;

use bilbo_kinds::remember::{self, Output, Request};
use bilbo_store::segments::{Notice, Store};
use clap::Command;
use clap::builder::NonEmptyStringValueParser;

use crate::console::args::usage;
use crate::console::env::Env;
use crate::console::message::{Message, sanitize};
use crate::console::outcome::{Exit, Failure, Outcome};
use crate::console::settings::Settings;
use crate::console::theme::Mark;
use disk::Disk;

const SUMMARY: &str = "Keep files as entries; a later run keeps what grew or changed";
const ABOUT: &str =
    "bilbo remember: keep files as entries, and what grew or changed in them since.";
const USAGE: &str = "bilbo remember <file|folder> [--kind <kind>] [--project <project>]";

#[derive(Debug, clap::Args)]
#[command(about = SUMMARY, before_help = ABOUT, override_usage = USAGE, after_help = page())]
pub struct Args {
    #[arg(value_name = "file|folder")]
    pub target: PathBuf,
    #[arg(
        long,
        value_name = "kind",
        value_parser = NonEmptyStringValueParser::new(),
        help = "Run this kind, not the one that claims the files"
    )]
    pub kind: Option<String>,
    #[arg(
        long,
        value_name = "project",
        value_parser = NonEmptyStringValueParser::new(),
        help = "Label each new entry with this project"
    )]
    pub project: Option<String>,
}

fn page() -> String {
    let kinds = registry::kinds();
    let width = kinds
        .iter()
        .map(|kind| kind.name().len())
        .max()
        .unwrap_or_default();
    let rows: String = kinds
        .iter()
        .map(|kind| format!("  {:<width$}  {}\n", kind.name(), kind.summary()))
        .collect();
    format!("Kinds:\n{rows}\n{PAGE}")
}

const PAGE: &str = "\
Output:
  new <n>, appended <n>, new generation <n>, unchanged <n>, refused <n>
  records <n>, raw bytes <n>, stored bytes <n>
Each refused file is named on stderr with its reason.

Exit:
  0  success
  1  a file was refused, nothing to keep, a missing target or a store failure
  2  usage or config error

Examples:
  bilbo remember notes/
  bilbo remember notes/decision-x.md --project bilbo

Docs: https://github.com/delucca/bilbo/wiki/Commands#remember";

pub fn run(
    root: &Command,
    args: Args,
    env: &mut Env,
    settings: &Settings,
) -> Result<Outcome, Failure> {
    let kinds = registry::kinds();
    let request = Request {
        target: args.target,
        kind: args.kind,
        project: args.project,
    };
    let plan = remember::plan(&request, &kinds, &Disk).map_err(|error| match error {
        remember::Error::UnknownKind { .. } | remember::Error::SeveralKinds { .. } => {
            usage(root, sanitize(&error.to_string()), Some("remember"))
        }
        remember::Error::Target { .. } | remember::Error::Store(_) => refused(&error),
    })?;

    let layout = settings.layout();
    Store::check(&layout).map_err(|error| Failure::from(&error))?;
    let (output, close_error) = if plan.files.is_empty() {
        (Output::unkept(&plan), None)
    } else {
        let notice = &mut env.notice;
        let mut store = Store::open(&layout, settings.store.options(), &mut |n| {
            notice(&notice_message(&n));
        })
        .map_err(|error| Failure::from(&error))?;
        let output = remember::keep(&plan, &Disk, &mut store).map_err(|error| refused(&error))?;
        (
            output,
            store
                .close()
                .err()
                .map(|error| sanitize(&error.to_string())),
        )
    };
    Ok(outcome(output, close_error))
}

fn outcome(output: Output, close_error: Option<String>) -> Outcome {
    let mut diagnostics: Vec<Message> = output
        .items
        .iter()
        .filter_map(|item| match &item.outcome {
            remember::Outcome::Refused(reason) => Some(warning(&format!(
                "refused {}: {reason}",
                item.path.display()
            ))),
            _ => None,
        })
        .collect();
    if output.claimed == 0 {
        let target = output.target.display();
        diagnostics.push(warning(&match &output.kind {
            None => format!("nothing to keep: no kind claims a file under {target}"),
            Some(kind) => format!("nothing to keep: {kind} claims no file under {target}"),
        }));
    }
    let failed = close_error.is_some();
    diagnostics.extend(close_error.map(Message::error));
    let refused = output.tally().refused > 0;
    let exit = if output.claimed == 0 || refused || failed {
        Exit::Refused
    } else {
        Exit::Success
    };
    Outcome::new(view::plain(&output))
        .with_human(move |view| view::human(&output, view))
        .with_diagnostics(diagnostics)
        .with_exit(exit)
}

fn refused(error: &dyn std::fmt::Display) -> Failure {
    Failure::Refused(sanitize(&error.to_string()))
}

fn warning(text: &str) -> Message {
    message(Mark::Warning, text)
}

fn message(mark: Mark, text: &str) -> Message {
    Message {
        mark,
        text: sanitize(text),
        details: Vec::new(),
    }
}

fn plural(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn notice_message(notice: &Notice) -> Message {
    match notice {
        Notice::Waiting => message(
            Mark::Info,
            "waiting for another bilbo to finish writing the store",
        ),
        Notice::CutTornTail {
            path,
            offset,
            bytes,
        } => message(
            Mark::Warning,
            &format!(
                "cut {bytes} bytes of a torn frame at byte {offset} of {}",
                path.display()
            ),
        ),
        Notice::RebuiltCatalog { entries, segments } => message(
            Mark::Info,
            &format!(
                "rebuilt the catalog from the store: {} in {}",
                plural(*entries, "entry", "entries"),
                plural(*segments, "segment", "segments")
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bilbo_store::segments::Notice;

    use super::{notice_message, page, registry};
    use crate::console::theme::Mark;

    #[test]
    fn every_store_notice_is_a_message() {
        let cases = [
            (
                Notice::Waiting,
                Mark::Info,
                "waiting for another bilbo to finish writing the store",
            ),
            (
                Notice::CutTornTail {
                    path: PathBuf::from("/s/open.seg"),
                    offset: 12,
                    bytes: 20,
                },
                Mark::Warning,
                "cut 20 bytes of a torn frame at byte 12 of /s/open.seg",
            ),
            (
                Notice::RebuiltCatalog {
                    entries: 3,
                    segments: 1,
                },
                Mark::Info,
                "rebuilt the catalog from the store: 3 entries in 1 segment",
            ),
            (
                Notice::RebuiltCatalog {
                    entries: 1,
                    segments: 2,
                },
                Mark::Info,
                "rebuilt the catalog from the store: 1 entry in 2 segments",
            ),
        ];
        for (notice, mark, text) in cases {
            let message = notice_message(&notice);
            assert_eq!(message.mark, mark);
            assert_eq!(message.text, text);
        }
    }

    #[test]
    fn the_page_lists_every_kind_of_the_registry() {
        let page = page();
        for kind in registry::kinds() {
            let row = format!("  {}  {}\n", kind.name(), kind.summary());
            assert!(page.contains(&row), "{row:?} missing in\n{page}");
        }
        assert!(page.starts_with("Kinds:\n"));
    }
}
