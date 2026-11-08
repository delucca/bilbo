//! A verb that exists only in tests: it stands in for the first real verb, so the page shape,
//! the dispatch and the guards are exercised before one exists.

use clap::builder::NonEmptyStringValueParser;
use clap::{Arg, ArgAction, Command};

/// A short+long option, a long-only flag, an option with `long_help` and one positional.
pub fn verb() -> Command {
    Command::new("probe")
        .about("bilbo probe: shows the shape of a verb page.")
        .override_usage("bilbo probe [options] <word>\n  bilbo probe --all <word>")
        .arg(Arg::new("word").value_name("word").required(true))
        .arg(
            Arg::new("kind")
                .short('k')
                .long("kind")
                .value_name("kind")
                .value_parser(NonEmptyStringValueParser::new())
                .help("Only this kind of note"),
        )
        .arg(
            Arg::new("all")
                .long("all")
                .action(ArgAction::SetTrue)
                .help("Every kind"),
        )
        .arg(
            Arg::new("times")
                .long("times")
                .value_name("n")
                .value_parser(NonEmptyStringValueParser::new())
                .help("How many times")
                .long_help("How many times.\n\nOnly the long help says this."),
        )
        .after_help(
            "\
Output:
  One line per word.

Exit: 0 success; 1 nothing matched; 2 usage error.

Examples:
  bilbo probe --kind note word

Docs: reference/commands.md#probe",
        )
}
