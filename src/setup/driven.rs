//! End-to-end runs of setup's wizard against a scripted terminal.

use std::cell::Cell;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use super::facts::gather;
use super::fakes::*;
use super::flags::{Flags, Mode, settle};
use super::plan::{EmbedderPlan, answer_batch};
use super::remove::remove;
use super::wizard::stopped;
use super::*;
use crate::Failure;
use crate::host::prompt::{self, Prompter};
use crate::host::{model, timer};
use crate::shared::{config, store};

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
        claudecode: None,
        codex_thread_id: None,
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
        write_script(&self.bin.join(name), "#!/bin/sh\nexit 0\n");
    }
    /// The service manager: on Linux a `systemctl` whose user session is live.
    fn manager(&self) -> &'static str {
        if cfg!(target_os = "macos") {
            self.tool("launchctl");
            "launchctl"
        } else {
            write_script(&self.bin.join("systemctl"), LIVE_MANAGER);
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
        for file in self.timer_files().into_iter().chain(self.watch_files()) {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, "old\n").unwrap();
        }
    }
    fn watch_files(&self) -> Vec<PathBuf> {
        timer::paths(
            timer::platform().unwrap(),
            &self.place(),
            timer::Name::Watch,
        )
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
    b.manager();
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
fn the_wizard_keeps_the_history_setting() {
    let b = boxed("keep-history");
    b.write_config("a");
    let mut text = std::fs::read_to_string(b.config()).unwrap();
    text.push_str("history.keep_days = 30\n");
    std::fs::write(b.config(), text).unwrap();
    let mut p = Scripted {
        inputs: vec![("Model name", "b")],
        selects: vec![NO_TIMER],
        ..Scripted::default()
    };
    let run = wizard_run(&b, &mut p);
    assert!(run.result.is_ok());
    let text = std::fs::read_to_string(b.config()).unwrap();
    assert!(text.contains("embedder.model = b"), "{text}");
    assert!(text.contains("history.keep_days = 30"), "{text}");
}

#[test]
fn the_wizard_keeps_the_scope_settings() {
    let b = boxed("keep-scope");
    b.write_config("a");
    let lines = [
        "digest.log = on",
        "history.keep_days = 30",
        "scope.work.paths = ~/Developer/acme, /srv/acme",
        "scope.work.marks = \"  acme, beta\"",
        "scope.work.embedder = local",
        "scope.home.sync = off",
        "scope.default = home",
    ];
    let mut text = std::fs::read_to_string(b.config()).unwrap();
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    std::fs::write(b.config(), text).unwrap();
    let before = config::load(&b.env).unwrap();
    let mut p = Scripted {
        inputs: vec![("Model name", "b")],
        selects: vec![NO_TIMER],
        ..Scripted::default()
    };
    let run = wizard_run(&b, &mut p);
    assert!(run.result.is_ok());
    let text = std::fs::read_to_string(b.config()).unwrap();
    assert!(text.contains("embedder.model = b"), "{text}");
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("embedder."))
        .collect();
    assert_eq!(
        kept,
        [
            "digest.log = on",
            "history.keep_days = 30",
            "scope.work.paths = ~/Developer/acme, /srv/acme",
            "scope.work.marks = \"  acme, beta\"",
            "scope.work.embedder = local",
            "scope.home.sync = off",
            "scope.default = home",
        ],
        "{text}"
    );
    let after = config::load(&b.env).unwrap();
    assert_eq!(after.scope_lines, before.scope_lines);
    assert_eq!(after.scope_names(), before.scope_names());
}

#[test]
fn the_wizard_asks_about_the_watcher_and_installs_it_by_default() {
    let b = boxed("watch-default");
    b.manager();
    let mut p = Scripted::default();
    let run = wizard_run(&b, &mut p);
    let notes = b.home().join(".local/share/bilbo/notes");
    assert!(
        report(&run).contains(&format!("watch installed: watching {}", notes.display())),
        "{:?}",
        report(&run)
    );
    assert!(b.watch_files().iter().all(|f| f.exists()));
    assert!(
        p.shown
            .contains(&"Record note history in the background?".to_string())
    );
    let summary = p
        .shown
        .iter()
        .find(|s| s.starts_with("note: Setup will"))
        .unwrap();
    assert!(
        summary.contains("Record note history in the background ("),
        "{summary}"
    );
}

#[test]
fn declining_the_watcher_removes_the_old_one() {
    let b = boxed("watch-declined");
    let manager = b.manager();
    b.install_timer_files();
    let mut p = Scripted {
        confirms: vec![("Record note history", false)],
        ..Scripted::default()
    };
    let run = wizard_run(&b, &mut p);
    assert!(
        report(&run).contains(&"watch removed: not chosen".to_string()),
        "{manager}: {:?}",
        report(&run)
    );
    assert!(b.watch_files().iter().all(|f| !f.exists()));
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
    assert!(
        outcome.lines.contains(&"watch removed".to_string()),
        "{:?}",
        outcome.lines
    );
    assert!(b.watch_files().iter().all(|f| !f.exists()));
}
