pub mod view;

use std::time::Duration;

use bilbo_entries::forget::{self, Error as ForgetError, Labelled, Request};
use bilbo_entries::read;
use bilbo_entries::read::refs::{Locator, Ref};
use bilbo_entries::reader::Reader;
use bilbo_store::segments::{Options, Store};
use clap::Command;
use clap::builder::NonEmptyStringValueParser;

use crate::console::args::usage;
use crate::console::closing::Closing;
use crate::console::env::Env;
use crate::console::message::{Message, notice_message, sanitize};
use crate::console::outcome::{Exit, Failure, Outcome};
use crate::console::settings::Settings;

const SUMMARY: &str = "Take entries out of recall; what they held stays readable";
const ABOUT: &str = "bilbo forget: take entries out of recall; what they held stays readable.";
const USAGE: &str = "bilbo forget <ref>...";

#[derive(Debug, clap::Args)]
#[command(about = SUMMARY, before_help = ABOUT, override_usage = USAGE, after_help = PAGE)]
pub struct Args {
    #[arg(value_name = "ref", value_parser = NonEmptyStringValueParser::new())]
    pub refs: Vec<String>,
}

const PAGE: &str = "\
An entry gives up its capture label, so its path can be kept again. What
belongs to it goes with it: its subagents, its notes and its summary.

Output:
  forgotten <n>, belonging <n>, unchanged <n>

Exit:
  0  success, also when every entry named was forgotten already
  1  a ref that names no entry, a store or index failure
  2  usage or config error, or a ref that names several entries

Examples:
  bilbo forget 2T6W9YB0DE

Docs: https://github.com/delucca/bilbo/wiki/Commands#forget";

pub type Finish<'a> = dyn Fn(Store, &mut dyn FnMut(&Message)) -> Closing + 'a;

fn request(args: &Args, listed: usize) -> Result<Request, String> {
    if args.refs.is_empty() {
        return Err("missing <ref>".to_owned());
    }
    let mut targets = Vec::new();
    for text in &args.refs {
        let parsed = Ref::parse(text).map_err(|malformed| malformed.to_string())?;
        if parsed.locator != Locator::default() {
            return Err(format!(
                "'{text}' names records; an entry is forgotten whole"
            ));
        }
        targets.push(parsed.target);
    }
    Ok(Request { targets, listed })
}

fn refused(root: &Command, error: &ForgetError) -> Failure {
    match error {
        ForgetError::Ref(ambiguous @ read::Error::Ambiguous { .. }) => {
            usage(root, sanitize(&ambiguous.to_string()), Some("forget"))
        }
        ForgetError::Store { error, .. } => Failure::from(error),
        ForgetError::Ref(_) | ForgetError::NoLabel(_) | ForgetError::NotEmptied { .. } => {
            Failure::Refused(sanitize(&error.to_string()))
        }
    }
}

pub fn writer_options(settings: &Settings) -> Options {
    settings.store.options()
}

pub fn ended(root: &Command, error: ForgetError) -> Result<Outcome, Failure> {
    let failure = refused(root, &error);
    match error {
        ForgetError::NotEmptied { named, .. } | ForgetError::Store { named, .. }
            if !named.is_empty() =>
        {
            Ok(Outcome::stopped(
                vec![Message::info(&view::stopped(&named))],
                &failure,
                "",
            ))
        }
        _ => Err(failure),
    }
}

pub fn run(
    root: &Command,
    args: &Args,
    env: &mut Env,
    settings: &Settings,
    readers: &[Box<dyn Reader>],
    finish: &Finish<'_>,
    labelled: &Labelled<'_>,
) -> Result<Outcome, Failure> {
    let request = request(args, settings.read_matches_listed.value)
        .map_err(|reason| usage(root, sanitize(&reason), Some("forget")))?;
    let layout = settings.layout();
    let waiting = Options {
        wait: Duration::MAX,
        ..settings.store.options()
    };
    let notice = &mut env.notice;
    let probe =
        Store::open_read_only(&layout, waiting, &mut |said| notice(&notice_message(&said)))?;
    forget::resolve(&request, &probe).map_err(|error| refused(root, &ForgetError::from(error)))?;
    drop(probe);
    Store::check(&layout)?;
    let notice = &mut env.notice;
    let mut store = Store::open(&layout, writer_options(settings), &mut |said| {
        notice(&notice_message(&said));
    })?;
    let output = match forget::forget(&request, labelled, &mut store) {
        Ok(output) => output,
        Err(error) => {
            store.release();
            return ended(root, error);
        }
    };
    let closed = finish(store, &mut *env.notice);
    let exit = if closed.failed || closed.refused {
        Exit::Refused
    } else {
        Exit::Success
    };
    let reads = |kind: &str| readers.iter().any(|reader| reader.kind() == kind);
    let mut diagnostics = if closed.failed {
        Vec::new()
    } else {
        view::lines(&output, &reads)
    };
    diagnostics.extend(closed.messages);
    Ok(Outcome::new(view::counts(&output))
        .with_diagnostics(diagnostics)
        .with_exit(exit))
}

#[cfg(test)]
mod tests {
    use super::{Args, request};

    fn reason(refs: &[&str]) -> String {
        let args = Args {
            refs: refs.iter().map(|text| (*text).to_owned()).collect(),
        };
        request(&args, 5).expect_err("a misuse")
    }

    #[test]
    fn each_misuse_of_forget_has_its_sentence() {
        let id = "01JC5Q3V7XKM4P8R2T6W9YB0DB";
        assert_eq!(reason(&[]), "missing <ref>");
        assert_eq!(
            reason(&[&format!("{id}#r1")]),
            format!("'{id}#r1' names records; an entry is forgotten whole")
        );
        assert_eq!(
            reason(&[&format!("{id}#g1")]),
            format!("'{id}#g1' names records; an entry is forgotten whole")
        );
        assert_eq!(
            reason(&[id, "not an id"]),
            "'not an id' is not an entry id, its last 10 characters, a session id or 8 or more characters of one"
        );
    }
}
