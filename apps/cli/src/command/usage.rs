use clap::Command;
use clap::error::{ContextKind, ContextValue, ErrorKind};

use crate::command::verbs;
use crate::outcome::failure::Failure;
use crate::terminal::message::sanitize;

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

/// `verb_named` says a known verb was run, so a stray word is an unexpected argument, not an
/// unknown verb.
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

/// Context values are escaped before clap renders them: a newline inside an argument would end
/// the line early.
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
    use clap::{Arg, Command};

    use super::usage_lines;

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
