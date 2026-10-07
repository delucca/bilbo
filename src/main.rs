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

use host::terminal;

/// Why a verb did not succeed; `main` maps it to stderr and an exit code.
pub enum Failure {
    /// Exit 2: the message, then the synopsis of the verb that was run, or of `bilbo` itself.
    Usage(String),
    /// Exit 2: the message only, for store-root resolution errors.
    Config(String),
    /// Exit 1: the message only.
    Refused(String),
    /// Exit 1: a search that found nothing. Off a terminal stderr gets each warning, then `message`; on one,
    /// `○  <message> '<query>'`, the hint, then the warnings after `▲`.
    Unmatched {
        warnings: Vec<String>,
        message: String,
        query: String,
        hint: String,
    },
}

/// The two streams' decisions and the time every view reads.
struct Io {
    out: terminal::Term,
    err: terminal::Term,
    now: jiff::Timestamp,
}

/// What a stderr line says about itself: its mark on a terminal.
#[derive(Clone, Copy)]
enum Level {
    Error,
    Warning,
    Info,
    Nothing,
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
Results go to stdout; diagnostics to stderr, prefixed 'bilbo: ' off a terminal.

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
    let env = shared::store::Env::from_process();
    let io = Io {
        out: terminal::open(terminal::Stream::Stdout, &env),
        err: terminal::open(terminal::Stream::Stderr, &env),
        now: jiff::Timestamp::now(),
    };
    terminal::init(&io.err);
    let args = match collect_args() {
        Ok(args) => args,
        Err(failure) => return report(&io, failure, &[]),
    };
    match run(&args, &env, &io) {
        Ok(code) => code,
        Err(failure) => report(&io, failure, &args),
    }
}

