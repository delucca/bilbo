//! `bilbo setup --remove`: its plan, summary and steps.

use std::path::PathBuf;

use super::Outcome;
use super::apply::{Report, short};
use super::facts::{TimerFacts, timer_facts, timer_place};
use super::flags::Flags;
use super::plan::{TimerRemoval, installed_service};
use super::wizard::{self, declined, stopped};
use crate::Failure;
use crate::host::prompt::Prompter;
use crate::host::{agents, command, model, timer};
use crate::shared::config::{self, Token};
use crate::shared::store;

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

enum ToolRemoval {
    Skipped(&'static str),
    /// The tool's lists could not be read.
    Failed(String),
    Run {
        program: PathBuf,
        commands: Vec<Vec<String>>,
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
    watch: TimerRemoval,
}

pub fn remove<P: Prompter>(
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
    let timer = job_removal(&facts, timer::Name::Index);
    let watch = job_removal(&facts, timer::Name::Watch);
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
        watch,
    }
}

/// The job's removal: its files are installed, or it is skipped.
fn job_removal(facts: &TimerFacts, name: timer::Name) -> TimerRemoval {
    let Some(platform) = facts.platform else {
        return TimerRemoval::Skipped("unsupported platform");
    };
    match timer_place(facts).filter(|place| timer::installed(platform, place, name)) {
        Some(place) => TimerRemoval::Run {
            platform,
            tool: facts.tool.clone(),
            place,
        },
        None => TimerRemoval::Skipped("not installed"),
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
    if matches!(plan.watch, TimerRemoval::Run { .. }) {
        lines.push("Remove the note watcher".to_string());
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
    match &plan.watch {
        TimerRemoval::Skipped(reason) => report.line("watch", "skipped", Some((*reason).into())),
        TimerRemoval::Run {
            platform,
            tool,
            place,
        } => match timer::uninstall(
            *platform,
            &command::System,
            tool.as_deref(),
            place,
            timer::Name::Watch,
        ) {
            Ok(()) => report.line("watch", "removed", None),
            Err(message) => report.line("watch", "failed", Some(message)),
        },
    }
    Outcome {
        lines: report.lines,
        failed: report.failed,
    }
}
