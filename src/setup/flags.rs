//! Setup's command line: the flag tables, `parse` and `settle`.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::Failure;
use crate::host::{agents, command, model};
use crate::shared::config::{self, Embedder, Token};
use crate::shared::store;

const BOOL_FLAGS: [&str; 6] = [
    "--yes",
    "--interactive",
    "--remove",
    "--no-plugin",
    "--no-timer",
    "--embedder-local",
];

const VALUE_FLAGS: [&str; 11] = [
    "--embedder-url",
    "--embedder-model",
    "--embedder-token-env",
    "--embedder-token-file",
    "--embedder-query-prefix",
    "--claude",
    "--codex",
    "--plugin-source",
    "--index-every",
    "--embedder-port",
    "--llama-server",
];

const EMBEDDER_FLAGS: [&str; 5] = [
    "--embedder-url",
    "--embedder-model",
    "--embedder-token-env",
    "--embedder-token-file",
    "--embedder-query-prefix",
];

const ANSWER_FLAGS: [&str; 11] = [
    "--embedder-url",
    "--embedder-model",
    "--embedder-token-env",
    "--embedder-token-file",
    "--embedder-query-prefix",
    "--no-plugin",
    "--no-timer",
    "--index-every",
    "--embedder-local",
    "--embedder-port",
    "--llama-server",
];

const REMOVE_FLAGS: [&str; 4] = ["--yes", "--interactive", "--claude", "--codex"];

fn usage<T>(message: impl Into<String>) -> Result<T, Failure> {
    Err(Failure::Usage(message.into()))
}

#[derive(Default)]
struct Args {
    /// Flag names in the order given.
    given: Vec<&'static str>,
    yes: bool,
    interactive: bool,
    remove: bool,
    no_plugin: bool,
    no_timer: bool,
    embedder_local: bool,
    embedder_url: Option<String>,
    embedder_model: Option<String>,
    token_env: Option<String>,
    token_file: Option<String>,
    query_prefix: Option<String>,
    claude: Option<String>,
    codex: Option<String>,
    plugin_source: Option<String>,
    index_every: Option<String>,
    embedder_port: Option<String>,
    llama_server: Option<String>,
}

impl Args {
    fn has(&self, flag: &str) -> bool {
        self.given.contains(&flag)
    }

    /// The first of `names` given on the command line.
    fn first(&self, names: &[&str]) -> Option<&'static str> {
        self.given.iter().copied().find(|g| names.contains(g))
    }
}

/// Walks every argument: unknown option, positional, missing value, repeat.
fn parse(args: &[String]) -> Result<Args, Failure> {
    let mut out = Args::default();
    let mut at = 0;
    while at < args.len() {
        let arg = &args[at];
        at += 1;
        if !arg.starts_with('-') || arg == "-" {
            return usage(format!(
                "setup takes only options; argument {at} is not one"
            ));
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) => (name, Some(value.to_string())),
            None => (arg.as_str(), None),
        };
        if let Some(flag) = BOOL_FLAGS.iter().find(|f| **f == name) {
            if inline.is_some() {
                return usage(format!("{flag} takes no value"));
            }
            if out.has(flag) {
                return usage(format!("{flag} given more than once"));
            }
            out.given.push(flag);
            match *flag {
                "--yes" => out.yes = true,
                "--interactive" => out.interactive = true,
                "--remove" => out.remove = true,
                "--no-plugin" => out.no_plugin = true,
                "--embedder-local" => out.embedder_local = true,
                _ => out.no_timer = true,
            }
        } else if let Some(flag) = VALUE_FLAGS.iter().find(|f| **f == name) {
            if out.has(flag) {
                return usage(format!("{flag} given more than once"));
            }
            let value = match inline {
                Some(value) => Some(value),
                None => {
                    let next = args.get(at).cloned();
                    if let Some(option) = next.as_deref().filter(|n| n.starts_with("--")) {
                        let name = option.split('=').next().unwrap_or(option);
                        return usage(format!("{flag} needs a value, got the option {name}"));
                    }
                    if next.is_some() {
                        at += 1;
                    }
                    next
                }
            };
            let Some(value) = value.filter(|v| !v.is_empty() || *flag == "--embedder-query-prefix")
            else {
                return usage(format!("{flag} needs a value"));
            };
            out.given.push(flag);
            let slot = match *flag {
                "--embedder-url" => &mut out.embedder_url,
                "--embedder-model" => &mut out.embedder_model,
                "--embedder-token-env" => &mut out.token_env,
                "--embedder-token-file" => &mut out.token_file,
                "--embedder-query-prefix" => &mut out.query_prefix,
                "--claude" => &mut out.claude,
                "--codex" => &mut out.codex,
                "--plugin-source" => &mut out.plugin_source,
                "--embedder-port" => &mut out.embedder_port,
                "--llama-server" => &mut out.llama_server,
                _ => &mut out.index_every,
            };
            *slot = Some(value);
        } else {
            return usage(format!("unknown option '{name}'"));
        }
    }
    Ok(out)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Batch,
    Wizard,
    Remove,
}

