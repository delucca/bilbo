use std::ffi::OsString;

use clap::Command;
use clap::builder::StyledStr;

use crate::command::usage::usage;
use crate::outcome::Outcome;
use crate::outcome::failure::Failure;
use crate::terminal::View;
use crate::terminal::message::sanitize;
use crate::terminal::theme::{Tone, paint};

pub const DESCRIPTION: &str = "\
bilbo keeps durable memory for coding agents: it ingests their sessions and
the pages they cite, and recalls them when a later session asks.";

pub const USAGE: &str = "bilbo <verb> [<args>]...";

/// clap's default template minus `{all-args}`: the verbs are listed in groups below and the
/// overview names no option. clap puts a blank line before `{after-help}` itself.
pub const TEMPLATE: &str = "{about-with-newline}\n{usage-heading} {usage}{after-help}";

/// The about text names the verb: `{name}` renders `bilbo-<verb>`. Several usage forms join in
/// `override_usage` with `\n  `, so each sits two spaces under `Usage:`.
pub const VERB_TEMPLATE: &str = "{about}\n\nUsage:\n  {usage}\n\nOptions:\n{options}{after-help}";

/// The Paths block repeats `bilbo_store::paths`; nothing checks it.
pub const AFTER: &str = "\
No verbs yet.

Exit codes: 0 success; 1 refused, problems found or nothing matched; 2 usage
or config error.
Results go to stdout; diagnostics to stderr, prefixed 'bilbo: ' off a terminal.

Paths, the first that applies:
  root    $BILBO_HOME, $XDG_DATA_HOME/bilbo, ~/.local/share/bilbo
  config  $BILBO_CONFIG, $XDG_CONFIG_HOME/bilbo/config, ~/.config/bilbo/config
  cache   $XDG_CACHE_HOME/bilbo, ~/.cache/bilbo
  state   $XDG_STATE_HOME/bilbo, ~/.local/state/bilbo

Run 'bilbo <verb> --help' or 'bilbo help <verb>' for a verb's options, output
and exit codes.";

pub fn overview(root: &Command) -> String {
    render(root, None).unwrap_or_default()
}

pub fn page(root: &Command, verb: &str) -> Option<String> {
    render(root, Some(verb))
}

/// Built first, so a verb's usage reads `bilbo <verb>`, not `<verb>`. `render_help` and never
/// `render_long_help`: the short form is the one page every help form shares.
fn render(root: &Command, verb: Option<&str>) -> Option<String> {
    let mut root = root.clone();
    root.build();
    let command = match verb {
        Some(verb) => root.find_subcommand_mut(verb)?,
        None => &mut root,
    };
    Some(normalize(&command.render_help()))
}

pub fn topic(root: &Command, rest: &[OsString]) -> Result<String, Failure> {
    let rest: Vec<&OsString> = rest
        .iter()
        .filter(|arg| *arg != "-h" && *arg != "--help")
        .collect();
    match rest.as_slice() {
        [] => Ok(overview(root)),
        [word] if *word == "help" => Ok(overview(root)),
        [word] => {
            let name = word.to_string_lossy();
            page(root, &name).ok_or_else(|| {
                let shown = sanitize(&name);
                if name.starts_with('-') {
                    usage(root, format!("unknown option '{shown}'"), None)
                } else {
                    usage(root, format!("unknown verb '{shown}'"), None)
                }
            })
        }
        [_, extra, ..] => Err(usage(
            root,
            format!(
                "unexpected argument '{}'",
                sanitize(&extra.to_string_lossy())
            ),
            None,
        )),
    }
}

pub fn outcome(text: String) -> Outcome {
    Outcome::new(text.clone())
        .with_human(move |view| text.lines().map(|line| bold(view, line) + "\n").collect())
}

fn normalize(styled: &StyledStr) -> String {
    let mut text = styled.to_string().trim_end().to_owned();
    text.push('\n');
    text
}