fn run(args: &[String], env: &shared::store::Env, io: &Io) -> Result<ExitCode, Failure> {
    if args.first().is_some_and(|arg| arg == "help") {
        print_help(&io.out, help_topic(&args[1..])?);
        return Ok(ExitCode::SUCCESS);
    }
    if wants_help(args) {
        let text = match args.first() {
            Some(verb) if !verb.starts_with('-') => page(verb).ok_or_else(|| unknown_verb(verb))?,
            _ => OVERVIEW,
        };
        print_help(&io.out, text);
        return Ok(ExitCode::SUCCESS);
    }
    if args == ["--version"] {
        print_stdout(concat!("bilbo ", env!("CARGO_PKG_VERSION")));
        return Ok(ExitCode::SUCCESS);
    }
    let info = |line: &str| print_stderr(&io.err, Level::Info, line);
    match args.first().map(String::as_str) {
        None => Err(Failure::Usage("missing verb".into())),
        Some("new") => {
            let output = note::new::run(&args[1..], env)?;
            let lines = [output.path.display().to_string()];
            emit(io, output.warning.as_slice(), &lines, || {
                output.view(&io.out)
            });
            Ok(ExitCode::SUCCESS)
        }
        Some("check") => {
            let output = check::run(&args[1..], env)?;
            emit(io, &[], &output.lines, || output.view(&io.out));
            Ok(exit(output.failed))
        }
        Some("recall") => {
            let found = search::recall::run(&args[1..], env)?;
            emit(io, &found.warnings, &found.lines, || {
                found.view(&io.out, io.now)
            });
            Ok(ExitCode::SUCCESS)
        }
        Some("index") => {
            let output = search::index::run(&args[1..], env)?;
            emit(
                io,
                &output.warnings,
                std::slice::from_ref(&output.line),
                || output.view(&io.out),
            );
            Ok(ExitCode::SUCCESS)
        }
        Some("setup") => {
            let outcome = setup::run(&args[1..], env, &mut |line: &str| info(line))?;
            emit(io, &[], &outcome.lines, || outcome.view(&io.out));
            Ok(exit(outcome.failed))
        }
        Some("digest") => {
            let outcome = search::digest::run(&args[1..], &mut std::io::stdin().lock(), env);
            outcome.lines.iter().for_each(|line| print_stdout(line));
            if let Some(line) = &outcome.diagnostic {
                print_stderr(&io.err, Level::Warning, line);
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("library") => {
            let output = library::cli::run(&args[1..], env)?;
            emit(io, &output.warnings, &output.lines, || output.view(&io.out));
            Ok(ExitCode::SUCCESS)
        }
        Some("cite") => {
            let output = citation::cite::run(&args[1..], &mut std::io::stdin().lock(), env)?;
            emit(io, &output.warnings, &output.lines, || output.lines.clone());
            Ok(exit(output.failed))
        }
        Some("watch") => {
            note::watch::run(&args[1..], env, &mut |line: &str| info(line))?;
            Ok(ExitCode::SUCCESS)
        }
        Some("history") => {
            let output = note::history::run(&args[1..], env)?;
            let version = matches!(output.shown, note::history::Shown::Version);
            if io.out.human && version {
                write_stdout(&output.bytes);
            }
            emit(io, &output.warnings, &output.lines, || {
                output.view(&io.out, io.now)
            });
            if !io.out.human {
                write_stdout(&output.bytes);
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("restore") => {
            let output = note::restore::run(&args[1..], env)?;
            emit(io, &output.warnings, &output.lines, || output.view(&io.out));
            Ok(ExitCode::SUCCESS)
        }
        Some("scope") => {
            let output = note::scope::run(&args[1..], env)?;
            emit(io, &output.warnings, &output.lines, || output.view(&io.out));
            Ok(exit(output.failed))
        }
        Some("device") => {
            let terminal = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
            let output =
                identity::device::run(&args[1..], env, terminal, &mut host::prompt::Terminal)?;
            emit(io, &output.warnings, &output.lines, || output.view(&io.out));
            Ok(exit(output.failed))
        }
        Some("sync") => {
            let output = sync::cli::run(&args[1..], env)?;
            emit(io, &output.warnings, &output.lines, || {
                output.view(&io.out, io.now)
            });
            Ok(exit(output.failed))
        }
        Some("pair") => {
            let terminal = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
            identity::pair::run(
                &args[1..],
                env,
                terminal,
                &mut std::io::stdin().lock(),
                &identity::pair::Limits::default(),
                &mut |line: &str| print_stdout(line),
                &mut |line: &str| info(line),
            )?;
            Ok(ExitCode::SUCCESS)
        }
        Some("relay") => {
            // A panicking request gets a 500 and the relay keeps serving, so the panic is one more line.
            let err = io.err.clone();
            std::panic::set_hook(Box::new(move |panic| {
                print_stderr(&err, Level::Error, &format!("internal error: {panic}"));
            }));
            relay::run(&args[1..], &|line: &str| info(line))?;
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

fn exit(failed: bool) -> ExitCode {
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
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

/// Prints a verb's result, and its warnings after it on a terminal, before it otherwise. `lines` is the plain
/// view; `view` makes the human one.
fn emit(io: &Io, warnings: &[String], lines: &[String], view: impl FnOnce() -> Vec<String>) {
    let shown = if io.out.human { view() } else { lines.to_vec() };
    for (stream, line) in order(io.out.human, warnings, &shown) {
        match stream {
            terminal::Stream::Stdout => print_stdout(line),
            terminal::Stream::Stderr => print_stderr(&io.err, Level::Warning, line),
        }
    }
}

/// What `emit` prints, in order: the result then its warnings for a person's terminal, else the warnings first.
fn order<'a>(
    human: bool,
    warnings: &'a [String],
    lines: &'a [String],
) -> Vec<(terminal::Stream, &'a str)> {
    let warnings = warnings
        .iter()
        .map(|w| (terminal::Stream::Stderr, w.as_str()));
    let lines = lines.iter().map(|l| (terminal::Stream::Stdout, l.as_str()));
    if human {
        lines.chain(warnings).collect()
    } else {
        warnings.chain(lines).collect()
    }
}

/// The one writer of stderr.
fn print_stderr(err: &terminal::Term, level: Level, message: &str) {
    let mut out = std::io::stderr().lock();
    for line in stderr_lines(err, level, message) {
        let _ = writeln!(out, "{line}");
    }
}

/// `message` as stderr shows it: off a terminal, or under an agent, every physical line gets the prefix, even
/// inside an echoed argument; on a person's terminal the first line follows the level's mark and the others are
/// indented under it.
fn stderr_lines(err: &terminal::Term, level: Level, message: &str) -> Vec<String> {
    if !err.human {
        return message
            .split('\n')
            .map(|line| format!("bilbo: {line}"))
            .collect();
    }
    let mark = match level {
        Level::Error => terminal::Mark::Error,
        Level::Warning => terminal::Mark::Warning,
        Level::Info => terminal::Mark::Info,
        Level::Nothing => terminal::Mark::Skipped,
    };
    terminal::marked(err, mark, message)
}

/// Prints a help text to stdout, in bold on a person's terminal that may paint.
fn print_help(out: &terminal::Term, text: &str) {
    let styled = out.human && out.paint;
    for line in text.lines() {
        if styled {
            print_stdout(&bold(line));
        } else {
            print_stdout(line);
        }
    }
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

/// A usage error as stderr shows it: the reason, then the usage lines, dim and indented under the mark on a
/// terminal.
fn usage_report(err: &terminal::Term, message: &str, args: &[String]) -> Vec<String> {
    let usage = usage_lines(args);
    if !err.human {
        return std::iter::once(message.to_string())
            .chain(usage)
            .flat_map(|line| stderr_lines(err, Level::Error, &line))
            .collect();
    }
    let mut lines = stderr_lines(err, Level::Error, message);
    for line in &usage {
        let line = match line
            .strip_prefix("see '")
            .and_then(|rest| rest.strip_suffix('\''))
        {
            Some(command) => format!(
                "{} {}",
                terminal::paint(err, terminal::Tone::Dim, "see"),
                terminal::paint(err, terminal::Tone::Cyan, command)
            ),
            None => terminal::paint(err, terminal::Tone::Dim, line),
        };
        lines.push(format!("   {line}"));
    }
    lines
}

/// A search that found nothing as stderr shows it; see `Failure::Unmatched`.
fn unmatched_report(
    err: &terminal::Term,
    warnings: &[String],
    message: &str,
    query: &str,
    hint: &str,
) -> Vec<String> {
    if !err.human {
        return warnings
            .iter()
            .flat_map(|warning| stderr_lines(err, Level::Warning, warning))
            .chain(stderr_lines(err, Level::Error, message))
            .collect();
    }
    let hint = hint.replace(
        "--library",
        &terminal::paint(err, terminal::Tone::Cyan, "--library"),
    );
    let mut lines = stderr_lines(err, Level::Nothing, &format!("{message} '{query}'\n{hint}"));
    for warning in warnings {
        lines.extend(stderr_lines(err, Level::Warning, warning));
    }
    lines
}

fn report(io: &Io, failure: Failure, args: &[String]) -> ExitCode {
    let lines = match &failure {
        Failure::Usage(message) => usage_report(&io.err, message, args),
        Failure::Config(message) | Failure::Refused(message) => {
            stderr_lines(&io.err, Level::Error, message)
        }
        Failure::Unmatched {
            warnings,
            message,
            query,
            hint,
        } => unmatched_report(&io.err, warnings, message, query, hint),
    };
    let mut err = std::io::stderr().lock();
    for line in lines {
        let _ = writeln!(err, "{line}");
    }
    match failure {
        Failure::Usage(_) | Failure::Config(_) => ExitCode::from(2),
        Failure::Refused(_) | Failure::Unmatched { .. } => ExitCode::FAILURE,
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

    fn plain_term() -> terminal::Term {
        terminal::Term {
            human: false,
            ..terminal::fixed(100, false, true)
        }
    }

    #[test]
    fn plain_stderr_prefixes_every_line() {
        assert_eq!(
            stderr_lines(&plain_term(), Level::Warning, "a\nb"),
            ["bilbo: a", "bilbo: b"]
        );
    }

    #[test]
    fn human_stderr_marks_the_level() {
        let t = terminal::fixed(100, false, true);
        for (level, mark) in [
            (Level::Error, "■"),
            (Level::Warning, "▲"),
            (Level::Info, "●"),
            (Level::Nothing, "○"),
        ] {
            assert_eq!(
                stderr_lines(&t, level, "no corpus 'x' in /r/library"),
                [format!("{mark}  no corpus 'x' in /r/library")]
            );
        }
        let painted = terminal::fixed(100, true, true);
        assert_eq!(
            stderr_lines(&painted, Level::Error, "no corpus 'x' in /r/library"),
            [terminal::styled("{r}■{/r}  no corpus 'x' in /r/library")]
        );
        assert_eq!(stderr_lines(&t, Level::Info, "a\nb"), ["●  a", "   b"]);
    }

    #[test]
    fn a_usage_error_on_a_terminal() {
        let message = "unknown verb 'recal'; did you mean 'recall'?";
        let plain = terminal::fixed(100, false, true);
        let verbs = format!("verbs: {}", PAGES.map(|(verb, _)| verb).join(", "));
        assert_eq!(
            usage_report(&plain, message, &args("recal x")),
            [
                format!("■  {message}"),
                "   usage: bilbo <verb> [<args>]...".to_string(),
                format!("   {verbs}"),
                "   see bilbo --help".to_string(),
            ]
        );
        let painted = terminal::fixed(100, true, true);
        let lines = usage_report(&painted, message, &args("recal x"));
        assert_eq!(
            lines[0],
            terminal::styled(&format!("{{r}}■{{/r}}  {message}"))
        );
        assert_eq!(
            lines[1],
            terminal::styled("   {d}usage: bilbo <verb> [<args>]...{/d}")
        );
        assert_eq!(
            lines[3],
            terminal::styled("   {d}see{/d} {c}bilbo --help{/c}")
        );
        assert_eq!(
            usage_report(&plain_term(), message, &args("recal x"))[3],
            "bilbo: see 'bilbo --help'"
        );
    }

    #[test]
    fn unmatched_is_plain_off_a_terminal() {
        let warnings = ["1 passage not indexed; run bilbo index".to_string()];
        let hint = "to search the library, add --library";
        assert_eq!(
            unmatched_report(&plain_term(), &warnings, "no notes match", "wumpus", hint),
            [
                "bilbo: 1 passage not indexed; run bilbo index",
                "bilbo: no notes match"
            ]
        );
        assert_eq!(
            unmatched_report(
                &terminal::fixed(100, false, true),
                &warnings,
                "no notes match",
                "wumpus",
                hint
            ),
            [
                "○  no notes match 'wumpus'",
                "   to search the library, add --library",
                "▲  1 passage not indexed; run bilbo index"
            ]
        );
        assert_eq!(
            unmatched_report(
                &terminal::fixed(100, true, true),
                &[],
                "no notes match",
                "wumpus",
                hint
            )[1],
            terminal::styled("   to search the library, add {c}--library{/c}")
        );
    }

    #[test]
    fn warnings_follow_a_human_stdout() {
        use terminal::Stream::{Stderr, Stdout};
        let (warnings, lines) = (["w".to_string()], ["l".to_string()]);
        assert_eq!(
            order(true, &warnings, &lines),
            [(Stdout, "l"), (Stderr, "w")]
        );
        assert_eq!(
            order(false, &warnings, &lines),
            [(Stderr, "w"), (Stdout, "l")]
        );
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
            "Results go to stdout; diagnostics to stderr, prefixed 'bilbo: ' off a terminal.",
        ] {
            assert_eq!(bold(plain), plain);
        }
    }
}
