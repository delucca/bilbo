//! `bilbo setup`: the flags or the wizard's answers, the plan they make, applying it, the local embedder, the sync
//! step, and `--remove`.

mod apply;
#[cfg(test)]
mod driven;
mod facts;
#[cfg(test)]
mod fakes;
mod flags;
mod local;
mod plan;
mod remove;
mod syncing;
mod wizard;

use std::path::Path;
use std::time::Duration;

use crate::Failure;
use crate::host::prompt::{self, Prompter};
use crate::host::{command, model, timer};
use crate::search::{documents, embed};
use crate::shared::config::Embedder;
use crate::shared::store;
use apply::{Report, apply};
use facts::{ConfigState, Facts, gather, timer_place};
use flags::{Flags, Mode, settle};
use local::{
    CHECK, Line, LocalLines, Net, Outside, check_embedder, check_kept, local_unavailable,
    plan_local, prepare, vector_length,
};
use plan::{ConfigPlan, EmbedderPlan, Plan, answer_batch, answer_wizard, summary};
use remove::remove;
use wizard::{declined, stopped};

const OLLAMA: &str = "http://localhost:11434";

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
        scopes: facts.scopes.clone(),
    };
    let mut answers = wizard::ask(p, &seen, check, |name| {
        std::env::var_os(name).is_some_and(|value| !value.is_empty())
    })
    .map_err(|e| stopped(p, e))?;
    if seen.managed.is_none() {
        let sync = wizard::SyncFacts {
            root: facts.root.clone(),
            keys: facts.keys.clone(),
            home: facts.timer.home.clone(),
            agent: facts.agent,
            host: facts.host.clone(),
            scopes: facts.scopes.clone(),
        };
        answers.sync = wizard::ask_sync(p, &sync, answers.watch)?;
    }
    let exe = facts.exe.clone();
    let kept = (answers.claude, answers.codex, answers.watch);
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
    settle_local(p, outside, &facts, &mut plan, kept)?;
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
/// would name it), offering a retry or keyword search only when that fails. `kept` are the
/// wizard's claude, codex and watch answers, which the keyword-only plan keeps.
fn settle_local<P: Prompter>(
    p: &mut P,
    outside: &mut impl Outside,
    facts: &Facts,
    plan: &mut Plan,
    kept: (bool, bool, bool),
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
                    claude: kept.0,
                    codex: kept.1,
                    timer: None,
                    watch: kept.2,
                    sync: std::mem::take(&mut plan.sync.choice),
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

/// `bilbo setup --help`; its Usage block is also the synopsis a usage error shows.
pub const HELP: &str = r#"bilbo setup: create the store and config, and install the agent plugin, the
index timer, the note watcher and, when asked, the local embedder.

Usage:
  bilbo setup [--yes | --interactive] [--remove] [<option>]...

With stdin and stderr on a terminal, setup runs a wizard, unless --yes or an
option that answers a question is given (every option below but --remove,
--claude, --codex and --plugin-source). Otherwise it asks nothing and each
missing answer takes its default. Run it again at any time.

Answers:
  --yes                           Ask nothing, even in a terminal
  --interactive                   Run the wizard; needs a terminal
  --remove                        Unload the services and remove the plugin;
                                  keep the store, config, key and model.
                                  Takes only --yes, --interactive, --claude
                                  and --codex
Embedder:
  --embedder-url <url>            An OpenAI-compatible embedder; needs
                                  --embedder-model
  --embedder-model <name>         The model to ask it for
  --embedder-token-env <var>      Read its key from this variable
  --embedder-token-file <path>    Read its key from this file, absolute or
                                  under ~/
  --embedder-query-prefix <text>  Text put before each query
  --embedder-local                Download a model and run it as a service
  --embedder-port <port>          Its port on 127.0.0.1 (default 8737)
  --llama-server <path>           The llama-server it runs (default: on PATH)
Plugin:
  --no-plugin                     Skip the Claude Code and Codex plugin
  --claude <path>                 The claude to use (default: on PATH)
  --codex <path>                  The codex to use (default: on PATH)
  --plugin-source <source>        Install from a folder or owner/repo#ref
Services:
  --no-timer                      Skip the index timer
  --index-every <minutes>         Timer interval, 1 to 1440 (default 15)
  --no-watch                      Skip the watcher; remove it if installed

Output: one line per step, in this order: store, config, key, model, server,
embedder, claude, codex, hook, timer, watch, sync. A line is
  <step> <status>[: <detail>]
where status is created, written, kept, ok, installed, updated, removed,
skipped or failed.

Exit: 0 no step failed; 1 a step failed; 2 usage or config error.

Examples:
  bilbo setup
  bilbo setup --yes --embedder-local
  bilbo setup --yes --no-plugin --no-timer --no-watch
  bilbo setup --remove

Docs: https://github.com/delucca/bilbo/wiki/Commands#setup
"#;

#[cfg(test)]
mod tests {
    use super::fakes::*;
    use super::*;

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
}