/// A heading starts the line with a capital letter and runs to the first `:` over letters,
/// spaces and commas, when that ends the line or takes at most three words. A left column starts
/// after two spaces (or six, for a long-only option that clap aligns under options with a short
/// flag) and ends before the first run of two or more spaces.
pub fn bold(view: &View, line: &str) -> String {
    let strong = |text: &str| paint(view, Tone::Bold, text);
    if let Some(end) = heading_end(line) {
        return format!("{}{}", strong(&line[..end]), &line[end..]);
    }
    match column(line) {
        Some((indent, width)) => format!(
            "{}{}{}",
            &line[..indent],
            strong(&line[indent..indent + width]),
            &line[indent + width..]
        ),
        None => line.to_owned(),
    }
}

fn column(line: &str) -> Option<(usize, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let width = rest.find("  ")?;
    let long_only = indent == 6 && rest.starts_with("--");
    ((indent == 2 && !rest.is_empty()) || long_only).then_some((indent, width))
}

fn heading_end(line: &str) -> Option<usize> {
    if !line.starts_with(|c: char| c.is_ascii_uppercase()) {
        return None;
    }
    let at = line.find(':')?;
    let (label, rest) = (&line[..at], &line[at + 1..]);
    let words = label
        .chars()
        .all(|c| c.is_ascii_alphabetic() || c == ' ' || c == ',');
    let ends = rest.is_empty() || (rest.starts_with(' ') && label.split(' ').count() <= 3);
    (words && ends).then_some(at + 1)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use clap::Command;

    use super::{TEMPLATE, USAGE, bold, overview, page, topic};
    use crate::bilbo;
    use crate::command::probe::{self, with_verb};
    use crate::command::verbs;
    use crate::outcome::failure::Failure;
    use crate::terminal::View;

    fn named_verbs(overview: &str) -> Vec<String> {
        overview
            .lines()
            .skip_while(|line| !line.starts_with("Usage: "))
            .take_while(|line| !line.starts_with("Exit codes:"))
            .filter_map(|line| line.strip_prefix("  "))
            .filter_map(|row| row.split_once("  ").map(|(name, _)| name))
            .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase()))
            .map(str::to_owned)
            .collect()
    }

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    fn every_page() -> Vec<(String, String)> {
        let root = with_verb(bilbo(), probe::verb());
        let mut pages = vec![("overview".to_owned(), overview(&root))];
        for verb in verbs(&root) {
            let text = page(&root, &verb).expect("a registered verb has a page");
            pages.push((verb, text));
        }
        pages
    }

    #[test]
    fn no_help_line_is_longer_than_80_columns() {
        for (name, text) in every_page() {
            for line in text.lines() {
                assert!(line.chars().count() <= 80, "{name}: {line:?}");
            }
        }
    }

    #[test]
    fn every_verb_page_has_its_sections_in_order() {
        for (name, text) in every_page().into_iter().skip(1) {
            assert!(
                text.starts_with(&format!("bilbo {name}: ")),
                "{name}: {text}"
            );
            let mut from = 0;
            for heading in [
                "\n\nUsage:\n  bilbo ",
                "\n\nOptions:\n",
                "\n\nOutput:\n",
                "\n\nExit: ",
                "\n\nExamples:\n",
                "\n\nDocs: ",
            ] {
                let at = text[from..].find(heading).unwrap_or_else(|| {
                    panic!("{name}: {heading:?} missing or out of order in\n{text}")
                });
                from += at + heading.len();
            }
            assert!(!text.contains("--help"), "{name}: {text}");
        }
    }

    #[test]
    fn the_overview_has_its_sections_in_order() {
        let text = overview(&bilbo());
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle:?} missing"))
        };
        assert!(text.starts_with("bilbo keeps durable memory for coding agents: "));
        assert!(at("\nUsage: bilbo <verb> [<args>]...\n") < at("\nExit codes: "));
        assert!(at("\nExit codes: ") < at("\nResults go to stdout; "));
        assert!(at("\nResults go to stdout; ") < at("\nPaths, the first that applies:\n"));
        assert!(at("\nPaths, the first that applies:\n") < at("\nRun 'bilbo <verb> --help' or "));
        assert!(text.ends_with("and exit codes.\n"));
        assert!(!text.contains("Options:"));
    }

    #[test]
    fn the_overview_names_exactly_the_registered_verbs() {
        let root = bilbo();
        assert_eq!(named_verbs(&overview(&root)), verbs(&root));
    }

    #[test]
    fn the_drift_guard_sees_a_verb_the_groups_leave_out_or_invent() {
        let named = |after: &'static str| {
            let bare = Command::new("bilbo")
                .help_template(TEMPLATE)
                .override_usage(USAGE)
                .disable_help_subcommand(true)
                .after_help(after);
            let root = with_verb(bare, probe::verb());
            (named_verbs(&overview(&root)), verbs(&root))
        };
        let (groups, registered) = named("\nProbing:\n  probe  Shows a verb page\n\nExit codes: x");
        assert_eq!(groups, registered);
        let (groups, registered) = named("\nExit codes: x");
        assert_ne!(groups, registered);
        let (groups, registered) = named("\n  probe  a\n  ghost  b\n\nExit codes: x");
        assert_ne!(groups, registered);
    }

    #[test]
    fn help_topics_are_the_overview_a_page_or_a_usage_error() {
        let root = bilbo();
        for list in [&[][..], &["help"], &["--help"], &["help", "-h"]] {
            assert_eq!(topic(&root, &args(list)), Ok(overview(&root)), "{list:?}");
        }
        let reason = |result: Result<String, Failure>| match result {
            Err(Failure::Usage { reason, page, .. }) => format!("{reason}; {page}"),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            reason(topic(&root, &args(&["frob"]))),
            "unknown verb 'frob'; bilbo --help"
        );
        assert_eq!(
            reason(topic(&root, &args(&["--verbose"]))),
            "unknown option '--verbose'; bilbo --help"
        );
        assert_eq!(
            reason(topic(&root, &args(&["frob", "x"]))),
            "unexpected argument 'x'; bilbo --help"
        );
        assert_eq!(page(&root, "frob"), None);
    }

    #[test]
    fn a_registered_verb_has_a_page_and_is_listed() {
        let root = with_verb(bilbo(), probe::verb());
        assert!(verbs(&root).contains(&"probe".to_owned()));
        assert_eq!(
            topic(&root, &args(&["probe"])),
            Ok(page(&root, "probe").unwrap())
        );
    }

    #[test]
    fn bold_marks_headings_and_left_columns_only() {
        assert_eq!(
            bold(&View::HUMAN, "Usage: bilbo <verb> [<args>]..."),
            "\x1b[1mUsage:\x1b[22m bilbo <verb> [<args>]..."
        );
        assert_eq!(
            bold(&View::HUMAN, "Paths, the first that applies:"),
            "\x1b[1mPaths, the first that applies:\x1b[22m"
        );
        assert_eq!(
            bold(&View::HUMAN, "Exit codes: 0 success"),
            "\x1b[1mExit codes:\x1b[22m 0 success"
        );
        assert_eq!(
            bold(&View::HUMAN, "  root    $BILBO_HOME"),
            "  \x1b[1mroot\x1b[22m    $BILBO_HOME"
        );
        for plain in [
            "Results go to stdout; diagnostics to stderr, prefixed 'bilbo: ' off a terminal.",
            "bilbo keeps durable memory for coding agents: it ingests their sessions and",
            "No verbs yet.",
            "   continued",
            "      text that is not an option  x",
            "",
        ] {
            assert_eq!(bold(&View::HUMAN, plain), plain);
        }
    }

    #[test]
    fn bold_follows_the_option_rows_clap_renders() {
        let root = with_verb(bilbo(), probe::verb());
        let rows: Vec<String> = page(&root, "probe")
            .expect("a page")
            .lines()
            .filter(|line| line.trim_start().starts_with('-'))
            .map(|line| bold(&View::HUMAN, line))
            .collect();
        assert_eq!(rows.len(), 3, "{rows:?}");
        let page = page(&root, "probe").expect("a page");
        let block: Vec<String> = page
            .lines()
            .filter(|line| line.starts_with("  bilbo probe "))
            .map(|line| bold(&View::HUMAN, line))
            .collect();
        assert_eq!(block.len(), 3, "{block:?}");
        assert!(block.iter().all(|line| !line.contains('\x1b')), "{block:?}");
        for (row, column) in rows
            .iter()
            .zip(["-k, --kind <kind>", "--all", "--times <n>"])
        {
            assert!(row.contains(&format!("\x1b[1m{column}\x1b[22m")), "{row:?}");
        }
    }
}
