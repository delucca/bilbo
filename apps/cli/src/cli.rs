//! The command line as clap parses it, and clap's errors in bilbo's words.

use std::ffi::OsString;

use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{ArgMatches, Command, CommandFactory, FromArgMatches, Parser, Subcommand};

use crate::failure::Failure;
use crate::help;
use crate::message::sanitize;

#[derive(Debug, Parser)]
#[command(
    name = "bilbo",
    version,
    about = help::DESCRIPTION,
    override_usage = help::USAGE,
    after_help = help::AFTER,
    help_template = help::TEMPLATE,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub verb: Option<Verb>,
}

/// Every verb is a variant, so registering one makes `run_command` name it in a `match`.
#[derive(Debug, Subcommand)]
pub enum Verb {}

/// What the arguments asked for.
#[derive(Debug)]
pub enum Parsed {
    /// A page, plain, ending in one newline.
    Help(String),
    Version,
    Matches(ArgMatches),
}

/// `bilbo` with every verb given the page shape of `help::verb`.
pub fn command() -> Command {
    let root = Cli::command();
    let names: Vec<String> = root
        .get_subcommands()
        .map(|verb| verb.get_name().to_owned())
        .collect();
    names
        .into_iter()
        .fold(root, |root, name| root.mut_subcommand(name, help::verb))
}

/// Registers a verb built by hand with the page shape of `help::verb`. Derive's typed arguments
/// know only the variants of `Verb`, so a registered verb that is not one cannot be run.
#[cfg(test)]
pub(crate) fn with_verb(root: Command, verb: Command) -> Command {
    root.subcommand(help::verb(verb))
}

/// The typed arguments behind `matches`, which `parse` got from `root`.
pub fn typed(root: &Command, matches: &ArgMatches) -> Result<Cli, Failure> {
    Cli::from_arg_matches(matches).map_err(|error| help::usage(root, reason(&error, false), None))
}

/// Parses `args` (`args[0]` is the program) against `root`. A help request renders through
/// `help::overview` or `help::page`, the same path `bilbo help <verb>` takes.
pub fn parse(root: &Command, args: &[OsString]) -> Result<Parsed, Failure> {
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
            _ => Err(help::usage(root, reason(&error, named.is_some()), named)),
        },
    }
}

/// clap's error in bilbo's words. `verb_named` says a known verb was run, so a stray word is an
/// unexpected argument, not an unknown verb. Outside text is escaped before it joins a reason.
fn reason(error: &clap::Error, verb_named: bool) -> String {
    let text = |kind: ContextKind| match error.get(kind) {
        Some(ContextValue::String(value)) => Some(sanitize(value)),
        _ => None,
    };
    match error.kind() {
        ErrorKind::UnknownArgument => match text(ContextKind::InvalidArg) {
            Some(arg) if arg.starts_with('-') => format!("unknown option '{arg}'"),
            Some(arg) if verb_named => format!("unexpected argument '{arg}'"),
            Some(arg) => format!("unknown verb '{arg}'"),
            None => bare(error),
        },
        ErrorKind::InvalidSubcommand => text(ContextKind::InvalidSubcommand)
            .map_or_else(|| bare(error), |name| format!("unknown verb '{name}'")),
        ErrorKind::MissingSubcommand => "missing verb".to_owned(),
        ErrorKind::MissingRequiredArgument => match error.get(ContextKind::InvalidArg) {
            Some(ContextValue::Strings(names)) if !names.is_empty() => {
                let names: Vec<String> = names.iter().map(|name| sanitize(name)).collect();
                format!("missing {}", list(&names))
            }
            _ => bare(error),
        },
        ErrorKind::InvalidValue
            if matches!(
                error.get(ContextKind::InvalidValue),
                Some(ContextValue::String(value)) if value.is_empty()
            ) =>
        {
            text(ContextKind::InvalidArg).map_or_else(
                || bare(error),
                |arg| format!("{} needs a value", option_name(&arg)),
            )
        }
        ErrorKind::InvalidUtf8 => "arguments must be valid UTF-8".to_owned(),
        _ => bare(error),
    }
}

/// `a`, `a and b`, `a, b and c`.
fn list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [only] => only.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

/// The long form of an option as clap shows it (`-k, --kind <kind>`), else its first word.
fn option_name(arg: &str) -> &str {
    let words = || {
        arg.split_whitespace()
            .map(|word| word.trim_end_matches(','))
    };
    words()
        .find(|word| word.starts_with("--"))
        .or_else(|| words().next())
        .unwrap_or(arg)
}

/// clap's own first line, without `error: `, its usage and its tips. Its context values are
/// escaped first: a newline inside an argument would end the line early.
fn bare(error: &clap::Error) -> String {
    let mut fresh = clap::Error::new(error.kind());
    for (kind, value) in error.context() {
        if !matches!(
            kind,
            ContextKind::Usage
                | ContextKind::Suggested
                | ContextKind::SuggestedArg
                | ContextKind::SuggestedCommand
                | ContextKind::SuggestedSubcommand
                | ContextKind::SuggestedValue
        ) {
            fresh.insert(kind, escaped(value));
        }
    }
    let rendered = fresh.render().to_string();
    let rendered = rendered.strip_prefix("error: ").unwrap_or(&rendered);
    let mut text = rendered.lines().next().unwrap_or_default().to_owned();
    if let (ErrorKind::ValueValidation, Some(source)) =
        (error.kind(), std::error::Error::source(error))
    {
        text = format!("{text}: {}", sanitize(&source.to_string()));
    }
    text
}

fn escaped(value: &ContextValue) -> ContextValue {
    match value {
        ContextValue::String(text) => ContextValue::String(sanitize(text)),
        ContextValue::Strings(texts) => {
            ContextValue::Strings(texts.iter().map(|text| sanitize(text)).collect())
        }
        ContextValue::StyledStr(text) => {
            ContextValue::StyledStr(sanitize(&text.to_string()).into())
        }
        ContextValue::StyledStrs(texts) => ContextValue::StyledStrs(
            texts
                .iter()
                .map(|text| sanitize(&text.to_string()).into())
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{Parsed, command, parse, typed, with_verb};
    use crate::failure::Failure;
    use crate::probe;

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
        let (reason, usage, page) = usage_error_of(&command(), list);
        format!("{reason} | {} | {page}", usage.join(" / "))
    }

    fn probe_root() -> clap::Command {
        with_verb(command(), probe::verb())
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
        command().debug_assert();
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
            command(),
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
        let root = command();
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
        let root = command();
        for list in [&["bilbo"][..], &["bilbo", "--"]] {
            match parse(&root, &args(list)) {
                Ok(Parsed::Matches(matches)) => {
                    let cli = typed(&root, &matches).expect("typed arguments");
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
