use clap::builder::NonEmptyStringValueParser;
use clap::{Arg, ArgAction, Command};

use crate::help::page::shape;

pub fn verb() -> Command {
    Command::new("probe")
        .about("Shows the shape of a verb page")
        .before_help("bilbo probe: shows the shape of a verb page.")
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
  <word>
One line per word.

Exit:
  0  success
  1  nothing matched
  2  usage error

Examples:
  bilbo probe --kind note word

Docs: https://github.com/delucca/bilbo/wiki/Commands#probe",
        )
}

/// Derive reads a verb outside `Verb` as none: one registered here runs as `missing verb`.
pub fn with_verb(root: Command, verb: Command) -> Command {
    root.subcommand(shape(verb))
}
