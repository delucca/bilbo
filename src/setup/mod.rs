//! The setup verb.

mod facts;
#[cfg(test)]
mod fakes;
mod flags;
mod local;
mod wizard;

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::Zeroizing;

use crate::Failure;
use crate::host::prompt::{self, Prompter};
use crate::host::{agents, command, model, timer};
use crate::search::{documents, embed};
use crate::shared::config::{self, Embedder, Token};
use crate::shared::store;
use facts::{ConfigState, Facts, TimerFacts, gather, timer_facts, timer_place};
use flags::{Flags, Mode, settle};
use local::{
    CHECK, Line, LocalLines, LocalPlan, Net, Outside, check_embedder, check_kept, kept_lines,
    local_unavailable, plan_local, prepare, vector_length,
};
use wizard::{declined, stopped};

const OLLAMA: &str = "http://localhost:11434";
const DEFAULT_MINUTES: u32 = 15;

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
        Mode::Remove => return remove(&flags, env, path, &mut prompt::Terminal),
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
        &mut prompt::Terminal,
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
        let notes = documents::read_notes(&plan.notes).map_or(0, |notes| notes.len());
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

// ---------------------------------------------------------------------------
// Flags

// ---------------------------------------------------------------------------
// Gather

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
    use super::fakes::*;
    use super::*;

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
                choices: &[prompt::Choice],
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
                _: &[prompt::Choice],
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
