#![warn(clippy::self_named_module_files)]

mod check;
mod citation;
mod host;
mod identity;
mod library;
mod note;
mod relay;
mod search;
mod setup;
mod shared;
mod sync;

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

/// Why a verb did not succeed; `main` maps it to stderr and an exit code.
pub enum Failure {
    /// Exit 2: the message, then the synopsis of the verb that was run, or of `bilbo` itself.
    Usage(String),
    /// Exit 2: the message only, for store-root resolution errors.
    Config(String),
    /// Exit 1: the message only.
    Refused(String),
}

/// `bilbo --help`, `bilbo -h` and `bilbo help`. Its groups list every verb of `PAGES`, in the same order.
const OVERVIEW: &str = r#"bilbo keeps durable memory for coding agents: notes they write and recall,
and a library of sources they cite.

Usage: bilbo <verb> [<args>]...

Notes:
  new       Create a note of a kind on a topic, and print its path
  recall    Search the notes, best match first; --library searches sources
  check     Print every problem in the notes and the library
  history   List a note's past versions, print one, or diff two
  restore   Make a past version of a note its newest version

Library:
  library   List corpora, show a source, add one, or read one through a plan
  cite      Check the bilbo: citations in a draft against the store

Scopes and sync:
  scope     List this device's scopes, or give notes a scope
  sync      Show each syncing scope's state; declare dropped text
  device    Show this device and its owner; set up keys; list or revoke devices
  pair      Join another device to your scopes with a one-time code
  relay     Serve the sync transport to your devices over HTTP

Setup and services:
  setup     Create the store and config; install the plugin and services
  index     Embed new passages for search by meaning (run by a timer)
  watch     Record note versions and sync scopes (run as a login service)
  digest    Print the notes that bear on a prompt (run by the prompt hook)

Examples:
  bilbo setup
  bilbo new decision release-tags
  bilbo recall release tags
  bilbo recall --library --corpus go -- 'goroutine leaks'

Exit codes: 0 success; 1 refused, problems found or nothing matched; 2 usage
or config error. digest always exits 0.
Results go to stdout. Diagnostics go to stderr, each line prefixed 'bilbo: '.

Paths, the first that applies:
  root    $BILBO_HOME, $XDG_DATA_HOME/bilbo, ~/.local/share/bilbo
  config  $BILBO_CONFIG, $XDG_CONFIG_HOME/bilbo/config, ~/.config/bilbo/config
  cache   $XDG_CACHE_HOME/bilbo, ~/.cache/bilbo
  state   $XDG_STATE_HOME/bilbo, ~/.local/state/bilbo

Run 'bilbo <verb> --help' or 'bilbo help <verb>' for a verb's options, output
and exit codes.
Docs: https://github.com/delucca/bilbo/wiki
"#;

/// Every verb and its page, in the order of the overview and of the `verbs:` line of a usage error.
const PAGES: [(&str, &str); 16] = [
    ("new", note::new::HELP),
    ("recall", search::recall::HELP),
    ("check", check::HELP),
    ("history", note::history::HELP),
    ("restore", note::restore::HELP),
    ("library", library::cli::HELP),
    ("cite", citation::cite::HELP),
    ("scope", note::scope::HELP),
    ("sync", sync::cli::HELP),
    ("device", identity::device::HELP),
    ("pair", identity::pair::HELP),
    ("relay", relay::HELP),
    ("setup", setup::HELP),
    ("index", search::index::HELP),
    ("watch", note::watch::HELP),
    ("digest", search::digest::HELP),
];

const BOLD: &str = "\x1b[1m";
const PLAIN: &str = "\x1b[22m";

fn main() -> ExitCode {
    let args = match collect_args() {
        Ok(args) => args,
        Err(failure) => return report(failure, &[]),
    };
    match run(&args) {
        Ok(code) => code,
        Err(failure) => report(failure, &args),
    }
}

