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
    /// Answers to prompts that hold the text, each used once, before `inputs`.
    entered: Vec<(&'static str, String)>,
    /// The words typed at `Word 1` to `Word 12`.
    typed: Vec<String>,
    /// `Word n` is answered from the last recovery phrase shown.
    reads_phrase: bool,
}

impl Scripted {
    /// The 12 words of the last recovery phrase shown.
    fn phrase(&self) -> Option<Vec<String>> {
        let body = self
            .shown
            .iter()
            .rev()
            .find_map(|s| s.strip_prefix("note: Recovery phrase\n"))?;
        let mut words = vec![String::new(); 12];
        for line in body.lines().take_while(|l| !l.is_empty()) {
            let mut tokens = line.split_whitespace();
            while let (Some(n), Some(word)) = (tokens.next(), tokens.next()) {
                let n: usize = n.trim_end_matches('.').parse().ok()?;
                *words.get_mut(n.checked_sub(1)?)? = word.to_string();
            }
        }
        Some(words)
    }

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
        if let Some(at) = self
            .entered
            .iter()
            .position(|(text, _)| prompt.contains(text))
        {
            return Ok(self.entered.remove(at).1);
        }
        if let Some(n) = prompt
            .strip_prefix("Word ")
            .and_then(|n| n.parse::<usize>().ok())
        {
            let word = match self.typed.get(n - 1) {
                Some(word) => Some(word.clone()),
                None if self.reads_phrase => self.phrase().map(|words| words[n - 1].clone()),
                None => None,
            };
            if let Some(word) = word {
                return Ok(word);
            }
        }
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
    wizard_as(b, p, outside, None)
}

