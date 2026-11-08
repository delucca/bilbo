pub mod help;
#[cfg(test)]
pub mod probe;
pub mod usage;

use std::ffi::OsString;

use clap::error::ErrorKind;
use clap::{Arg, ArgAction, ArgMatches, Command, FromArgMatches};

use crate::outcome::failure::Failure;

#[derive(Debug)]
pub enum Parsed {
    Help(String),
    Version,
    Matches(ArgMatches),
}

/// A hidden `-h, --help` keeps the Options list to the verb's own. clap's help flag stays off
/// because its long form would render `long_help` text that `help <verb>` does not.
pub fn shape(command: Command) -> Command {
    command
        .help_template(help::VERB_TEMPLATE)
        .disable_help_flag(true)
        .arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .action(ArgAction::Help)
                .hide(true),
        )
}

pub fn verbs(root: &Command) -> Vec<String> {
    root.get_subcommands()
        .map(|verb| verb.get_name().to_owned())
        .collect()
}

pub fn typed<T: FromArgMatches>(root: &Command, matches: &ArgMatches) -> Result<T, Failure> {
    T::from_arg_matches(matches)
        .map_err(|error| usage::usage(root, usage::reason(&error, false), None))
}

/// `args[0]` is the program. Every help request, `help <topic>` included, renders through
/// `help::overview` or `help::page`.
pub fn parse(root: &Command, args: &[OsString]) -> Result<Parsed, Failure> {
    if args.get(1).is_some_and(|arg| arg == "help") {
        return help::topic(root, &args[2..]).map(Parsed::Help);
    }
    let named = args
        .get(1)
        .and_then(|arg| arg.to_str())
        .filter(|arg| help::page(root, arg).is_some());
    match root.clone().try_get_matches_from(args) {
        Ok(matches) => Ok(Parsed::Matches(matches)),
        Err(error) => match error.kind() {
            ErrorKind::DisplayHelp => Ok(Parsed::Help(
                named
                    .and_then(|verb| help::page(root, verb))
                    .unwrap_or_else(|| help::overview(root)),
            )),
            ErrorKind::DisplayVersion => Ok(Parsed::Version),
            _ => Err(usage::usage(
                root,
                usage::reason(&error, named.is_some()),
                named,
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{Parsed, parse, typed};
    use crate::command::probe::{self, with_verb};
    use crate::outcome::failure::Failure;
    use crate::{Cli, bilbo};

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    fn usage_error_of(root: &clap::Command, list: &[&str]) -> (String, Vec<String>, String) {
        match parse(root, &args(list)) {
            Err(Failure::Usage {
                reason,
                usage,
                page,
            }) => (reason, usage, page),
            other => panic!("{list:?}: {other:?}"),
        }
    }

    fn usage_error(list: &[&str]) -> String {
        let (reason, usage, page) = usage_error_of(&bilbo(), list);
        format!("{reason} | {} | {page}", usage.join(" / "))
    }

    fn probe_root() -> clap::Command {
        with_verb(bilbo(), probe::verb())
    }

    fn probe_reason(list: &[&str]) -> String {
        let (reason, usage, page) = usage_error_of(&probe_root(), list);
        assert_eq!(
            usage,
            [
                "usage: bilbo probe [options] <word>",
                "       bilbo probe --all <word>"
            ],
            "{list:?}"
        );
        assert_eq!(page, "bilbo probe --help", "{list:?}");
        reason
    }

    const TOP: &str = "usage: bilbo <verb> [<args>]... | bilbo --help";

    #[test]
    fn the_definition_is_valid() {
        bilbo().debug_assert();
        probe_root().debug_assert();
    }

    #[test]
    fn claps_errors_come_out_in_bilbos_words() {
        assert_eq!(
            usage_error(&["bilbo", "--frobnicate"]),
            format!("unknown option '--frobnicate' | {TOP}")
        );
        assert_eq!(
            usage_error(&["bilbo", "-x"]),
            format!("unknown option '-x' | {TOP}")
        );
        assert_eq!(
            usage_error(&["bilbo", "frob"]),
            format!("unknown verb 'frob' | {TOP}")
        );
        assert_eq!(
            usage_error(&["bilbo", "frob", "--help"]),
            format!("unknown verb 'frob' | {TOP}")
        );
        assert_eq!(
            usage_error(&["bilbo", "--frobnicate", "--help"]),
            format!("unknown option '--frobnicate' | {TOP}")
        );
        assert_eq!(
            usage_error(&["bilbo", "--help=x"]),
            format!("unexpected value 'x' for '--help' found; no more were expected | {TOP}")
        );
    }

    #[test]
    fn a_missing_argument_names_what_is_missing() {
        assert_eq!(probe_reason(&["bilbo", "probe"]), "missing <word>");
        assert_eq!(
            probe_reason(&["bilbo", "probe", "--kind", "note"]),
            "missing <word>"
        );
    }

    #[test]
    fn several_missing_arguments_are_named_together() {
        let root = with_verb(
            bilbo(),
            clap::Command::new("pair")
                .arg(clap::Arg::new("kind").value_name("kind").required(true))
                .arg(clap::Arg::new("topic").value_name("topic").required(true)),
        );
        assert_eq!(
            usage_error_of(&root, &["bilbo", "pair"]).0,
            "missing <kind> and <topic>"
        );
    }

    #[test]
    fn an_option_without_its_value_says_it_needs_one() {
        assert_eq!(
            probe_reason(&["bilbo", "probe", "w", "--kind"]),
            "--kind needs a value"
        );
        assert_eq!(
            probe_reason(&["bilbo", "probe", "w", "-k"]),
            "--kind needs a value"
        );
    }

    #[test]
    fn an_empty_option_value_is_refused_as_a_missing_one() {
        for list in [
            &["bilbo", "probe", "w", "--kind="][..],
            &["bilbo", "probe", "w", "--kind", ""],
            &["bilbo", "probe", "w", "--times="],
        ] {
            let reason = probe_reason(list);
            assert!(reason.ends_with(" needs a value"), "{list:?}: {reason}");
        }
        assert_eq!(
            probe_reason(&["bilbo", "probe", "w", "--kind="]),
            "--kind needs a value"
        );
    }

    #[test]
    fn a_stray_argument_after_a_verb_is_unexpected_with_that_verbs_usage() {
        assert_eq!(
            probe_reason(&["bilbo", "probe", "one", "two"]),
            "unexpected argument 'two'"
        );
        assert_eq!(
            probe_reason(&["bilbo", "probe", "w", "--nope"]),
            "unknown option '--nope'"
        );
    }

    #[test]
    fn a_newline_in_an_argument_cannot_cut_the_reason() {
        assert_eq!(
            usage_error(&["bilbo", "--help=a\nb"]),
            format!("unexpected value 'a\\nb' for '--help' found; no more were expected | {TOP}")
        );
        assert_eq!(
            usage_error(&["bilbo", "a\r\nb"]),
            format!("unknown verb 'a\\r\\nb' | {TOP}")
        );
        assert_eq!(
            usage_error(&["bilbo", "--a\u{1b}[2J"]),
            format!("unknown option '--a\\u{{1b}}[2J' | {TOP}")
        );
    }

    #[test]
    fn help_and_version_are_results() {
        let root = bilbo();
        for list in [
            &["bilbo", "--help"][..],
            &["bilbo", "-h"],
            &["bilbo", "--help", "--frobnicate"],
        ] {
            assert!(
                matches!(parse(&root, &args(list)), Ok(Parsed::Help(_))),
                "{list:?}"
            );
        }
        for list in [
            &["bilbo", "--version"][..],
            &["bilbo", "-V"],
            &["bilbo", "--version", "x"],
        ] {
            assert!(
                matches!(parse(&root, &args(list)), Ok(Parsed::Version)),
                "{list:?}"
            );
        }
    }

    #[test]
    fn no_verb_parses_to_no_verb() {
        let root = bilbo();
        for list in [&["bilbo"][..], &["bilbo", "--"]] {
            match parse(&root, &args(list)) {
                Ok(Parsed::Matches(matches)) => {
                    let cli: Cli = typed(&root, &matches).expect("typed arguments");
                    assert!(cli.verb.is_none(), "{list:?}");
                }
                other => panic!("{list:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn a_registered_verb_parses_to_its_matches() {
        match parse(
            &probe_root(),
            &args(&["bilbo", "probe", "-k", "note", "word"]),
        ) {
            Ok(Parsed::Matches(matches)) => {
                let sub = matches.subcommand_matches("probe").expect("probe matches");
                assert_eq!(
                    sub.get_one::<String>("kind").map(String::as_str),
                    Some("note")
                );
                assert_eq!(
                    sub.get_one::<String>("word").map(String::as_str),
                    Some("word")
                );
            }
            other => panic!("{other:?}"),
        }
    }
}
