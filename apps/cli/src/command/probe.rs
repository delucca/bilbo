use clap::builder::NonEmptyStringValueParser;
use clap::{Arg, ArgAction, Command};

use crate::command::shape;

/// A short+long option, a long-only flag, an option with `long_help` and one positional: one per
/// help-layout branch the page tests cover.
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

/// Derive's typed arguments know only the variants of `Verb`, so a verb registered here parses
/// and pages but cannot be run.
pub fn with_verb(root: Command, verb: Command) -> Command {
    root.subcommand(shape(verb))
}