/// The wizard on a machine whose host name is `host`, when given.
fn wizard_as(b: &Sandbox, p: &mut Scripted, outside: &mut Script, host: Option<&str>) -> Run {
    let probed = Cell::new(false);
    let checked = Cell::new(0);
    let indexed = Cell::new(false);
    let flags = flags(&["--yes"], Mode::Wizard, &b.env);
    let result = gather(&flags, &b.env, b.path()).and_then(|mut facts| {
        if let Some(host) = host {
            facts.host = Some(host.to_string());
        }
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

fn outcome(run: &Run) -> &Outcome {
    run.result.as_ref().ok().expect("an outcome")
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

// ---------------------------------------------------------------------------
// The sync questions and what applying them writes

use crate::identity::keys::{self, Identity};
use crate::identity::manifest;
use crate::identity::phrase;

const SYNC_ON: (&str, bool) = ("Sync notes between", true);
const WRITTEN: (&str, bool) = ("Written down?", true);
const MATCHES: (&str, bool) = ("Does it match", true);

impl Sandbox {
    fn keys(&self) -> PathBuf {
        self.home().join(".local/state/bilbo/keys")
    }
    fn store(&self) -> PathBuf {
        self.home().join(".local/share/bilbo")
    }
    fn folder(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
    fn identity(&self) -> Option<Identity> {
        keys::read_identity(&self.keys()).unwrap()
    }
    /// Every scope the store holds, as `survey` reads it for this box's device.
    fn known(&self) -> Vec<manifest::Known> {
        let id = self.identity().unwrap();
        let public = id.owner.sign.public();
        manifest::survey(
            &self.store(),
            Some(&public),
            Some(&manifest::Recipient::device(&id.device)),
        )
        .unwrap()
    }
    fn scope_ids(&self) -> Vec<String> {
        manifest::scope_ids(&self.store()).unwrap()
    }
    /// The text of the config.
    fn config_text(&self) -> String {
        std::fs::read_to_string(self.config()).unwrap_or_default()
    }
}

/// Copies each scope's versions from `from`'s store into `folder`, as a watcher would push them.
fn publish(from: &Sandbox, folder: &Path) {
    let scopes = from.store().join(".bilbo/scopes");
    for id in from.scope_ids() {
        let target = folder.join("scopes").join(&id).join("manifest");
        std::fs::create_dir_all(&target).unwrap();
        for entry in std::fs::read_dir(scopes.join(&id).join("manifest")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "json") {
                std::fs::copy(&path, target.join(path.file_name().unwrap())).unwrap();
            }
        }
    }
}

/// The answers of a person with no key who sets `scope` up through `folder`: no phrase yet, a new one written down.
fn new_owner(folder: &Path) -> Scripted {
    Scripted {
        confirms: vec![
            SYNC_ON,
            WRITTEN,
            ("recovery phrase from another", false),
            ("Apply these changes?", true),
        ],
        entered: vec![("Folder or relay URL to sync", folder.display().to_string())],
        reads_phrase: true,
        ..Scripted::default()
    }
}

fn ok_line(name: &str, folder: &Path, notes: usize) -> String {
    format!(
        "sync ok: {name} through file://{} ({notes} notes)",
        folder.display()
    )
}

#[test]
fn a_first_device_syncs_through_a_new_folder() {
    let b = boxed("sync-first");
    b.manager();
    let folder = b.folder("bilbo-sync");
    let mut p = new_owner(&folder);
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert!(
        report(&run).contains(&ok_line("personal", &folder, 0)),
        "{:?}",
        report(&run)
    );
    assert!(!outcome(&run).failed);
    assert!(folder.is_dir());
    assert!(b.identity().is_some());
    assert!(
        b.config_text().contains(&format!(
            "scope.personal.sync = file://{}\n",
            folder.display()
        )),
        "{}",
        b.config_text()
    );
    let known = b.known();
    assert_eq!(known.len(), 1);
    assert_eq!(known[0].name(), Some("personal"));
    let summary = p
        .shown
        .iter()
        .find(|s| s.starts_with("note: Setup will"))
        .unwrap();
    assert!(
        summary.contains(&format!(
            "Sync personal through file://{}",
            folder.display()
        )),
        "{summary}"
    );
    assert!(summary.contains(&format!("Create the folder {}", folder.display())));
    assert!(summary.contains("Create the device keys in"));
    assert!(summary.contains("Create the scope personal"));
    assert!(
        !summary.contains("Owner fingerprint"),
        "the phrase is not in the summary"
    );
    assert!(
        p.shown
            .iter()
            .any(|s| s.starts_with("note: Recovery phrase\n"))
    );
}

#[test]
fn the_phrase_is_asked_for_three_words_and_never_reaches_the_summary() {
    let b = boxed("sync-first-words");
    b.manager();
    let mut p = new_owner(&b.folder("f"));
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert!(run.result.is_ok());
    let words = p.phrase().unwrap();
    let asked = p.shown.iter().filter(|s| s.starts_with("Word ")).count();
    assert_eq!(asked, 3, "three words are asked");
    let summary = p
        .shown
        .iter()
        .find(|s| s.starts_with("note: Setup will"))
        .unwrap();
    assert!(!summary.contains("Owner fingerprint"), "{summary}");
    for (i, word) in words.iter().enumerate() {
        assert!(
            !summary.contains(&format!("{}. {word}", i + 1)),
            "{word} in {summary}"
        );
    }
}

#[test]
fn declining_after_the_phrase_writes_no_key_no_folder_and_no_config() {
    let b = boxed("sync-declined");
    b.manager();
    let before = b.snapshot();
    let folder = b.folder("never");
    let mut p = new_owner(&folder);
    p.confirms
        .retain(|(text, _)| *text != "Apply these changes?");
    p.confirms.push(("Apply these changes?", false));
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert_eq!(refused(run), "setup cancelled; nothing changed");
    assert!(
        p.shown
            .iter()
            .any(|s| s.starts_with("note: Recovery phrase\n"))
    );
    assert_eq!(b.snapshot(), before);
    assert!(b.identity().is_none());
    assert!(!folder.exists());
    assert!(!b.config().exists());
}

#[test]
fn interrupting_at_the_summary_after_the_phrase_writes_nothing() {
    let b = boxed("sync-interrupt");
    b.manager();
    let before = b.snapshot();
    let folder = b.folder("never");
    let mut counting = new_owner(&folder);
    let counted = wizard_as(&b, &mut counting, &mut Script::working(), Some("rivendell"));
    assert!(counted.result.is_ok());
    let last = counting.at_confirm.unwrap();
    let b = boxed("sync-interrupt-run");
    b.manager();
    let mut p = new_owner(&folder);
    p.interrupt_at = Some(last);
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert_eq!(refused(run), "setup cancelled; nothing changed");
    assert!(b.identity().is_none());
    assert!(!b.config().exists());
    let _ = before;
}

#[test]
fn a_second_device_joins_the_scope_with_the_first_ones_phrase() {
    let first = boxed("sync-second-a");
    first.manager();
    let folder = first.folder("shared-folder");
    let mut p = new_owner(&folder);
    assert!(
        wizard_as(&first, &mut p, &mut Script::working(), Some("rivendell"))
            .result
            .is_ok()
    );
    publish(&first, &folder);
    let words = p.phrase().unwrap();

    let second = boxed("sync-second-b");
    second.manager();
    let mut q = Scripted {
        confirms: vec![
            SYNC_ON,
            ("recovery phrase from another", true),
            MATCHES,
            ("Apply these changes?", true),
        ],
        entered: vec![("Folder or relay URL to sync", folder.display().to_string())],
        typed: words,
        ..Scripted::default()
    };
    let run = wizard_as(&second, &mut q, &mut Script::working(), Some("bagend"));
    assert!(
        report(&run).contains(&ok_line("personal", &folder, 0)),
        "{:?}",
        report(&run)
    );
    let a = first.identity().unwrap();
    let b = second.identity().unwrap();
    assert_eq!(a.owner.sign.public(), b.owner.sign.public());
    assert_ne!(a.device.id(), b.device.id());
    assert_eq!(second.scope_ids(), first.scope_ids());
    let known = second.known();
    let latest = known[0].scope.latest().unwrap();
    assert!(latest.manifest.lists(&a.device.id()));
    assert!(latest.manifest.lists(&b.device.id()));
    let summary = q
        .shown
        .iter()
        .find(|s| s.starts_with("note: Setup will"))
        .unwrap();
    assert!(
        summary.contains("Copy the manifest of personal from the folder"),
        "{summary}"
    );
    assert!(!summary.contains("Create the scope"), "{summary}");
}

#[test]
fn only_the_picked_scope_is_copied_and_joined() {
    let first = boxed("sync-only-a");
    first.manager();
    let folder = first.folder("f");
    let mut p = new_owner(&folder);
    assert!(
        wizard_as(&first, &mut p, &mut Script::working(), Some("rivendell"))
            .result
            .is_ok()
    );
    let mut text = first.config_text();
    text.push_str("scope.shared.sync = off\n");
    std::fs::write(first.config(), text).unwrap();
    let mut again = Scripted {
        confirms: vec![SYNC_ON, ("Apply these changes?", true)],
        entered: vec![("Folder or relay URL to sync", folder.display().to_string())],
        selects: vec![("Which scope", 1)],
        ..Scripted::default()
    };
    let run = wizard_as(
        &first,
        &mut again,
        &mut Script::working(),
        Some("rivendell"),
    );
    assert!(run.result.is_ok(), "{:?}", again.shown);
    assert_eq!(first.scope_ids().len(), 2, "personal and the second scope");
    let personal = first.config_text();
    assert!(personal.contains("scope.personal.sync = "), "{personal}");
    publish(&first, &folder);

    let second = boxed("sync-only-b");
    second.manager();
    let mut q = Scripted {
        confirms: vec![
            SYNC_ON,
            ("recovery phrase from another", true),
            MATCHES,
            ("Apply these changes?", true),
        ],
        entered: vec![("Folder or relay URL to sync", folder.display().to_string())],
        typed: p.phrase().unwrap(),
        ..Scripted::default()
    };
    let run = wizard_as(&second, &mut q, &mut Script::working(), Some("bagend"));
    assert!(run.result.is_ok(), "{:?}", q.shown);
    assert_eq!(second.scope_ids().len(), 1);
    let known = second.known();
    assert_eq!(known[0].name(), Some("personal"));
    let id = second.identity().unwrap();
    for scope in first.scope_ids() {
        let listed = manifest::read_scope(&second.store(), &scope)
            .ok()
            .and_then(|s| s.latest().map(|v| v.manifest.lists(&id.device.id())));
        assert_eq!(
            listed.unwrap_or(false),
            scope == known[0].scope.id,
            "{scope}"
        );
    }
}

#[test]
fn a_phrase_of_another_owner_is_refused_against_the_stores_manifests() {
    let first = boxed("sync-owner-a");
    first.manager();
    let folder = first.folder("f");
    let mut p = new_owner(&folder);
    assert!(
        wizard_as(&first, &mut p, &mut Script::working(), Some("rivendell"))
            .result
            .is_ok()
    );
    let second = boxed("sync-owner-b");
    second.manager();
    publish(&first, &second.store().join("copy"));
    let scopes = first.store().join(".bilbo/scopes");
    let target = second.store().join(".bilbo/scopes");
    std::fs::create_dir_all(second.store().join("notes")).unwrap();
    copy_dir(&scopes, &target);
    let before = second.snapshot();
    let other: Vec<String> = (0..12)
        .map(|i| phrase::word(phrase::encode(&[0u8; 16])[i]).to_string())
        .collect();
    let mut q = Scripted {
        confirms: vec![SYNC_ON, MATCHES],
        entered: vec![("Folder or relay URL to sync", folder.display().to_string())],
        typed: other,
        ..Scripted::default()
    };
    let run = wizard_as(&second, &mut q, &mut Script::working(), Some("bagend"));
    let message = refused(run);
    assert!(message.contains("the phrase derives owner"), "{message}");
    assert!(message.ends_with("; nothing was written"), "{message}");
    assert_eq!(second.snapshot(), before);
    assert!(second.identity().is_none());
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

#[test]
fn under_an_agent_sync_stays_off_and_the_line_says_why() {
    let mut b = boxed("sync-agent");
    b.manager();
    b.env.claudecode = Some("1".into());
    let mut p = Scripted {
        confirms: vec![SYNC_ON, ("Apply these changes?", true)],
        ..Scripted::default()
    };
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert!(
        report(&run).contains(
            &"sync skipped: no device key; run bilbo device init in a terminal".to_string()
        ),
        "{:?}",
        report(&run)
    );
    assert!(!outcome(&run).failed);
    assert!(
        p.shown.iter().any(|s| s == "warn: The recovery phrase is shown only in a terminal outside an agent, so sync stays off for this run."),
        "{:?}",
        p.shown
    );
    assert!(!p.shown.iter().any(|s| s.contains("Which scope")));
    assert!(!b.config_text().contains("sync"));
    assert!(b.identity().is_none());
}

#[test]
fn a_folder_without_a_parent_is_asked_again() {
    let b = boxed("sync-parent");
    b.manager();
    let folder = b.folder("good");
    let mut p = new_owner(&folder);
    p.entered = vec![
        ("Folder or relay URL to sync", "/nope/bilbo".to_string()),
        ("Folder or relay URL to sync", folder.display().to_string()),
    ];
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert!(run.result.is_ok(), "{:?}", p.shown);
    assert!(
        p.shown
            .contains(&"warn: The parent folder /nope does not exist".to_string()),
        "{:?}",
        p.shown
    );
    assert_eq!(
        p.shown
            .iter()
            .filter(|s| s.starts_with("Folder or relay URL to sync"))
            .count(),
        2
    );
    assert!(!Path::new("/nope").exists());
}

#[test]
fn a_relative_folder_is_refused_by_the_question_and_by_the_check() {
    let b = boxed("sync-relative");
    b.manager();
    let folder = b.folder("good");
    let mut p = new_owner(&folder);
    p.entered = vec![
        ("Folder or relay URL to sync", "relative/bilbo".to_string()),
        ("Folder or relay URL to sync", folder.display().to_string()),
    ];
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert!(run.result.is_ok());
    assert!(
        p.shown
            .contains(&"warn: Enter an absolute path, or one starting with ~/".to_string()),
        "{:?}",
        p.shown
    );
}

#[test]
fn declining_sync_adds_no_setting_and_the_line_says_no_scope_syncs() {
    let b = boxed("sync-no");
    b.manager();
    let mut p = Scripted::default();
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert!(report(&run).contains(&"sync skipped: no scope syncs".to_string()));
    assert!(
        p.shown
            .contains(&"Sync notes between your devices?".to_string())
    );
    assert!(!b.config_text().contains("sync"));
    assert!(!p.shown.iter().any(|s| s.contains("Which scope")));
}

#[test]
fn the_sync_question_defaults_to_yes_only_when_a_scope_syncs() {
    let initial = |config: &str| {
        let b = boxed("sync-default");
        b.manager();
        b.write_config("a");
        let mut text = b.config_text();
        text.push_str(config);
        std::fs::write(b.config(), text).unwrap();
        let mut p = Scripted::default();
        wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
        p.shown.iter().any(|s| s == "Which scope should sync?")
    };
    let folder = std::env::temp_dir();
    assert!(!initial("scope.work.sync = off\n"));
    assert!(initial(&format!(
        "scope.work.sync = file://{}\n",
        folder.display()
    )));
}

#[test]
fn turning_sync_on_keeps_the_other_settings_and_the_old_config() {
    let b = boxed("sync-keeps");
    b.manager();
    b.write_config("a");
    let mut text = b.config_text();
    text.push_str("scope.work.sync = off\nsync.poll_seconds = 60\n");
    std::fs::write(b.config(), &text).unwrap();
    let folder = b.folder("bilbo");
    let mut p = new_owner(&folder);
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert!(run.result.is_ok(), "{:?}", p.shown);
    let new = b.config_text();
    for line in [
        "embedder.model = a",
        "scope.work.sync = off",
        "sync.poll_seconds = 60",
    ] {
        assert!(new.contains(line), "{line} in {new}");
    }
    assert!(
        new.contains(&format!(
            "scope.personal.sync = file://{}",
            folder.display()
        )),
        "{new}"
    );
    assert_eq!(
        std::fs::read_to_string(b.config().with_file_name("config.bak")).unwrap(),
        text
    );
    assert_eq!(
        report(&run)
            .iter()
            .find(|l| l.starts_with("config "))
            .map(String::as_str),
        Some(format!("config updated: {}", b.config().display()).as_str())
    );
}

#[test]
fn a_config_that_already_syncs_there_is_kept() {
    let b = boxed("sync-same");
    b.manager();
    let folder = b.folder("bilbo");
    let mut p = new_owner(&folder);
    assert!(
        wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"))
            .result
            .is_ok()
    );
    let mut again = Scripted {
        confirms: vec![SYNC_ON, ("Apply these changes?", true)],
        entered: vec![("Folder or relay URL to sync", folder.display().to_string())],
        ..Scripted::default()
    };
    let before = b.config_text();
    let run = wizard_as(&b, &mut again, &mut Script::working(), Some("rivendell"));
    assert!(
        report(&run).contains(&format!("config kept: {}", b.config().display())),
        "{:?}",
        report(&run)
    );
    assert_eq!(b.config_text(), before);
    assert!(report(&run).contains(&ok_line("personal", &folder, 0)));
}

#[test]
fn a_managed_config_shows_the_syncing_scopes_and_asks_nothing_about_sync() {
    let b = boxed("sync-managed");
    b.manager();
    let target = b.dir.join("managed-config");
    std::fs::write(
        &target,
        "scope.personal.sync = file:///srv/bilbo\nscope.work.sync = off\n",
    )
    .unwrap();
    std::fs::create_dir_all(b.config().parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, b.config()).unwrap();
    let mut p = Scripted::default();
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert!(run.result.is_ok());
    let note = p
        .shown
        .iter()
        .find(|s| s.starts_with("note: Config managed elsewhere"))
        .unwrap();
    assert!(
        note.contains("scope.personal.sync = file:///srv/bilbo"),
        "{note}"
    );
    assert!(!note.contains("scope.work"), "{note}");
    assert!(
        !p.shown
            .contains(&"Sync notes between your devices?".to_string())
    );
}

/// The fixture owner's keys and a folder holding the fixture `personal` scope, version 1 only, which lists
/// `bagend` and not `rivendell`.
fn outsider(b: &Sandbox, folder: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/device");
    copy_dir(&Path::new(fixtures).join("rivendell"), &b.keys());
    std::fs::set_permissions(b.keys(), std::fs::Permissions::from_mode(0o700)).unwrap();
    for file in ["owner.key", "device.key"] {
        std::fs::set_permissions(b.keys().join(file), std::fs::Permissions::from_mode(0o600))
            .unwrap();
    }
    let store = Path::new(fixtures).join("store/.bilbo/scopes");
    let id = std::fs::read_dir(&store)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name();
    let target = folder.join("scopes").join(&id).join("manifest");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::copy(
        store.join(&id).join("manifest/1.json"),
        target.join("1.json"),
    )
    .unwrap();
}

#[test]
fn an_enrolled_device_outside_a_scope_of_its_owner_mints_nothing() {
    let b = boxed("sync-outsider");
    b.manager();
    let folder = b.folder("held");
    outsider(&b, &folder);
    let before = snapshot_of(&folder);
    let mut p = Scripted {
        confirms: vec![SYNC_ON, ("Apply these changes?", true)],
        entered: vec![("Folder or relay URL to sync", folder.display().to_string())],
        ..Scripted::default()
    };
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    let line = format!("warn: {}", crate::setup::syncing::BLOCKED);
    assert!(p.shown.contains(&line), "{:?}", p.shown);
    assert_eq!(
        report(&run)
            .iter()
            .find(|l| l.starts_with("sync "))
            .map(String::as_str),
        Some(format!("sync skipped: {}", crate::setup::syncing::BLOCKED).as_str())
    );
    assert!(!outcome(&run).failed);
    assert!(b.scope_ids().is_empty());
    assert!(!b.config_text().contains("sync"));
    assert_eq!(snapshot_of(&folder), before);
}

fn snapshot_of(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push((
                    path.clone(),
                    std::fs::read_to_string(&path).unwrap_or_default(),
                ));
            }
        }
    }
    walk(dir, &mut out);
    out
}

#[test]
fn an_enrolled_device_with_no_keys_to_ask_for_asks_no_phrase() {
    let b = boxed("sync-enrolled");
    b.manager();
    let folder = b.folder("empty");
    std::fs::create_dir_all(&folder).unwrap();
    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/device");
    use std::os::unix::fs::PermissionsExt;
    copy_dir(&Path::new(fixtures).join("rivendell"), &b.keys());
    std::fs::set_permissions(b.keys(), std::fs::Permissions::from_mode(0o700)).unwrap();
    for file in ["owner.key", "device.key"] {
        std::fs::set_permissions(b.keys().join(file), std::fs::Permissions::from_mode(0o600))
            .unwrap();
    }
    let mut p = Scripted {
        confirms: vec![SYNC_ON, ("Apply these changes?", true)],
        entered: vec![("Folder or relay URL to sync", folder.display().to_string())],
        ..Scripted::default()
    };
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert!(
        report(&run).contains(&ok_line("personal", &folder, 0)),
        "{:?}",
        report(&run)
    );
    assert!(!p.shown.iter().any(|s| s.contains("recovery phrase")));
    assert_eq!(b.known().len(), 1);
}

#[test]
fn sync_with_the_watcher_declined_stays_off_before_any_question() {
    let b = boxed("sync-no-watcher");
    b.manager();
    let before = b.snapshot();
    let folder = b.folder("f");
    let mut p = new_owner(&folder);
    p.confirms.push(("Record note history", false));
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert!(
        report(&run)
            .iter()
            .any(|l| l.starts_with("sync skipped: sync needs the watcher")),
        "{:?}",
        report(&run)
    );
    assert!(!outcome(&run).failed);
    assert!(
        p.shown
            .iter()
            .any(|s| s.starts_with("warn: Sync needs the watcher"))
    );
    assert!(
        !p.shown
            .iter()
            .any(|s| s.contains("Which scope") || s.starts_with("note: Recovery"))
    );
    assert!(b.identity().is_none());
    assert!(!folder.exists());
    assert!(!b.config_text().contains("sync"));
    let _ = before;
}

/// The phrase owner's store as a copy of `first`'s, with no keys.
fn copied_store(first: &Sandbox, name: &str) -> Sandbox {
    let second = boxed(name);
    second.manager();
    std::fs::create_dir_all(second.store().join("notes")).unwrap();
    copy_dir(
        &first.store().join(".bilbo/scopes"),
        &second.store().join(".bilbo/scopes"),
    );
    second
}

fn phrase_answers(folder: &Path, words: Vec<String>) -> Scripted {
    phrase_at(folder.display().to_string(), words)
}

/// The answers of a person who types the phrase and names `spot`, a folder or a relay URL.
fn phrase_at(spot: String, words: Vec<String>) -> Scripted {
    Scripted {
        confirms: vec![SYNC_ON, MATCHES, ("Apply these changes?", true)],
        entered: vec![("Folder or relay URL to sync", spot)],
        typed: words,
        ..Scripted::default()
    }
}

fn summary_of(p: &Scripted) -> String {
    p.shown
        .iter()
        .find(|s| s.starts_with("note: Setup will"))
        .cloned()
        .unwrap_or_default()
}

#[test]
fn a_wiped_laptop_whose_store_and_folder_hold_the_scope_joins_it() {
    let first = boxed("sync-wiped-a");
    first.manager();
    let folder = first.folder("f");
    let mut p = new_owner(&folder);
    assert!(
        wizard_as(&first, &mut p, &mut Script::working(), Some("rivendell"))
            .result
            .is_ok()
    );
    publish(&first, &folder);
    let second = copied_store(&first, "sync-wiped-b");
    let mut q = phrase_answers(&folder, p.phrase().unwrap());
    let run = wizard_as(&second, &mut q, &mut Script::working(), Some("bagend"));
    assert!(
        report(&run).contains(&ok_line("personal", &folder, 0)),
        "{:?}",
        report(&run)
    );
    let summary = summary_of(&q);
    assert!(
        !summary.contains("Copy the manifest") && !summary.contains("Create the scope"),
        "{summary}"
    );
    assert!(
        summary.contains("Add this device to the scope personal"),
        "{summary}"
    );
    let id = second.identity().unwrap();
    let latest = second.known()[0].scope.latest().unwrap().manifest.clone();
    assert!(latest.lists(&id.device.id()));
}

#[test]
fn a_store_that_holds_the_scope_and_an_empty_folder_creates_nothing() {
    let first = boxed("sync-held-a");
    first.manager();
    let mut p = new_owner(&first.folder("f1"));
    assert!(
        wizard_as(&first, &mut p, &mut Script::working(), Some("rivendell"))
            .result
            .is_ok()
    );
    let second = copied_store(&first, "sync-held-b");
    let mut q = phrase_answers(&second.folder("f2"), p.phrase().unwrap());
    let run = wizard_as(&second, &mut q, &mut Script::working(), Some("bagend"));
    assert!(run.result.is_ok(), "{:?}", q.shown);
    assert!(
        !summary_of(&q).contains("Create the scope"),
        "{}",
        summary_of(&q)
    );
    assert_eq!(second.scope_ids(), first.scope_ids());
}

#[test]
fn a_dead_end_fork_is_set_aside_before_the_folders_scope_is_joined() {
    let one = boxed("sync-fork-1");
    one.manager();
    let mut p = new_owner(&one.folder("f1"));
    assert!(
        wizard_as(&one, &mut p, &mut Script::working(), Some("rivendell"))
            .result
            .is_ok()
    );
    let a = one.scope_ids()[0].clone();
    let two = boxed("sync-fork-2");
    two.manager();
    let f2 = two.folder("f2");
    let mut q = phrase_answers(&f2, p.phrase().unwrap());
    q.confirms.push(("recovery phrase from another", true));
    assert!(
        wizard_as(&two, &mut q, &mut Script::working(), Some("bagend"))
            .result
            .is_ok(),
        "{:?}",
        q.shown
    );
    let b = two.scope_ids()[0].clone();
    assert_ne!(a, b);
    publish(&two, &f2);
    let three = copied_store(&one, "sync-fork-3");
    let mut r = phrase_answers(&f2, p.phrase().unwrap());
    let run = wizard_as(&three, &mut r, &mut Script::working(), Some("gondor"));
    assert!(
        report(&run).contains(&ok_line("personal", &f2, 0)),
        "{:?}",
        report(&run)
    );
    assert!(
        summary_of(&r).contains("Copy the manifest of personal"),
        "{}",
        summary_of(&r)
    );
    let known = three.known();
    assert_eq!(known.len(), 1, "the fork holds no version any more");
    assert_eq!(known[0].scope.id, b);
    let lost = three
        .store()
        .join(".bilbo/scopes")
        .join(&a)
        .join("manifest/lost");
    assert!(lost.is_dir(), "the fork's version moved to lost/");
    let id = three.identity().unwrap();
    assert!(
        three.known()[0]
            .scope
            .latest()
            .unwrap()
            .manifest
            .lists(&id.device.id())
    );
}

#[test]
fn a_new_scope_beside_a_manifest_that_never_listed_the_device_is_refused_up_front() {
    let first = boxed("sync-unsealed-a");
    first.manager();
    let mut p = new_owner(&first.folder("f1"));
    assert!(
        wizard_as(&first, &mut p, &mut Script::working(), Some("rivendell"))
            .result
            .is_ok()
    );
    let second = copied_store(&first, "sync-unsealed-b");
    second.write_config("a");
    let mut text = second.config_text();
    text.push_str("scope.work.sync = off\n");
    std::fs::write(second.config(), text).unwrap();
    let mut q = phrase_answers(&second.folder("f2"), p.phrase().unwrap());
    q.selects = vec![("Which scope", 1)];
    let run = wizard_as(&second, &mut q, &mut Script::working(), Some("bagend"));
    assert!(
        report(&run).contains(&format!(
            "sync skipped: {}",
            crate::setup::syncing::UNSEALED
        )),
        "{:?}",
        report(&run)
    );
    assert!(second.identity().is_none());
    assert!(!second.folder("f2").exists());
    assert!(!outcome(&run).failed);
}

#[test]
fn a_host_name_with_no_letter_turns_sync_off_instead_of_ending_the_wizard() {
    let b = boxed("sync-no-host");
    b.manager();
    let mut p = new_owner(&b.folder("f"));
    let mut facts = gather(&flags(&["--yes"], Mode::Wizard, &b.env), &b.env, b.path())
        .ok()
        .unwrap();
    facts.host = None;
    let run = wizard_with(
        facts,
        &mut p,
        &mut Script::working(),
        || None,
        |_, _| Ok(8),
        |_| Ok("indexed".into()),
    );
    let lines = &run.ok().unwrap().lines;
    let line = lines.iter().find(|l| l.starts_with("sync ")).unwrap();
    assert!(line.contains("bilbo device init --name <name>"), "{line}");
    assert!(b.identity().is_none());
}

#[test]
fn a_folder_that_cannot_hold_a_scope_is_found_before_the_phrase_is_shown() {
    let b = boxed("sync-blocked-first");
    b.manager();
    let folder = b.folder("f");
    let arriving = folder.join("scopes/6bfzzv5eukiswvtgipvo5mx7ze/manifest");
    std::fs::create_dir_all(&arriving).unwrap();
    std::fs::write(arriving.join("2.json"), "{}").unwrap();
    let mut p = new_owner(&folder);
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert!(
        report(&run).iter().any(|l| l.starts_with("sync skipped: ")),
        "{:?}",
        report(&run)
    );
    assert!(
        !p.shown
            .iter()
            .any(|s| s.starts_with("note: Recovery phrase"))
    );
    assert!(b.identity().is_none());
}

#[test]
fn a_damaged_key_fails_the_step_in_the_wizard_as_it_does_under_yes() {
    let b = boxed("sync-damaged-wizard");
    b.manager();
    let folder = b.folder("held");
    outsider(&b, &folder);
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        b.keys().join("device.key"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let mut p = Scripted {
        confirms: vec![SYNC_ON, ("Apply these changes?", true)],
        ..Scripted::default()
    };
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    let line = report(&run)
        .iter()
        .find(|l| l.starts_with("sync "))
        .unwrap()
        .clone();
    assert!(line.starts_with("sync failed: "), "{line}");
    assert!(outcome(&run).failed);
}

#[test]
fn a_scope_turned_off_is_named_after_the_scopes_that_still_sync() {
    let b = boxed("sync-off-named");
    b.manager();
    let held = b.folder("held");
    outsider(&b, &held);
    let work = b.folder("work");
    std::fs::create_dir_all(&work).unwrap();
    b.write_config("a");
    let mut text = b.config_text();
    text.push_str(&format!("scope.work.sync = file://{}\n", work.display()));
    std::fs::write(b.config(), text).unwrap();
    let mut p = Scripted {
        confirms: vec![SYNC_ON, ("Apply these changes?", true)],
        entered: vec![("Folder or relay URL to sync", held.display().to_string())],
        selects: vec![("Which scope", 0)],
        ..Scripted::default()
    };
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    let line = report(&run)
        .iter()
        .find(|l| l.starts_with("sync "))
        .unwrap()
        .clone();
    assert!(line.starts_with("sync ok: work through "), "{line}");
    assert!(
        line.ends_with(&format!(
            "; personal skipped: {}",
            crate::setup::syncing::BLOCKED
        )),
        "{line}"
    );
}

// ---------------------------------------------------------------------------
// Sync through a relay

use crate::sync::manifests;
use crate::sync::transport::{self, Keys};

/// The answers of a person with no key who sets `personal` up through the relay at `url`.
fn new_owner_at(url: &str) -> Scripted {
    Scripted {
        entered: vec![("Folder or relay URL to sync", url.to_string())],
        ..new_owner(Path::new("/unused"))
    }
}

/// The relay `first` synced through, restarted on its port with `first`'s owner admitted, and `first`'s scope
/// published to it as a watcher cycle does.
fn admitting(first: &Sandbox, url: &str, port: u16) -> crate::relay::Running {
    let id = first.identity().unwrap();
    let print = keys::owner_fingerprint(&id.owner.sign.public());
    let relay = relay("driven-relay-b", &[print], port);
    let transport = transport::open(url, &Keys::of(&id)).unwrap();
    let outcome = manifests::step(
        &*transport,
        &manifests::Input {
            root: &first.store(),
            name: "personal",
            url,
            identity: &id,
            now: jiff::Timestamp::now(),
        },
    )
    .unwrap();
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.stop, None);
    relay
}

#[test]
fn a_first_device_on_a_relay_writes_the_url_and_the_watcher_publishes_the_scope() {
    let first = boxed("relay-first");
    first.manager();
    let before = relay("driven-relay-a", &[], 0);
    let (url, port) = (before.url(), before.port);
    let mut p = new_owner_at(&url);
    let run = wizard_as(&first, &mut p, &mut Script::working(), Some("rivendell"));
    assert!(
        report(&run).contains(&format!("sync ok: personal through {url} (0 notes)")),
        "{:?}",
        report(&run)
    );
    assert!(!outcome(&run).failed);
    assert!(first.identity().is_some());
    assert!(
        first
            .config_text()
            .contains(&format!("scope.personal.sync = {url}\n")),
        "{}",
        first.config_text()
    );
    let summary = summary_of(&p);
    assert!(
        summary.contains(&format!("Sync personal through {url}")),
        "{summary}"
    );
    assert!(!summary.contains("Create the folder"), "{summary}");
    assert!(summary.contains("Create the scope personal"), "{summary}");
    assert!(
        p.shown
            .iter()
            .all(|s| !s.contains("does not admit this owner")),
        "{:?}",
        p.shown
    );
    drop(before);
    let relay = admitting(&first, &url, port);
    let id = first.identity().unwrap();
    let held = transport::open(&url, &Keys::of(&id)).unwrap();
    assert_eq!(held.scopes().unwrap(), first.scope_ids());
    drop(relay);
}

#[test]
fn every_device_lost_the_wizard_fetches_the_scope_from_the_relay_with_the_owner_key() {
    let first = boxed("relay-lost-a");
    first.manager();
    let before = relay("driven-relay-a2", &[], 0);
    let (url, port) = (before.url(), before.port);
    let mut p = new_owner_at(&url);
    assert!(
        wizard_as(&first, &mut p, &mut Script::working(), Some("rivendell"))
            .result
            .is_ok()
    );
    drop(before);
    let relay = admitting(&first, &url, port);
    let second = boxed("relay-lost-b");
    second.manager();
    let mut q = phrase_at(url.clone(), p.phrase().unwrap());
    q.confirms.push(("recovery phrase from another", true));
    let run = wizard_as(&second, &mut q, &mut Script::working(), Some("bagend"));
    assert!(
        report(&run).contains(&format!("sync ok: personal through {url} (0 notes)")),
        "{:?} {:?}",
        report(&run),
        q.shown
    );
    let summary = summary_of(&q);
    assert!(
        summary.contains("Copy the manifest of personal"),
        "{summary}"
    );
    assert!(!summary.contains("Create the scope"), "{summary}");
    assert_eq!(second.scope_ids(), first.scope_ids());
    let id = second.identity().unwrap();
    assert!(
        second.known()[0]
            .scope
            .latest()
            .unwrap()
            .manifest
            .lists(&id.device.id())
    );
    drop(relay);
}

#[test]
fn an_enrolled_device_is_told_when_the_relay_does_not_admit_its_owner() {
    let first = boxed("relay-unadmitted");
    first.manager();
    let mut p = new_owner(&first.folder("f"));
    assert!(
        wizard_as(&first, &mut p, &mut Script::working(), Some("rivendell"))
            .result
            .is_ok()
    );
    first.write_config("a");
    let mut text = first.config_text();
    text.push_str("scope.work.sync = off\n");
    std::fs::write(first.config(), text).unwrap();
    let relay = relay("driven-relay-c", &[], 0);
    let mut q = Scripted {
        confirms: vec![SYNC_ON, ("Apply these changes?", true)],
        entered: vec![("Folder or relay URL to sync", relay.url())],
        selects: vec![("Which scope", 1)],
        ..Scripted::default()
    };
    let run = wizard_as(&first, &mut q, &mut Script::working(), Some("rivendell"));
    let why = q
        .shown
        .iter()
        .find(|s| s.contains("does not admit this owner; start it with --owner"))
        .unwrap_or_else(|| panic!("{:?}", q.shown));
    assert!(why.starts_with("warn: relay http://127.0.0.1:"), "{why}");
    assert!(
        report(&run).iter().any(|l| l.contains("skipped")),
        "{:?}",
        report(&run)
    );
}

#[test]
fn a_url_that_is_not_a_relay_is_asked_again() {
    let b = boxed("relay-not");
    b.manager();
    let other = Answering::start(404);
    let folder = b.folder("good");
    let mut p = new_owner(&folder);
    p.entered = vec![
        ("Folder or relay URL to sync", other.url()),
        ("Folder or relay URL to sync", folder.display().to_string()),
    ];
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert!(run.result.is_ok(), "{:?}", p.shown);
    assert!(
        p.shown
            .contains(&format!("warn: {} is not a bilbo relay", other.url())),
        "{:?}",
        p.shown
    );
    assert_eq!(
        p.shown
            .iter()
            .filter(|s| s.starts_with("Folder or relay URL to sync"))
            .count(),
        2
    );
    assert_eq!(other.heads().len(), 1);
}

#[test]
fn a_relay_that_is_down_is_asked_again() {
    let b = boxed("relay-down");
    b.manager();
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let down = format!("http://127.0.0.1:{port}");
    let folder = b.folder("good");
    let mut p = new_owner(&folder);
    p.entered = vec![
        ("Folder or relay URL to sync", down.clone()),
        ("Folder or relay URL to sync", folder.display().to_string()),
    ];
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert!(run.result.is_ok(), "{:?}", p.shown);
    assert!(
        p.shown
            .iter()
            .any(|s| s.starts_with(&format!("warn: {down} is not reachable: "))),
        "{:?}",
        p.shown
    );
}

#[test]
fn plain_http_to_another_host_is_refused_and_no_request_is_sent() {
    let b = boxed("relay-plain");
    b.manager();
    let listening = Answering::start(200);
    let remote = format!("http://0.0.0.0:{}", listening.port());
    let folder = b.folder("good");
    let mut p = new_owner(&folder);
    p.entered = vec![
        ("Folder or relay URL to sync", remote.clone()),
        ("Folder or relay URL to sync", folder.display().to_string()),
    ];
    let run = wizard_as(&b, &mut p, &mut Script::working(), Some("rivendell"));
    assert!(run.result.is_ok(), "{:?}", p.shown);
    assert!(
        p.shown.iter().any(|s| s.starts_with(&format!(
            "warn: {remote} cannot be a relay: plain http:// reaches only a loopback host"
        ))),
        "{:?}",
        p.shown
    );
    assert!(listening.heads().is_empty(), "{:?}", listening.heads());
}
