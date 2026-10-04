mod agents;
mod check;
mod command;
mod config;
mod corpus;
mod digest;
mod embed;
mod hash;
mod index;
mod model;
mod new;
mod note;
mod rank;
mod recall;
mod setup;
mod source;
mod store;
mod timer;
mod vectors;
mod wizard;

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
usage: bilbo new <kind> <topic> [--title <text>]
       bilbo check
       bilbo recall <query>... [--kind <kind>]... [--limit <n>]
       bilbo index
       bilbo setup [--yes | --interactive] [--remove] [<setup option>]...
       bilbo digest
       bilbo --help
       bilbo --version
new creates <root>/notes/<kind>-<topic>.md and prints its path.
check prints every problem in the store and changes nothing.
recall prints the notes that best match the query, best first, 10 unless --limit says otherwise.
index embeds the passages the vector cache lacks and drops the ones no note holds any more.
digest reads a prompt hook's JSON on stdin and prints the notes that bear on the prompt; it always exits 0.
setup creates the store and the config and installs the agent plugin, the index timer and, when asked, the local embedder; in a terminal it asks first.
setup options: --embedder-url <url>, --embedder-model <name>, --embedder-token-env <var>, --embedder-token-file <path>, --embedder-query-prefix <text>, --embedder-local, --embedder-port <port>, --llama-server <path>, --no-plugin, --claude <path>, --codex <path>, --plugin-source <folder|owner/repo#ref>, --no-timer, --index-every <minutes>
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
    let env = store::Env::from_process();
    match args.first().map(String::as_str) {
        None => Err(Failure::Usage("missing verb".into())),
        Some("new") => {
            let path = new::run(&args[1..], &env)?;
            print_stdout(&path.display().to_string());
            Ok(ExitCode::SUCCESS)
        }
        Some("check") => {
            let problems = check::run(&args[1..], &env)?;
            problems.iter().for_each(|line| print_stdout(line));
            Ok(if problems.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Some("recall") => {
            let found = recall::run(&args[1..], &env)?;
            found.warnings.iter().for_each(|line| print_stderr(line));
            found.lines.iter().for_each(|line| print_stdout(line));
            Ok(ExitCode::SUCCESS)
        }
        Some("index") => {
            let line = index::run(&args[1..], &env)?;
            print_stdout(&line);
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
            let outcome = digest::run(&args[1..], &mut std::io::stdin().lock(), &env);
            outcome.lines.iter().for_each(|line| print_stdout(line));
            if let Some(line) = &outcome.diagnostic {
                print_stderr(line);
            }
            Ok(ExitCode::SUCCESS)
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

/// `-h` or `--help` before a bare `--`, except as the value of `new`'s `--title`.
fn wants_help(args: &[String]) -> bool {
    let is_new = args.first().is_some_and(|verb| verb == "new");
    let mut title_value = false;
    for arg in args.iter().take_while(|arg| *arg != "--") {
        if std::mem::take(&mut title_value) {
            continue;
        }
        title_value = is_new && arg == "--title";
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
