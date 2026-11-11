pub mod disk;
pub mod follows;
pub mod hook;
pub mod index;
pub mod messages;
pub mod save;
pub mod view;

use std::path::PathBuf;

use bilbo_kinds::follows::Follow;
use bilbo_kinds::kind::Kind;
use bilbo_kinds::remember::{self, Output, Request};
use bilbo_store::segments::Store;
use clap::Command;
use clap::builder::NonEmptyStringValueParser;

use crate::console::args::usage;
use crate::console::closing::Closing;
use crate::console::env::Env;
use crate::console::indexing::Indexing;
use crate::console::message::{notice_message, sanitize};
use crate::console::outcome::{Exit, Failure, Outcome};
use crate::console::settings::Settings;
use disk::Disk;
use messages::{refusals, warning};

const SUMMARY: &str = "Keep files as entries; a later run keeps what grew or changed";
const ABOUT: &str =
    "bilbo remember: keep files as entries, and what grew or changed in them since.";
const USAGE: &str = "bilbo remember <file|folder> [--kind <kind>] [--follow]
  bilbo remember
  bilbo remember --hook";

#[derive(Debug, clap::Args)]
#[command(about = SUMMARY, before_help = ABOUT, override_usage = USAGE)]
pub struct Args {
    #[arg(value_name = "file|folder")]
    pub target: Option<PathBuf>,
    #[arg(
        long,
        value_name = "kind",
        value_parser = NonEmptyStringValueParser::new(),
        requires = "target",
        help = "Run this kind, not the one that claims the files"
    )]
    pub kind: Option<String>,
    #[arg(
        long,
        requires = "target",
        help = "Save the target in the config file, so a bare run keeps it"
    )]
    pub follow: bool,
    #[arg(
        long,
        conflicts_with_all = ["target", "kind", "follow"],
        help = "Keep the transcript a harness hook names on stdin"
    )]
    pub hook: bool,
}

pub fn page(kinds: &[Box<dyn Kind>]) -> String {
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

Exit:
  0  success, and --hook once it runs
  1  a refused file, nothing to keep, a missing target, a store or index failure
  2  usage or config error, or a config file --follow cannot write

Examples:
  bilbo remember ~/.claude/projects --kind session
  bilbo remember ~/notes --follow

Docs: https://github.com/delucca/bilbo/wiki/Commands#remember";

pub fn run(
    root: &Command,
    args: Args,
    env: &mut Env,
    settings: &Settings,
    kinds: &[Box<dyn Kind>],
    indexing: &Indexing<'_>,
) -> Result<Outcome, Failure> {
    let Some(target) = args.target else {
        return follows::run(env, settings, kinds, indexing);
    };
    let request = Request {
        target,
        kind: args.kind,
    };
    let plan = remember::plan(&request, kinds, &Disk).map_err(|error| match error {
        remember::Error::UnknownKind { .. } | remember::Error::SeveralKinds { .. } => {
            usage(root, sanitize(&error.to_string()), Some("remember"))
        }
        remember::Error::Target { .. } | remember::Error::Store(_) => refused(&error),
    })?;

    let layout = settings.layout();
    Store::check(&layout).map_err(|error| Failure::from(&error))?;
    let (output, mut closing) = if plan.files.is_empty() {
        (Output::unkept(&plan), Closing::default())
    } else {
        let notice = &mut env.notice;
        let mut store = Store::open(&layout, settings.store.options(), &mut |n| {
            notice(&notice_message(&n));
        })
        .map_err(|error| Failure::from(&error))?;
        let output = remember::keep(&plan, &Disk, &mut store).map_err(|error| refused(&error))?;
        let kept = view::Totals::of(std::slice::from_ref(&output)).kept();
        let closing = follows::finish(
            store,
            &settings.follow.value,
            kinds,
            indexing,
            &mut *env.notice,
            kept,
        );
        (output, closing)
    };
    let mut config = false;
    if args.follow && !closing.failed {
        let follow = Follow {
            path: plan.target.clone(),
            kind: plan.kind.map(|kind| kind.name().to_owned()),
        };
        match save::save(settings, &follow, &|name| (env.var)(name)) {
            Ok(saved) => closing.messages.push(saved.message()),
            Err(refusal) => {
                closing.messages.push(refusal.message());
                config = true;
            }
        }
    }
    Ok(outcome(output, closing, config))
}

fn outcome(output: Output, closing: Closing, config: bool) -> Outcome {
    let mut diagnostics = refusals(std::slice::from_ref(&output));
    if output.claimed == 0 {
        let target = output.target.display();
        diagnostics.push(warning(&match &output.kind {
            None => format!("nothing to keep: no kind claims a file under {target}"),
            Some(kind) => format!("nothing to keep: {kind} claims no file under {target}"),
        }));
    }
    diagnostics.extend(closing.messages);
    let refused = output.tally().refused > 0 || closing.refused;
    let exit = if config {
        Exit::Config
    } else if output.claimed == 0 || refused || closing.failed {
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
