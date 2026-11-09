pub mod disk;
pub mod follows;
pub mod hook;
pub mod messages;
pub mod registry;
pub mod save;
pub mod view;

use std::path::PathBuf;

use bilbo_kinds::follows::Follow;
use bilbo_kinds::remember::{self, Output, Request};
use bilbo_store::segments::Store;
use clap::Command;
use clap::builder::NonEmptyStringValueParser;

use crate::console::args::usage;
use crate::console::env::Env;
use crate::console::message::sanitize;
use crate::console::outcome::{Exit, Failure, Outcome};
use crate::console::settings::Settings;
use disk::Disk;
use follows::Closing;
use messages::{notice_message, refusals, warning};

const ABOUT: &str =
    "bilbo remember: keep files as entries, and what grew or changed in them since.";
const USAGE: &str = "bilbo remember <file|folder> [--kind <kind>] [--follow]
  bilbo remember
  bilbo remember --hook";

#[derive(Debug, clap::Args)]
#[command(about = ABOUT, override_usage = USAGE, after_help = page())]
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
    format!("Kinds:\n{rows}{PAGE}")
}

const PAGE: &str = "\
Without --kind, the one kind that claims files under the target runs; when
several do, name one with --kind.

A folder is walked whole, skipping the hidden entries and symbolic links it
finds; a target named here is taken as given, even a link or a hidden name,
and its origin is the resolved path. A kind tests the resolved file, so a
note.md link to a file without .md keeps nothing. A file remembered before
is found by its path: a run keeps only the records it gained, or starts a
new generation when an earlier record changed, and the older generation
stays. A file whose size and modification time did not change is not read.
A file that left the folder keeps its entry.

--follow saves the target and its kind as a [[follow]] in the config file.
With no target, remember runs every follow: ~/.claude/projects and
~/.codex/sessions as session until the config file declares one. --hook
reads a Claude Code or Codex hook's JSON on stdin and keeps the new lines of
its transcript_path; it never seals open.seg and prints nothing on stdout.
When another bilbo is writing the store, a run with no target or with --hook
asks for one more run of the follows and stops at once; a target waits.

Output:
  new <n>, appended <n>, new generation <n>, unchanged <n>, refused <n>
  records <n>, raw bytes <n>, stored bytes <n>
Raw bytes are the input bytes kept; stored bytes are what the store grew by.
Each refused file is named on stderr with its reason.

Exit: 0 success; 1 a file was refused, nothing to keep, a missing target or a
store failure; 2 usage or config error, or a config file --follow cannot
write. --hook exits 0 once it runs; wrong arguments to it are a usage
error (2).

Examples:
  bilbo remember ~/.claude/projects --kind session
  bilbo remember ~/notes --follow
  bilbo remember

Docs: reference/commands.md#remember";

pub fn run(
    root: &Command,
    args: Args,
    env: &mut Env,
    settings: &Settings,
) -> Result<Outcome, Failure> {
    let Some(target) = args.target else {
        return follows::run(env, settings);
    };
    let kinds = registry::kinds();
    let request = Request {
        target,
        kind: args.kind,
    };
    let plan = remember::plan(&request, &kinds, &Disk).map_err(|error| match error {
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
        (output, follows::close(store, &settings.follow.value))
    };
    if args.follow && !closing.failed {
        let follow = Follow {
            path: plan.target.clone(),
            kind: plan.kind.map(|kind| kind.name().to_owned()),
        };
        match save::save(settings, &follow, &|name| (env.var)(name)) {
            Ok(saved) => closing.messages.push(saved.message()),
            Err(refusal) => {
                closing.messages.push(refusal.message());
                closing.config = true;
            }
        }
    }
    Ok(outcome(output, closing))
}

fn outcome(output: Output, closing: Closing) -> Outcome {
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
    let exit = if closing.config {
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

#[cfg(test)]
mod tests {
    use super::{page, registry};

    #[test]
    fn the_page_lists_every_kind_of_the_registry() {
        let page = page();
        for kind in registry::kinds() {
            let row = format!("  {:<8}  {}\n", kind.name(), kind.summary());
            assert!(page.contains(&row), "{row:?} missing in\n{page}");
        }
        assert!(page.starts_with("Kinds:\n"));
    }
}
