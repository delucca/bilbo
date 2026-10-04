//! The setup verb.
use std::io::{IsTerminal, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::Zeroizing;

use crate::config::{self, Embedder, Token};
use crate::{Failure, agents, command, embed, model, store, timer, wizard};
use wizard::Prompter;

const CHECK: Duration = Duration::from_secs(15);
const READY: Duration = Duration::from_secs(120);
const OLLAMA: &str = "http://localhost:11434";
const DEFAULT_MINUTES: u32 = 15;

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

/// The report lines for stdout and whether any step failed.
pub struct Outcome {
    pub lines: Vec<String>,
    pub failed: bool,
}

pub fn run(
    args: &[String],
    env: &store::Env,
    say: &mut dyn FnMut(&str),
) -> Result<Outcome, Failure> {
    let flags = settle(args, env)?;
    let path = std::env::var_os("PATH");
    match flags.mode {
        Mode::Wizard => return run_wizard(&flags, env, path),
        Mode::Remove => return remove(&flags, env, path, &mut wizard::Terminal),
        Mode::Batch => {}
    }
    let facts = gather(&flags, env, path)?;
    let mut plan = answer_batch(&flags, facts, &mut Net)?;
    if let (Some(local), Some(embedder)) = (&plan.local, &plan.embedder)
        && local.prepares()
    {
        if local.download {
            say(&format!(
                "downloading {} ({} MB) to {}",
                model::FILE,
                model::PINNED.size / 1_000_000,
                local.model.display()
            ));
        }
        match prepare(local, embedder, &mut Net) {
            Ok((lines, dims)) => {
                plan.local_lines = Some(lines);
                plan.check = EmbedderPlan::Checked(dims);
            }
            Err((lines, message)) => {
                say(&format!("{message}; setup changed nothing else"));
                let mut report = Report::default();
                report.line("model", lines.model.status, Some(lines.model.detail));
                report.line("server", lines.server.status, Some(lines.server.detail));
                return Ok(Outcome {
                    lines: report.lines,
                    failed: true,
                });
            }
        }
    }
    Ok(apply(&plan))
}

/// The wizard on the terminal, with the real Ollama probe, embedder check and first index.
fn run_wizard(
    flags: &Flags,
    env: &store::Env,
    path: Option<std::ffi::OsString>,
) -> Result<Outcome, Failure> {
    let facts = gather(flags, env, path)?;
    wizard_with(
        facts,
        &mut wizard::Terminal,
        &mut Net,
        || embed::ollama_models(OLLAMA, Duration::from_secs(1)),
        |embedder, pasted| match pasted {
            Some(key) => {
                let client = embed::Client::with_token(embedder, Some(key.to_string()), CHECK);
                vector_length(&client)
            }
            None => check_embedder(embedder),
        },
        run_index,
    )
}

/// The wizard: ask, plan, confirm, apply, then offer the first index. `outside` reaches the model
/// host, the local server and the port; `probe` finds the Ollama models, `check` tries an embedder
/// and `index` runs the first index; all four are the caller's so a test can run the wizard
/// without a terminal, a network or the real tools.
fn wizard_with<P: Prompter>(
    facts: Facts,
    p: &mut P,
    outside: &mut impl Outside,
    probe: impl FnOnce() -> Option<Vec<String>>,
    check: impl FnMut(&Embedder, Option<&str>) -> Result<usize, String>,
    index: impl FnOnce(&Path) -> Result<String, String>,
) -> Result<Outcome, Failure> {
    let managed = match &facts.config {
        ConfigState::Managed { target } => Some(target.clone()),
        _ => None,
    };
    let ollama = match managed {
        Some(_) => None,
        None => probe(),
    };
    let local = wizard::Local {
        url: format!("http://127.0.0.1:{}", model::PORT),
        model_name: model::NAME.to_string(),
        download_mb: model::PINNED.size / 1_000_000,
        llama_server: facts.llama_server.clone(),
        unavailable: match managed {
            Some(_) => None,
            None => local_unavailable(&facts, model::PORT, outside),
        },
    };
    let seen = wizard::Facts {
        root: facts.root.clone(),
        config_path: facts.config_path.clone(),
        existing: facts.existing.clone(),
        managed,
        token_path: facts.token_path.clone(),
        token_exists: facts.token_path.exists(),
        ollama,
        claude: facts.claude.clone(),
        codex: facts.codex.clone(),
        timer: facts.timer.platform.is_some(),
        timer_minutes: facts
            .timer
            .platform
            .zip(timer_place(&facts.timer))
            .and_then(|(platform, place)| timer::minutes(platform, &place)),
        local,
    };
    let answers = wizard::ask(p, &seen, check, |name| {
        std::env::var_os(name).is_some_and(|value| !value.is_empty())
    })
    .map_err(|e| stopped(p, e))?;
    let exe = facts.exe.clone();
    let plugins = (answers.claude, answers.codex);
    let local = match &answers.local {
        Some(llama_server) => {
            match plan_local(&facts, model::PORT, Some(llama_server.clone()), outside) {
                Ok(local) => Some(local),
                Err(message) => {
                    let _ = p.warn(&message);
                    wizard::cancelled(p);
                    return Err(Failure::Refused(message));
                }
            }
        }
        None => None,
    };
    let mut plan = answer_wizard(&facts, answers, local);
    match wizard::confirm(p, &summary(&plan)) {
        Ok(true) => {}
        Ok(false) => return Err(declined(p)),
        Err(e) => return Err(stopped(p, e)),
    }
    settle_local(p, outside, &facts, &mut plan, plugins)?;
    let outcome = apply(&plan);
    if !outcome.failed && plan.embedder.is_some() {
        let notes = store::read_notes(&plan.notes).map_or(0, |notes| notes.len());
        let _ = wizard::first_index(p, notes, || index(&exe));
    }
    let _ = wizard::finish(p, outcome.failed);
    Ok(outcome)
}

/// `<exe> index` with stdin closed: its first stdout line, or its stderr without the `bilbo: ` prefixes.
fn run_index(exe: &Path) -> Result<String, String> {
    let output = std::process::Command::new(exe)
        .arg("index")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {} index: {e}", exe.display()))?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Ok(stdout.lines().next().unwrap_or("").to_string());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr
        .lines()
        .map(|line| line.strip_prefix("bilbo: ").unwrap_or(line))
        .collect();
    Err(if lines.is_empty() {
        format!("index {}", output.status)
    } else {
        lines.join("\n")
    })
}

fn usage<T>(message: impl Into<String>) -> Result<T, Failure> {
    Err(Failure::Usage(message.into()))
}

// ---------------------------------------------------------------------------
// Flags

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
enum Mode {
    Batch,
    Wizard,
    Remove,
}

/// The embedder the flags describe.
struct EmbedderFlags {
    url: String,
    model: String,
    token: Option<Token>,
    query_prefix: Option<String>,
}

impl EmbedderFlags {
    fn embedder(&self) -> Embedder {
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
struct LocalFlags {
    port: u16,
    llama_server: Option<PathBuf>,
}

/// Every flag, checked and typed.
struct Flags {
    mode: Mode,
    /// `--remove` asks first: in a terminal without `--yes`, or with `--interactive`.
    confirm: bool,
    no_plugin: bool,
    no_timer: bool,
    minutes: Option<u32>,
    embedder: Option<EmbedderFlags>,
    local: Option<LocalFlags>,
    /// The first embedder flag on the command line, for messages.
    embedder_flag: Option<&'static str>,
    claude: Option<PathBuf>,
    codex: Option<PathBuf>,
    source: Option<agents::Source>,
}

fn settle(args: &[String], env: &store::Env) -> Result<Flags, Failure> {
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

// ---------------------------------------------------------------------------
// Gather

enum ConfigState {
    Absent,
    Present,
    /// `target` is the link's target as written, or the config path when it is not a link.
    Managed {
        target: String,
    },
}

/// What the machine holds now.
struct Facts {
    root: PathBuf,
    notes: PathBuf,
    config_path: PathBuf,
    config: ConfigState,
    /// The embedder of the file read; `None` for an absent file or one without `embedder.url`.
    existing: Option<Embedder>,
    /// The digest lines of the file read, as written, which a rewrite keeps.
    digest: Vec<(&'static str, String)>,
    /// The file is there and sets no key: only comments and blank lines.
    config_empty: bool,
    token_path: PathBuf,
    exe: PathBuf,
    source: agents::Source,
    claude: Option<PathBuf>,
    codex: Option<PathBuf>,
    timer: TimerFacts,
    /// `llama-server` from `--llama-server` or PATH, as given: links are not resolved.
    llama_server: Option<PathBuf>,
    /// Where the model file goes, under the cache folder.
    model: Option<PathBuf>,
}

/// What the timer step reads from the machine and the environment.
struct TimerFacts {
    platform: Option<timer::Platform>,
    home: Option<PathBuf>,
    config_home: Option<PathBuf>,
    state_dir: Option<PathBuf>,
    /// `launchctl` or `systemctl` on PATH.
    tool: Option<PathBuf>,
    /// The locations setup saw, in the order the job carries them, or the first one that cannot be.
    locations: Result<Vec<(&'static str, String)>, String>,
}

fn timer_facts(env: &store::Env, path: Option<&std::ffi::OsStr>) -> TimerFacts {
    let platform = timer::platform();
    let program = match platform {
        Some(timer::Platform::Launchd) => Some("launchctl"),
        Some(timer::Platform::Systemd) => Some("systemctl"),
        None => None,
    };
    TimerFacts {
        platform,
        home: store::absolute(&env.home),
        config_home: store::config_home(env),
        state_dir: store::state_dir(env),
        tool: program.and_then(|name| command::find(name, path)),
        locations: locations(env),
    }
}

fn locations(env: &store::Env) -> Result<Vec<(&'static str, String)>, String> {
    let vars = [
        ("BILBO_HOME", &env.bilbo_home),
        ("BILBO_CONFIG", &env.bilbo_config),
        ("XDG_DATA_HOME", &env.xdg_data_home),
        ("XDG_CONFIG_HOME", &env.xdg_config_home),
        ("XDG_CACHE_HOME", &env.xdg_cache_home),
        ("XDG_STATE_HOME", &env.xdg_state_home),
    ];
    let mut out = Vec::new();
    for (name, value) in vars {
        if let Some(path) = store::absolute(value) {
            let text = path
                .into_os_string()
                .into_string()
                .map_err(|_| format!("{name} is not valid UTF-8"))?;
            out.push((name, text));
        }
    }
    Ok(out)
}

fn gather(
    flags: &Flags,
    env: &store::Env,
    path: Option<std::ffi::OsString>,
) -> Result<Facts, Failure> {
    let root = store::root(env).map_err(Failure::Config)?;
    let notes = root.join("notes");
    let (config_path, _) = config::path(env).map_err(Failure::Config)?.ok_or_else(|| {
        Failure::Config(
            "cannot find the config file: set BILBO_CONFIG, XDG_CONFIG_HOME or HOME to an absolute path"
                .into(),
        )
    })?;
    let config = config_state(&config_path);
    let (existing, digest) = match config {
        ConfigState::Absent => (None, Vec::new()),
        _ if std::fs::symlink_metadata(&config_path).is_err() => (None, Vec::new()),
        ConfigState::Managed { .. } if !config_path.exists() => (None, Vec::new()),
        _ => {
            let settings = config::load(env).map_err(Failure::Config)?;
            (settings.embedder, settings.digest_lines)
        }
    };
    let config_empty = matches!(config, ConfigState::Present) && sets_no_key(&config_path);
    let token_path = config_path.parent().unwrap_or(Path::new("/")).join("token");
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|e| Failure::Refused(format!("cannot find the bilbo executable: {e}")))?;
    let source = flags
        .source
        .clone()
        .unwrap_or_else(|| agents::default_source(&exe, env!("CARGO_PKG_VERSION")));
    let tool = |given: &Option<PathBuf>, name: &str| {
        if flags.no_plugin {
            return None;
        }
        given
            .clone()
            .or_else(|| command::find(name, path.as_deref()))
    };
    let claude = tool(&flags.claude, "claude");
    let codex = tool(&flags.codex, "codex");
    let llama_server = flags
        .local
        .as_ref()
        .and_then(|local| local.llama_server.clone())
        .or_else(|| command::find("llama-server", path.as_deref()));
    Ok(Facts {
        root,
        notes,
        config_path,
        config,
        existing,
        digest,
        config_empty,
        token_path,
        exe,
        source,
        claude,
        codex,
        timer: timer_facts(env, path.as_deref()),
        llama_server,
        model: store::cache_dir(env).map(|cache| model::path(&cache)),
    })
}

/// Whether the file holds only comments and blank lines.
fn sets_no_key(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| {
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        text.lines().all(|line| {
            let line = line.trim();
            line.is_empty() || line.starts_with('#')
        })
    })
}

fn config_state(path: &Path) -> ConfigState {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(path)
                .map_or_else(|_| path.display().to_string(), |t| t.display().to_string());
            return ConfigState::Managed { target };
        }
        return if folder_is_readonly(path) {
            ConfigState::Managed {
                target: path.display().to_string(),
            }
        } else {
            ConfigState::Present
        };
    }
    if folder_is_readonly(path) {
        ConfigState::Managed {
            target: path.display().to_string(),
        }
    } else {
        ConfigState::Absent
    }
}

/// The config's folder exists and has no write bit; nothing is written to find out.
fn folder_is_readonly(path: &Path) -> bool {
    path.parent()
        .and_then(|dir| std::fs::metadata(dir).ok())
        .is_some_and(|meta| meta.is_dir() && meta.permissions().readonly())
}

// ---------------------------------------------------------------------------
// Plan

enum ConfigPlan {
    Create(Option<Embedder>),
    Update(Option<Embedder>),
    Keep,
    Managed(String),
}

enum KeyPlan {
    NoEmbedder,
    /// The key comes from `token`; `kept` when the config already named it.
    Token {
        token: Token,
        kept: bool,
    },
    /// The wizard's pasted key is saved to `path`.
    Pasted {
        path: PathBuf,
    },
    Local,
    NoKey,
}

enum EmbedderPlan {
    None,
    Kept,
    Checked(usize),
}

enum PluginPlan {
    Skipped(&'static str),
    /// The tool's lists could not be read; the message is the step's failure.
    Unreadable(String),
    Run {
        program: PathBuf,
        change: agents::Change,
    },
}

enum TimerPlan {
    Skipped(&'static str),
    Failed(String),
    /// The key comes from this variable, which a timer cannot read.
    KeyVariable(String),
    /// A timer is installed and none is wanted.
    Remove {
        reason: &'static str,
        platform: timer::Platform,
        tool: Option<PathBuf>,
        place: timer::Place,
    },
    /// The installed files already hold the job.
    Keep,
    Install {
        platform: timer::Platform,
        tool: PathBuf,
        job: timer::Job,
        files: Vec<(PathBuf, String)>,
        update: bool,
    },
}

/// One step's report line: its status and detail.
struct Line {
    status: &'static str,
    detail: String,
}

/// The model and server lines of the local embedder.
struct LocalLines {
    model: Line,
    server: Line,
}

/// What the local embedder needs, settled before anything is written.
struct LocalPlan {
    port: u16,
    /// `http://127.0.0.1:<port>`
    url: String,
    llama_server: PathBuf,
    model: PathBuf,
    /// The model file is not there with the pinned size.
    download: bool,
    platform: timer::Platform,
    tool: PathBuf,
    place: timer::Place,
    job: timer::Job,
    files: Vec<(PathBuf, String)>,
    service: timer::Current,
}

impl LocalPlan {
    fn log(&self) -> &Path {
        &self.job.log
    }

    fn address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// The prepare stage has work: a download, or service files to write or reload.
    fn prepares(&self) -> bool {
        self.download || self.service != timer::Current::Same
    }
}

/// What the prepare stage and planning reach outside the process; `Net` for real, a script in tests.
trait Outside {
    /// Downloads the pinned model to `path`; `progress` gets (bytes done, total).
    fn fetch(&mut self, path: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String>;
    /// Waits for the server at `url` to report ready.
    fn ready(&mut self, url: &str) -> Result<(), String>;
    /// One embed request; the vector length.
    fn check(&mut self, embedder: &Embedder) -> Result<usize, String>;
    /// Whether something accepts a connection on 127.0.0.1:`port` within 1 s.
    fn listening(&mut self, port: u16) -> bool;
}

struct Net;

impl Outside for Net {
    fn fetch(&mut self, path: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
        model::download(path, &model::PINNED, model::WINDOW, progress)
    }

    fn ready(&mut self, url: &str) -> Result<(), String> {
        embed::ready(url, READY)
    }

    fn check(&mut self, embedder: &Embedder) -> Result<usize, String> {
        check_embedder(embedder)
    }

    fn listening(&mut self, port: u16) -> bool {
        let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        std::net::TcpStream::connect_timeout(&address, Duration::from_secs(1)).is_ok()
    }
}

struct Plan {
    notes: PathBuf,
    store_exists: bool,
    config_path: PathBuf,
    config: ConfigPlan,
    /// The embedder the config holds once setup is done.
    embedder: Option<Embedder>,
    /// The digest lines a rewritten config keeps.
    digest: Vec<(&'static str, String)>,
    key: KeyPlan,
    /// The key the wizard was given; written to the token file and never shown.
    pasted: Option<Zeroizing<String>>,
    check: EmbedderPlan,
    /// What the local embedder needs; `None` when it is not asked for.
    local: Option<LocalPlan>,
    /// The model and server lines; `None` when the local embedder is not asked for.
    local_lines: Option<LocalLines>,
    /// An installed embedder service the config setup leaves has no use for.
    unused_service: Option<TimerRemoval>,
    source: agents::Source,
    claude: PluginPlan,
    codex: PluginPlan,
    timer: TimerPlan,
}

/// Non-interactive answers: the flags over the config that is there, the embedder checked when a new config gets one.
fn answer_batch(flags: &Flags, facts: Facts, outside: &mut impl Outside) -> Result<Plan, Failure> {
    let path = facts.config_path.display();
    let mut check = EmbedderPlan::None;
    let local_embedder = flags.embedder.as_ref().filter(|_| flags.local.is_some());
    let local_managed = matches!(facts.config, ConfigState::Managed { .. })
        && local_embedder.is_some_and(|given| {
            facts
                .existing
                .as_ref()
                .is_some_and(|e| e.url == given.url && e.model == given.model)
        });
    if local_embedder.is_some()
        && matches!(facts.config, ConfigState::Managed { .. })
        && !local_managed
    {
        return Err(Failure::Config(format!(
            "{path} is managed elsewhere; change the embedder there, not with --embedder-local"
        )));
    }
    let local = match &flags.local {
        Some(asked) => Some(
            plan_local(&facts, asked.port, facts.llama_server.clone(), outside)
                .map_err(Failure::Refused)?,
        ),
        None => None,
    };
    let prepares = local.as_ref().is_some_and(LocalPlan::prepares);
    let (config, embedder) = match (&facts.config, &flags.embedder) {
        (ConfigState::Managed { target }, Some(_)) if local_managed => {
            (ConfigPlan::Managed(target.clone()), facts.existing.clone())
        }
        (ConfigState::Managed { .. }, Some(_)) => {
            let flag = flags.embedder_flag.unwrap_or("--embedder-url");
            return Err(Failure::Config(format!(
                "{path} is managed elsewhere; change the embedder there, not with {flag}"
            )));
        }
        (ConfigState::Managed { target }, None) => {
            (ConfigPlan::Managed(target.clone()), facts.existing.clone())
        }
        (ConfigState::Present, Some(given)) if facts.config_empty => {
            let planned = given.embedder();
            if !prepares {
                let dims =
                    check_planned(local.as_ref(), &planned, outside).map_err(Failure::Refused)?;
                check = EmbedderPlan::Checked(dims);
            }
            (ConfigPlan::Update(Some(planned.clone())), Some(planned))
        }
        (ConfigState::Present, Some(given)) => {
            let planned = given.embedder();
            if !same_embedder(facts.existing.as_ref(), &planned) {
                let what = if facts.existing.is_some() {
                    "other embedder settings"
                } else {
                    "settings but no embedder"
                };
                return Err(Failure::Refused(format!(
                    "{path} already sets {what}; edit it, or run bilbo setup in a terminal to change it"
                )));
            }
            (ConfigPlan::Keep, facts.existing.clone())
        }
        (ConfigState::Present, None) => (ConfigPlan::Keep, facts.existing.clone()),
        (ConfigState::Absent, Some(given)) => {
            let planned = given.embedder();
            if !prepares {
                let dims =
                    check_planned(local.as_ref(), &planned, outside).map_err(Failure::Refused)?;
                check = EmbedderPlan::Checked(dims);
            }
            (ConfigPlan::Create(Some(planned.clone())), Some(planned))
        }
        (ConfigState::Absent, None) => (ConfigPlan::Create(None), None),
    };
    if matches!(check, EmbedderPlan::None) && embedder.is_some() {
        check = EmbedderPlan::Kept;
    }
    let kept = !matches!(config, ConfigPlan::Create(_) | ConfigPlan::Update(_));
    let key = match &embedder {
        None => KeyPlan::NoEmbedder,
        Some(e) => match &e.token {
            Some(token) => KeyPlan::Token {
                token: token.clone(),
                kept,
            },
            None if config::is_local(&e.url) => KeyPlan::Local,
            None => KeyPlan::NoKey,
        },
    };
    let plugin = |tool, found: &Option<PathBuf>| match found {
        Some(program) => plan_plugin(tool, program, &facts.source),
        None if flags.no_plugin => PluginPlan::Skipped("--no-plugin"),
        None => PluginPlan::Skipped("not found"),
    };
    let claude = plugin(agents::Tool::Claude, &facts.claude);
    let codex = plugin(agents::Tool::Codex, &facts.codex);
    let timer = plan_timer(
        &facts,
        embedder.as_ref(),
        flags.no_timer.then_some("--no-timer"),
        flags.minutes.unwrap_or(DEFAULT_MINUTES),
    );
    let mut local_lines = local.as_ref().filter(|l| !l.prepares()).map(kept_lines);
    let mut unused_service = None;
    if local.is_none() {
        (local_lines, unused_service) = unused_local(&facts, embedder.as_ref());
    }
    Ok(Plan {
        store_exists: facts.notes.is_dir(),
        notes: facts.notes,
        config_path: facts.config_path,
        config,
        embedder,
        digest: facts.digest,
        key,
        pasted: None,
        check,
        local,
        local_lines,
        unused_service,
        source: facts.source,
        claude,
        codex,
        timer,
    })
}

/// The model and server lines of a local embedder that needs no preparing.
fn kept_lines(local: &LocalPlan) -> LocalLines {
    LocalLines {
        model: Line {
            status: "kept",
            detail: local.model.display().to_string(),
        },
        server: Line {
            status: "kept",
            detail: local.address(),
        },
    }
}

/// Whether the embedder is the local one: its model and a loopback URL with a numeric port.
fn is_local_embedder(embedder: &Embedder) -> bool {
    embedder.model == model::NAME
        && embedder
            .url
            .strip_prefix("http://127.0.0.1:")
            .is_some_and(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))
}

/// The embedder service as a removal, when its files are installed. Reads files only.
fn installed_service(t: &TimerFacts) -> Option<TimerRemoval> {
    let platform = t.platform?;
    let place = timer_place(t)?;
    timer::installed(platform, &place, timer::Name::Embedder).then(|| TimerRemoval::Run {
        platform,
        tool: t.tool.clone(),
        place,
    })
}

/// For a run that is not asked for the local embedder: the config it leaves is a local one, so the
/// service stays (`not asked`), or it is not, so an installed service goes. Runs no command.
fn unused_local(
    facts: &Facts,
    embedder: Option<&Embedder>,
) -> (Option<LocalLines>, Option<TimerRemoval>) {
    if embedder.is_some_and(is_local_embedder) {
        let skipped = || Line {
            status: "skipped",
            detail: "not asked".into(),
        };
        let lines = LocalLines {
            model: skipped(),
            server: skipped(),
        };
        return (Some(lines), None);
    }
    (None, installed_service(&facts.timer))
}

/// The wizard's answers over what the machine holds, as the same plan the batch builds. `local`
/// is the plan of the local embedder when the answers chose it.
fn answer_wizard(facts: &Facts, answers: wizard::Answers, local: Option<LocalPlan>) -> Plan {
    let embedder = answers.embedder;
    let config = match &facts.config {
        ConfigState::Managed { target } => ConfigPlan::Managed(target.clone()),
        ConfigState::Absent => ConfigPlan::Create(embedder.clone()),
        ConfigState::Present => {
            let unchanged = match &embedder {
                Some(planned) => same_embedder(facts.existing.as_ref(), planned),
                None => facts.existing.is_none(),
            };
            if unchanged {
                ConfigPlan::Keep
            } else {
                ConfigPlan::Update(embedder.clone())
            }
        }
    };
    let config_kept = matches!(config, ConfigPlan::Keep | ConfigPlan::Managed(_));
    let key = match (&embedder, &answers.pasted) {
        (None, _) => KeyPlan::NoEmbedder,
        (Some(_), Some(_)) => KeyPlan::Pasted {
            path: facts.token_path.clone(),
        },
        (Some(e), None) => match &e.token {
            Some(token) => KeyPlan::Token {
                token: token.clone(),
                kept: config_kept
                    || (*token == Token::File(facts.token_path.clone())
                        && facts.token_path.exists()),
            },
            None if config::is_local(&e.url) => KeyPlan::Local,
            None => KeyPlan::NoKey,
        },
    };
    let check = match (&embedder, answers.dims) {
        (None, _) => EmbedderPlan::None,
        (Some(_), Some(dims)) => EmbedderPlan::Checked(dims),
        (Some(_), None) => EmbedderPlan::Kept,
    };
    let plugin = |tool, found: &Option<PathBuf>, chosen: bool| match found {
        Some(program) if chosen => plan_plugin(tool, program, &facts.source),
        Some(_) => PluginPlan::Skipped("not chosen"),
        None => PluginPlan::Skipped("not found"),
    };
    let claude = plugin(agents::Tool::Claude, &facts.claude, answers.claude);
    let codex = plugin(agents::Tool::Codex, &facts.codex, answers.codex);
    let asked = embedder.is_some() && facts.timer.platform.is_some();
    let timer = plan_timer(
        facts,
        embedder.as_ref(),
        (asked && answers.timer.is_none()).then_some("not chosen"),
        answers.timer.unwrap_or(DEFAULT_MINUTES),
    );
    let (local_lines, unused_service) = match &local {
        Some(l) => (Some(l).filter(|l| !l.prepares()).map(kept_lines), None),
        None => unused_local(facts, embedder.as_ref()),
    };
    Plan {
        store_exists: facts.notes.is_dir(),
        notes: facts.notes.clone(),
        config_path: facts.config_path.clone(),
        config,
        embedder,
        digest: facts.digest.clone(),
        key,
        pasted: answers.pasted,
        check,
        local,
        local_lines,
        unused_service,
        source: facts.source.clone(),
        claude,
        codex,
        timer,
    }
}

/// The wizard's outside work, run with progress bars and spinners.
struct Shown<'a, P, O> {
    p: &'a mut P,
    outside: &'a mut O,
}

impl<P: Prompter, O: Outside> Outside for Shown<'_, P, O> {
    fn fetch(&mut self, path: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
        let outside = &mut *self.outside;
        self.p.progress(
            &format!(
                "Downloading {} ({} MB)",
                model::FILE,
                model::PINNED.size / 1_000_000
            ),
            model::PINNED.size,
            |report| {
                outside.fetch(path, &mut |done, total| {
                    report(done);
                    progress(done, total);
                })
            },
            |_| format!("Downloaded {}", path.display()),
        )
    }

    fn ready(&mut self, url: &str) -> Result<(), String> {
        let outside = &mut *self.outside;
        self.p.spin(
            "Starting llama-server and loading the model",
            || outside.ready(url),
            |_| "llama-server is ready".to_string(),
        )
    }

    fn check(&mut self, embedder: &Embedder) -> Result<usize, String> {
        let outside = &mut *self.outside;
        self.p.spin(
            &format!("Checking {} at {}", embedder.model, embedder.url),
            || outside.check(embedder),
            |dims| format!("{} answered with {dims}-dimensional vectors", embedder.url),
        )
    }

    fn listening(&mut self, port: u16) -> bool {
        self.outside.listening(port)
    }
}

/// An embed request to a local server that needs no preparing, once it reports ready.
fn check_kept(
    local: &LocalPlan,
    embedder: &Embedder,
    outside: &mut impl Outside,
) -> Result<(LocalLines, usize), (LocalLines, String)> {
    match outside
        .ready(&local.url)
        .and_then(|()| outside.check(embedder))
    {
        Ok(dims) => Ok((kept_lines(local), dims)),
        Err(message) => Err((kept_lines(local), message)),
    }
}

/// After the confirmation: prepares the local embedder (or only checks it, when a new config
/// would name it), offering a retry or keyword search only when that fails. `plugins` are the
/// wizard's claude and codex answers, which the keyword-only plan keeps.
fn settle_local<P: Prompter>(
    p: &mut P,
    outside: &mut impl Outside,
    facts: &Facts,
    plan: &mut Plan,
    plugins: (bool, bool),
) -> Result<(), Failure> {
    let Some(mut local) = plan.local.take() else {
        return Ok(());
    };
    let Some(embedder) = plan.embedder.clone() else {
        plan.local = Some(local);
        return Ok(());
    };
    let heavy = local.prepares();
    let check_only = !heavy
        && matches!(
            plan.config,
            ConfigPlan::Create(Some(_)) | ConfigPlan::Update(Some(_))
        );
    if !heavy && !check_only {
        plan.local = Some(local);
        return Ok(());
    }
    let mut downloaded = false;
    loop {
        let result = {
            let mut shown = Shown {
                p: &mut *p,
                outside: &mut *outside,
            };
            if heavy {
                prepare(&local, &embedder, &mut shown)
            } else {
                check_kept(&local, &embedder, &mut shown)
            }
        };
        match result {
            Ok((mut lines, dims)) => {
                if downloaded {
                    lines.model.status = "installed";
                }
                plan.local_lines = Some(lines);
                plan.check = EmbedderPlan::Checked(dims);
                plan.local = Some(local);
                return Ok(());
            }
            Err((lines, message)) => {
                downloaded |= lines.model.status == "installed";
                let again = wizard::local_failed(p, &message, local.log()).map_err(|e| {
                    if downloaded && e.kind() == std::io::ErrorKind::Interrupted {
                        let _ = p.cancel(&format!(
                            "Cancelled. Only the model was kept, at {}.",
                            local.model.display()
                        ));
                        return Failure::Refused(format!(
                            "setup cancelled; only the model was kept, at {}",
                            local.model.display()
                        ));
                    }
                    stopped(p, e)
                })?;
                if again {
                    local.download = !model::kept(&local.model, model::PINNED.size);
                    local.service = timer::current(&local.files);
                    continue;
                }
                let model = match lines.model.status {
                    "failed" => Line {
                        status: "skipped",
                        detail: "keyword search only".into(),
                    },
                    _ if downloaded => Line {
                        status: "installed",
                        detail: lines.model.detail,
                    },
                    _ => lines.model,
                };
                let server =
                    if timer::installed(local.platform, &local.place, timer::Name::Embedder) {
                        match timer::uninstall(
                            local.platform,
                            &command::System,
                            Some(&local.tool),
                            &local.place,
                            timer::Name::Embedder,
                        ) {
                            Ok(()) => Line {
                                status: "removed",
                                detail: "keyword search only".into(),
                            },
                            Err(message) => Line {
                                status: "failed",
                                detail: message,
                            },
                        }
                    } else {
                        Line {
                            status: "skipped",
                            detail: "keyword search only".into(),
                        }
                    };
                let keyword = wizard::Answers {
                    embedder: None,
                    pasted: None,
                    dims: None,
                    claude: plugins.0,
                    codex: plugins.1,
                    timer: None,
                    local: None,
                };
                *plan = answer_wizard(facts, keyword, None);
                plan.unused_service = None;
                plan.local_lines = Some(LocalLines { model, server });
                return Ok(());
            }
        }
    }
}

/// The platform and its service manager; on Linux a live user session too.
fn local_manager(facts: &Facts) -> Result<(timer::Platform, PathBuf), String> {
    let t = &facts.timer;
    let Some(platform) = t.platform else {
        return Err("the local embedder runs only on macOS and Linux".into());
    };
    let no_session = "the local embedder needs a systemd user session; none answers here";
    match (platform, &t.tool) {
        (timer::Platform::Launchd, None) => {
            Err("the local embedder needs launchctl, which is not on PATH".into())
        }
        (timer::Platform::Systemd, None) => Err(no_session.into()),
        (timer::Platform::Systemd, Some(tool)) if !timer::session(&command::System, tool) => {
            Err(no_session.into())
        }
        (_, Some(tool)) => Ok((platform, tool.clone())),
    }
}

/// The service files' place, the state folder and the model path.
fn local_folders(facts: &Facts) -> Result<(timer::Place, PathBuf, PathBuf), String> {
    let Some(model) = &facts.model else {
        return Err(
            "cannot find the cache folder: set XDG_CACHE_HOME, or HOME, to an absolute path".into(),
        );
    };
    let Some(state_dir) = &facts.timer.state_dir else {
        return Err(
            "cannot find the state folder: set XDG_STATE_HOME, or HOME, to an absolute path".into(),
        );
    };
    let Some(place) = timer_place(&facts.timer) else {
        return Err(match facts.timer.platform {
            Some(timer::Platform::Systemd) => {
                "cannot find the config folder: set XDG_CONFIG_HOME, or HOME, to an absolute path"
            }
            _ => "cannot find the home folder: set HOME to an absolute path",
        }
        .into());
    };
    Ok((place, state_dir.clone(), model.clone()))
}

/// Something answers on the port and no embedder service of ours is installed.
fn port_busy(
    platform: timer::Platform,
    place: &timer::Place,
    port: u16,
    outside: &mut impl Outside,
) -> Option<String> {
    (!timer::installed(platform, place, timer::Name::Embedder) && outside.listening(port)).then(
        || format!("127.0.0.1:{port} is already in use; pick another port with --embedder-port"),
    )
}

/// Why the local embedder cannot run here, or None: platform, service manager (on Linux a live user
/// session), state and home or config folders, and the port (busy and no embedder service file).
fn local_unavailable(facts: &Facts, port: u16, outside: &mut impl Outside) -> Option<String> {
    let (platform, _) = match local_manager(facts) {
        Ok(found) => found,
        Err(message) => return Some(message),
    };
    let (place, _, _) = match local_folders(facts) {
        Ok(found) => found,
        Err(message) => return Some(message),
    };
    port_busy(platform, &place, port, outside)
}

/// Settles the local embedder. Reads files, may run `systemctl --user is-system-running`, connects to the port; writes nothing.
fn plan_local(
    facts: &Facts,
    port: u16,
    llama_server: Option<PathBuf>,
    outside: &mut impl Outside,
) -> Result<LocalPlan, String> {
    let (platform, tool) = local_manager(facts)?;
    let Some(llama_server) = llama_server else {
        return Err("llama-server not found on PATH; install it (brew install llama.cpp, your distribution's llama.cpp package, or Nix's llama-cpp) or pass --llama-server <path>".into());
    };
    let (place, state_dir, model) = local_folders(facts)?;
    let job = timer::Job {
        kind: timer::Kind::Service,
        program: llama_server.clone(),
        args: model::server_args(&model, port),
        log: state_dir.join("bilbo/embedder.log"),
        env: Vec::new(),
    };
    let files = timer::files(platform, &place, &job)?;
    if let Some(message) = port_busy(platform, &place, port, outside) {
        return Err(message);
    }
    Ok(LocalPlan {
        port,
        url: format!("http://127.0.0.1:{port}"),
        llama_server,
        download: !model::kept(&model, model::PINNED.size),
        model,
        platform,
        tool,
        place,
        service: timer::current(&files),
        job,
        files,
    })
}

/// Download, service, readiness, check, in that order. On failure, unloads and deletes a service
/// this call wrote and returns the two report lines and the message for stderr.
fn prepare(
    local: &LocalPlan,
    embedder: &Embedder,
    outside: &mut impl Outside,
) -> Result<(LocalLines, usize), (LocalLines, String)> {
    let line = |status, detail: String| Line { status, detail };
    let model_line = if local.download {
        if let Err(message) = outside.fetch(&local.model, &mut |_, _| {}) {
            return Err((
                LocalLines {
                    model: line("failed", message.clone()),
                    server: line("skipped", "no model".into()),
                },
                message,
            ));
        }
        line("installed", local.model.display().to_string())
    } else {
        line("kept", local.model.display().to_string())
    };
    let wrote = local.service != timer::Current::Same;
    let log = local.log().display();
    let failed = |model: Line, detail: String, message: String| {
        Err((
            LocalLines {
                model,
                server: line("failed", detail),
            },
            message,
        ))
    };
    let loaded = if wrote {
        timer::install(
            local.platform,
            &command::System,
            &local.tool,
            &local.job,
            &local.files,
        )
    } else {
        timer::reload(
            local.platform,
            &command::System,
            &local.tool,
            &local.place,
            timer::Name::Embedder,
        )
    };
    if let Err(message) = loaded {
        return failed(model_line, format!("{message}; see {log}"), message);
    }
    let unload = || {
        if wrote {
            let _ = timer::uninstall(
                local.platform,
                &command::System,
                Some(&local.tool),
                &local.place,
                timer::Name::Embedder,
            );
        }
    };
    if let Err(message) = outside.ready(&local.url) {
        unload();
        return failed(model_line, format!("{message}; see {log}"), message);
    }
    let dims = match outside.check(embedder) {
        Ok(dims) => dims,
        Err(message) => {
            unload();
            let removed = if wrote {
                "; the service was removed"
            } else {
                ""
            };
            return failed(
                model_line,
                format!("{message}{removed}; see {log}"),
                message,
            );
        }
    };
    let status = match local.service {
        timer::Current::Missing => "installed",
        timer::Current::Different => "updated",
        timer::Current::Same => "kept",
    };
    Ok((
        LocalLines {
            model: model_line,
            server: line(status, local.address()),
        },
        dims,
    ))
}

/// Where the timer's files live, when the folders it needs are known.
fn timer_place(t: &TimerFacts) -> Option<timer::Place> {
    match t.platform? {
        timer::Platform::Launchd => t.home.clone().map(|home| timer::Place {
            home,
            config_home: t.config_home.clone().unwrap_or_default(),
        }),
        timer::Platform::Systemd => t.config_home.clone().map(|config_home| timer::Place {
            home: t.home.clone().unwrap_or_default(),
            config_home,
        }),
    }
}

/// Decides the timer step. `off` is why the user wants no timer; no embedder means none either.
/// Reads files, and on Linux asks `systemctl` whether a user manager answers; changes nothing.
fn plan_timer(
    facts: &Facts,
    embedder: Option<&Embedder>,
    off: Option<&'static str>,
    minutes: u32,
) -> TimerPlan {
    let t = &facts.timer;
    let Some(platform) = t.platform else {
        return TimerPlan::Skipped(off.unwrap_or(if embedder.is_none() {
            "no embedder"
        } else {
            "unsupported platform"
        }));
    };
    let place = timer_place(t);
    let wanted = match (off, embedder) {
        (Some(reason), _) => Err(reason),
        (None, None) => Err("no embedder"),
        (None, Some(embedder)) => Ok(embedder),
    };
    let embedder = match wanted {
        Ok(embedder) => embedder,
        Err(reason) => {
            return match place.filter(|place| timer::installed(platform, place, timer::Name::Index))
            {
                Some(_) if t.tool.is_none() => TimerPlan::Failed(format!(
                    "{} not found on PATH",
                    match platform {
                        timer::Platform::Launchd => "launchctl",
                        timer::Platform::Systemd => "systemctl",
                    }
                )),
                Some(place) => TimerPlan::Remove {
                    reason,
                    platform,
                    tool: t.tool.clone(),
                    place,
                },
                None => TimerPlan::Skipped(reason),
            };
        }
    };
    let tool = match (platform, &t.tool) {
        (timer::Platform::Launchd, None) => {
            return TimerPlan::Failed("launchctl not found on PATH".into());
        }
        (timer::Platform::Systemd, Some(tool)) if !timer::session(&command::System, tool) => {
            return TimerPlan::Skipped("no systemd user session");
        }
        (timer::Platform::Systemd, None) => return TimerPlan::Skipped("no systemd user session"),
        (_, Some(tool)) => tool.clone(),
    };
    if let Some(Token::Var(name)) = &embedder.token {
        return TimerPlan::KeyVariable(name.clone());
    }
    let env = match &t.locations {
        Ok(env) => env.clone(),
        Err(message) => return TimerPlan::Failed(message.clone()),
    };
    let Some(state_dir) = &t.state_dir else {
        return TimerPlan::Failed(
            "cannot find the state folder: set XDG_STATE_HOME, or HOME, to an absolute path".into(),
        );
    };
    let Some(place) = place else {
        return TimerPlan::Failed(match platform {
            timer::Platform::Launchd => "cannot find the home folder: set HOME to an absolute path",
            timer::Platform::Systemd => {
                "cannot find the config folder: set XDG_CONFIG_HOME, or HOME, to an absolute path"
            }
        }
        .into());
    };
    let job = timer::Job {
        kind: timer::Kind::Periodic { minutes },
        program: facts.exe.clone(),
        args: vec!["index".into()],
        log: state_dir.join("bilbo/index.log"),
        env,
    };
    let files = match timer::files(platform, &place, &job) {
        Ok(files) => files,
        Err(message) => return TimerPlan::Failed(message),
    };
    match timer::current(&files) {
        timer::Current::Same => TimerPlan::Keep,
        current => TimerPlan::Install {
            platform,
            tool,
            job,
            files,
            update: current == timer::Current::Different,
        },
    }
}

/// Reads the tool's lists, the only commands planning runs, and decides what to change.
fn plan_plugin(tool: agents::Tool, program: &Path, source: &agents::Source) -> PluginPlan {
    match agents::read(tool, &command::System, program) {
        Ok(state) => PluginPlan::Run {
            program: program.to_path_buf(),
            change: agents::plan(tool, &state, source, env!("CARGO_PKG_VERSION")),
        },
        Err(message) => PluginPlan::Unreadable(message),
    }
}

/// Equal on url, model, token and query prefix; the similarity floor is not a flag.
fn same_embedder(existing: Option<&Embedder>, planned: &Embedder) -> bool {
    existing.is_some_and(|e| {
        e.url == planned.url
            && e.model == planned.model
            && e.token == planned.token
            && e.query_prefix == planned.query_prefix
    })
}

/// The planned embedder's check at plan time. A kept local server may still be loading its
/// model (after a login or a restart), so it is waited for first.
fn check_planned(
    local: Option<&LocalPlan>,
    planned: &Embedder,
    outside: &mut impl Outside,
) -> Result<usize, String> {
    match local {
        Some(local) => {
            outside.ready(&local.url)?;
            outside.check(planned)
        }
        None => check_embedder(planned),
    }
}

/// One embed request, with the 15 s limit; the vector length, or the message.
fn check_embedder(embedder: &Embedder) -> Result<usize, String> {
    let client = embed::Client::new(embedder, |name| std::env::var_os(name), CHECK)?;
    vector_length(&client)
}

fn vector_length(client: &embed::Client) -> Result<usize, String> {
    let vectors = client.embed(&["bilbo setup check".to_string()])?;
    Ok(vectors.first().map_or(0, Vec::len))
}

/// What setup will do, one line each, for the wizard's confirmation.
fn summary(plan: &Plan) -> Vec<String> {
    let mut lines = Vec::new();
    let notes = plan.notes.display();
    let path = plan.config_path.display();
    lines.push(if plan.store_exists {
        format!("Keep the store folder {notes}")
    } else {
        format!("Create the store folder {notes}")
    });
    lines.push(match &plan.config {
        ConfigPlan::Create(_) => format!("Write the config {path}"),
        ConfigPlan::Update(_) => {
            let name = plan
                .config_path
                .file_name()
                .map_or(String::new(), |n| n.to_string_lossy().into_owned());
            format!("Update the config {path} (the old one becomes {name}.bak)")
        }
        ConfigPlan::Keep => format!("Keep the config {path}"),
        ConfigPlan::Managed(_) => format!("Keep the config {path}, managed elsewhere"),
    });
    match &plan.key {
        KeyPlan::Token { token, .. } => lines.push(match token {
            Token::Var(name) => format!("Read the key from the variable {name}"),
            Token::File(file) => format!("Read the key from {}", file.display()),
        }),
        KeyPlan::Pasted { path } => lines.push(format!(
            "Save the pasted key to {}, readable only by you",
            path.display()
        )),
        _ => {}
    }
    lines.push(match (&plan.embedder, &plan.check) {
        (None, _) => "Search by keywords only".to_string(),
        (Some(e), EmbedderPlan::Checked(dims)) => {
            format!("Embed with {} at {} ({dims} dimensions)", e.model, e.url)
        }
        (Some(e), _) => format!("Embed with {} at {}", e.model, e.url),
    });
    if let Some(local) = &plan.local {
        lines.push(if local.download {
            format!(
                "Download {} ({} MB) to {}",
                model::FILE,
                model::PINNED.size / 1_000_000,
                local.model.display()
            )
        } else {
            format!("Keep the model {}", local.model.display())
        });
        let server = local.llama_server.display();
        let address = local.address();
        lines.push(match (&local.service, local.platform) {
            (timer::Current::Missing, timer::Platform::Launchd) => format!(
                "Run {server} on {address} as the launchd agent {}",
                timer::EMBEDDER_LABEL
            ),
            (timer::Current::Missing, timer::Platform::Systemd) => format!(
                "Run {server} on {address} as the systemd user service {}",
                timer::Name::Embedder.unit()
            ),
            (timer::Current::Different, _) => {
                format!("Update the local embedder service to run {server} on {address}")
            }
            (timer::Current::Same, _) => format!("Keep the local embedder service on {address}"),
        });
        lines.push("llama-server keeps about 1 GB of memory in use".to_string());
        lines.push("The first index of a large store takes a while".to_string());
    }
    if matches!(plan.unused_service, Some(TimerRemoval::Run { .. })) {
        lines.push("Remove the local embedder service".to_string());
    }
    for (tool, plugin) in plugins(plan) {
        let label = tool.label();
        let change = match plugin {
            PluginPlan::Run { change, .. } => change,
            PluginPlan::Unreadable(message) => {
                lines.push(format!("{label} plugin: failed, {message}"));
                continue;
            }
            PluginPlan::Skipped(_) => continue,
        };
        lines.push(match change {
            agents::Change::Install(_) => format!(
                "Install the bilbo plugin in {label} from {}{}",
                plan.source.display(),
                if tool == agents::Tool::Codex {
                    " and trust its hooks"
                } else {
                    ""
                }
            ),
            agents::Change::Update(_) => format!(
                "Update the bilbo plugin in {label} to {}",
                plan.source.display()
            ),
            agents::Change::Keep => format!("Keep the bilbo plugin in {label}"),
        });
    }
    match &plan.timer {
        TimerPlan::Skipped("--no-timer" | "not chosen" | "no embedder") => {}
        TimerPlan::Skipped(reason) => lines.push(format!("Timer: skipped, {reason}")),
        TimerPlan::Failed(message) => lines.push(format!("Timer: failed, {message}")),
        TimerPlan::KeyVariable(name) => lines.push(format!(
            "The index timer will fail: the key comes from the variable {name}"
        )),
        TimerPlan::Remove { .. } => lines.push("Remove the index timer".to_string()),
        TimerPlan::Keep => lines.push("Keep the index timer".to_string()),
        TimerPlan::Install { platform, job, .. } => lines.push(format!(
            "Run bilbo index every {} min ({})",
            job.minutes(),
            match platform {
                timer::Platform::Launchd => format!("launchd agent {}", timer::LABEL),
                timer::Platform::Systemd => "systemd timer bilbo-index.timer".to_string(),
            }
        )),
    }
    lines
}

fn plugins(plan: &Plan) -> [(agents::Tool, &PluginPlan); 2] {
    [
        (agents::Tool::Claude, &plan.claude),
        (agents::Tool::Codex, &plan.codex),
    ]
}

// ---------------------------------------------------------------------------
// Apply

#[derive(Default)]
struct Report {
    lines: Vec<String>,
    failed: bool,
}

impl Report {
    fn line(&mut self, step: &str, status: &str, detail: Option<String>) {
        self.failed |= status == "failed";
        self.lines.push(match detail {
            Some(detail) => format!("{step} {status}: {detail}"),
            None => format!("{step} {status}"),
        });
    }
}

fn apply(plan: &Plan) -> Outcome {
    let mut report = Report::default();
    store_step(plan, &mut report);
    config_step(plan, &mut report);
    key_step(plan, &mut report);
    model_step(plan, &mut report);
    server_step(plan, &mut report);
    embedder_step(plan, &mut report);
    let mut codex_installed = false;
    for (tool, plugin) in plugins(plan) {
        let left = plugin_step(tool, plugin, &plan.source, &mut report);
        codex_installed = tool == agents::Tool::Codex && left;
    }
    hook_step(&plan.codex, codex_installed, &mut report);
    timer_step(&plan.timer, &mut report);
    Outcome {
        lines: report.lines,
        failed: report.failed,
    }
}

fn timer_step(plan: &TimerPlan, report: &mut Report) {
    match plan {
        TimerPlan::Skipped(reason) => report.line("timer", "skipped", Some((*reason).into())),
        TimerPlan::Failed(message) => report.line("timer", "failed", Some(message.clone())),
        TimerPlan::KeyVariable(name) => report.line(
            "timer",
            "failed",
            Some(format!(
                "the index timer cannot read the key variable {name}; keep the key in a file (--embedder-token-file) or pass --no-timer"
            )),
        ),
        TimerPlan::Keep => report.line("timer", "kept", None),
        TimerPlan::Remove {
            reason,
            platform,
            tool,
            place,
        } => match timer::uninstall(*platform, &command::System, tool.as_deref(), place, timer::Name::Index) {
            Ok(()) => report.line("timer", "removed", Some((*reason).into())),
            Err(message) => report.line("timer", "failed", Some(message)),
        },
        TimerPlan::Install {
            platform,
            tool,
            job,
            files,
            update,
        } => match timer::install(*platform, &command::System, tool, job, files) {
            Ok(()) => report.line(
                "timer",
                if *update { "updated" } else { "installed" },
                Some(format!("every {} min", job.minutes())),
            ),
            Err(message) => report.line("timer", "failed", Some(message)),
        },
    }
}

fn plugin_step(
    tool: agents::Tool,
    plugin: &PluginPlan,
    source: &agents::Source,
    report: &mut Report,
) -> bool {
    let step = tool.name();
    let (program, change) = match plugin {
        PluginPlan::Skipped(reason) => {
            report.line(step, "skipped", Some((*reason).into()));
            return false;
        }
        PluginPlan::Unreadable(message) => {
            report.line(step, "failed", Some(message.clone()));
            return false;
        }
        PluginPlan::Run { program, change } => (program, change),
    };
    let (status, commands) = match change {
        agents::Change::Keep => {
            report.line(step, "kept", None);
            return true;
        }
        agents::Change::Install(commands) => ("installed", commands),
        agents::Change::Update(commands) => ("updated", commands),
    };
    match agents::run_all(tool, &command::System, program, commands) {
        Ok(()) => {
            report.line(step, status, Some(source.display()));
            true
        }
        Err(failed) => {
            let mut detail = format!("{}: {}", source.display(), failed.message);
            let adding = failed.args.get(..3).is_some_and(|head| {
                head.iter()
                    .map(String::as_str)
                    .eq(["plugin", "marketplace", "add"])
            });
            if adding && matches!(source, agents::Source::GitHub { .. }) {
                detail
                    .push_str("; for a build without a release tag, pass --plugin-source <folder>");
            }
            report.line(step, "failed", Some(detail));
            false
        }
    }
}

/// The detail of a failed hook step: the first line of the message, cut to 200 characters.
fn short(message: &str) -> String {
    command::first_line(message).chars().take(200).collect()
}

/// Trusts the bilbo hook in Codex when the codex step left the plugin installed.
fn hook_step(codex: &PluginPlan, installed: bool, report: &mut Report) {
    let program = match codex {
        PluginPlan::Run { program, .. } if installed => program,
        _ => return report.line("hook", "skipped", Some("no codex plugin".into())),
    };
    let cwd = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/"));
    let mut rpc = match agents::app_server(program) {
        Ok(rpc) => rpc,
        Err(message) => return report.line("hook", "failed", Some(short(&message))),
    };
    let trusted = agents::trust_hooks(&mut rpc, &cwd);
    rpc.finish();
    match trusted {
        Ok(agents::Trust::Kept) => report.line("hook", "kept", Some("trusted in Codex".into())),
        Ok(agents::Trust::Wrote { changed }) => report.line(
            "hook",
            if changed { "updated" } else { "installed" },
            Some("trusted in Codex".into()),
        ),
        Ok(agents::Trust::NoHook) => {
            report.line("hook", "skipped", Some("codex lists no bilbo hook".into()))
        }
        Err(message) => report.line("hook", "failed", Some(short(&message))),
    }
}

/// Forgets the trust of every bilbo hook in Codex, whether or not the plugin is still there.
fn forget_step(program: &Option<PathBuf>, report: &mut Report) {
    let Some(program) = program else {
        return report.line("hook", "skipped", Some("not found".into()));
    };
    let mut rpc = match agents::app_server(program) {
        Ok(rpc) => rpc,
        Err(message) => return report.line("hook", "failed", Some(short(&message))),
    };
    let forgotten = agents::forget_hooks(&mut rpc);
    rpc.finish();
    match forgotten {
        Ok(0) => report.line("hook", "skipped", Some("not trusted".into())),
        Ok(_) => report.line("hook", "removed", None),
        Err(message) => report.line("hook", "failed", Some(short(&message))),
    }
}

// ---------------------------------------------------------------------------
// Remove

enum ToolRemoval {
    Skipped(&'static str),
    /// The tool's lists could not be read.
    Failed(String),
    Run {
        program: PathBuf,
        commands: Vec<Vec<String>>,
    },
}

enum TimerRemoval {
    Skipped(&'static str),
    Run {
        platform: timer::Platform,
        tool: Option<PathBuf>,
        place: timer::Place,
    },
}

/// What `--remove` will do; the store, the config and the key file are only named.
struct RemovePlan {
    store: Option<PathBuf>,
    config: Option<PathBuf>,
    key: Option<PathBuf>,
    /// The model file, when it exists.
    model: Option<PathBuf>,
    server: TimerRemoval,
    claude: ToolRemoval,
    codex: ToolRemoval,
    /// The `codex` whose app-server forgets the hook trust; `None` when there is none.
    hook: Option<PathBuf>,
    timer: TimerRemoval,
}

fn remove<P: Prompter>(
    flags: &Flags,
    env: &store::Env,
    path: Option<std::ffi::OsString>,
    p: &mut P,
) -> Result<Outcome, Failure> {
    store::root(env).map_err(Failure::Config)?;
    config::path(env).map_err(Failure::Config)?;
    let plan = plan_remove(flags, env, path);
    let summary = remove_summary(&plan);
    if !flags.confirm || summary.is_empty() {
        return Ok(apply_remove(&plan));
    }
    let asked = p
        .intro("bilbo setup --remove")
        .and_then(|()| wizard::confirm_remove(p, &summary));
    match asked {
        Ok(true) => {}
        Ok(false) => return Err(declined(p)),
        Err(e) => return Err(stopped(p, e)),
    }
    let outcome = apply_remove(&plan);
    let _ = wizard::finish(p, outcome.failed);
    Ok(outcome)
}

/// A wizard error: Ctrl-C or Esc cancels, anything else stops it; nothing was changed either way.
fn stopped<P: Prompter>(p: &mut P, e: std::io::Error) -> Failure {
    if e.kind() == std::io::ErrorKind::Interrupted {
        return declined(p);
    }
    wizard::cancelled(p);
    Failure::Refused(format!("the wizard stopped: {e}; nothing changed"))
}

fn declined<P: Prompter>(p: &mut P) -> Failure {
    wizard::cancelled(p);
    Failure::Refused("setup cancelled; nothing changed".into())
}

fn plan_remove(flags: &Flags, env: &store::Env, path: Option<std::ffi::OsString>) -> RemovePlan {
    let notes = store::root(env)
        .ok()
        .map(|root| root.join("notes"))
        .filter(|notes| notes.is_dir());
    let config_path = config::path(env).ok().flatten().map(|(path, _)| path);
    let config = config_path
        .clone()
        .filter(|path| std::fs::symlink_metadata(path).is_ok());
    let named = config::load(env)
        .ok()
        .and_then(|settings| settings.embedder)
        .and_then(|embedder| match embedder.token {
            Some(Token::File(file)) => Some(file),
            _ => None,
        });
    let key = named.filter(|file| file.exists()).or_else(|| {
        config_path
            .and_then(|path| path.parent().map(|dir| dir.join("token")))
            .filter(|file| file.exists())
    });
    let tool = |tool: agents::Tool, given: &Option<PathBuf>| {
        let Some(program) = given
            .clone()
            .or_else(|| command::find(tool.name(), path.as_deref()))
        else {
            return ToolRemoval::Skipped("not found");
        };
        match agents::read(tool, &command::System, &program) {
            Err(message) => ToolRemoval::Failed(message),
            Ok(state) => match agents::removal(tool, &state) {
                commands if commands.is_empty() => ToolRemoval::Skipped("not installed"),
                commands => ToolRemoval::Run { program, commands },
            },
        }
    };
    let hook = flags
        .codex
        .clone()
        .or_else(|| command::find("codex", path.as_deref()));
    let facts = timer_facts(env, path.as_deref());
    let server = installed_service(&facts).unwrap_or(TimerRemoval::Skipped("not installed"));
    let timer = match facts.platform {
        None => TimerRemoval::Skipped("unsupported platform"),
        Some(platform) => {
            let place = match platform {
                timer::Platform::Launchd => facts.home.clone().map(|home| timer::Place {
                    home,
                    config_home: facts.config_home.clone().unwrap_or_default(),
                }),
                timer::Platform::Systemd => {
                    facts.config_home.clone().map(|config_home| timer::Place {
                        home: facts.home.clone().unwrap_or_default(),
                        config_home,
                    })
                }
            };
            match place.filter(|place| timer::installed(platform, place, timer::Name::Index)) {
                Some(place) => TimerRemoval::Run {
                    platform,
                    tool: facts.tool,
                    place,
                },
                None => TimerRemoval::Skipped("not installed"),
            }
        }
    };
    let model = store::cache_dir(env)
        .map(|cache| model::path(&cache))
        .filter(|path| path.exists());
    RemovePlan {
        store: notes,
        config,
        key,
        model,
        server,
        claude: tool(agents::Tool::Claude, &flags.claude),
        codex: tool(agents::Tool::Codex, &flags.codex),
        hook,
        timer,
    }
}

/// The confirmation text: only what is removed; empty when nothing is.
fn remove_summary(plan: &RemovePlan) -> Vec<String> {
    let mut lines = Vec::new();
    for (tool, removal) in [
        (agents::Tool::Claude, &plan.claude),
        (agents::Tool::Codex, &plan.codex),
    ] {
        if matches!(removal, ToolRemoval::Run { .. }) {
            lines.push(format!("Remove the bilbo plugin from {}", tool.label()));
        }
    }
    if matches!(plan.server, TimerRemoval::Run { .. }) {
        lines.push("Remove the local embedder service".to_string());
    }
    if matches!(plan.timer, TimerRemoval::Run { .. }) {
        lines.push("Remove the index timer".to_string());
    }
    if !lines.is_empty() {
        lines.push("Keep the store, the config, the key file and the model".to_string());
    }
    lines
}

fn apply_remove(plan: &RemovePlan) -> Outcome {
    let mut report = Report::default();
    let kept = |step: &str, path: &Option<PathBuf>, none: &str, report: &mut Report| {
        report.line(
            step,
            "skipped",
            Some(match path {
                Some(path) => format!("kept {}", path.display()),
                None => none.to_string(),
            }),
        );
    };
    kept("store", &plan.store, "no store", &mut report);
    kept("config", &plan.config, "no config", &mut report);
    kept("key", &plan.key, "no key file", &mut report);
    kept("model", &plan.model, "no model", &mut report);
    match &plan.server {
        TimerRemoval::Skipped(reason) => report.line("server", "skipped", Some((*reason).into())),
        TimerRemoval::Run {
            platform,
            tool,
            place,
        } => match timer::uninstall(
            *platform,
            &command::System,
            tool.as_deref(),
            place,
            timer::Name::Embedder,
        ) {
            Ok(()) => report.line("server", "removed", None),
            Err(message) => report.line("server", "failed", Some(message)),
        },
    }
    for (tool, removal) in [
        (agents::Tool::Claude, &plan.claude),
        (agents::Tool::Codex, &plan.codex),
    ] {
        let step = tool.name();
        match removal {
            ToolRemoval::Skipped(reason) => report.line(step, "skipped", Some((*reason).into())),
            ToolRemoval::Failed(message) => report.line(step, "failed", Some(message.clone())),
            ToolRemoval::Run { program, commands } => {
                match agents::run_all(tool, &command::System, program, commands) {
                    Ok(()) => report.line(step, "removed", None),
                    Err(failed) => report.line(step, "failed", Some(failed.message)),
                }
            }
        }
    }
    forget_step(&plan.hook, &mut report);
    match &plan.timer {
        TimerRemoval::Skipped(reason) => report.line("timer", "skipped", Some((*reason).into())),
        TimerRemoval::Run {
            platform,
            tool,
            place,
        } => match timer::uninstall(
            *platform,
            &command::System,
            tool.as_deref(),
            place,
            timer::Name::Index,
        ) {
            Ok(()) => report.line("timer", "removed", None),
            Err(message) => report.line("timer", "failed", Some(message)),
        },
    }
    Outcome {
        lines: report.lines,
        failed: report.failed,
    }
}

fn store_step(plan: &Plan, report: &mut Report) {
    let notes = plan.notes.display().to_string();
    if plan.notes.is_dir() {
        return report.line("store", "kept", Some(notes));
    }
    match std::fs::create_dir_all(&plan.notes) {
        Ok(()) => report.line("store", "created", Some(notes)),
        Err(e) => report.line(
            "store",
            "failed",
            Some(format!("cannot create {notes}: {e}")),
        ),
    }
}

fn config_step(plan: &Plan, report: &mut Report) {
    let path = plan.config_path.display().to_string();
    let (embedder, backup) = match &plan.config {
        ConfigPlan::Keep => return report.line("config", "kept", Some(path)),
        ConfigPlan::Managed(target) => {
            return report.line(
                "config",
                "kept",
                Some(format!("managed elsewhere ({target})")),
            );
        }
        ConfigPlan::Create(embedder) => (embedder, false),
        ConfigPlan::Update(embedder) => (embedder, true),
    };
    let mut settings = embedder.as_ref().map(config::settings).unwrap_or_default();
    settings.extend(plan.digest.iter().cloned());
    let header = format!(
        "# bilbo config, written by bilbo setup {} on {}",
        env!("CARGO_PKG_VERSION"),
        jiff::Zoned::now().date()
    );
    let text = config::render(&header, &settings);
    match write_config(&plan.config_path, &text, backup) {
        Ok(()) => report.line(
            "config",
            if backup { "updated" } else { "written" },
            Some(path),
        ),
        Err(e) => report.line("config", "failed", Some(e)),
    }
}

fn key_step(plan: &Plan, report: &mut Report) {
    match &plan.key {
        KeyPlan::NoEmbedder => report.line("key", "skipped", Some("no embedder".into())),
        KeyPlan::Token { token, kept } => {
            let status = if *kept { "kept" } else { "ok" };
            let detail = match token {
                Token::File(file) if *kept => file.display().to_string(),
                Token::File(file) => format!("file {}", file.display()),
                Token::Var(name) => format!("variable {name}"),
            };
            report.line("key", status, Some(detail));
        }
        KeyPlan::Pasted { path } => {
            let key = plan.pasted.as_ref().map_or("", |key| key.as_str());
            match write_token(path, key) {
                Ok(()) => report.line("key", "written", Some(path.display().to_string())),
                Err(e) => report.line("key", "failed", Some(e)),
            }
        }
        KeyPlan::Local => report.line("key", "skipped", Some("local embedder".into())),
        KeyPlan::NoKey => report.line("key", "skipped", Some("no key".into())),
    }
}

fn model_step(plan: &Plan, report: &mut Report) {
    match &plan.local_lines {
        Some(lines) => report.line(
            "model",
            lines.model.status,
            Some(lines.model.detail.clone()),
        ),
        None => report.line("model", "skipped", Some("not local".into())),
    }
}

fn server_step(plan: &Plan, report: &mut Report) {
    match &plan.local_lines {
        Some(lines) => report.line(
            "server",
            lines.server.status,
            Some(lines.server.detail.clone()),
        ),
        None => match &plan.unused_service {
            Some(TimerRemoval::Run {
                platform,
                tool,
                place,
            }) => match timer::uninstall(
                *platform,
                &command::System,
                tool.as_deref(),
                place,
                timer::Name::Embedder,
            ) {
                Ok(()) => report.line("server", "removed", Some("not local".into())),
                Err(message) => report.line("server", "failed", Some(message)),
            },
            _ => report.line("server", "skipped", Some("not local".into())),
        },
    }
}

fn embedder_step(plan: &Plan, report: &mut Report) {
    match &plan.check {
        EmbedderPlan::None => report.line("embedder", "skipped", Some("none configured".into())),
        EmbedderPlan::Kept => report.line("embedder", "skipped", Some("config kept".into())),
        EmbedderPlan::Checked(dims) => {
            report.line("embedder", "ok", Some(format!("{dims} dimensions")))
        }
    }
}

/// Writes `text` to a temporary file in the folder, then renames it over `path`, after moving an old file to `<name>.bak` when `backup`.
fn write_config(path: &Path, text: &str, backup: bool) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    let dir = path.parent().unwrap_or(Path::new("/"));
    std::fs::create_dir_all(dir).map_err(fail)?;
    let temp = dir.join(format!(".config.tmp-{}", std::process::id()));
    let _ = std::fs::remove_file(&temp);
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        if backup {
            let name = path
                .file_name()
                .map_or(String::new(), |n| n.to_string_lossy().into_owned());
            std::fs::rename(path, dir.join(format!("{name}.bak")))?;
        }
        std::fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written.map_err(fail)
}

/// Writes the key and a newline to `path` through a temporary file that is created with mode 0600 before any byte goes in, then renamed over `path`.
fn write_token(path: &Path, key: &str) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    let dir = path.parent().unwrap_or(Path::new("/"));
    std::fs::create_dir_all(dir).map_err(fail)?;
    let temp = dir.join(format!(".token.tmp-{}", std::process::id()));
    let _ = std::fs::remove_file(&temp);
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(key.trim().as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written.map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    fn embedder(model: &str) -> Embedder {
        Embedder {
            url: "http://127.0.0.1:8081".into(),
            model: model.into(),
            token: None,
            query_prefix: config::default_query_prefix(model).into(),
            min_similarity: config::DEFAULT_MIN_SIMILARITY,
        }
    }

    fn plan(config: ConfigPlan, embedder: Option<Embedder>, check: EmbedderPlan) -> Plan {
        Plan {
            notes: "/r/notes".into(),
            store_exists: false,
            config_path: "/c/bilbo/config".into(),
            config,
            embedder,
            digest: Vec::new(),
            key: KeyPlan::NoEmbedder,
            pasted: None,
            check,
            local: None,
            local_lines: None,
            unused_service: None,
            source: agents::Source::GitHub {
                repo: "delucca/bilbo".into(),
                git_ref: "v1.2.3".into(),
            },
            claude: PluginPlan::Skipped("--no-plugin"),
            codex: PluginPlan::Skipped("--no-plugin"),
            timer: TimerPlan::Skipped("no embedder"),
        }
    }

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

    #[test]
    fn same_embedder_ignores_the_similarity_floor() {
        let mut stored = embedder("m");
        stored.min_similarity = 0.7;
        assert!(same_embedder(Some(&stored), &embedder("m")));
        assert!(!same_embedder(Some(&stored), &embedder("n")));
        assert!(!same_embedder(None, &embedder("m")));
    }

    #[test]
    fn summary_fresh_keyword_run() {
        let lines = summary(&plan(ConfigPlan::Create(None), None, EmbedderPlan::None));
        assert_eq!(
            lines,
            [
                "Create the store folder /r/notes",
                "Write the config /c/bilbo/config",
                "Search by keywords only",
            ]
        );
    }

    #[test]
    fn summary_names_the_embedder_and_the_key_source() {
        let mut p = plan(
            ConfigPlan::Update(Some(embedder("m"))),
            Some(embedder("m")),
            EmbedderPlan::Checked(1024),
        );
        p.store_exists = true;
        p.key = KeyPlan::Token {
            token: Token::Var("MY_KEY".into()),
            kept: false,
        };
        p.claude = PluginPlan::Run {
            program: "/bin/claude".into(),
            change: agents::Change::Install(Vec::new()),
        };
        p.codex = PluginPlan::Run {
            program: "/bin/codex".into(),
            change: agents::Change::Keep,
        };
        assert_eq!(
            summary(&p),
            [
                "Keep the store folder /r/notes",
                "Update the config /c/bilbo/config (the old one becomes config.bak)",
                "Read the key from the variable MY_KEY",
                "Embed with m at http://127.0.0.1:8081 (1024 dimensions)",
                "Install the bilbo plugin in Claude Code from delucca/bilbo#v1.2.3",
                "Keep the bilbo plugin in Codex",
            ]
        );
    }

    #[test]
    fn summary_names_the_hook_trust_when_codex_gets_the_plugin() {
        let mut p = plan(ConfigPlan::Keep, Some(embedder("m")), EmbedderPlan::Kept);
        p.claude = PluginPlan::Run {
            program: "/bin/claude".into(),
            change: agents::Change::Install(Vec::new()),
        };
        p.codex = PluginPlan::Run {
            program: "/bin/codex".into(),
            change: agents::Change::Install(Vec::new()),
        };
        let lines = summary(&p);
        assert!(
            lines.contains(
                &"Install the bilbo plugin in Codex from delucca/bilbo#v1.2.3 and trust its hooks"
                    .to_string()
            )
        );
        assert!(lines.contains(
            &"Install the bilbo plugin in Claude Code from delucca/bilbo#v1.2.3".to_string()
        ));
    }

    #[test]
    fn summary_managed_config() {
        let lines = summary(&plan(
            ConfigPlan::Managed("/nix/store/x".into()),
            Some(embedder("m")),
            EmbedderPlan::Kept,
        ));
        assert!(lines.contains(&"Keep the config /c/bilbo/config, managed elsewhere".to_string()));
        assert!(lines.contains(&"Embed with m at http://127.0.0.1:8081".to_string()));
    }

    #[test]
    fn write_config_replaces_atomically_and_keeps_a_backup() {
        let dir = std::env::temp_dir().join(format!("bilbo-setup-unit-{}", std::process::id()));
        let path = dir.join("nested/config");
        write_config(&path, "one\n", false).unwrap();
        write_config(&path, "two\n", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two\n");
        assert_eq!(
            std::fs::read_to_string(dir.join("nested/config.bak")).unwrap(),
            "one\n"
        );
        let leftovers = std::fs::read_dir(dir.join("nested"))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".tmp-")
            })
            .count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Writes an executable through a child `sh`, so this process never holds it open for
    /// writing: a test forking at that moment would inherit the descriptor, and running the
    /// file would fail with "Text file busy" on Linux (rust-lang/rust#114554).
    fn write_script(path: &Path, text: &str) {
        use std::io::Write;
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", r#"cat > "$1" && chmod 755 "$1""#, "sh"])
            .arg(path)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bilbo-setup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sets_no_key_reads_like_the_config_parser() {
        let dir = scratch("sets-no-key");
        let cases = [
            ("", true),
            ("# a\n\n  # b\r\n", true),
            ("\u{feff}", true),
            ("\u{feff}# comment\n", true),
            ("embedder.min_similarity = 0.6\n", false),
            ("\u{feff}embedder.url = http://x\n", false),
        ];
        for (i, (text, want)) in cases.iter().enumerate() {
            let path = dir.join(format!("config-{i}"));
            std::fs::write(&path, text).unwrap();
            assert_eq!(sets_no_key(&path), *want, "{text:?}");
        }
        assert!(!sets_no_key(&dir.join("missing")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_file_is_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("key-mode");
        let path = dir.join("nested/token");
        write_token(&path, "  sk-abc123 \n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "sk-abc123\n");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_file_is_replaced() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("key-replace");
        let path = dir.join("token");
        std::fs::write(&path, "old\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_token(&path, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let leftovers = std::fs::read_dir(&dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".tmp-")
            })
            .count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_never_in_report_or_summary() {
        let dir = scratch("key-report");
        let token = dir.join("cfg/token");
        let mut stored = embedder("m");
        stored.url = "https://api.example.com".into();
        stored.token = Some(Token::File(token.clone()));
        let mut p = plan(
            ConfigPlan::Create(Some(stored.clone())),
            Some(stored),
            EmbedderPlan::Checked(8),
        );
        p.notes = dir.join("notes");
        p.config_path = dir.join("cfg/config");
        p.key = KeyPlan::Pasted {
            path: token.clone(),
        };
        p.pasted = Some(Zeroizing::new("sk-secret-9".to_string()));
        let shown = summary(&p);
        assert!(shown.contains(&format!(
            "Save the pasted key to {}, readable only by you",
            token.display()
        )));
        let outcome = apply(&p);
        assert!(!outcome.failed, "{:?}", outcome.lines);
        assert!(
            outcome
                .lines
                .contains(&format!("key written: {}", token.display()))
        );
        for line in shown.iter().chain(&outcome.lines) {
            assert!(!line.contains("sk-secret"), "{line}");
        }
        assert_eq!(std::fs::read_to_string(&token).unwrap(), "sk-secret-9\n");
        assert!(
            !std::fs::read_to_string(dir.join("cfg/config"))
                .unwrap()
                .contains("sk-secret")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_write_failure_is_reported_without_the_key() {
        let dir = scratch("key-fail");
        std::fs::write(dir.join("blocker"), "x").unwrap();
        let path = dir.join("blocker/token");
        let mut p = plan(ConfigPlan::Keep, Some(embedder("m")), EmbedderPlan::Kept);
        p.key = KeyPlan::Pasted { path };
        p.pasted = Some(Zeroizing::new("sk-secret-9".to_string()));
        p.notes = dir.join("notes");
        let outcome = apply(&p);
        assert!(outcome.failed);
        let line = outcome
            .lines
            .iter()
            .find(|l| l.starts_with("key "))
            .unwrap();
        assert!(line.starts_with("key failed: cannot write "));
        assert!(!line.contains("sk-secret"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn seen(dir: &Path, existing: Option<Embedder>, config: ConfigState) -> Facts {
        Facts {
            root: dir.to_path_buf(),
            notes: dir.join("notes"),
            config_path: dir.join("cfg/config"),
            config,
            existing,
            digest: Vec::new(),
            config_empty: false,
            token_path: dir.join("cfg/token"),
            exe: "/bin/bilbo".into(),
            source: agents::Source::GitHub {
                repo: "delucca/bilbo".into(),
                git_ref: "v1.2.3".into(),
            },
            claude: None,
            codex: None,
            timer: TimerFacts {
                platform: None,
                home: None,
                config_home: None,
                state_dir: None,
                tool: None,
                locations: Ok(Vec::new()),
            },
            llama_server: None,
            model: None,
        }
    }

    /// An `Outside` that only answers whether a port is taken.
    struct Taken(bool);

    impl Outside for Taken {
        fn fetch(&mut self, _: &Path, _: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
            Err("fetch is not scripted".into())
        }
        fn ready(&mut self, _: &str) -> Result<(), String> {
            Err("ready is not scripted".into())
        }
        fn check(&mut self, _: &Embedder) -> Result<usize, String> {
            Err("check is not scripted".into())
        }
        fn listening(&mut self, _: u16) -> bool {
            self.0
        }
    }

    /// A manager that exits 0 and, for systemd, reports a live user session.
    const LIVE_MANAGER: &str =
        "#!/bin/sh\ncase \"$*\" in\n\"--user is-system-running\") echo running;;\nesac\nexit 0\n";

    /// Facts for a box whose manager answers: a script that exits 0 stands in for `launchctl` or `systemctl`.
    fn local_facts(dir: &Path) -> Facts {
        let tool = dir.join("manager");
        write_script(&tool, LIVE_MANAGER);
        let mut facts = seen(dir, None, ConfigState::Absent);
        facts.timer = TimerFacts {
            platform: timer::platform(),
            home: Some(dir.join("home")),
            config_home: Some(dir.join("home/.config")),
            state_dir: Some(dir.join("state")),
            tool: Some(tool),
            locations: Ok(Vec::new()),
        };
        facts.llama_server = Some(dir.join("llama-server"));
        facts.model = Some(dir.join("cache").join(model::FILE));
        facts
    }

    fn place_model(facts: &Facts, size: u64) {
        let path = facts.model.as_ref().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::File::create(path).unwrap().set_len(size).unwrap();
    }

    #[test]
    fn local_plan_states() {
        let dir = scratch("local-plan");
        let facts = local_facts(&dir);
        let plan = plan_local(&facts, 9100, facts.llama_server.clone(), &mut Taken(false))
            .ok()
            .unwrap();
        assert!(plan.download);
        assert_eq!(plan.service, timer::Current::Missing);
        assert!(plan.prepares());
        assert_eq!(plan.url, "http://127.0.0.1:9100");
        assert_eq!(plan.address(), "127.0.0.1:9100");
        assert_eq!(plan.log(), dir.join("state/bilbo/embedder.log"));
        assert_eq!(plan.job.args, model::server_args(&plan.model, 9100));

        place_model(&facts, model::PINNED.size - 1);
        let short = plan_local(&facts, 9100, facts.llama_server.clone(), &mut Taken(false))
            .ok()
            .unwrap();
        assert!(short.download, "a file of another size is not the model");

        place_model(&facts, model::PINNED.size);
        let placed = plan_local(&facts, 9100, facts.llama_server.clone(), &mut Taken(false))
            .ok()
            .unwrap();
        assert!(!placed.download);
        assert!(placed.prepares(), "the service is missing");

        for (path, text) in &placed.files {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let same = plan_local(&facts, 9100, facts.llama_server.clone(), &mut Taken(true))
            .ok()
            .unwrap();
        assert_eq!(same.service, timer::Current::Same);
        assert!(
            !same.prepares(),
            "bilbo's own server on the port is no conflict"
        );

        std::fs::write(&placed.files[0].0, "changed").unwrap();
        let moved = plan_local(&facts, 9100, facts.llama_server.clone(), &mut Taken(false))
            .ok()
            .unwrap();
        assert_eq!(moved.service, timer::Current::Different);
        assert!(moved.prepares());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An `Outside` that answers from a script and records its calls.
    struct Script {
        fetch: Result<(), String>,
        ready: Result<(), String>,
        check: Result<usize, String>,
        /// Check answers that fail before `check` takes over, first to last.
        failures: Vec<String>,
        calls: Vec<&'static str>,
    }

    impl Script {
        fn working() -> Script {
            Script {
                fetch: Ok(()),
                ready: Ok(()),
                check: Ok(1024),
                failures: Vec::new(),
                calls: Vec::new(),
            }
        }
    }

    impl Outside for Script {
        fn fetch(&mut self, path: &Path, _: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
            self.calls.push("fetch");
            if self.fetch.is_ok() {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::File::create(path)
                    .unwrap()
                    .set_len(model::PINNED.size)
                    .unwrap();
            }
            self.fetch.clone()
        }
        fn ready(&mut self, _: &str) -> Result<(), String> {
            self.calls.push("ready");
            self.ready.clone()
        }
        fn check(&mut self, _: &Embedder) -> Result<usize, String> {
            self.calls.push("check");
            if !self.failures.is_empty() {
                return Err(self.failures.remove(0));
            }
            self.check.clone()
        }
        fn listening(&mut self, _: u16) -> bool {
            false
        }
    }

    fn local_plan_of(facts: &Facts) -> LocalPlan {
        plan_local(facts, 9100, facts.llama_server.clone(), &mut Taken(false))
            .ok()
            .unwrap()
    }

    fn write_service(plan: &LocalPlan) {
        for (path, text) in &plan.files {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    fn lines_of(lines: &LocalLines) -> [String; 2] {
        [
            format!("{}: {}", lines.model.status, lines.model.detail),
            format!("{}: {}", lines.server.status, lines.server.detail),
        ]
    }

    #[test]
    fn local_prepare_downloads_then_installs() {
        let dir = scratch("local-prepare-fresh");
        let facts = local_facts(&dir);
        let plan = local_plan_of(&facts);
        let mut outside = Script::working();
        let (lines, dims) = prepare(&plan, &embedder("m"), &mut outside).ok().unwrap();
        assert_eq!(dims, 1024);
        assert_eq!(outside.calls, ["fetch", "ready", "check"]);
        assert_eq!(
            lines_of(&lines),
            [
                format!("installed: {}", plan.model.display()),
                "installed: 127.0.0.1:9100".to_string()
            ]
        );
        assert!(plan.files.iter().all(|(path, _)| path.exists()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_prepare_model_failure() {
        let dir = scratch("local-prepare-model");
        let facts = local_facts(&dir);
        let plan = local_plan_of(&facts);
        let mut outside = Script::working();
        let sha = "the download's SHA-256 is aa, expected bb";
        outside.fetch = Err(sha.into());
        let (lines, message) = prepare(&plan, &embedder("m"), &mut outside).err().unwrap();
        assert_eq!(message, sha);
        assert_eq!(
            lines_of(&lines),
            [format!("failed: {sha}"), "skipped: no model".to_string()]
        );
        assert_eq!(outside.calls, ["fetch"]);
        assert!(plan.files.iter().all(|(path, _)| !path.exists()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_prepare_not_ready_removes_the_service() {
        let dir = scratch("local-prepare-not-ready");
        let facts = local_facts(&dir);
        place_model(&facts, model::PINNED.size);
        let plan = local_plan_of(&facts);
        let mut outside = Script::working();
        outside.ready = Err("embedder http://127.0.0.1:9100 was not ready within 120 s".into());
        let (lines, message) = prepare(&plan, &embedder("m"), &mut outside).err().unwrap();
        assert!(message.contains("was not ready"));
        let [model_line, server] = lines_of(&lines);
        assert_eq!(model_line, format!("kept: {}", plan.model.display()));
        assert!(server.starts_with("failed: embedder http://127.0.0.1:9100 was not ready"));
        assert!(server.ends_with(&format!("; see {}", plan.log().display())));
        assert_eq!(outside.calls, ["ready"]);
        assert!(plan.files.iter().all(|(path, _)| !path.exists()));
        assert!(plan.model.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_prepare_check_failure_removes_the_service() {
        let dir = scratch("local-prepare-check");
        let facts = local_facts(&dir);
        place_model(&facts, model::PINNED.size);
        let plan = local_plan_of(&facts);
        let mut outside = Script::working();
        outside.check = Err("http://127.0.0.1:9100 answered 500".into());
        let (lines, message) = prepare(&plan, &embedder("m"), &mut outside).err().unwrap();
        assert_eq!(message, "http://127.0.0.1:9100 answered 500");
        assert_eq!(
            lines_of(&lines)[1],
            format!(
                "failed: http://127.0.0.1:9100 answered 500; the service was removed; see {}",
                plan.log().display()
            )
        );
        assert!(plan.files.iter().all(|(path, _)| !path.exists()));
        assert!(plan.model.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_prepare_kept_service_is_reloaded_after_a_download() {
        let dir = scratch("local-prepare-kept");
        let facts = local_facts(&dir);
        write_service(&local_plan_of(&facts));
        let plan = local_plan_of(&facts);
        assert!(plan.download);
        assert_eq!(plan.service, timer::Current::Same);
        let mut outside = Script::working();
        let (lines, _) = prepare(&plan, &embedder("m"), &mut outside).ok().unwrap();
        assert_eq!(
            lines_of(&lines),
            [
                format!("installed: {}", plan.model.display()),
                "kept: 127.0.0.1:9100".to_string()
            ]
        );
        assert_eq!(outside.calls, ["fetch", "ready", "check"]);

        let mut outside = Script::working();
        outside.check = Err("http://127.0.0.1:9100 answered 500".into());
        let (lines, _) = prepare(&plan, &embedder("m"), &mut outside).err().unwrap();
        assert_eq!(
            lines_of(&lines)[1],
            format!(
                "failed: http://127.0.0.1:9100 answered 500; see {}",
                plan.log().display()
            )
        );
        assert!(
            plan.files.iter().all(|(path, _)| path.exists()),
            "a service this run did not write stays"
        );
        assert_eq!(timer::current(&plan.files), timer::Current::Same);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_plan_refusals() {
        let dir = scratch("local-plan-refusals");
        let facts = local_facts(&dir);
        let busy = plan_local(&facts, 9100, facts.llama_server.clone(), &mut Taken(true));
        assert_eq!(
            busy.err().unwrap(),
            "127.0.0.1:9100 is already in use; pick another port with --embedder-port"
        );
        let missing = plan_local(&facts, 9100, None, &mut Taken(false));
        assert!(
            missing
                .err()
                .unwrap()
                .starts_with("llama-server not found on PATH;")
        );
        let mut no_cache = local_facts(&dir);
        no_cache.model = None;
        let message = plan_local(
            &no_cache,
            9100,
            no_cache.llama_server.clone(),
            &mut Taken(false),
        );
        assert!(
            message
                .err()
                .unwrap()
                .starts_with("cannot find the cache folder")
        );
        assert!(local_unavailable(&facts, 9100, &mut Taken(true)).is_some());
        assert!(local_unavailable(&facts, 9100, &mut Taken(false)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn answered(embedder: Option<Embedder>, pasted: Option<&str>) -> wizard::Answers {
        wizard::Answers {
            dims: embedder.as_ref().map(|_| 1024),
            embedder,
            pasted: pasted.map(|key| Zeroizing::new(key.to_string())),
            claude: true,
            codex: true,
            timer: None,
            local: None,
        }
    }

    #[test]
    fn wizard_same_embedder_keeps_the_config_and_a_changed_one_updates_it() {
        let dir = scratch("wizard-config");
        let stored = embedder("m");
        let same = answer_wizard(
            &seen(&dir, Some(stored.clone()), ConfigState::Present),
            answered(Some(stored.clone()), None),
            None,
        );
        assert!(matches!(same.config, ConfigPlan::Keep));
        assert!(matches!(same.check, EmbedderPlan::Checked(1024)));
        let other = answer_wizard(
            &seen(&dir, Some(stored), ConfigState::Present),
            answered(Some(embedder("n")), None),
            None,
        );
        assert!(matches!(other.config, ConfigPlan::Update(Some(_))));
        let keyword = answer_wizard(
            &seen(&dir, None, ConfigState::Present),
            answered(None, None),
            None,
        );
        assert!(matches!(keyword.config, ConfigPlan::Keep));
        assert!(matches!(keyword.key, KeyPlan::NoEmbedder));
        let fresh = answer_wizard(
            &seen(&dir, None, ConfigState::Absent),
            answered(None, None),
            None,
        );
        assert!(matches!(fresh.config, ConfigPlan::Create(None)));
        assert!(matches!(fresh.codex, PluginPlan::Skipped("not found")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_pasted_plans_a_write_and_keeps_it_out_of_the_summary() {
        let dir = scratch("key-plan");
        let mut chosen = embedder("m");
        chosen.url = "https://api.example.com".into();
        chosen.token = Some(Token::File(dir.join("cfg/token")));
        let plan = answer_wizard(
            &seen(&dir, None, ConfigState::Absent),
            answered(Some(chosen), Some("sk-secret-9")),
            None,
        );
        assert!(matches!(&plan.key, KeyPlan::Pasted { path } if *path == dir.join("cfg/token")));
        assert!(summary(&plan).iter().all(|l| !l.contains("sk-secret")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_declined_replace_is_kept() {
        let dir = scratch("key-declined");
        std::fs::create_dir_all(dir.join("cfg")).unwrap();
        std::fs::write(dir.join("cfg/token"), "old\n").unwrap();
        let mut chosen = embedder("m");
        chosen.url = "https://api.example.com".into();
        chosen.token = Some(Token::File(dir.join("cfg/token")));
        let plan = answer_wizard(
            &seen(&dir, None, ConfigState::Absent),
            answered(Some(chosen), None),
            None,
        );
        assert!(matches!(plan.key, KeyPlan::Token { kept: true, .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn first_index_runs_the_binary_with_the_verb() {
        let dir = scratch("run-index");
        let exe = dir.join("fake");
        write_script(
            &exe,
            "#!/bin/sh\nif [ \"$1\" = index ]; then echo 'embedded 2, kept 0, dropped 0'; echo second; else exit 3; fi\n",
        );
        assert_eq!(run_index(&exe).unwrap(), "embedded 2, kept 0, dropped 0");
        let failing = dir.join("failing");
        write_script(
            &failing,
            "#!/bin/sh\necho 'bilbo: no embedder' >&2\necho 'bilbo: more' >&2\nexit 1\n",
        );
        assert_eq!(run_index(&failing).unwrap_err(), "no embedder\nmore");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summary_shows_a_timer_that_will_not_run_and_a_plugin_that_cannot_be_read() {
        let mut p = plan(ConfigPlan::Keep, Some(embedder("m")), EmbedderPlan::Kept);
        p.timer = TimerPlan::Skipped("no systemd user session");
        p.claude = PluginPlan::Unreadable("cannot read `claude plugin list --json`: boom".into());
        p.codex = PluginPlan::Skipped("not chosen");
        assert_eq!(
            summary(&p),
            [
                "Create the store folder /r/notes",
                "Keep the config /c/bilbo/config",
                "Embed with m at http://127.0.0.1:8081",
                "Claude Code plugin: failed, cannot read `claude plugin list --json`: boom",
                "Timer: skipped, no systemd user session",
            ]
        );
        p.timer = TimerPlan::Failed("launchctl not found on PATH".into());
        assert!(summary(&p).contains(&"Timer: failed, launchctl not found on PATH".to_string()));
        for reason in ["--no-timer", "not chosen", "no embedder"] {
            p.timer = TimerPlan::Skipped(reason);
            assert!(
                !summary(&p).iter().any(|l| l.starts_with("Timer")),
                "{reason}"
            );
        }
    }

    /// The wizard and `--remove` driven without a terminal, against temp folders.
    mod driven {
        use super::*;
        use std::cell::Cell;
        use std::ffi::OsString;
        use std::io;

        /// Answers by prompt text; anything not listed takes the prompt's own initial value.
        #[derive(Default)]
        struct Scripted {
            /// (text the prompt holds, choice index; `usize::MAX` is the last one)
            selects: Vec<(&'static str, usize)>,
            multi: Vec<(&'static str, Vec<usize>)>,
            inputs: Vec<(&'static str, &'static str)>,
            confirms: Vec<(&'static str, bool)>,
            /// Ctrl-C at the nth prompt (0-based).
            interrupt_at: Option<usize>,
            asked: usize,
            /// The prompt count when "Apply these changes?" or "Remove these?" came up.
            at_confirm: Option<usize>,
            shown: Vec<String>,
        }

        impl Scripted {
            fn prompt(&mut self, text: &str) -> io::Result<()> {
                assert!(
                    self.asked < 100,
                    "the wizard asked {} prompts without ending; last shown: {:#?}",
                    self.asked,
                    &self.shown[self.shown.len().saturating_sub(12)..]
                );
                self.shown.push(text.to_string());
                let n = self.asked;
                self.asked += 1;
                if text == "Apply these changes?" || text == "Remove these?" {
                    self.at_confirm = Some(n);
                }
                if self.interrupt_at == Some(n) {
                    return Err(io::Error::from(io::ErrorKind::Interrupted));
                }
                Ok(())
            }
        }

        impl Prompter for Scripted {
            fn intro(&mut self, title: &str) -> io::Result<()> {
                self.shown.push(format!("intro: {title}"));
                Ok(())
            }
            fn info(&mut self, text: &str) -> io::Result<()> {
                self.shown.push(format!("info: {text}"));
                Ok(())
            }
            fn warn(&mut self, text: &str) -> io::Result<()> {
                self.shown.push(format!("warn: {text}"));
                Ok(())
            }
            fn note(&mut self, title: &str, body: &str) -> io::Result<()> {
                self.shown.push(format!("note: {title}\n{body}"));
                Ok(())
            }
            fn select(
                &mut self,
                prompt: &str,
                choices: &[wizard::Choice],
                initial: usize,
            ) -> io::Result<usize> {
                self.prompt(prompt)?;
                Ok(self
                    .selects
                    .iter()
                    .find(|(text, _)| prompt.contains(text))
                    .map_or(initial, |(_, i)| (*i).min(choices.len() - 1)))
            }
            fn multiselect(
                &mut self,
                prompt: &str,
                _: &[wizard::Choice],
                initial: &[usize],
            ) -> io::Result<Vec<usize>> {
                self.prompt(prompt)?;
                Ok(self
                    .multi
                    .iter()
                    .find(|(text, _)| prompt.contains(text))
                    .map_or(initial.to_vec(), |(_, picked)| picked.clone()))
            }
            fn input(
                &mut self,
                prompt: &str,
                default: &str,
                _: fn(&str) -> Result<(), String>,
            ) -> io::Result<String> {
                self.prompt(prompt)?;
                Ok(self
                    .inputs
                    .iter()
                    .find(|(text, _)| prompt.contains(text))
                    .map_or(default, |(_, value)| value)
                    .to_string())
            }
            fn password(
                &mut self,
                prompt: &str,
                _: fn(&str) -> Result<(), String>,
            ) -> io::Result<Zeroizing<String>> {
                self.prompt(prompt)?;
                Ok(Zeroizing::new(String::new()))
            }
            fn confirm(&mut self, prompt: &str, initial: bool) -> io::Result<bool> {
                self.prompt(prompt)?;
                Ok(self
                    .confirms
                    .iter()
                    .find(|(text, _)| prompt.contains(text))
                    .map_or(initial, |(_, yes)| *yes))
            }
            fn spin<T>(
                &mut self,
                message: &str,
                work: impl FnOnce() -> Result<T, String>,
                _: impl FnOnce(&T) -> String,
            ) -> Result<T, String> {
                self.shown.push(format!("spin: {message}"));
                work()
            }
            fn progress<T>(
                &mut self,
                message: &str,
                _: u64,
                work: impl FnOnce(&mut dyn FnMut(u64)) -> Result<T, String>,
                _: impl FnOnce(&T) -> String,
            ) -> Result<T, String> {
                self.shown.push(format!("progress: {message}"));
                work(&mut |_| {})
            }
            fn outro(&mut self, text: &str) -> io::Result<()> {
                self.shown.push(format!("outro: {text}"));
                Ok(())
            }
            fn cancel(&mut self, text: &str) -> io::Result<()> {
                self.shown.push(format!("cancel: {text}"));
                Ok(())
            }
        }

        struct Sandbox {
            dir: PathBuf,
            bin: PathBuf,
            env: store::Env,
        }

        impl Drop for Sandbox {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        }

        fn boxed(name: &str) -> Sandbox {
            let dir = scratch(&format!("driven-{name}"));
            let bin = dir.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let env = store::Env {
                bilbo_home: None,
                xdg_data_home: None,
                home: Some(dir.join("home").into()),
                bilbo_config: None,
                xdg_config_home: None,
                xdg_cache_home: None,
                xdg_state_home: None,
            };
            std::fs::create_dir_all(dir.join("home")).unwrap();
            Sandbox { dir, bin, env }
        }

        impl Sandbox {
            fn home(&self) -> PathBuf {
                self.dir.join("home")
            }
            fn config(&self) -> PathBuf {
                self.home().join(".config/bilbo/config")
            }
            fn path(&self) -> Option<OsString> {
                Some(self.bin.clone().into())
            }
            /// An executable that does nothing, on this box's PATH.
            fn tool(&self, name: &str) {
                super::write_script(&self.bin.join(name), "#!/bin/sh\nexit 0\n");
            }
            /// The service manager: on Linux a `systemctl` whose user session is live.
            fn manager(&self) -> &'static str {
                if cfg!(target_os = "macos") {
                    self.tool("launchctl");
                    "launchctl"
                } else {
                    super::write_script(&self.bin.join("systemctl"), super::LIVE_MANAGER);
                    "systemctl"
                }
            }
            fn write_config(&self, model: &str) {
                std::fs::create_dir_all(self.config().parent().unwrap()).unwrap();
                std::fs::write(
                    self.config(),
                    format!("embedder.url = http://127.0.0.1:8081\nembedder.model = {model}\n"),
                )
                .unwrap();
            }
            /// Every file under the box's home with its content, for before and after.
            fn snapshot(&self) -> Vec<(PathBuf, String)> {
                fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
                    let mut entries: Vec<_> = std::fs::read_dir(dir)
                        .unwrap()
                        .map(|e| e.unwrap().path())
                        .collect();
                    entries.sort();
                    for path in entries {
                        if path.is_dir() {
                            out.push((path.clone(), String::new()));
                            walk(&path, out);
                        } else {
                            out.push((
                                path.clone(),
                                std::fs::read_to_string(&path).unwrap_or_default(),
                            ));
                        }
                    }
                }
                let mut out = Vec::new();
                walk(&self.home(), &mut out);
                out
            }
            fn place(&self) -> timer::Place {
                timer::Place {
                    home: self.home(),
                    config_home: self.home().join(".config"),
                }
            }
            fn timer_files(&self) -> Vec<PathBuf> {
                timer::paths(
                    timer::platform().unwrap(),
                    &self.place(),
                    timer::Name::Index,
                )
            }
            fn install_timer_files(&self) {
                for file in self.timer_files() {
                    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                    std::fs::write(file, "old\n").unwrap();
                }
            }
        }

        fn flags(args: &[&str], mode: Mode, env: &store::Env) -> Flags {
            let strings: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            let mut flags = settle(&strings, env).ok().unwrap();
            flags.mode = mode;
            flags
        }

        struct Run {
            result: Result<Outcome, Failure>,
            probed: Cell<bool>,
            checked: Cell<usize>,
            indexed: Cell<bool>,
        }

        fn wizard_run(b: &Sandbox, p: &mut Scripted) -> Run {
            wizard_outside(b, p, &mut Script::working())
        }

        fn wizard_outside(b: &Sandbox, p: &mut Scripted, outside: &mut Script) -> Run {
            let probed = Cell::new(false);
            let checked = Cell::new(0);
            let indexed = Cell::new(false);
            let flags = flags(&["--yes"], Mode::Wizard, &b.env);
            let result = gather(&flags, &b.env, b.path()).and_then(|facts| {
                wizard_with(
                    facts,
                    p,
                    outside,
                    || {
                        probed.set(true);
                        None
                    },
                    |_, _| {
                        checked.set(checked.get() + 1);
                        Ok(8)
                    },
                    |_| {
                        indexed.set(true);
                        Ok("indexed".into())
                    },
                )
            });
            Run {
                result,
                probed,
                checked,
                indexed,
            }
        }

        fn refused(run: Run) -> String {
            match run.result {
                Err(Failure::Refused(message)) => message,
                Err(_) => panic!("expected Refused"),
                Ok(_) => panic!("expected a refusal"),
            }
        }

        fn report(run: &Run) -> &[String] {
            &run.result.as_ref().ok().expect("an outcome").lines
        }

        #[test]
        fn a_wizard_error_closes_the_frame_too() {
            let mut p = Scripted::default();
            let failure = stopped(&mut p, io::Error::other("boom"));
            assert!(
                matches!(&failure, Failure::Refused(m) if m == "the wizard stopped: boom; nothing changed")
            );
            assert_eq!(p.shown, ["cancel: Cancelled. Nothing changed."]);
        }

        const NO_TIMER: (&str, usize) = ("index fresh", usize::MAX);

        #[test]
        fn declining_the_summary_writes_nothing() {
            let b = boxed("decline");
            b.manager();
            let before = b.snapshot();
            let mut p = Scripted {
                confirms: vec![("Apply these changes?", false)],
                selects: vec![NO_TIMER],
                ..Scripted::default()
            };
            let run = wizard_run(&b, &mut p);
            assert_eq!(refused(run), "setup cancelled; nothing changed");
            assert_eq!(b.snapshot(), before);
            assert!(
                p.shown
                    .contains(&"cancel: Cancelled. Nothing changed.".to_string())
            );
        }

        #[test]
        fn interrupting_at_any_prompt_writes_nothing() {
            let b = boxed("interrupt-count");
            b.manager();
            let mut counting = Scripted {
                selects: vec![("embedder should", 4), ("Which embedder", 4)],
                inputs: vec![
                    ("Embedder URL", "http://127.0.0.1:8081"),
                    ("Model name", "m"),
                ],
                ..Scripted::default()
            };
            let counted = wizard_run(&b, &mut counting);
            assert!(counted.result.is_ok());
            let last = counting.at_confirm.expect("the summary was confirmed");
            assert!(last >= 4, "{last}");
            for i in 0..=last {
                let b = boxed(&format!("interrupt-{i}"));
                b.manager();
                let before = b.snapshot();
                let mut p = Scripted {
                    selects: counting.selects.clone(),
                    inputs: counting.inputs.clone(),
                    interrupt_at: Some(i),
                    ..Scripted::default()
                };
                let run = wizard_run(&b, &mut p);
                assert_eq!(
                    refused(run),
                    "setup cancelled; nothing changed",
                    "prompt {i}"
                );
                assert_eq!(b.snapshot(), before, "prompt {i}");
                assert!(!b.config().exists(), "prompt {i}");
            }
        }

        #[test]
        fn changing_the_model_updates_the_config_and_keeps_the_old_one() {
            let b = boxed("change-model");
            b.write_config("a");
            let old = std::fs::read_to_string(b.config()).unwrap();
            let mut p = Scripted {
                inputs: vec![("Model name", "b")],
                selects: vec![NO_TIMER],
                ..Scripted::default()
            };
            let run = wizard_run(&b, &mut p);
            assert_eq!(
                report(&run)[1],
                format!("config updated: {}", b.config().display())
            );
            assert_eq!(
                std::fs::read_to_string(b.config().with_file_name("config.bak")).unwrap(),
                old
            );
            assert!(
                std::fs::read_to_string(b.config())
                    .unwrap()
                    .contains("embedder.model = b")
            );
            assert_eq!(run.checked.get(), 1);
            assert!(run.indexed.get());
        }

        #[test]
        fn the_wizard_keeps_the_digest_settings() {
            let b = boxed("keep-digest");
            b.write_config("a");
            let mut text = std::fs::read_to_string(b.config()).unwrap();
            text.push_str("digest.enable = on\ndigest.min_similarity = 0.6\ndigest.log = on\n");
            std::fs::write(b.config(), text).unwrap();
            let mut p = Scripted {
                inputs: vec![("Model name", "b")],
                selects: vec![NO_TIMER],
                ..Scripted::default()
            };
            let run = wizard_run(&b, &mut p);
            assert!(run.result.is_ok());
            let text = std::fs::read_to_string(b.config()).unwrap();
            let at = |needle: &str| {
                text.find(needle)
                    .unwrap_or_else(|| panic!("{needle}: {text}"))
            };
            assert!(at("embedder.model = b") < at("digest.enable = on"));
            assert!(at("digest.enable = on") < at("digest.min_similarity = 0.6"));
            assert!(at("digest.min_similarity = 0.6") < at("digest.log = on"));
        }

        #[test]
        fn keeping_every_default_keeps_the_config() {
            let b = boxed("keep");
            b.write_config("a");
            let old = std::fs::read_to_string(b.config()).unwrap();
            let mut p = Scripted {
                selects: vec![NO_TIMER],
                ..Scripted::default()
            };
            let run = wizard_run(&b, &mut p);
            assert_eq!(
                report(&run)[1],
                format!("config kept: {}", b.config().display())
            );
            assert_eq!(std::fs::read_to_string(b.config()).unwrap(), old);
            assert!(!b.config().with_file_name("config.bak").exists());
        }

        #[test]
        fn a_managed_config_is_neither_probed_nor_checked() {
            let b = boxed("managed");
            let target = b.dir.join("managed-config");
            std::fs::write(
                &target,
                "embedder.url = http://127.0.0.1:8081\nembedder.model = m\n",
            )
            .unwrap();
            std::fs::create_dir_all(b.config().parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&target, b.config()).unwrap();
            let mut p = Scripted {
                selects: vec![NO_TIMER],
                ..Scripted::default()
            };
            let run = wizard_run(&b, &mut p);
            assert_eq!(
                report(&run)[1],
                format!("config kept: managed elsewhere ({})", target.display())
            );
            assert!(!run.probed.get());
            assert_eq!(run.checked.get(), 0);
            assert!(!p.shown.iter().any(|s| s.contains("Which embedder")));
            assert!(std::fs::symlink_metadata(b.config()).unwrap().is_symlink());
        }

        #[test]
        fn unticked_codex_is_skipped_and_no_timer_removes_the_old_one() {
            let b = boxed("unticked");
            b.tool("codex");
            let manager = b.manager();
            b.write_config("m");
            b.install_timer_files();
            let mut p = Scripted {
                multi: vec![("Install the bilbo plugin in", vec![])],
                selects: vec![NO_TIMER],
                ..Scripted::default()
            };
            let run = wizard_run(&b, &mut p);
            let lines = report(&run);
            assert!(
                lines.contains(&"codex skipped: not chosen".to_string()),
                "{lines:?}"
            );
            assert!(
                lines.contains(&"hook skipped: no codex plugin".to_string()),
                "{lines:?}"
            );
            assert!(
                lines.contains(&"timer removed: not chosen".to_string()),
                "{manager}: {lines:?}"
            );
            assert!(b.timer_files().iter().all(|f| !f.exists()));
            assert!(!run.result.as_ref().ok().unwrap().failed);
        }

        /// A box whose manager and `llama-server` answer, with the local embedder's files in its home.
        fn local_box(name: &str) -> Sandbox {
            let b = boxed(name);
            b.manager();
            b.tool("llama-server");
            b
        }

        impl Sandbox {
            fn model(&self) -> PathBuf {
                model::path(&store::cache_dir(&self.env).unwrap())
            }
            fn service_files(&self) -> Vec<PathBuf> {
                timer::paths(
                    timer::platform().unwrap(),
                    &self.place(),
                    timer::Name::Embedder,
                )
            }
        }

        const LOCAL: (&str, usize) = ("Which embedder", 1);

        fn lines_hold(run: &Run, line: &str) -> bool {
            report(run).iter().any(|l| l == line)
        }

        #[test]
        fn local_declined_downloads_nothing() {
            let b = local_box("local-declined");
            let before = b.snapshot();
            let mut p = Scripted {
                confirms: vec![("Apply these changes?", false)],
                selects: vec![LOCAL, NO_TIMER],
                ..Scripted::default()
            };
            let mut outside = Script::working();
            let run = wizard_outside(&b, &mut p, &mut outside);
            assert_eq!(refused(run), "setup cancelled; nothing changed");
            assert!(outside.calls.is_empty(), "{:?}", outside.calls);
            assert_eq!(b.snapshot(), before);
            assert!(!b.model().exists());
            assert!(b.service_files().iter().all(|f| !f.exists()));
        }

        #[test]
        fn local_summary_names_download_service_memory_and_first_index() {
            let b = local_box("local-summary");
            let mut p = Scripted {
                confirms: vec![("Apply these changes?", false)],
                selects: vec![LOCAL, NO_TIMER],
                ..Scripted::default()
            };
            wizard_run(&b, &mut p);
            let summary = p
                .shown
                .iter()
                .find(|s| s.starts_with("note: Setup will"))
                .expect("the summary was shown");
            let server = b.bin.join("llama-server");
            let manager = match timer::platform().unwrap() {
                timer::Platform::Launchd => "the launchd agent io.github.delucca.bilbo.embedder",
                timer::Platform::Systemd => "the systemd user service bilbo-embedder.service",
            };
            for line in [
                format!(
                    "Download Qwen3-Embedding-0.6B-Q8_0.gguf (639 MB) to {}",
                    b.model().display()
                ),
                format!("Run {} on 127.0.0.1:8737 as {manager}", server.display()),
                "llama-server keeps about 1 GB of memory in use".to_string(),
                "The first index of a large store takes a while".to_string(),
                "Embed with qwen3-embedding-0.6b at http://127.0.0.1:8737".to_string(),
            ] {
                assert!(summary.contains(&line), "{line}\n{summary}");
            }
        }

        #[test]
        fn local_progress_bar_runs_the_download() {
            let b = local_box("local-progress");
            let mut p = Scripted {
                selects: vec![LOCAL, NO_TIMER],
                ..Scripted::default()
            };
            let mut outside = Script::working();
            let run = wizard_outside(&b, &mut p, &mut outside);
            assert_eq!(outside.calls, ["fetch", "ready", "check"]);
            let shown = &p.shown;
            let at = |text: &str| {
                shown
                    .iter()
                    .position(|s| s == text)
                    .unwrap_or_else(|| panic!("{text}: {shown:?}"))
            };
            let bar = at("progress: Downloading Qwen3-Embedding-0.6B-Q8_0.gguf (639 MB)");
            let ready = at("spin: Starting llama-server and loading the model");
            let check = at("spin: Checking qwen3-embedding-0.6b at http://127.0.0.1:8737");
            assert!(bar < ready && ready < check);
            assert!(lines_hold(
                &run,
                &format!("model installed: {}", b.model().display())
            ));
            assert!(lines_hold(&run, "server installed: 127.0.0.1:8737"));
            assert!(lines_hold(&run, "embedder ok: 1024 dimensions"));
            assert_eq!(run.checked.get(), 0, "the local check runs through Outside");
            assert!(run.indexed.get());
            assert!(b.service_files().iter().all(|f| f.exists()));
        }

        #[test]
        fn local_failure_then_keyword_only() {
            let b = local_box("local-keyword-only");
            let mut p = Scripted {
                selects: vec![LOCAL, ("local embedder failed", 1), NO_TIMER],
                ..Scripted::default()
            };
            let mut outside = Script::working();
            outside.failures = vec!["http://127.0.0.1:8737 answered 500".into()];
            let run = wizard_outside(&b, &mut p, &mut outside);
            let log = b.home().join(".local/state/bilbo/embedder.log");
            assert!(
                p.shown.iter().any(
                    |s| s.starts_with("warn: http://127.0.0.1:8737 answered 500")
                        && s.contains(&format!("The server's log is {}", log.display()))
                ),
                "{:?}",
                p.shown
            );
            assert_eq!(outside.calls, ["fetch", "ready", "check"]);
            assert!(b.service_files().iter().all(|f| !f.exists()));
            assert!(b.model().exists());
            assert!(
                !std::fs::read_to_string(b.config())
                    .unwrap()
                    .lines()
                    .any(|l| l.starts_with("embedder."))
            );
            assert!(lines_hold(
                &run,
                &format!("model installed: {}", b.model().display())
            ));
            assert!(lines_hold(&run, "server skipped: keyword search only"));
            assert!(lines_hold(&run, "embedder skipped: none configured"));
            assert!(!run.result.as_ref().ok().unwrap().failed);
            assert!(!run.indexed.get());
        }

        #[test]
        fn local_failure_then_retry() {
            let b = local_box("local-retry");
            let mut p = Scripted {
                selects: vec![LOCAL, NO_TIMER],
                ..Scripted::default()
            };
            let mut outside = Script::working();
            outside.failures = vec!["http://127.0.0.1:8737 answered 500".into()];
            let run = wizard_outside(&b, &mut p, &mut outside);
            assert_eq!(
                outside.calls,
                ["fetch", "ready", "check", "ready", "check"],
                "the model stays; the service is written again"
            );
            assert!(lines_hold(
                &run,
                &format!("model installed: {}", b.model().display())
            ));
            assert!(lines_hold(&run, "server installed: 127.0.0.1:8737"));
            assert!(lines_hold(&run, "embedder ok: 1024 dimensions"));
            assert!(b.service_files().iter().all(|f| f.exists()));
        }

        #[test]
        fn local_retry_reports_the_service_as_installed_after_a_removal() {
            let b = local_box("local-retry-state");
            let mut first = Scripted {
                selects: vec![LOCAL, NO_TIMER],
                ..Scripted::default()
            };
            wizard_run(&b, &mut first);
            std::fs::write(&b.service_files()[0], "changed").unwrap();
            let mut p = Scripted {
                selects: vec![LOCAL, NO_TIMER],
                ..Scripted::default()
            };
            let mut outside = Script::working();
            outside.failures = vec!["http://127.0.0.1:8737 answered 500".into()];
            let run = wizard_outside(&b, &mut p, &mut outside);
            assert_eq!(outside.calls, ["ready", "check", "ready", "check"]);
            assert!(lines_hold(&run, "server installed: 127.0.0.1:8737"));
            assert!(!lines_hold(&run, "server updated: 127.0.0.1:8737"));
        }

        #[test]
        fn local_rerun_preselects_and_keeps() {
            let b = local_box("local-rerun");
            let mut first = Scripted {
                selects: vec![LOCAL, NO_TIMER],
                ..Scripted::default()
            };
            wizard_run(&b, &mut first);
            let mut p = Scripted {
                selects: vec![NO_TIMER],
                ..Scripted::default()
            };
            let mut outside = Script::working();
            let run = wizard_outside(&b, &mut p, &mut outside);
            assert!(outside.calls.is_empty(), "{:?}", outside.calls);
            assert!(lines_hold(
                &run,
                &format!("model kept: {}", b.model().display())
            ));
            assert!(lines_hold(&run, "server kept: 127.0.0.1:8737"));
            assert!(lines_hold(&run, "embedder skipped: config kept"));
            assert!(
                p.shown
                    .iter()
                    .any(|s| s.starts_with("note: Setup will") && s.contains("Keep the model")),
                "{:?}",
                p.shown
            );
        }

        #[test]
        fn local_keyword_only_removes_the_installed_service() {
            let b = local_box("local-removes-service");
            let mut first = Scripted {
                selects: vec![LOCAL, NO_TIMER],
                ..Scripted::default()
            };
            wizard_run(&b, &mut first);
            assert!(b.service_files().iter().all(|f| f.exists()));
            let mut p = Scripted {
                selects: vec![("Which embedder", 0)],
                ..Scripted::default()
            };
            let run = wizard_run(&b, &mut p);
            assert!(
                p.shown.iter().any(|s| s.starts_with("note: Setup will")
                    && s.contains("Remove the local embedder service")),
                "{:?}",
                p.shown
            );
            assert!(b.service_files().iter().all(|f| !f.exists()));
            assert!(lines_hold(&run, "server removed: not local"));
            assert!(b.model().exists());
        }

        /// A box with the model and service kept and no config, so the check is all setup does.
        fn local_kept_without_config(name: &str) -> Sandbox {
            let b = local_box(name);
            let mut first = Scripted {
                selects: vec![LOCAL, NO_TIMER],
                ..Scripted::default()
            };
            wizard_run(&b, &mut first);
            std::fs::remove_file(b.config()).unwrap();
            b
        }

        #[test]
        fn local_kept_server_is_waited_for_before_the_check() {
            let b = local_kept_without_config("local-kept-ready");
            let mut p = Scripted {
                selects: vec![LOCAL, NO_TIMER],
                ..Scripted::default()
            };
            let mut outside = Script::working();
            let run = wizard_outside(&b, &mut p, &mut outside);
            assert_eq!(outside.calls, ["ready", "check"]);
            assert!(lines_hold(&run, "server kept: 127.0.0.1:8737"));
            assert!(lines_hold(&run, "embedder ok: 1024 dimensions"));
        }

        #[test]
        fn local_kept_server_that_never_gets_ready_is_not_checked() {
            let b = local_kept_without_config("local-kept-not-ready");
            let mut p = Scripted {
                selects: vec![LOCAL, ("local embedder failed", 1), NO_TIMER],
                ..Scripted::default()
            };
            let mut outside = Script::working();
            outside.ready = Err("http://127.0.0.1:8737 did not become ready".into());
            wizard_outside(&b, &mut p, &mut outside);
            assert_eq!(outside.calls, ["ready"]);
            assert!(
                p.shown.iter().any(|s| s.contains("did not become ready")),
                "{:?}",
                p.shown
            );
        }

        #[test]
        fn local_batch_waits_for_a_kept_server_before_the_check() {
            for ready in [Ok(()), Err("not ready".to_string())] {
                let b = local_kept_without_config("local-batch-kept-ready");
                let flags = flags(&["--yes", "--embedder-local"], Mode::Batch, &b.env);
                let facts = gather(&flags, &b.env, b.path()).ok().unwrap();
                let mut outside = Script::working();
                outside.ready = ready.clone();
                let plan = answer_batch(&flags, facts, &mut outside);
                match ready {
                    Ok(()) => {
                        assert_eq!(outside.calls, ["ready", "check"]);
                        assert!(matches!(
                            plan.ok().unwrap().check,
                            EmbedderPlan::Checked(1024)
                        ));
                    }
                    Err(message) => {
                        assert_eq!(outside.calls, ["ready"]);
                        match plan {
                            Err(Failure::Refused(m)) => assert_eq!(m, message),
                            _ => panic!("expected a refusal"),
                        }
                    }
                }
            }
        }

        fn remove_run(b: &Sandbox, p: &mut Scripted) -> Result<Outcome, Failure> {
            let mut flags = flags(&["--remove"], Mode::Remove, &b.env);
            flags.confirm = true;
            remove(&flags, &b.env, b.path(), p)
        }

        #[test]
        fn remove_with_nothing_to_remove_asks_nothing() {
            let b = boxed("remove-empty");
            let mut p = Scripted::default();
            let outcome = remove_run(&b, &mut p).ok().unwrap();
            assert!(!outcome.failed);
            assert!(p.shown.is_empty(), "{:?}", p.shown);
        }

        #[test]
        fn remove_declined_or_interrupted_changes_nothing() {
            for interrupt in [None, Some(0)] {
                let b = boxed("remove-decline");
                b.manager();
                b.install_timer_files();
                let before = b.snapshot();
                let mut p = Scripted {
                    interrupt_at: interrupt,
                    ..Scripted::default()
                };
                match remove_run(&b, &mut p) {
                    Err(Failure::Refused(message)) => {
                        assert_eq!(message, "setup cancelled; nothing changed");
                    }
                    _ => panic!("expected a refusal"),
                }
                assert!(
                    p.shown.iter().any(|s| s.contains("Remove the index timer")),
                    "{:?}",
                    p.shown
                );
                assert_eq!(b.snapshot(), before);
            }
        }

        #[test]
        fn remove_confirmed_removes_the_timer() {
            let b = boxed("remove-yes");
            b.manager();
            b.install_timer_files();
            let mut p = Scripted {
                confirms: vec![("Remove these?", true)],
                ..Scripted::default()
            };
            let outcome = remove_run(&b, &mut p).ok().unwrap();
            assert!(
                outcome.lines.contains(&"timer removed".to_string()),
                "{:?}",
                outcome.lines
            );
            assert!(b.timer_files().iter().all(|f| !f.exists()));
        }
    }
}
