#![warn(clippy::self_named_module_files)]

mod check;
mod citation;
mod host;
mod library;
mod note;
mod search;
mod setup;
mod shared;

use std::io::Write;
use std::process::ExitCode;

/// Why a verb did not succeed; `main` maps it to stderr and an exit code.
pub enum Failure {
    /// Exit 2: the message, then the usage text.
    Usage(String),
    /// Exit 2: the message only, for store-root resolution errors.
    Config(String),
    /// Exit 1: the message only.
    Refused(String),
}

const USAGE: &str = "\
usage: bilbo new <kind> <topic> [--title <text>] [--scope <name>]
       bilbo check
       bilbo recall <query>... [--kind <kind>]... [--limit <n>]
       bilbo recall <query>... --library [--corpus <corpus>]... [--limit <n>]
       bilbo index
       bilbo setup [--yes | --interactive] [--remove] [<setup option>]...
       bilbo digest
       bilbo library [<corpus>]
       bilbo library show <corpus>/<name>|<id>[#<anchor>] [--depth <n>]
       bilbo library stage <url> | <file> --origin \"<url|doc>: <value>\" [--fetched <YYYY-MM-DD>] [--html]
       bilbo library land <stage> <corpus>/<name> --keep <a>-<b>[,<c>-<d>]... [--title <text>] [--replace [--force]]
       bilbo library plan <ref>... [--budget-tokens <n>] [--slice-bytes <n>] [--slice-lines <n>]
       bilbo library read <plan> <slice>... [--part <k>/<n>]
       bilbo cite [--plan <plan>]... [<file> | -]
       bilbo watch
       bilbo history <note> [<version> | --diff <a> [<b>]]
       bilbo restore <note> <version>
       bilbo scope
       bilbo scope set [--force] <name> <file>...
       bilbo --help
       bilbo --version
new creates <root>/notes/<kind>-<topic>.md and prints its path.
check prints every problem in the store and changes nothing.
recall prints the notes that best match the query, best first, 10 unless --limit says otherwise.
recall --library searches the sources and guides of the library by keyword instead of the notes; --corpus narrows it.
index embeds the passages the vector cache lacks and drops the ones no note holds any more.
digest reads a prompt hook's JSON on stdin and prints the notes that bear on the prompt; it always exits 0.
library lists the corpora, prints a corpus's guide with the facts of each source, or a source's outline; stage and land add a source; plan cuts picks into slices and partitions; read prints slices and logs them.
cite checks every bilbo: citation in a draft, and with --plan prints the coverage of the plans' reads.
watch records a version of each note when it changes, until it is stopped.
history lists the versions of a note, newest first, prints one, or shows what changed between two versions, or between one and the note's file now.
restore writes a past version of a note back as its newest version, keeping what the note held before.
scope lists the scopes this device declares with their note counts; scope set gives notes a scope.
setup creates the store and the config and installs the agent plugin, the index timer, the note watcher and, when asked, the local embedder; in a terminal it asks first.
setup options: --embedder-url <url>, --embedder-model <name>, --embedder-token-env <var>, --embedder-token-file <path>, --embedder-query-prefix <text>, --embedder-local, --embedder-port <port>, --llama-server <path>, --no-plugin, --claude <path>, --codex <path>, --plugin-source <folder|owner/repo#ref>, --no-timer, --index-every <minutes>, --no-watch
kinds: plan, spec, design, decision, gotcha, research, review, report, reference
root: $BILBO_HOME, else $XDG_DATA_HOME/bilbo, else $HOME/.local/share/bilbo
config: $BILBO_CONFIG, else $XDG_CONFIG_HOME/bilbo/config, else $HOME/.config/bilbo/config
cache: $XDG_CACHE_HOME/bilbo, else $HOME/.cache/bilbo
state: $XDG_STATE_HOME/bilbo, else $HOME/.local/state/bilbo
";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(failure) => report(failure),
    }
}

fn run() -> Result<ExitCode, Failure> {
    let args = collect_args()?;
    if wants_help(&args) {
        print_stdout(USAGE.trim_end());
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
        Some(arg) if arg.starts_with('-') => Err(Failure::Usage(format!("unknown option '{arg}'"))),
        Some(arg) => Err(Failure::Usage(format!("unknown verb '{arg}'"))),
    }
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

fn report(failure: Failure) -> ExitCode {
    match failure {
        Failure::Usage(message) => {
            print_stderr(&message);
            USAGE.lines().for_each(print_stderr);
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
