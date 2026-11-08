//! The help pages: the overview's text, `bilbo help <topic>`, and the bold of the human view.
//! Every form of help (`--help`, `-h`, `help <verb>`) renders through `overview` and `page`.

use std::ffi::OsString;

use clap::builder::StyledStr;
use clap::{Arg, ArgAction, Command};

use crate::failure::Failure;
use crate::message::sanitize;
use crate::view::View;

pub const DESCRIPTION: &str = "\
bilbo keeps durable memory for coding agents: it ingests their sessions and
the pages they cite, and recalls them when a later session asks.";

pub const USAGE: &str = "bilbo <verb> [<args>]...";

/// clap's default template minus `{all-args}`: the verbs are listed in groups below and the
/// overview names no option. clap puts a blank line before `{after-help}` itself.
pub const TEMPLATE: &str = "{about-with-newline}\n{usage-heading} {usage}{after-help}";

/// A verb's page: what it does (the about text names the verb literally, because `{name}`
/// renders `bilbo-<verb>`), the usage block, the options, then the verb's `after_help` with its
/// Output, Exit, Examples and Docs. A verb with several usage forms joins them in
/// `override_usage` with `\n  `, so each form is indented two spaces under `Usage:`.
pub const VERB_TEMPLATE: &str = "{about}\n\nUsage:\n  {usage}\n\nOptions:\n{options}{after-help}";

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

/// A verb as bilbo pages it: its own template, and a hidden `-h, --help` so the Options list
/// holds only the verb's. clap's help flag stays off because its long form would render
/// `long_help` text that `help <verb>` does not.
pub fn verb(command: Command) -> Command {
    command
        .help_template(VERB_TEMPLATE)
        .disable_help_flag(true)
        .arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .action(ArgAction::Help)
                .hide(true),
        )
}

/// `bilbo --help`.
pub fn overview(root: &Command) -> String {
    render(root, None).unwrap_or_default()
}

/// `bilbo <verb> --help`, when `verb` exists.
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

/// Every verb, in the order they are declared.
pub fn verbs(root: &Command) -> Vec<String> {
    root.get_subcommands()
        .map(|verb| verb.get_name().to_owned())
        .collect()
}

/// The verbs the overview's groups name: the rows (two spaces, the verb, two spaces) between the
/// usage line and the exit codes.
pub fn named_verbs(overview: &str) -> Vec<String> {
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

/// The page `bilbo help` was asked for: the overview, or one verb's page. `-h` and `--help`
/// add nothing to it.
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

/// A usage failure for `bilbo` itself, or for `verb` when one was run.
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

/// `usage: <first form>`, later forms under it at column 7, and the verbs of the overview when
/// `top`.
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

/// The page on stdout: plain, or with bold headings and left columns when the view paints.
pub fn styled(page: &str, view: &View) -> String {
    if !view.paint {
        return page.to_owned();
    }
    page.lines().map(|line| bold(line) + "\n").collect()
}

/// The rendered page as bilbo prints it: plain, one trailing newline.
fn normalize(styled: &StyledStr) -> String {
    let mut text = styled.to_string().trim_end().to_owned();
    text.push('\n');
    text
}

/// `line` with its heading or its left column in bold. A heading starts the line with a capital
/// letter and runs to the first `:` over letters, spaces and commas, when that ends the line or
/// takes at most three words. A left column starts after two spaces (or six, for a long-only
/// option that clap aligns under options with a short flag) and ends before the first run of two
/// or more spaces.
pub fn bold(line: &str) -> String {
    const BOLD: &str = "\x1b[1m";
    const PLAIN: &str = "\x1b[22m";
    if let Some(end) = heading_end(line) {
        return format!("{BOLD}{}{PLAIN}{}", &line[..end], &line[end..]);
    }
    match column(line) {
        Some((indent, width)) => format!(
            "{}{BOLD}{}{PLAIN}{}",
            &line[..indent],
            &line[indent..indent + width],
            &line[indent + width..]
        ),
        None => line.to_owned(),
    }
}

/// The indent and width of the left column of `line`, if it has one.
fn column(line: &str) -> Option<(usize, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let width = rest.find("  ")?;
    let long_only = indent == 6 && rest.starts_with("--");
    ((indent == 2 && !rest.is_empty()) || long_only).then_some((indent, width))
}

/// Where the heading of `line` ends, just after its colon, if `line` opens with one.
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

    use clap::{Arg, Command};

    use super::{TEMPLATE, USAGE, bold, named_verbs, overview, page, topic, usage_lines, verbs};
    use crate::cli;
    use crate::failure::Failure;
    use crate::probe;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    /// The overview and the page of every verb of this build, plus the test-only verb.
    fn every_page() -> Vec<(String, String)> {
        let root = cli::with_verb(cli::command(), probe::verb());
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
    fn every_verb_page_has_the_shape_of_0_20() {
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
    fn the_overview_has_the_shape_of_0_20() {
        let text = overview(&cli::command());
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
        let root = cli::command();
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
            let root = cli::with_verb(bare, probe::verb());
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
        let root = cli::command();
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
        let root = cli::with_verb(cli::command(), probe::verb());
        assert!(verbs(&root).contains(&"probe".to_owned()));
        assert_eq!(
            topic(&root, &args(&["probe"])),
            Ok(page(&root, "probe").unwrap())
        );
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

    #[test]
    fn bold_marks_headings_and_left_columns_only() {
        assert_eq!(
            bold("Usage: bilbo <verb> [<args>]..."),
            "\x1b[1mUsage:\x1b[22m bilbo <verb> [<args>]..."
        );
        assert_eq!(
            bold("Paths, the first that applies:"),
            "\x1b[1mPaths, the first that applies:\x1b[22m"
        );
        assert_eq!(
            bold("Exit codes: 0 success"),
            "\x1b[1mExit codes:\x1b[22m 0 success"
        );
        assert_eq!(
            bold("  root    $BILBO_HOME"),
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
            assert_eq!(bold(plain), plain);
        }
    }

    #[test]
    fn bold_follows_the_option_rows_clap_renders() {
        let root = cli::with_verb(cli::command(), probe::verb());
        let rows: Vec<String> = page(&root, "probe")
            .expect("a page")
            .lines()
            .filter(|line| line.trim_start().starts_with('-'))
            .map(bold)
            .collect();
        assert_eq!(rows.len(), 3, "{rows:?}");
        let page = page(&root, "probe").expect("a page");
        let block: Vec<String> = page
            .lines()
            .filter(|line| line.starts_with("  bilbo probe "))
            .map(bold)
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
