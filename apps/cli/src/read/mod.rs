pub mod view;

use bilbo_kinds::read::refs::Ref;
use bilbo_kinds::read::{self, BUDGET, Budget, Fetched, Output, Request, Shown};
use bilbo_kinds::reader::Reader;
use bilbo_kinds::tokens::Estimate;
use bilbo_settings::file::File;
use bilbo_settings::key::{Flag, Origin, Source};
use bilbo_store::segments::{Notice, Store};
use clap::Command;
use clap::builder::NonEmptyStringValueParser;

use crate::console::args::usage;
use crate::console::env::Env;
use crate::console::message::{Message, sanitize};
use crate::console::outcome::{Failure, Outcome};
use crate::console::settings::Settings;
use crate::console::theme::Mark;

const SUMMARY: &str = "Print an entry: its outline, or its records a page at a time";
const ABOUT: &str = "bilbo read: print an entry's outline, or its records a page at a time.";
const USAGE: &str = "bilbo read <ref> [--budget <tokens>]
  bilbo read <ref> --outline [--budget <tokens>]
<ref> is an entry id or a session id, then #r<a>-<b> for records a to b, #g<n>
for an older generation, or #g<n>:r<a>-<b>.";
const BUDGET_FLAG: &str = "--budget";

#[derive(Debug, clap::Args)]
#[command(about = SUMMARY, before_help = ABOUT, override_usage = USAGE, after_help = PAGE)]
pub struct Args {
    #[arg(value_name = "ref", value_parser = NonEmptyStringValueParser::new())]
    pub reference: String,
    #[arg(long, help = "List the prompts or sections of the range, not its text")]
    pub outline: bool,
    #[arg(
        long,
        value_name = "tokens",
        value_parser = NonEmptyStringValueParser::new(),
        help = "Print at most this many tokens a page"
    )]
    pub budget: Option<String>,
}

const PAGE: &str = "\
Output:
  #r<n> <label>: <text>  each record after its locator, as it was kept
  more: bilbo read ...   the last line: the command for the next page
  end: ...               the last line: nothing is left of what was asked
  cut: ...; more: ...    the last line after a cut record: how to read it whole
With no #r: metadata and outline. Only the last line is bilbo's own.

Exit:
  0  success, also when no record of the range renders
  1  no such entry, a range outside it, or no store
  2  usage or config error, or a session id that names several entries

Examples:
  bilbo read 01M6ZGE4KTBYYH07AS2KVMSCAF
  bilbo read 01M6ZGE4KTBYYH07AS2KVMSCAF#r120-184
  bilbo read 2d6b1857 --outline

Docs: https://github.com/delucca/bilbo/wiki/Commands#read";

const WAITING: &str = "waiting for another bilbo to finish writing the store";

/// The store is open, and its shared lock held, only while the entry's records are fetched: a
/// writer never waits for a page to render or to be written out.
pub fn run(
    root: &Command,
    args: &Args,
    env: &mut Env,
    settings: &Settings,
    readers: &[Box<dyn Reader>],
) -> Result<Outcome, Failure> {
    let reference = Ref::parse(&args.reference)
        .map_err(|malformed| usage(root, sanitize(&malformed.to_string()), Some("read")))?;
    let none = File::default();
    let source = Source {
        var: &*env.var,
        file: &none,
    };
    let flag = args
        .budget
        .as_deref()
        .map(|text| Flag::parse(&BUDGET, &source, BUDGET_FLAG, text))
        .transpose()?;
    let budget = settings.read_budget.clone().with_flag(flag);
    let request = Request {
        reference,
        outline: args.outline,
        budget: budget.value,
    };

    let notice = &mut env.notice;
    let store = Store::open_read_only(&settings.layout(), settings.store.options(), &mut |said| {
        if said == Notice::Waiting {
            notice(&Message {
                mark: Mark::Info,
                text: WAITING.to_owned(),
                details: Vec::new(),
            });
        }
    })
    .map_err(|error| Failure::from(&error))?;
    let fetched = read::fetch(&request.reference, &store);
    drop(store);

    let given = matches!(budget.origin, Origin::Flag(_)).then_some(budget.value);
    let output = fetched
        .and_then(|fetched| rendered(&fetched, &request, readers, given))
        .map_err(|error| failure(root, &error))?;
    Ok(Outcome::new(view::plain(&output, given))
        .with_human(move |out| view::shown(&output, given, out))
        .on_every_terminal())
}

/// The page the views of this verb print: counted by the estimate, an outline's lines chosen
/// after what `view::plain` prints around them. `given` is the budget a flag gave.
pub fn rendered(
    fetched: &Fetched,
    request: &Request,
    readers: &[Box<dyn Reader>],
    given: Option<Budget>,
) -> Result<Output, read::Error> {
    let shown = Shown {
        tokens: &Estimate,
        beside: &|frame| view::beside(frame, given),
    };
    read::render(fetched, request, readers, &shown)
}

fn failure(root: &Command, error: &read::Error) -> Failure {
    match error {
        read::Error::Store(error) => Failure::from(error),
        read::Error::Ambiguous { .. } => usage(root, sanitize(&error.to_string()), Some("read")),
        read::Error::NoEntry(_)
        | read::Error::NoGeneration { .. }
        | read::Error::OutOfRange { .. }
        | read::Error::UnreadKind { .. } => Failure::Refused(sanitize(&error.to_string())),
    }
}
