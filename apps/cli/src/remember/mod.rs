pub mod disk;
pub mod edit;
pub mod follows;
pub mod index;
pub mod messages;
pub mod note;
pub mod save;
pub mod view;

use std::collections::BTreeSet;
use std::path::PathBuf;

use bilbo_entries::follows::Follow;
use bilbo_entries::kind::Kind;
use bilbo_entries::remember::{self, Output, Request, Tagging};
use bilbo_entries::tag;
use bilbo_store::id::Id;
use bilbo_store::labels::Tag;
use bilbo_store::segments::Store;
use clap::Command;
use clap::builder::NonEmptyStringValueParser;

use crate::console::args::usage;
use crate::console::closing::Closing;
use crate::console::env::Env;
use crate::console::indexing::Indexing;
use crate::console::message::{Message, not_a_tag, notice_message, sanitize};
use crate::console::outcome::{Exit, Failure, Outcome};
use crate::console::settings::Settings;
use disk::Disk;
use messages::{moves, refusals, tagged, withdrawals};

const SUMMARY: &str = "Keep files or a note as entries; a later run keeps what changed";
const ABOUT: &str =
    "bilbo remember: keep files or a note as entries, and what changed in them since.";
const USAGE: &str = "bilbo remember <file|folder> [--kind <kind>] [--tag <tag>]...
                 [--follow]
  bilbo remember - [--tag <tag>]...
  bilbo remember --edit [--tag <tag>]...";

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
        long = "tag",
        value_name = "tag",
        value_parser = NonEmptyStringValueParser::new(),
        help = "Declare this tag for every entry the run keeps"
    )]
    pub tags: Vec<String>,
    #[arg(
        long,
        requires = "target",
        help = "Save the target as a follow in the config file"
    )]
    pub follow: bool,
    #[arg(
        long,
        help = "Keep a note written in the editor VISUAL or EDITOR names"
    )]
    pub edit: bool,
}

pub fn page(kinds: &[Box<dyn Kind>]) -> String {
    let names: Vec<&str> = kinds.iter().map(|kind| kind.name()).collect();
    format!("Kinds: {}\n\n{PAGE}", names.join(", "))
}

const PAGE: &str = "\
Output:
  new <n>, appended <n>, new generation <n>, unchanged <n>, refused <n>
  records <n>, raw bytes <n>, stored bytes <n>

Exit:
  0  success
  1  a refused file or note, nothing to keep, a store, index or editor failure
  2  usage or config error, no editor for --edit, a config --follow cannot write

Examples:
  bilbo remember ~/notes --follow
  echo 'The beet seeds are in the drawer.' | bilbo remember - --tag garden

Docs: https://github.com/delucca/bilbo/wiki/Commands#remember";

pub fn run(
    root: &Command,
    args: Args,
    env: &mut Env,
    settings: &Settings,
    kinds: &[Box<dyn Kind>],
    indexing: &Indexing<'_>,
    written: &note::Written,
) -> Result<Outcome, Failure> {
    if args.target.is_none() && !args.edit {
        return Err(usage(
            root,
            "missing <file|folder>, - or --edit; bilbo refresh keeps every follow".to_owned(),
            Some("remember"),
        ));
    }
    if args.edit && args.target.is_some() {
        return Err(usage(
            root,
            "--edit does not stand with a target: it keeps a note written in the editor".to_owned(),
            Some("remember"),
        ));
    }
    let from_input = args
        .target
        .as_ref()
        .is_some_and(|target| target.as_os_str() == "-");
    if from_input && args.kind.is_some() {
        return Err(usage(
            root,
            "--kind does not stand with a note: its kind is note".to_owned(),
            Some("remember"),
        ));
    }
    if from_input && args.follow {
        return Err(usage(
            root,
            "--follow does not stand with a note: a follow is a file or folder".to_owned(),
            Some("remember"),
        ));
    }
    let mut named = BTreeSet::new();
    for text in &args.tags {
        let Some(name) = Tag::new(text) else {
            return Err(usage(
                root,
                sanitize(&not_a_tag(&format!("--tag '{text}'"))),
                Some("remember"),
            ));
        };
        named.insert(name);
    }
    if args.follow && !named.is_empty() {
        return Err(usage(
            root,
            "--tag does not stand with --follow: a follow keeps no tag".to_owned(),
            Some("remember"),
        ));
    }
    let tags = Tagging {
        tags: named,
        cap: settings.tag_cap.value,
    };
    let Some(target) = args.target else {
        let Some(editor) = edit::editor(&|name| (env.var)(name)) else {
            return Err(Failure::Config(
                "--edit needs an editor: set VISUAL or EDITOR".to_owned(),
            ));
        };
        return edit::run(&editor, &tags, written, env, settings, kinds, indexing);
    };
    if from_input {
        let text = note::from_input(&mut env.stdin)?;
        let id = Id::mint().map_err(|error| refused(&error))?;
        let note = note::Note {
            text: &text,
            id,
            tags: &tags,
            draft: None,
        };
        return note::keep(note, written, env, settings, kinds, indexing);
    }
    let request = Request {
        target,
        kind: args.kind,
        tags,
    };
    let plan = remember::plan(&request, kinds, &Disk).map_err(|error| match error {
        remember::Error::UnknownKind { .. } | remember::Error::SeveralKinds { .. } => {
            usage(root, sanitize(&error.to_string()), Some("remember"))
        }
        remember::Error::Target { .. } | remember::Error::Store(_) => refused(&error),
    })?;

    let layout = settings.layout();
    Store::check(&layout)?;
    let mut notes = Vec::new();
    let (output, mut closing) = if plan.claims.is_empty() {
        (Output::unkept(&plan), Closing::default())
    } else {
        let notice = &mut env.notice;
        let mut store = Store::open(&layout, settings.store.options(), &mut |n| {
            notice(&notice_message(&n));
        })?;
        let fresh = if plan.tags.tags.is_empty() {
            Vec::new()
        } else {
            tag::unused(&plan.tags.tags, &store).map_err(|error| refused(&error))?
        };
        let output = remember::keep(&plan, &Disk, &mut store).map_err(|error| refused(&error))?;
        notes = tagged(&output, &fresh);
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
    Ok(outcome(output, notes, closing, config))
}

fn outcome(output: Output, notes: Vec<Message>, closing: Closing, config: bool) -> Outcome {
    let mut diagnostics = refusals(std::slice::from_ref(&output));
    if !closing.failed {
        diagnostics.extend(moves(std::slice::from_ref(&output)));
        diagnostics.extend(withdrawals(std::slice::from_ref(&output)));
    }
    diagnostics.extend(notes);
    if output.claimed == 0 {
        let target = output.target.display();
        diagnostics.push(Message::warning(&match &output.kind {
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
