//! Carrying out a plan, one step at a time, into a `Report`.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use super::Outcome;
use super::plan::{
    ConfigPlan, EmbedderPlan, KeyPlan, Plan, PluginPlan, TimerPlan, TimerRemoval, plugins,
};
use super::syncing;
use crate::host::terminal::Step;
use crate::host::{agents, command, timer};
use crate::shared::config::{self, Token};

#[derive(Default)]
pub struct Report {
    pub lines: Vec<String>,
    pub steps: Vec<Step>,
    pub failed: bool,
}

impl Report {
    pub fn line(&mut self, step: &str, status: &str, detail: Option<String>) {
        self.failed |= status == "failed";
        self.lines.push(match &detail {
            Some(detail) => format!("{step} {status}: {detail}"),
            None => format!("{step} {status}"),
        });
        self.steps.push(Step {
            step: step.to_string(),
            status: status.to_string(),
            detail,
        });
    }
}

pub fn apply(plan: &Plan) -> Outcome {
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
    watch_step(&plan.watch, &plan.notes, &mut report);
    sync_step(plan, &mut report);
    Outcome {
        lines: report.lines,
        steps: report.steps,
        failed: report.failed,
    }
}

fn sync_step(plan: &Plan, report: &mut Report) {
    let (status, detail) = syncing::step(&plan.sync, &plan.notes);
    report.line("sync", status, Some(detail));
}

fn timer_step(plan: &TimerPlan, report: &mut Report) {
    job_step("timer", timer::Name::Index, plan, report, |job| {
        format!("every {} min", job.minutes())
    });
}

fn watch_step(plan: &TimerPlan, notes: &Path, report: &mut Report) {
    job_step("watch", timer::Name::Watch, plan, report, |_| {
        format!("watching {}", notes.display())
    });
}

/// A background job's step; `detail` is what an install says about the job.
fn job_step(
    step: &str,
    name: timer::Name,
    plan: &TimerPlan,
    report: &mut Report,
    detail: impl Fn(&timer::Job) -> String,
) {
    match plan {
        TimerPlan::Skipped(reason) => report.line(step, "skipped", Some((*reason).into())),
        TimerPlan::Failed(message) => report.line(step, "failed", Some(message.clone())),
        TimerPlan::KeyVariable(name) => report.line(
            step,
            "failed",
            Some(format!(
                "the index timer cannot read the key variable {name}; keep the key in a file (--embedder-token-file) or pass --no-timer"
            )),
        ),
        TimerPlan::Keep => report.line(step, "kept", None),
        TimerPlan::Remove {
            reason,
            platform,
            tool,
            place,
        } => match timer::uninstall(*platform, &command::System, tool.as_deref(), place, name) {
            Ok(()) => report.line(step, "removed", Some((*reason).into())),
            Err(message) => report.line(step, "failed", Some(message)),
        },
        TimerPlan::Install {
            platform,
            tool,
            job,
            files,
            update,
        } => match timer::install(*platform, &command::System, tool, job, files) {
            Ok(()) => report.line(
                step,
                if *update { "updated" } else { "installed" },
                Some(detail(job)),
            ),
            Err(message) => report.line(step, "failed", Some(message)),
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
pub fn short(message: &str) -> String {
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
    let mut settings: Vec<(String, String)> = embedder
        .as_ref()
        .map(config::settings)
        .unwrap_or_default()
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    settings.extend(plan.kept.iter().cloned());
    let header = format!(
        "# bilbo config, written by bilbo setup {} on {}",
        env!("CARGO_PKG_VERSION"),
        jiff::Zoned::now().date()
    );
    let text = config::render(&header, &settings);
    match config::write_config(&plan.config_path, &text, backup) {
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
    use super::super::fakes::*;
    use super::super::plan::summary;
    use super::*;
    use zeroize::Zeroizing;

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
}