/// The embedder the flags describe.
pub struct EmbedderFlags {
    pub url: String,
    pub model: String,
    token: Option<Token>,
    query_prefix: Option<String>,
}

impl EmbedderFlags {
    pub fn embedder(&self) -> Embedder {
        Embedder {
            url: self.url.clone(),
            model: self.model.clone(),
            token: self.token.clone(),
            query_prefix: self
                .query_prefix
                .clone()
                .unwrap_or_else(|| config::default_query_prefix(&self.model).to_string()),
            min_similarity: config::DEFAULT_MIN_SIMILARITY,
        }
    }
}

/// What `--embedder-local` asks for.
pub struct LocalFlags {
    pub port: u16,
    pub llama_server: Option<PathBuf>,
}

/// Every flag, checked and typed.
pub struct Flags {
    pub mode: Mode,
    /// `--remove` asks first: in a terminal without `--yes`, or with `--interactive`.
    pub confirm: bool,
    pub no_plugin: bool,
    pub no_timer: bool,
    pub minutes: Option<u32>,
    pub embedder: Option<EmbedderFlags>,
    pub local: Option<LocalFlags>,
    /// The first embedder flag on the command line, for messages.
    pub embedder_flag: Option<&'static str>,
    pub claude: Option<PathBuf>,
    pub codex: Option<PathBuf>,
    pub source: Option<agents::Source>,
}

pub fn settle(args: &[String], env: &store::Env) -> Result<Flags, Failure> {
    let args = parse(args)?;
    if args.yes && args.interactive {
        return usage("--yes and --interactive cannot be used together");
    }
    if args.interactive
        && let Some(flag) = args.first(&ANSWER_FLAGS)
    {
        return usage(format!(
            "--interactive cannot be used with {flag}; it answers a wizard question"
        ));
    }
    if args.remove
        && let Some(flag) = args
            .given
            .iter()
            .find(|f| **f != "--remove" && !REMOVE_FLAGS.contains(f))
    {
        return usage(format!("--remove cannot be used with {flag}"));
    }
    if args.embedder_local
        && let Some(flag) = args.first(&EMBEDDER_FLAGS)
    {
        return usage(format!(
            "--embedder-local and {flag} cannot be used together"
        ));
    }
    if !args.embedder_local
        && let Some(flag) = args.first(&["--embedder-port", "--llama-server"])
    {
        return usage(format!("{flag} needs --embedder-local"));
    }
    let port = match &args.embedder_port {
        Some(value) => parse_port(value)?,
        None => model::PORT,
    };
    if args.embedder_url.is_none()
        && let Some(flag) = args.first(&EMBEDDER_FLAGS)
    {
        return usage(format!("{flag} needs --embedder-url"));
    }
    if args.embedder_url.is_some() && args.embedder_model.is_none() {
        return usage("--embedder-url needs --embedder-model");
    }
    if args.token_env.is_some() && args.token_file.is_some() {
        return usage("--embedder-token-env and --embedder-token-file cannot be used together");
    }
    if args.no_timer && args.index_every.is_some() {
        return usage("--no-timer and --index-every cannot be used together");
    }
    if args.no_plugin
        && let Some(flag) = args.first(&["--claude", "--codex", "--plugin-source"])
    {
        return usage(format!("--no-plugin and {flag} cannot be used together"));
    }

    let minutes = match &args.index_every {
        Some(value) => Some(parse_minutes(value)?),
        None => None,
    };
    let home = store::absolute(&env.home);
    let mut embedder = match (&args.embedder_url, &args.embedder_model) {
        (Some(url), Some(model)) => Some(embedder_flags(&args, url, model, home.as_deref())?),
        _ => None,
    };
    let cwd = std::env::current_dir()
        .map_err(|e| Failure::Refused(format!("cannot read the working directory: {e}")))?;
    let local = if args.embedder_local {
        embedder = Some(EmbedderFlags {
            url: format!("http://127.0.0.1:{port}"),
            model: model::NAME.to_string(),
            token: None,
            query_prefix: None,
        });
        Some(LocalFlags {
            port,
            llama_server: tool_flag("--llama-server", &args.llama_server, &cwd)?,
        })
    } else {
        None
    };
    let claude = tool_flag("--claude", &args.claude, &cwd)?;
    let codex = tool_flag("--codex", &args.codex, &cwd)?;
    let source = match &args.plugin_source {
        Some(text) => Some(
            agents::parse_source(text, &cwd, home.as_deref())
                .map_err(|e| Failure::Usage(format!("--plugin-source {e}")))?,
        ),
        None => None,
    };

    let terminals = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if args.interactive && !terminals {
        return usage(
            "the wizard needs a terminal on stdin and stderr; --interactive cannot run here",
        );
    }
    let answers = args.first(&ANSWER_FLAGS).is_some();
    let mode = if args.remove {
        Mode::Remove
    } else if args.interactive || (!args.yes && !answers && terminals) {
        Mode::Wizard
    } else {
        Mode::Batch
    };
    Ok(Flags {
        mode,
        confirm: args.remove && (args.interactive || (!args.yes && terminals)),
        no_plugin: args.no_plugin,
        no_timer: args.no_timer,
        minutes,
        embedder,
        local,
        embedder_flag: if args.embedder_local {
            Some("--embedder-local")
        } else {
            args.first(&EMBEDDER_FLAGS)
        },
        claude,
        codex,
        source,
    })
}