fn run(args: &[String]) -> Result<ExitCode, Failure> {
    if args.first().is_some_and(|arg| arg == "help") {
        print_help(help_topic(&args[1..])?);
        return Ok(ExitCode::SUCCESS);
    }
    if wants_help(args) {
        let text = match args.first() {
            Some(verb) if !verb.starts_with('-') => page(verb).ok_or_else(|| unknown_verb(verb))?,
            _ => OVERVIEW,
        };
        print_help(text);
        return Ok(ExitCode::SUCCESS);
    }
    if args == ["--version"] {
        print_stdout(concat!("bilbo ", env!("CARGO_PKG_VERSION")));
        return Ok(ExitCode::SUCCESS);
    }
    let env = shared::store::Env::from_process();
    match args.first().map(String::as_str) {
        None => Err(Failure::Usage("missing verb".into())),
        Some("new") => {
            let output = note::new::run(&args[1..], &env)?;
            output.warning.iter().for_each(|line| print_stderr(line));
            print_stdout(&output.path.display().to_string());
            Ok(ExitCode::SUCCESS)
        }
        Some("check") => {
            let output = check::run(&args[1..], &env)?;
            output.lines.iter().for_each(|line| print_stdout(line));
            Ok(if output.failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Some("recall") => {
            let found = search::recall::run(&args[1..], &env)?;
            found.warnings.iter().for_each(|line| print_stderr(line));
            found.lines.iter().for_each(|line| print_stdout(line));
            Ok(ExitCode::SUCCESS)
        }
        Some("index") => {
            let output = search::index::run(&args[1..], &env)?;
            output.warnings.iter().for_each(|line| print_stderr(line));
            print_stdout(&output.line);
            Ok(ExitCode::SUCCESS)
        }
        Some("setup") => {
            let outcome = setup::run(&args[1..], &env, &mut |line: &str| print_stderr(line))?;
            outcome.lines.iter().for_each(|line| print_stdout(line));
            Ok(if outcome.failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Some("digest") => {
            let outcome = search::digest::run(&args[1..], &mut std::io::stdin().lock(), &env);
            outcome.lines.iter().for_each(|line| print_stdout(line));
            if let Some(line) = &outcome.diagnostic {
                print_stderr(line);
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("library") => {
            let output = library::cli::run(&args[1..], &env)?;
            output.warnings.iter().for_each(|line| print_stderr(line));
            output.lines.iter().for_each(|line| print_stdout(line));
            Ok(ExitCode::SUCCESS)
        }
        Some("cite") => {
            let output = citation::cite::run(&args[1..], &mut std::io::stdin().lock(), &env)?;
            output.warnings.iter().for_each(|line| print_stderr(line));
            output.lines.iter().for_each(|line| print_stdout(line));
            Ok(if output.failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Some("watch") => {
            note::watch::run(&args[1..], &env, &mut |line: &str| print_stderr(line))?;
            Ok(ExitCode::SUCCESS)
        }
        Some("history") => {
            let output = note::history::run(&args[1..], &env)?;
            output.warnings.iter().for_each(|line| print_stderr(line));
            output.lines.iter().for_each(|line| print_stdout(line));
            write_stdout(&output.bytes);
            Ok(ExitCode::SUCCESS)
        }
        Some("restore") => {
            let output = note::restore::run(&args[1..], &env)?;
            output.warnings.iter().for_each(|line| print_stderr(line));
            output.lines.iter().for_each(|line| print_stdout(line));
            Ok(ExitCode::SUCCESS)
        }
        Some("scope") => {
            let output = note::scope::run(&args[1..], &env)?;
            output.warnings.iter().for_each(|line| print_stderr(line));
            output.lines.iter().for_each(|line| print_stdout(line));
            Ok(if output.failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Some("device") => {
            let terminal = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
            let output =
                identity::device::run(&args[1..], &env, terminal, &mut host::prompt::Terminal)?;
            output.warnings.iter().for_each(|line| print_stderr(line));
            output.lines.iter().for_each(|line| print_stdout(line));
            Ok(if output.failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Some("sync") => {
            let output = sync::cli::run(&args[1..], &env)?;
            output.warnings.iter().for_each(|line| print_stderr(line));
            output.lines.iter().for_each(|line| print_stdout(line));
            Ok(if output.failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Some("pair") => {
            let terminal = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
            identity::pair::run(
                &args[1..],
                &env,
                terminal,
                &mut std::io::stdin().lock(),
                &identity::pair::Limits::default(),
                &mut |line: &str| print_stdout(line),
                &mut |line: &str| print_stderr(line),
            )?;
            Ok(ExitCode::SUCCESS)
        }
        Some("relay") => {
            // A panicking request gets a 500 and the relay keeps serving, so the panic is one more line.
            std::panic::set_hook(Box::new(|info| {
                print_stderr(&format!("internal error: {info}"));
            }));
            relay::run(&args[1..], &|line: &str| print_stderr(line))?;
            Ok(ExitCode::SUCCESS)
        }
        Some("--version") => Err(Failure::Usage(format!("unexpected argument '{}'", args[1]))),
        Some(arg) if arg.starts_with('-') => Err(Failure::Usage(format!("unknown option '{arg}'"))),
        Some(arg) => Err(unknown_verb(arg)),
    }
}

/// The page `bilbo help` was asked for: the overview, or one verb's page. `-h` and `--help` add nothing to it.
fn help_topic(rest: &[String]) -> Result<&'static str, Failure> {
    let rest: Vec<&String> = rest
        .iter()
        .filter(|arg| *arg != "-h" && *arg != "--help")
        .collect();
    match rest.as_slice() {
        [] => Ok(OVERVIEW),
        [verb] if *verb == "help" => Ok(OVERVIEW),
        [verb] => page(verb).ok_or_else(|| {
            if verb.starts_with('-') {
                Failure::Usage(format!("unknown option '{verb}'"))
            } else {
                unknown_verb(verb)
            }
        }),
        [_, extra, ..] => Err(Failure::Usage(format!("unexpected argument '{extra}'"))),
    }
}

fn page(verb: &str) -> Option<&'static str> {
    PAGES
        .iter()
        .find(|(name, _)| *name == verb)
        .map(|(_, text)| *text)
}

fn unknown_verb(name: &str) -> Failure {
    Failure::Usage(match suggestion(name) {
        Some(verb) => format!("unknown verb '{name}'; did you mean '{verb}'?"),
        None => format!("unknown verb '{name}'"),
    })
}

/// The verb `name` most likely meant: the only verb it is a prefix of; else, when it is a prefix of none, the only
/// verb nearest to it, at an edit distance of at most 2 and below its length.
fn suggestion(name: &str) -> Option<&'static str> {
    let verbs = PAGES.map(|(verb, _)| verb);
    let prefixed: Vec<&str> = verbs
        .into_iter()
        .filter(|verb| !name.is_empty() && verb.starts_with(name))
        .collect();
    match prefixed.as_slice() {
        [verb] => return Some(verb),
        [] => {}
        _ => return None,
    }
    let distances = verbs.map(|verb| (distance(name, verb), verb));
    let nearest = distances.iter().map(|(d, _)| *d).min()?;
    let closest: Vec<&str> = distances
        .iter()
        .filter(|(d, _)| *d == nearest)
        .map(|(_, verb)| *verb)
        .collect();
    match closest.as_slice() {
        [verb] if nearest <= 2 && nearest < name.chars().count() => Some(verb),
        _ => None,
    }
}

/// The Levenshtein distance between `a` and `b`, by characters.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (diagonal + usize::from(ca != *cb))
                .min(row[j] + 1)
                .min(row[j + 1] + 1);
            diagonal = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

fn collect_args() -> Result<Vec<String>, Failure> {
    std::env::args_os()
        .skip(1)
        .map(|arg| {
            arg.into_string()
                .map_err(|_| Failure::Usage("arguments must be valid UTF-8".into()))
        })
        .collect()
}

/// `-h` or `--help` before a bare `--`, except as the value of `--title` of `new` and `library`, or of `--scope` of `new`.
fn wants_help(args: &[String]) -> bool {
    let verb = args.first().map(String::as_str);
    let has_title = matches!(verb, Some("new" | "library"));
    let has_scope = verb == Some("new");
    let mut skip_value = false;
    for arg in args.iter().take_while(|arg| *arg != "--") {
        if std::mem::take(&mut skip_value) {
            continue;
        }
        skip_value = (has_title && arg == "--title") || (has_scope && arg == "--scope");
        if arg == "-h" || arg == "--help" {
            return true;
        }
    }
    false
}

/// Write errors are ignored: a closed pipe must not panic.
fn print_stdout(line: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{line}");
}

/// Writes bytes as they are, with no newline added: a printed version may not end in one.
fn write_stdout(bytes: &[u8]) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(bytes).and_then(|()| out.flush());
}

/// The one writer of stderr: every physical line gets the prefix, even inside an echoed argument.
fn print_stderr(message: &str) {
    let mut err = std::io::stderr().lock();
    for line in message.split('\n') {
        let _ = writeln!(err, "bilbo: {line}");
    }
}

/// Prints a help text to stdout, in bold where `bold` says when `styled` allows it.
fn print_help(text: &str) {
    let styled = styled(
        std::io::stdout().is_terminal(),
        std::env::var_os("NO_COLOR"),
        std::env::var_os("TERM"),
    );
    for line in text.lines() {
        if styled {
            print_stdout(&bold(line));
        } else {
            print_stdout(line);
        }
    }
}

/// Bold only for a terminal, with `NO_COLOR` unset or empty and `TERM` not `dumb`.
fn styled(
    terminal: bool,
    no_color: Option<std::ffi::OsString>,
    term: Option<std::ffi::OsString>,
) -> bool {
    terminal && no_color.is_none_or(|v| v.is_empty()) && term.is_none_or(|t| t != "dumb")
}

/// `line` with its heading or its left column in bold. A heading starts the line with a capital letter and runs to
/// the first `:` over letters, spaces and commas, when that ends the line or takes at most three words. A left column
/// starts after exactly two spaces and ends before the first run of two or more spaces.
fn bold(line: &str) -> String {
    if let Some(end) = heading_end(line) {
        return format!("{BOLD}{}{PLAIN}{}", &line[..end], &line[end..]);
    }
    let column = line
        .strip_prefix("  ")
        .filter(|rest| !rest.is_empty() && !rest.starts_with(' '))
        .and_then(|rest| rest.find("  "));
    match column {
        Some(width) => format!(
            "  {BOLD}{}{PLAIN}{}",
            &line[2..2 + width],
            &line[2 + width..]
        ),
        None => line.to_string(),
    }
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

/// The forms of a page's `Usage:` block, a wrapped form joined back into one line.
fn forms(text: &str) -> Vec<String> {
    let mut forms: Vec<String> = Vec::new();
    for line in text.lines().skip_while(|l| *l != "Usage:").skip(1) {
        if let Some(form) = line.strip_prefix("  bilbo ") {
            forms.push(format!("bilbo {form}"));
        } else if line.starts_with("   ") && !forms.is_empty() {
            let last = forms.len() - 1;
            forms[last] = format!("{} {}", forms[last], line.trim());
        } else {
            break;
        }
    }
    forms
}

/// What a usage error prints after its message: the forms of the verb that was run, only those of its subcommand when
/// the argument after the verb names one, and the page to read; or, for no verb or an unknown one, `bilbo`'s.
fn usage_lines(args: &[String]) -> Vec<String> {
    let Some((verb, text)) = args
        .first()
        .and_then(|verb| page(verb).map(|text| (verb, text)))
    else {
        return vec![
            "usage: bilbo <verb> [<args>]...".to_string(),
            format!("verbs: {}", PAGES.map(|(verb, _)| verb).join(", ")),
            "see 'bilbo --help'".to_string(),
        ];
    };
    let all = forms(text);
    let named: Vec<&String> = all
        .iter()
        .filter(|form| {
            let word = form.split(' ').nth(2);
            word.is_some_and(|w| w.chars().all(|c| c.is_ascii_lowercase()))
                && word == args.get(1).map(String::as_str)
        })
        .collect();
    let shown: Vec<&String> = if named.is_empty() {
        all.iter().collect()
    } else {
        named
    };
    let mut lines: Vec<String> = shown
        .iter()
        .enumerate()
        .map(|(i, form)| format!("{}{form}", if i == 0 { "usage: " } else { "       " }))
        .collect();
    lines.push(format!("see 'bilbo {verb} --help'"));
    lines
}

fn report(failure: Failure, args: &[String]) -> ExitCode {
    match failure {
        Failure::Usage(message) => {
            print_stderr(&message);
            usage_lines(args).iter().for_each(|line| print_stderr(line));
            ExitCode::from(2)
        }
        Failure::Config(message) => {
            print_stderr(&message);
            ExitCode::from(2)
        }
        Failure::Refused(message) => {
            print_stderr(&message);
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_text() -> Vec<(&'static str, &'static str)> {
        let mut texts = vec![("bilbo", OVERVIEW)];
        texts.extend(PAGES);
        texts
    }

    fn args(text: &str) -> Vec<String> {
        text.split(' ').map(String::from).collect()
    }

    #[test]
    fn every_help_line_fits_80_columns() {
        for (name, text) in every_text() {
            for line in text.lines() {
                assert!(line.chars().count() <= 80, "{name}: {line}");
                assert_eq!(line, line.trim_end(), "{name}: a trailing space");
            }
            assert!(text.ends_with('\n') && !text.ends_with("\n\n"), "{name}");
            assert!(!text.contains(['\t', '\x1b']), "{name}");
        }
    }

    #[test]
    fn the_overview_groups_every_verb_once_in_order() {
        let groups = OVERVIEW.split("\nExamples:").next().unwrap();
        let listed: Vec<&str> = groups
            .lines()
            .filter_map(|line| line.strip_prefix("  "))
            .filter_map(|line| line.split_once("  ").map(|(verb, _)| verb))
            .collect();
        assert_eq!(listed, PAGES.map(|(verb, _)| verb));
    }

    #[test]
    fn help_for_help_is_the_overview() {
        let ask = |words: &[&str]| {
            let words: Vec<String> = words.iter().map(|w| w.to_string()).collect();
            help_topic(&words)
        };
        assert_eq!(ask(&["help"]).ok(), Some(OVERVIEW));
        assert_eq!(ask(&["help", "--help"]).ok(), Some(OVERVIEW));
        assert!(ask(&["help", "help"]).is_err());
    }

    #[test]
    fn every_page_has_the_shape() {
        for (verb, text) in PAGES {
            let lines: Vec<&str> = text.lines().collect();
            assert!(lines[0].starts_with(&format!("bilbo {verb}: ")), "{verb}");
            let docs = format!(
                "Docs: {}/wiki/Commands#{verb}",
                env!("CARGO_PKG_REPOSITORY")
            );
            assert_eq!(lines.last(), Some(&docs.as_str()), "{verb}");
            let at = |head: &str| {
                lines
                    .iter()
                    .position(|line| line.starts_with(head))
                    .unwrap_or_else(|| panic!("{verb}: no {head}"))
            };
            assert!(at("Usage:") < at("Exit:"), "{verb}");
            assert!(at("Exit:") < at("Examples:"), "{verb}");
            let forms = forms(text);
            let own = format!("bilbo {verb}");
            assert!(!forms.is_empty(), "{verb}");
            for form in &forms {
                assert!(
                    *form == own || form.starts_with(&format!("{own} ")),
                    "{verb}: {form}"
                );
            }
        }
    }

    #[test]
    fn forms_join_a_wrapped_form() {
        let library = forms(library::cli::HELP);
        assert_eq!(library.len(), 7);
        assert_eq!(
            library[3],
            "bilbo library stage <file> --origin \"<url|doc>: <value>\" [--fetched <YYYY-MM-DD>] [--html]"
        );
        assert_eq!(forms(check::HELP), ["bilbo check"]);
    }

    #[test]
    fn a_usage_error_shows_the_forms_of_the_verb_or_its_subcommand() {
        assert_eq!(
            usage_lines(&args("library land")),
            [
                "usage: bilbo library land <stage> <corpus>/<name> --keep <a>-<b>[,<c>-<d>]... [--title <text>] [--replace [--force]]",
                "see 'bilbo library --help'",
            ]
        );
        assert_eq!(usage_lines(&args("library stage x")).len(), 3);
        assert_eq!(usage_lines(&args("library go x")).len(), 8);
        assert_eq!(
            usage_lines(&args("recall --bogus")),
            [
                "usage: bilbo recall <query>... [--kind <kind>]... [--limit <n>]",
                "       bilbo recall <query>... --library [--corpus <corpus>]... [--limit <n>]",
                "see 'bilbo recall --help'",
            ]
        );
        assert_eq!(usage_lines(&args("relay --data")).len(), 2);
        let top = usage_lines(&[]);
        assert_eq!(top.len(), 3);
        assert_eq!(top[0], "usage: bilbo <verb> [<args>]...");
        assert_eq!(top[2], "see 'bilbo --help'");
        assert_eq!(usage_lines(&args("frobnicate x")), top);
        assert_eq!(usage_lines(&args("help recall x")), top);
    }

    #[test]
    fn a_near_name_or_a_unique_prefix_gets_a_suggestion() {
        assert_eq!(suggestion("recal"), Some("recall"));
        assert_eq!(suggestion("rcall"), Some("recall"));
        assert_eq!(suggestion("hist"), Some("history"));
        assert_eq!(suggestion("sope"), Some("scope"));
        assert_eq!(suggestion("nw"), Some("new"));
        assert_eq!(suggestion("re"), None);
        assert_eq!(suggestion("frobnicate"), None);
        assert_eq!(suggestion("x"), None);
        assert_eq!(suggestion(""), None);
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("", "new"), 3);
    }

    #[test]
    fn bold_only_for_a_terminal_without_no_color_or_a_dumb_term() {
        let os = |s: &str| Some(std::ffi::OsString::from(s));
        assert!(styled(true, None, os("xterm-256color")));
        assert!(styled(true, os(""), None));
        assert!(!styled(false, None, os("xterm-256color")));
        assert!(!styled(true, os("1"), os("xterm-256color")));
        assert!(!styled(true, None, os("dumb")));
    }

    #[test]
    fn bold_marks_headings_and_left_columns_only() {
        assert_eq!(bold("Usage:"), "\x1b[1mUsage:\x1b[22m");
        assert_eq!(
            bold("Exit codes: 0 success"),
            "\x1b[1mExit codes:\x1b[22m 0 success"
        );
        assert_eq!(
            bold("  --limit <n>        Print at most n hits"),
            "  \x1b[1m--limit <n>\x1b[22m        Print at most n hits"
        );
        assert_eq!(
            bold("  recall    Search"),
            "  \x1b[1mrecall\x1b[22m    Search"
        );
        for plain in [
            "",
            "bilbo recall: search the notes",
            "  bilbo recall <query>... [--limit <n>]",
            "                     tokens and heading path",
            "The first form runs on an enrolled device, in a terminal: it waits",
            "Results go to stdout. Diagnostics go to stderr, each line prefixed 'bilbo: '.",
        ] {
            assert_eq!(bold(plain), plain);
        }
    }
}
