//! Setup's plan: what each step will do, built from the flags or the wizard's answers, and its
//! summary.

use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

use super::facts::{ConfigState, Facts, TimerFacts, timer_place};
use super::flags::Flags;
use super::local::{Line, LocalLines, LocalPlan, Outside, check_embedder, kept_lines, plan_local};
use super::syncing::{self, SyncPlan, Turn};
use super::wizard;
use crate::Failure;
use crate::host::{agents, command, model, timer};
use crate::shared::config::{self, Embedder, Token};

const DEFAULT_MINUTES: u32 = 15;

pub enum ConfigPlan {
    Create(Option<Embedder>),
    Update(Option<Embedder>),
    Keep,
    Managed(String),
}

pub enum KeyPlan {
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

pub enum EmbedderPlan {
    None,
    Kept,
    Checked(usize),
}

pub enum PluginPlan {
    Skipped(&'static str),
    /// The tool's lists could not be read; the message is the step's failure.
    Unreadable(String),
    Run {
        program: PathBuf,
        change: agents::Change,
    },
}

/// A job's plan: the index timer, or the watcher, which never gets `KeyVariable`.
pub enum TimerPlan {
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

pub enum TimerRemoval {
    Skipped(&'static str),
    Run {
        platform: timer::Platform,
        tool: Option<PathBuf>,
        place: timer::Place,
    },
}

pub struct Plan {
    pub notes: PathBuf,
    pub store_exists: bool,
    pub config_path: PathBuf,
    pub config: ConfigPlan,
    /// The embedder the config holds once setup is done.
    pub embedder: Option<Embedder>,
    /// The digest, history and scope lines a rewritten config keeps.
    pub kept: Vec<(String, String)>,
    pub key: KeyPlan,
    /// The key the wizard was given; written to the token file and never shown.
    pub pasted: Option<Zeroizing<String>>,
    pub check: EmbedderPlan,
    /// What the local embedder needs; `None` when it is not asked for.
    pub local: Option<LocalPlan>,
    /// The model and server lines; `None` when the local embedder is not asked for.
    pub local_lines: Option<LocalLines>,
    /// An installed embedder service the config setup leaves has no use for.
    pub unused_service: Option<TimerRemoval>,
    pub source: agents::Source,
    pub claude: PluginPlan,
    pub codex: PluginPlan,
    pub timer: TimerPlan,
    pub watch: TimerPlan,
    pub sync: SyncPlan,
}

/// Non-interactive answers: the flags over the config that is there, the embedder checked when a new config gets one.
pub fn answer_batch(
    flags: &Flags,
    facts: Facts,
    outside: &mut impl Outside,
) -> Result<Plan, Failure> {
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
    let watch = plan_watch(&facts, flags.no_watch.then_some("--no-watch"));
    let sync = syncing::plan(&facts, !flags.no_watch, Turn::Unchanged);
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
        kept: facts.kept,
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
        watch,
        sync,
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
pub fn installed_service(t: &TimerFacts) -> Option<TimerRemoval> {
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
pub fn answer_wizard(facts: &Facts, answers: wizard::Answers, local: Option<LocalPlan>) -> Plan {
    let embedder = answers.embedder;
    let mut kept = facts.kept.clone();
    let mut turned_on = false;
    if let Turn::On(turned) = &answers.sync {
        let current = facts.scopes.iter().find(|(name, _)| *name == turned.name);
        turned_on = current.is_none_or(|(_, sync)| *sync != turned.url);
        syncing::put_line(&mut kept, &turned.name, &turned.url);
    }
    let config = match &facts.config {
        ConfigState::Managed { target } => ConfigPlan::Managed(target.clone()),
        ConfigState::Absent => ConfigPlan::Create(embedder.clone()),
        ConfigState::Present => {
            let unchanged = match &embedder {
                Some(planned) => same_embedder(facts.existing.as_ref(), planned),
                None => facts.existing.is_none(),
            };
            if unchanged && !turned_on {
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
    let watch = plan_watch(facts, (!answers.watch).then_some("not chosen"));
    let sync = syncing::plan(facts, answers.watch, answers.sync);
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
        kept,
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
        watch,
        sync,
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
        Err(reason) => return plan_off(t, platform, place, timer::Name::Index, reason),
    };
    let tool = match plan_tool(platform, t) {
        Ok(tool) => tool,
        Err(plan) => return *plan,
    };
    if let Some(Token::Var(name)) = &embedder.token {
        return TimerPlan::KeyVariable(name.clone());
    }
    plan_job(
        facts,
        platform,
        place,
        tool,
        timer::Kind::Periodic { minutes },
        ("index", "bilbo/index.log"),
    )
}

/// Decides the watch step. `off` is why the user wants no watcher. It needs no embedder and reads
/// no key, so a key variable does not stop it.
fn plan_watch(facts: &Facts, off: Option<&'static str>) -> TimerPlan {
    let t = &facts.timer;
    let Some(platform) = t.platform else {
        return TimerPlan::Skipped(off.unwrap_or("unsupported platform"));
    };
    let place = timer_place(t);
    if let Some(reason) = off {
        return plan_off(t, platform, place, timer::Name::Watch, reason);
    }
    let tool = match plan_tool(platform, t) {
        Ok(tool) => tool,
        Err(plan) => return *plan,
    };
    plan_job(
        facts,
        platform,
        place,
        tool,
        timer::Kind::Watch,
        ("watch", "bilbo/watch.log"),
    )
}

/// A job that is not wanted: removed when its files are installed, else skipped for `reason`.
fn plan_off(
    t: &TimerFacts,
    platform: timer::Platform,
    place: Option<timer::Place>,
    name: timer::Name,
    reason: &'static str,
) -> TimerPlan {
    match place.filter(|place| timer::installed(platform, place, name)) {
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
    }
}

/// The service manager's tool, or the plan when there is none to use.
fn plan_tool(platform: timer::Platform, t: &TimerFacts) -> Result<PathBuf, Box<TimerPlan>> {
    match (platform, &t.tool) {
        (timer::Platform::Launchd, None) => Err(Box::new(TimerPlan::Failed(
            "launchctl not found on PATH".into(),
        ))),
        (timer::Platform::Systemd, Some(tool)) if !timer::session(&command::System, tool) => {
            Err(Box::new(TimerPlan::Skipped("no systemd user session")))
        }
        (timer::Platform::Systemd, None) => {
            Err(Box::new(TimerPlan::Skipped("no systemd user session")))
        }
        (_, Some(tool)) => Ok(tool.clone()),
    }
}

/// The job `bilbo <verb>` with its files, as keep or install; `log` is under the state folder.
fn plan_job(
    facts: &Facts,
    platform: timer::Platform,
    place: Option<timer::Place>,
    tool: PathBuf,
    kind: timer::Kind,
    (verb, log): (&str, &str),
) -> TimerPlan {
    let t = &facts.timer;
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
        kind,
        program: facts.exe.clone(),
        args: vec![verb.into()],
        log: state_dir.join(log),
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
pub fn summary(plan: &Plan) -> Vec<String> {
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
    match &plan.watch {
        TimerPlan::Skipped("--no-watch" | "not chosen") => {}
        TimerPlan::Skipped(reason) => lines.push(format!("Watcher: skipped, {reason}")),
        TimerPlan::Failed(message) => lines.push(format!("Watcher: failed, {message}")),
        TimerPlan::KeyVariable(_) => {}
        TimerPlan::Remove { .. } => lines.push("Remove the note watcher".to_string()),
        TimerPlan::Keep => lines.push("Keep the note watcher".to_string()),
        TimerPlan::Install { platform, .. } => lines.push(format!(
            "Record note history in the background ({})",
            match platform {
                timer::Platform::Launchd => format!("launchd agent {}", timer::WATCH_LABEL),
                timer::Platform::Systemd =>
                    format!("systemd user service {}", timer::Name::Watch.unit()),
            }
        )),
    }
    if let Turn::On(turned) = &plan.sync.choice {
        lines.push(format!("Sync {} through {}", turned.name, turned.url));
        if !turned.folder.exists() {
            lines.push(format!("Create the folder {}", turned.folder.display()));
        }
        let keys = plan.sync.keys.as_deref().map(|keys| keys.display());
        match (&turned.enrol, keys) {
            (syncing::Enrol::New { .. }, Some(keys)) => {
                lines.push(format!("Create the device keys in {keys}"));
            }
            (syncing::Enrol::Phrase { .. }, Some(keys)) => {
                lines.push(format!(
                    "Create the device keys in {keys} from your recovery phrase"
                ));
            }
            _ => {}
        }
        if turned.take.is_some() {
            lines.push(format!(
                "Copy the manifest of {} from the folder, and add this device to it",
                turned.name
            ));
        } else if turned.mint {
            lines.push(format!("Create the scope {}", turned.name));
        } else if matches!(turned.enrol, syncing::Enrol::Phrase { .. }) {
            lines.push(format!("Add this device to the scope {}", turned.name));
        }
    }
    lines
}

pub fn plugins(plan: &Plan) -> [(agents::Tool, &PluginPlan); 2] {
    [
        (agents::Tool::Claude, &plan.claude),
        (agents::Tool::Codex, &plan.codex),
    ]
}

#[cfg(test)]
mod tests {
    use super::super::fakes::*;
    use super::*;

    fn answered(embedder: Option<Embedder>, pasted: Option<&str>) -> wizard::Answers {
        wizard::Answers {
            dims: embedder.as_ref().map(|_| 1024),
            embedder,
            pasted: pasted.map(|key| Zeroizing::new(key.to_string())),
            claude: true,
            codex: true,
            timer: None,
            watch: true,
            local: None,
            sync: Turn::Unchanged,
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
    fn summary_names_the_watcher() {
        let mut p = plan(ConfigPlan::Keep, None, EmbedderPlan::None);
        let platform = timer::Platform::Launchd;
        p.watch = TimerPlan::Install {
            platform,
            tool: "/bin/launchctl".into(),
            job: timer::Job {
                kind: timer::Kind::Watch,
                program: "/bin/bilbo".into(),
                args: vec!["watch".into()],
                log: "/s/bilbo/watch.log".into(),
                env: Vec::new(),
            },
            files: Vec::new(),
            update: false,
        };
        assert_eq!(
            summary(&p).last().unwrap(),
            "Record note history in the background (launchd agent io.github.delucca.bilbo.watch)"
        );
        p.watch = TimerPlan::Keep;
        assert_eq!(summary(&p).last().unwrap(), "Keep the note watcher");
        p.watch = TimerPlan::Skipped("--no-watch");
        assert_eq!(summary(&p).last().unwrap(), "Search by keywords only");
        p.watch = TimerPlan::Skipped("no systemd user session");
        assert_eq!(
            summary(&p).last().unwrap(),
            "Watcher: skipped, no systemd user session"
        );
    }

    #[test]
    fn the_watcher_needs_no_embedder_and_ignores_a_key_variable() {
        let dir = scratch("plan-watch");
        let mut facts = seen(&dir, None, ConfigState::Absent);
        facts.timer.platform = Some(timer::Platform::Launchd);
        facts.timer.home = Some(dir.join("home"));
        facts.timer.state_dir = Some(dir.join("state"));
        facts.timer.tool = Some("/bin/launchctl".into());
        let planned = plan_watch(&facts, None);
        let TimerPlan::Install { job, update, .. } = planned else {
            panic!("expected an install");
        };
        assert!(!update);
        assert_eq!(job.kind, timer::Kind::Watch);
        assert_eq!(job.args, ["watch"]);
        assert_eq!(job.log, dir.join("state/bilbo/watch.log"));
        let mut keyed = embedder("m");
        keyed.token = Some(Token::Var("OPENAI_API_KEY".into()));
        assert!(matches!(
            plan_timer(&facts, Some(&keyed), None, 15),
            TimerPlan::KeyVariable(_)
        ));
        assert!(matches!(
            plan_watch(&facts, Some("--no-watch")),
            TimerPlan::Skipped("--no-watch")
        ));
        facts.timer.tool = None;
        assert!(matches!(
            plan_watch(&facts, None),
            TimerPlan::Failed(m) if m == "launchctl not found on PATH"
        ));
        let _ = std::fs::remove_dir_all(&dir);
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
}