fn parse_minutes(value: &str) -> Result<u32, Failure> {
    let digits = !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
    match value.parse::<u32>() {
        Ok(n) if digits && (1..=1440).contains(&n) => Ok(n),
        _ => usage(format!(
            "--index-every takes 1 to 1440 minutes, got '{value}'"
        )),
    }
}

fn parse_port(value: &str) -> Result<u16, Failure> {
    let digits = !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
    match value.parse::<u16>() {
        Ok(n) if digits && n >= 1024 => Ok(n),
        _ => usage(format!(
            "--embedder-port takes 1024 to 65535, got '{value}'"
        )),
    }
}

fn embedder_flags(
    args: &Args,
    url: &str,
    model: &str,
    home: Option<&Path>,
) -> Result<EmbedderFlags, Failure> {
    if let Some(problem) = config::url_problem(url) {
        return if problem.contains("user name") {
            usage("--embedder-url must not hold a user name or password")
        } else {
            usage(format!(
                "--embedder-url must be an http:// or https:// URL with a host, got '{url}'"
            ))
        };
    }
    let token = match (&args.token_env, &args.token_file) {
        (Some(name), _) => {
            if !config::is_variable_name(name) {
                return usage(
                    "--embedder-token-env must be a variable name (letters, digits and _, not starting with a digit)",
                );
            }
            Some(Token::Var(name.clone()))
        }
        (_, Some(file)) => Some(Token::File(token_file(file, home)?)),
        _ => None,
    };
    Ok(EmbedderFlags {
        url: url.to_string(),
        model: model.to_string(),
        token,
        query_prefix: args.query_prefix.clone(),
    })
}

fn token_file(value: &str, home: Option<&Path>) -> Result<PathBuf, Failure> {
    if Path::new(value).is_absolute() {
        return Ok(PathBuf::from(value));
    }
    let Some(rest) = value.strip_prefix("~/") else {
        return usage("--embedder-token-file must be an absolute path or start with ~/");
    };
    match home {
        Some(home) => Ok(home.join(rest)),
        None => usage("--embedder-token-file starts with ~/ but HOME is not an absolute path"),
    }
}

/// A `--claude` or `--codex` path; a relative one joins the working directory.
fn tool_flag(flag: &str, value: &Option<String>, cwd: &Path) -> Result<Option<PathBuf>, Failure> {
    let Some(value) = value else {
        return Ok(None);
    };
    let path = cwd.join(value);
    if command::is_executable(&path) {
        Ok(Some(path))
    } else {
        usage(format!("{flag} {value} is not an executable file"))
    }
}

#[cfg(test)]
mod tests {
    use super::super::fakes::*;
    use super::*;

    #[test]
    fn parse_takes_both_value_forms_and_an_empty_prefix() {
        let args = parse(&strings(&[
            "--embedder-url=http://x:1",
            "--embedder-model",
            "m",
            "--embedder-query-prefix=",
            "--index-every",
            "30",
        ]))
        .ok()
        .unwrap();
        assert_eq!(args.embedder_url.as_deref(), Some("http://x:1"));
        assert_eq!(args.embedder_model.as_deref(), Some("m"));
        assert_eq!(args.query_prefix.as_deref(), Some(""));
        assert_eq!(args.index_every.as_deref(), Some("30"));
        assert_eq!(args.given.len(), 4);
    }

    #[test]
    fn parse_refuses_what_it_cannot_read() {
        let message = |args: &[&str]| match parse(&strings(args)) {
            Err(Failure::Usage(m)) => m,
            _ => panic!("expected a usage error"),
        };
        assert_eq!(message(&["--token=sk-1"]), "unknown option '--token'");
        assert_eq!(
            message(&["--yes", "x"]),
            "setup takes only options; argument 2 is not one"
        );
        assert_eq!(message(&["--claude"]), "--claude needs a value");
        assert_eq!(message(&["--claude="]), "--claude needs a value");
        assert_eq!(message(&["--yes", "--yes"]), "--yes given more than once");
        assert_eq!(message(&["--yes=1"]), "--yes takes no value");
    }

    #[test]
    fn minutes_range() {
        assert!(parse_minutes("1").is_ok());
        assert!(parse_minutes("1440").is_ok());
        for bad in ["0", "1441", "abc", "", "+5", "-1"] {
            assert!(parse_minutes(bad).is_err(), "{bad}");
        }
    }
}
