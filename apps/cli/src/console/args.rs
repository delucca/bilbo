use std::ffi::OsString;

use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{ArgMatches, Command, FromArgMatches};

use crate::console::message::sanitize;
use crate::console::outcome::Failure;

#[derive(Debug)]
pub enum Parsed {
    Help(Vec<OsString>),
    Version,
    Matches(ArgMatches),
}

pub fn verbs(root: &Command) -> Vec<String> {
    root.get_subcommands()
        .map(|verb| verb.get_name().to_owned())
        .collect()
}

pub fn typed<T: FromArgMatches>(root: &Command, matches: &ArgMatches) -> Result<T, Failure> {
    T::from_arg_matches(matches).map_err(|error| usage(root, reason(&error, false), None))
}

pub fn parse(root: &Command, args: &[OsString]) -> Result<Parsed, Failure> {
    if args.get(1).is_some_and(|arg| arg == "help") {
        return Ok(Parsed::Help(args[2..].to_vec()));
    }
    let named = args
        .get(1)
        .and_then(|arg| arg.to_str())
        .filter(|arg| root.find_subcommand(arg).is_some());
    match root.clone().try_get_matches_from(args) {
        Ok(matches) => Ok(Parsed::Matches(matches)),
        Err(error) => match error.kind() {
            ErrorKind::DisplayHelp => Ok(Parsed::Help(
                named.map(OsString::from).into_iter().collect(),
            )),
            ErrorKind::DisplayVersion => Ok(Parsed::Version),
            _ => Err(usage(root, reason(&error, named.is_some()), named)),
        },
    }
}

pub fn usage(root: &Command, reason: String, verb: Option<&str>) -> Failure {
    let mut root = root.clone();
    root.build();
    let known = verb.filter(|verb| root.find_subcommand(verb).is_some());
    let (usage, page) = match known {
        Some(verb) => {
            let command = root
                .find_subcommand_mut(verb)
                .expect("the verb was just found");
            (usage_lines(command, false), format!("bilbo {verb} --help"))
        }
        None => (usage_lines(&mut root, true), "bilbo --help".to_owned()),
    };
    Failure::Usage {
        reason,
        usage,
        page,
    }
}

pub fn usage_lines(command: &mut Command, top: bool) -> Vec<String> {
    let rendered = command.render_usage().to_string();
    let mut lines: Vec<String> = rendered
        .lines()
        .enumerate()
        .map(|(i, line)| match i {
            0 => line.replacen("Usage: ", "usage: ", 1),
            _ => format!("       {}", line.trim_start()),
        })
        .collect();
    let verbs = verbs(command);
    if top && !verbs.is_empty() {
        lines.push(format!("verbs: {}", verbs.join(", ")));
    }
    lines
}

pub fn reason(error: &clap::Error, verb_named: bool) -> String {
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

fn list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [only] => only.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

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

    use clap::{Arg, Command};

    use super::{Parsed, parse, typed, usage_lines};
    use crate::console::outcome::Failure;
    use crate::probe::{self, with_verb};
    use crate::{Cli, bilbo};

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    fn usage_error_of(root: &Command, list: &[&str]) -> (String, Vec<String>, String) {
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

    fn probe_root() -> Command {
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

    const TOP: &str = "usage: bilbo <verb> [<args>]... / verbs: remember | bilbo --help";

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
            Command::new("pair")
                .arg(Arg::new("kind").value_name("kind").required(true))
                .arg(Arg::new("topic").value_name("topic").required(true)),
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
    fn help_comes_back_as_the_topic_words() {
        let root = probe_root();
        let topic = |list: &[&str]| match parse(&root, &args(list)) {
            Ok(Parsed::Help(words)) => words,
            other => panic!("{list:?}: {other:?}"),
        };
        for list in [
            &["bilbo", "--help"][..],
            &["bilbo", "-h"],
            &["bilbo", "help"],
        ] {
            assert!(topic(list).is_empty(), "{list:?}");
        }
        assert_eq!(topic(&["bilbo", "--help", "--frobnicate"]), args(&[]));
        assert_eq!(topic(&["bilbo", "probe", "--help"]), args(&["probe"]));
        assert_eq!(topic(&["bilbo", "probe", "-h"]), args(&["probe"]));
        assert_eq!(
            topic(&["bilbo", "help", "probe", "--help"]),
            args(&["probe", "--help"])
        );
        assert_eq!(topic(&["bilbo", "help", "x", "y"]), args(&["x", "y"]));
    }

    #[test]
    fn version_is_a_result() {
        let root = bilbo();
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

    #[test]
    fn usage_lines_fold_claps_forms_and_list_the_verbs() {
        let mut two_forms = Command::new("bilbo")
            .override_usage("bilbo recall <query>...\n  bilbo recall --library <query>...");
        assert_eq!(
            usage_lines(&mut two_forms, false),
            [
                "usage: bilbo recall <query>...",
                "       bilbo recall --library <query>..."
            ]
        );
        let mut with_verbs = Command::new("bilbo")
            .disable_help_subcommand(true)
            .override_usage("bilbo <verb> [<args>]...")
            .subcommand(Command::new("ingest"))
            .subcommand(Command::new("recall").arg(Arg::new("q")));
        assert_eq!(
            usage_lines(&mut with_verbs, true),
            ["usage: bilbo <verb> [<args>]...", "verbs: ingest, recall"]
        );
    }
}
