//! The interactive setup wizard.

use crate::Failure;
use crate::config::{self, Embedder, Token};
use crate::host::prompt::{Choice, Prompter};
use std::io;
use std::path::PathBuf;
use zeroize::Zeroizing;

const OLLAMA_URL: &str = "http://localhost:11434";
const OPENAI_URL: &str = "https://api.openai.com";

const NONE: usize = 0;
const LOCAL: usize = 1;
const OLLAMA: usize = 2;
const OPENAI: usize = 3;
const OTHER: usize = 4;

/// What the wizard knows about the local embedder.
pub struct Local {
    /// `http://127.0.0.1:8737` and `qwen3-embedding-0.6b`: what the choice sets.
    pub url: String,
    pub model_name: String,
    /// The model's download size in MB.
    pub download_mb: u64,
    /// `llama-server` on PATH.
    pub llama_server: Option<PathBuf>,
    /// Why the choice cannot run here, if it cannot.
    pub unavailable: Option<String>,
}

pub struct Facts {
    pub root: PathBuf,
    pub config_path: PathBuf,
    /// The current config's embedder, if any.
    pub existing: Option<Embedder>,
    /// Some(target) when the config is managed elsewhere.
    pub managed: Option<String>,
    /// `<config folder>/token`.
    pub token_path: PathBuf,
    pub token_exists: bool,
    /// `None`: no Ollama on localhost:11434.
    pub ollama: Option<Vec<String>>,
    pub claude: Option<PathBuf>,
    pub codex: Option<PathBuf>,
    /// This platform has a timer.
    pub timer: bool,
    /// The interval of the installed timer, if one is installed.
    pub timer_minutes: Option<u32>,
    pub local: Local,
}

pub struct Answers {
    /// `None`: keyword search only. With a pasted key, the token is `File(facts.token_path)`.
    pub embedder: Option<Embedder>,
    pub pasted: Option<Zeroizing<String>>,
    /// From the wizard's own check.
    pub dims: Option<usize>,
    pub claude: bool,
    pub codex: bool,
    /// `None`: not chosen, or no embedder.
    pub timer: Option<u32>,
    /// Some(llama-server) when the local embedder was chosen; `embedder` is then the local one and `dims` None.
    pub local: Option<PathBuf>,
}

type Keyed = (Option<Token>, Option<Zeroizing<String>>);
type Settled = (
    Option<Embedder>,
    Option<Zeroizing<String>>,
    Option<usize>,
    Option<PathBuf>,
);

pub fn ask<P: Prompter>(
    p: &mut P,
    facts: &Facts,
    mut check: impl FnMut(&Embedder, Option<&str>) -> Result<usize, String>,
    var_set: impl Fn(&str) -> bool,
) -> io::Result<Answers> {
    p.intro("bilbo setup")?;
    p.info(&format!(
        "Notes go to {}/notes. Set BILBO_HOME to put them elsewhere.",
        facts.root.display()
    ))?;

    let (embedder, pasted, dims, local) = match &facts.managed {
        Some(target) => {
            let lines = match &facts.existing {
                Some(e) => format!("embedder.url = {}\nembedder.model = {}", e.url, e.model),
                None => "No embedder: recall uses keywords only.".to_string(),
            };
            p.note(
                "Config managed elsewhere",
                &format!(
                    "{} links to {}.\n{}\nChange it where it is managed.",
                    facts.config_path.display(),
                    target,
                    lines
                ),
            )?;
            (facts.existing.clone(), None, None, None)
        }
        None => ask_embedder(p, facts, &mut check, &var_set)?,
    };

    let (claude, codex) = ask_plugin(p, facts)?;
    let timer = match &embedder {
        Some(e) if facts.timer => ask_timer(p, e, facts.timer_minutes)?,
        _ => None,
    };
    Ok(Answers {
        embedder,
        pasted,
        dims,
        claude,
        codex,
        timer,
        local,
    })
}

fn ask_embedder<P: Prompter>(
    p: &mut P,
    facts: &Facts,
    check: &mut impl FnMut(&Embedder, Option<&str>) -> Result<usize, String>,
    var_set: &impl Fn(&str) -> bool,
) -> io::Result<Settled> {
    let existing = facts.existing.as_ref();
    loop {
        let initial = initial_provider(existing, &facts.local);
        let provider = p.select(
            "Which embedder should recall use?",
            &[
                Choice::new("No embedder", "keyword search only"),
                Choice::new("Local embedder, run by bilbo", local_hint(&facts.local)),
                Choice::new(
                    "Ollama on this machine",
                    match usable_models(facts) {
                        Some(models) => format!("found, {} models", models.len()),
                        None => {
                            format!("not found on {}", OLLAMA_URL.trim_start_matches("http://"))
                        }
                    },
                ),
                Choice::new("OpenAI", "api.openai.com"),
                Choice::new("Another OpenAI-compatible URL", ""),
            ],
            initial,
        )?;
        if provider == NONE {
            return Ok((None, None, None, None));
        }
        if provider == LOCAL {
            if let Some(reason) = &facts.local.unavailable {
                p.warn(reason)?;
                continue;
            }
            let Some(llama_server) = ask_llama_server(p, &facts.local)? else {
                continue;
            };
            let embedder = Embedder {
                url: facts.local.url.clone(),
                model: facts.local.model_name.clone(),
                token: None,
                query_prefix: config::default_query_prefix(&facts.local.model_name).to_string(),
                min_similarity: existing
                    .map(|e| e.min_similarity)
                    .unwrap_or(config::DEFAULT_MIN_SIMILARITY),
            };
            return Ok((Some(embedder), None, None, Some(llama_server)));
        }

        let existing_model = existing.map(|e| e.model.as_str()).unwrap_or("");
        let ollama_url = match existing {
            Some(e) if initial == OLLAMA => e.url.as_str(),
            _ => OLLAMA_URL,
        };
        let (url, model) = match provider {
            OLLAMA => match usable_models(facts) {
                Some(models) => {
                    let initial = existing
                        .and_then(|e| models.iter().position(|m| *m == e.model))
                        .or_else(|| models.iter().position(|m| m.contains("embed")))
                        .unwrap_or(0);
                    let choices: Vec<Choice> =
                        models.iter().map(|m| Choice::new(m.as_str(), "")).collect();
                    let i = p.select("Which Ollama model?", &choices, initial)?;
                    (ollama_url.to_string(), models[i].clone())
                }
                None => {
                    let url = p
                        .input("Ollama URL", ollama_url, check_url)?
                        .trim()
                        .to_string();
                    let model = p
                        .input("Model name", existing_model, check_model)?
                        .trim()
                        .to_string();
                    (url, model)
                }
            },
            OPENAI => {
                let default = match existing {
                    Some(e) if e.url == OPENAI_URL => e.model.as_str(),
                    _ => "text-embedding-3-small",
                };
                let model = p
                    .input("Model name", default, check_model)?
                    .trim()
                    .to_string();
                (OPENAI_URL.to_string(), model)
            }
            _ => {
                let default = existing
                    .filter(|_| initial == OTHER)
                    .map(|e| e.url.as_str())
                    .unwrap_or("");
                let url = p
                    .input("Embedder URL", default, check_url)?
                    .trim()
                    .to_string();
                let model = p
                    .input("Model name", existing_model, check_model)?
                    .trim()
                    .to_string();
                (url, model)
            }
        };

        let same_embedder = existing.is_some_and(|e| e.url == url && e.model == model);
        let default_prefix = match existing {
            Some(e) if same_embedder => e.query_prefix.clone(),
            _ => config::default_query_prefix(&model).to_string(),
        };
        let query_prefix = if p.confirm("Change advanced settings (the query prefix)?", false)? {
            let shown = default_prefix.replace('\n', "\\n");
            let hint = if shown.is_empty() {
                "no prefix".to_string()
            } else {
                shown.clone()
            };
            let pick = p.select(
                "Query prefix",
                &[
                    Choice::new("Keep the default", hint),
                    Choice::new("No prefix", ""),
                    Choice::new("Type one", ""),
                ],
                0,
            )?;
            match pick {
                0 => default_prefix,
                1 => String::new(),
                _ => p
                    .input("Query prefix (\\n for a newline)", &shown, no_check)?
                    .replace("\\n", "\n"),
            }
        } else {
            default_prefix
        };

        let (token, pasted) = if config::is_local(&url) {
            (None, None)
        } else {
            ask_key(p, facts, provider, var_set)?
        };

        let embedder = Embedder {
            url,
            model,
            token,
            query_prefix,
            min_similarity: existing
                .map(|e| e.min_similarity)
                .unwrap_or(config::DEFAULT_MIN_SIMILARITY),
        };

        loop {
            let secret = pasted.as_ref().map(|k| k.as_str());
            let checked = p.spin(
                &format!("Checking {} at {}", embedder.model, embedder.url),
                || check(&embedder, secret),
                |n| format!("{} answered with {n}-dimensional vectors", embedder.url),
            );
            match checked {
                Ok(dims) => return Ok((Some(embedder), pasted, Some(dims), None)),
                Err(_) => {
                    let next = p.select(
                        "The embedder check failed. What now?",
                        &[
                            Choice::new("Try again", ""),
                            Choice::new("Change the embedder settings", ""),
                            Choice::new("Continue with keyword search only", ""),
                        ],
                        0,
                    )?;
                    match next {
                        0 => continue,
                        1 => break,
                        _ => return Ok((None, None, None, None)),
                    }
                }
            }
        }
    }
}

fn local_hint(local: &Local) -> String {
    match (&local.unavailable, &local.llama_server) {
        (Some(reason), _) => reason.clone(),
        (None, Some(_)) => format!("llama-server found, {} MB download", local.download_mb),
        (None, None) => format!("llama-server not found, {} MB download", local.download_mb),
    }
}

/// The llama-server to run: the one found, or a path typed after the note. `None`: back to the list.
fn ask_llama_server<P: Prompter>(p: &mut P, local: &Local) -> io::Result<Option<PathBuf>> {
    if let Some(found) = &local.llama_server {
        return Ok(Some(found.clone()));
    }
    p.note(
        "llama-server not found",
        "bilbo runs llama-server but does not install it. Install it with brew install llama.cpp, your distribution's llama.cpp package or Nix's llama-cpp, or type its path. Leave the path empty to go back.",
    )?;
    let path = p.input("Path to llama-server", "", check_program)?;
    let path = path.trim();
    if path.is_empty() {
        return Ok(None);
    }
    let home = std::env::var_os("HOME");
    Ok(Some(expand_home(
        path,
        home.as_deref().map(std::path::Path::new),
    )))
}

/// After the local embedder failed past the confirmation: shows the message and the log,
/// and returns true to try again, false to continue with keyword search only.
pub fn local_failed<P: Prompter>(
    p: &mut P,
    message: &str,
    log: &std::path::Path,
) -> io::Result<bool> {
    p.warn(&format!(
        "{message}\nThe server's log is {}.",
        log.display()
    ))?;
    let next = p.select(
        "The local embedder failed. What now?",
        &[
            Choice::new("Try again", ""),
            Choice::new("Continue with keyword search only", ""),
        ],
        0,
    )?;
    Ok(next == 0)
}

/// The key question for a non-local URL: the token and the pasted key.
fn ask_key<P: Prompter>(
    p: &mut P,
    facts: &Facts,
    provider: usize,
    var_set: &impl Fn(&str) -> bool,
) -> io::Result<Keyed> {
    let existing_token = facts.existing.as_ref().and_then(|e| e.token.as_ref());
    let initial = match existing_token {
        Some(Token::Var(_)) => 0,
        Some(Token::File(_)) => 1,
        None => 2,
    };
    let source = p.select(
        "Where does the API key come from?",
        &[
            Choice::new("An environment variable", "the index timer cannot read it"),
            Choice::new("A file", ""),
            Choice::new(
                "Paste it now",
                format!(
                    "saved to {}, readable only by you",
                    facts.token_path.display()
                ),
            ),
            Choice::new("No key", ""),
        ],
        initial,
    )?;
    let answer = match source {
        0 => {
            let default = match existing_token {
                Some(Token::Var(name)) => name.as_str(),
                _ if provider == OPENAI => "OPENAI_API_KEY",
                _ => "",
            };
            loop {
                let name = p.input("Variable name", default, check_var)?;
                let name = name.trim().to_string();
                if var_set(&name) {
                    break (Some(Token::Var(name)), None);
                }
                p.warn(&format!("{name} is not set or is empty in this shell."))?;
            }
        }
        1 => {
            let default = match existing_token {
                Some(Token::File(path)) => path.display().to_string(),
                _ => String::new(),
            };
            let path = p.input("Key file", &default, check_file)?;
            let home = std::env::var_os("HOME");
            let path = expand_home(path.trim(), home.as_deref().map(std::path::Path::new));
            (Some(Token::File(path)), None)
        }
        2 => {
            let token = Some(Token::File(facts.token_path.clone()));
            let replace = !facts.token_exists
                || p.confirm(
                    &format!("Replace the key in {}?", facts.token_path.display()),
                    false,
                )?;
            if replace {
                let key = p.password("API key", check_key)?;
                let key = if key.trim().len() == key.len() {
                    key
                } else {
                    Zeroizing::new(key.trim().to_string())
                };
                (token, Some(key))
            } else {
                (token, None)
            }
        }
        _ => (None, None),
    };
    Ok(answer)
}

fn ask_plugin<P: Prompter>(p: &mut P, facts: &Facts) -> io::Result<(bool, bool)> {
    let found: Vec<(bool, &PathBuf)> = [(true, &facts.claude), (false, &facts.codex)]
        .into_iter()
        .filter_map(|(is_claude, path)| path.as_ref().map(|path| (is_claude, path)))
        .collect();
    if found.is_empty() {
        p.info("Neither claude nor codex was found, so the plugin is skipped.")?;
        return Ok((false, false));
    }
    let choices: Vec<Choice> = found
        .iter()
        .map(|(is_claude, path)| {
            Choice::new(
                if *is_claude { "Claude Code" } else { "Codex" },
                path.display().to_string(),
            )
        })
        .collect();
    let all: Vec<usize> = (0..choices.len()).collect();
    let picked = p.multiselect("Install the bilbo plugin in", &choices, &all)?;
    let chose = |want_claude: bool| {
        found
            .iter()
            .enumerate()
            .any(|(i, (is_claude, _))| *is_claude == want_claude && picked.contains(&i))
    };
    Ok((chose(true), chose(false)))
}

fn ask_timer<P: Prompter>(
    p: &mut P,
    embedder: &Embedder,
    current: Option<u32>,
) -> io::Result<Option<u32>> {
    if let Some(Token::Var(name)) = &embedder.token {
        p.warn(&format!(
            "The index timer cannot read {name}, so the timer step will fail. Pick \"No timer\", or start over and keep the key in a file."
        ))?;
    }
    let choice = p.select(
        "Keep the index fresh in the background?",
        &[
            Choice::new("Every 15 minutes", ""),
            Choice::new("Another interval", ""),
            Choice::new("No timer", ""),
        ],
        usize::from(current.is_some_and(|n| n != 15)),
    )?;
    Ok(match choice {
        0 => Some(15),
        1 => {
            let default = current.unwrap_or(15).to_string();
            let minutes = p.input("Minutes between runs (1 to 1440)", &default, check_minutes)?;
            minutes.trim().parse().ok()
        }
        _ => None,
    })
}

pub fn confirm<P: Prompter>(p: &mut P, summary: &[String]) -> io::Result<bool> {
    p.note("Setup will", &summary.join("\n"))?;
    p.confirm("Apply these changes?", true)
}

pub fn confirm_remove<P: Prompter>(p: &mut P, summary: &[String]) -> io::Result<bool> {
    p.note("Setup will remove", &summary.join("\n"))?;
    p.confirm("Remove these?", false)
}

pub fn first_index<P: Prompter>(
    p: &mut P,
    notes: usize,
    run: impl FnOnce() -> Result<String, String>,
) -> io::Result<()> {
    if p.confirm(
        &format!("Run the first index now? The store holds {notes} notes."),
        true,
    )? {
        let _ = p.spin(&format!("Indexing {notes} notes"), run, |line| line.clone());
    }
    Ok(())
}

pub fn finish<P: Prompter>(p: &mut P, failed: bool) -> io::Result<()> {
    p.outro(if failed {
        "Some steps failed. The report follows."
    } else {
        "Done. The report follows."
    })
}

pub fn cancelled<P: Prompter>(p: &mut P) {
    let _ = p.cancel("Cancelled. Nothing changed.");
}

fn usable_models(facts: &Facts) -> Option<&Vec<String>> {
    facts.ollama.as_ref().filter(|models| !models.is_empty())
}

fn initial_provider(existing: Option<&Embedder>, local: &Local) -> usize {
    match existing {
        None => NONE,
        Some(e) if e.url == local.url && e.model == local.model_name => LOCAL,
        Some(e) => match e.url.as_str() {
            "http://localhost:11434" | "http://127.0.0.1:11434" => OLLAMA,
            OPENAI_URL => OPENAI,
            _ => OTHER,
        },
    }
}

fn expand_home(text: &str, home: Option<&std::path::Path>) -> PathBuf {
    match (text.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(text),
    }
}

fn no_check(_: &str) -> Result<(), String> {
    Ok(())
}

fn check_url(text: &str) -> Result<(), String> {
    match config::url_problem(text.trim()) {
        None => Ok(()),
        Some(problem) if problem.contains("user name") => {
            Err("Leave the user name and password out of the URL".to_string())
        }
        Some(_) => Err("Enter an http:// or https:// URL with a host".to_string()),
    }
}

fn check_program(text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    let home = std::env::var_os("HOME").filter(|h| std::path::Path::new(h).is_absolute());
    let path = if std::path::Path::new(text).is_absolute() {
        Some(PathBuf::from(text))
    } else if text.starts_with("~/") {
        home.map(|h| expand_home(text, Some(std::path::Path::new(&h))))
    } else {
        None
    };
    match path {
        Some(path) if crate::host::command::is_executable(&path) => Ok(()),
        _ => Err(
            "Enter the path of an executable llama-server, or leave it empty to go back"
                .to_string(),
        ),
    }
}

fn check_model(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        Err("Enter a model name".to_string())
    } else {
        Ok(())
    }
}

fn check_var(text: &str) -> Result<(), String> {
    if config::is_variable_name(text.trim()) {
        Ok(())
    } else {
        Err("Use letters, digits and _, not starting with a digit".to_string())
    }
}

fn check_file(text: &str) -> Result<(), String> {
    let text = text.trim();
    let home_ok = std::env::var_os("HOME").is_some_and(|h| std::path::Path::new(&h).is_absolute());
    if std::path::Path::new(text).is_absolute() || (text.starts_with("~/") && home_ok) {
        Ok(())
    } else {
        Err("Enter an absolute path, or one starting with ~/".to_string())
    }
}

fn check_minutes(text: &str) -> Result<(), String> {
    let text = text.trim();
    match text.parse::<u32>() {
        Ok(n) if (1..=1440).contains(&n) && text.bytes().all(|b| b.is_ascii_digit()) => Ok(()),
        _ => Err("Enter a whole number from 1 to 1440".to_string()),
    }
}

fn check_key(text: &str) -> Result<(), String> {
    let text = text.trim();
    if !text.is_empty() && text.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        Ok(())
    } else {
        Err("the key must be printable ASCII with no spaces".to_string())
    }
}

/// A wizard error: Ctrl-C or Esc cancels, anything else stops it; nothing was changed either way.
pub fn stopped<P: Prompter>(p: &mut P, e: std::io::Error) -> Failure {
    if e.kind() == std::io::ErrorKind::Interrupted {
        return declined(p);
    }
    cancelled(p);
    Failure::Refused(format!("the wizard stopped: {e}; nothing changed"))
}

pub fn declined<P: Prompter>(p: &mut P) -> Failure {
    cancelled(p);
    Failure::Refused("setup cancelled; nothing changed".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    enum Answer {
        Select(usize),
        Multi(Vec<usize>),
        Text(String),
        Secret(String),
        Yes,
        No,
        /// The prompt's own initial value or default.
        Default,
        Interrupt,
    }

    #[derive(Default)]
    struct Script {
        answers: VecDeque<Answer>,
        shown: Vec<String>,
    }

    fn text(s: &str) -> Answer {
        Answer::Text(s.to_string())
    }

    impl Script {
        fn new(answers: Vec<Answer>) -> Script {
            Script {
                answers: answers.into(),
                shown: Vec::new(),
            }
        }

        fn next(&mut self) -> io::Result<Answer> {
            match self.answers.pop_front() {
                Some(Answer::Interrupt) => Err(io::ErrorKind::Interrupted.into()),
                Some(answer) => Ok(answer),
                None => Err(io::ErrorKind::UnexpectedEof.into()),
            }
        }

        fn wrong() -> io::Error {
            io::Error::new(io::ErrorKind::InvalidData, "answer of the wrong kind")
        }

        fn saw(&self, needle: &str) -> bool {
            self.shown.iter().any(|s| s.contains(needle))
        }

        fn log(&mut self, line: String) {
            self.shown.push(line);
        }
    }

    impl Prompter for Script {
        fn intro(&mut self, title: &str) -> io::Result<()> {
            self.log(format!("intro: {title}"));
            Ok(())
        }
        fn info(&mut self, text: &str) -> io::Result<()> {
            self.log(format!("info: {text}"));
            Ok(())
        }
        fn warn(&mut self, text: &str) -> io::Result<()> {
            self.log(format!("warn: {text}"));
            Ok(())
        }
        fn note(&mut self, title: &str, body: &str) -> io::Result<()> {
            self.log(format!("note: {title}\n{body}"));
            Ok(())
        }
        fn select(
            &mut self,
            prompt: &str,
            choices: &[Choice],
            initial: usize,
        ) -> io::Result<usize> {
            let list: Vec<String> = choices
                .iter()
                .map(|c| format!("{} / {}", c.label, c.hint))
                .collect();
            self.log(format!(
                "select: {prompt} initial={initial} [{}]",
                list.join("; ")
            ));
            match self.next()? {
                Answer::Select(i) if i < choices.len() => Ok(i),
                Answer::Default => Ok(initial),
                _ => Err(Script::wrong()),
            }
        }
        fn multiselect(
            &mut self,
            prompt: &str,
            choices: &[Choice],
            initial: &[usize],
        ) -> io::Result<Vec<usize>> {
            let list: Vec<String> = choices
                .iter()
                .map(|c| format!("{} / {}", c.label, c.hint))
                .collect();
            self.log(format!("multiselect: {prompt} [{}]", list.join("; ")));
            match self.next()? {
                Answer::Multi(v) => Ok(v),
                Answer::Default => Ok(initial.to_vec()),
                _ => Err(Script::wrong()),
            }
        }
        fn input(
            &mut self,
            prompt: &str,
            default: &str,
            check: fn(&str) -> Result<(), String>,
        ) -> io::Result<String> {
            self.log(format!("input: {prompt} default={default}"));
            let value = match self.next()? {
                Answer::Text(s) if s.is_empty() => default.to_string(),
                Answer::Text(s) => s,
                Answer::Default => default.to_string(),
                _ => return Err(Script::wrong()),
            };
            check(&value).map_err(|m| io::Error::new(io::ErrorKind::InvalidInput, m))?;
            Ok(value)
        }
        fn password(
            &mut self,
            prompt: &str,
            check: fn(&str) -> Result<(), String>,
        ) -> io::Result<Zeroizing<String>> {
            self.log(format!("password: {prompt}"));
            match self.next()? {
                Answer::Secret(s) => {
                    check(&s).map_err(|m| io::Error::new(io::ErrorKind::InvalidInput, m))?;
                    Ok(Zeroizing::new(s))
                }
                _ => Err(Script::wrong()),
            }
        }
        fn confirm(&mut self, prompt: &str, initial: bool) -> io::Result<bool> {
            self.log(format!("confirm: {prompt} initial={initial}"));
            match self.next()? {
                Answer::Yes => Ok(true),
                Answer::No => Ok(false),
                Answer::Default => Ok(initial),
                _ => Err(Script::wrong()),
            }
        }
        fn spin<T>(
            &mut self,
            message: &str,
            work: impl FnOnce() -> Result<T, String>,
            done: impl FnOnce(&T) -> String,
        ) -> Result<T, String> {
            self.log(format!("spin: {message}"));
            match work() {
                Ok(v) => {
                    self.log(format!("spin done: {}", done(&v)));
                    Ok(v)
                }
                Err(e) => {
                    self.log(format!("spin error: {e}"));
                    Err(e)
                }
            }
        }
        fn progress<T>(
            &mut self,
            message: &str,
            total: u64,
            work: impl FnOnce(&mut dyn FnMut(u64)) -> Result<T, String>,
            done: impl FnOnce(&T) -> String,
        ) -> Result<T, String> {
            self.log(format!("progress: {message} of {total}"));
            match work(&mut |_| {}) {
                Ok(v) => {
                    self.log(format!("progress done: {}", done(&v)));
                    Ok(v)
                }
                Err(e) => {
                    self.log(format!("progress error: {e}"));
                    Err(e)
                }
            }
        }
        fn outro(&mut self, text: &str) -> io::Result<()> {
            self.log(format!("outro: {text}"));
            Ok(())
        }
        fn cancel(&mut self, text: &str) -> io::Result<()> {
            self.log(format!("cancel: {text}"));
            Ok(())
        }
    }

    fn facts() -> Facts {
        Facts {
            root: PathBuf::from("/r"),
            config_path: PathBuf::from("/c/bilbo/config"),
            existing: None,
            managed: None,
            token_path: PathBuf::from("/c/bilbo/token"),
            token_exists: false,
            ollama: None,
            claude: Some(PathBuf::from("/bin/claude")),
            codex: Some(PathBuf::from("/bin/codex")),
            timer: true,
            timer_minutes: None,
            local: Local {
                url: "http://127.0.0.1:8737".to_string(),
                model_name: "qwen3-embedding-0.6b".to_string(),
                download_mb: 639,
                llama_server: Some(PathBuf::from("/bin/llama-server")),
                unavailable: None,
            },
        }
    }

    fn embedder(url: &str, model: &str) -> Embedder {
        Embedder {
            url: url.to_string(),
            model: model.to_string(),
            token: None,
            query_prefix: String::new(),
            min_similarity: 0.5,
        }
    }

    fn ok(_: &Embedder, _: Option<&str>) -> Result<usize, String> {
        Ok(1024)
    }

    fn run(facts: &Facts, answers: Vec<Answer>) -> (io::Result<Answers>, Script) {
        let mut script = Script::new(answers);
        let result = ask(&mut script, facts, ok, |_| true);
        (result, script)
    }

    /// The tail of every run that ends with an embedder: plugins and timer.
    fn tail() -> Vec<Answer> {
        vec![Answer::Multi(vec![0, 1]), Answer::Select(0)]
    }

    fn with_tail(mut head: Vec<Answer>) -> Vec<Answer> {
        head.extend(tail());
        head
    }

    fn openai_paste_script() -> Vec<Answer> {
        with_tail(vec![
            Answer::Select(OPENAI),
            text(""),
            Answer::No,
            Answer::Select(2),
            Answer::Secret("sk-abc123".to_string()),
        ])
    }

    #[test]
    fn keyword_only_asks_nothing_more() {
        let (r, s) = run(
            &facts(),
            vec![Answer::Select(NONE), Answer::Multi(vec![0, 1])],
        );
        let a = r.unwrap();
        assert!(a.embedder.is_none() && a.timer.is_none() && a.dims.is_none());
        assert!(a.claude && a.codex);
        assert!(!s.saw("input:") && !s.saw("confirm:") && !s.saw("Keep the index fresh"));
    }

    #[test]
    fn ollama_found_selects_the_embed_model() {
        let mut f = facts();
        f.ollama = Some(vec!["llama3".into(), "nomic-embed-text:latest".into()]);
        let (r, s) = run(
            &f,
            with_tail(vec![Answer::Select(OLLAMA), Answer::Default, Answer::No]),
        );
        let e = r.unwrap().embedder.unwrap();
        assert_eq!(e.url, "http://localhost:11434");
        assert_eq!(e.model, "nomic-embed-text:latest");
        assert!(s.saw("Ollama on this machine / found, 2 models"));
        assert!(s.saw("Which Ollama model? initial=1"));
    }

    #[test]
    fn ollama_missing_asks_the_url() {
        let (r, s) = run(
            &facts(),
            with_tail(vec![
                Answer::Select(OLLAMA),
                text(""),
                text("nomic-embed-text"),
                Answer::No,
            ]),
        );
        let e = r.unwrap().embedder.unwrap();
        assert_eq!(e.url, "http://localhost:11434");
        assert_eq!(e.model, "nomic-embed-text");
        assert!(s.saw("not found on localhost:11434"));
        assert!(s.saw("input: Ollama URL default=http://localhost:11434"));
    }

    #[test]
    fn openai_defaults_the_model_and_asks_the_key() {
        let (r, s) = run(&facts(), openai_paste_script());
        let a = r.unwrap();
        let e = a.embedder.unwrap();
        assert_eq!(e.url, "https://api.openai.com");
        assert_eq!(e.model, "text-embedding-3-small");
        assert!(s.saw("input: Model name default=text-embedding-3-small"));
        assert!(s.saw("select: Where does the API key come from?"));
        assert_eq!(a.dims, Some(1024));
    }

    #[test]
    fn qwen_gets_the_prefix() {
        let mut f = facts();
        f.ollama = Some(vec!["Qwen3-Embedding-0.6B".into()]);
        let (r, _) = run(
            &f,
            with_tail(vec![Answer::Select(OLLAMA), Answer::Default, Answer::No]),
        );
        assert_eq!(
            r.unwrap().embedder.unwrap().query_prefix,
            config::QWEN_PREFIX
        );
    }

    #[test]
    fn advanced_edits_the_prefix_with_escaped_newline() {
        let mut f = facts();
        f.ollama = Some(vec!["Qwen3-Embedding-0.6B".into()]);
        let (r, s) = run(
            &f,
            with_tail(vec![
                Answer::Select(OLLAMA),
                Answer::Default,
                Answer::Yes,
                Answer::Select(2),
                text("Ask: \\nQ: "),
            ]),
        );
        assert_eq!(r.unwrap().embedder.unwrap().query_prefix, "Ask: \nQ: ");
        assert!(s.saw("Query: ") && s.saw("\\nQuery: "));
        assert!(
            !s.shown
                .iter()
                .any(|l| l.starts_with("input: Query prefix") && l.contains('\n'))
        );
    }

    #[test]
    fn advanced_none_clears_the_qwen_default() {
        let mut f = facts();
        f.ollama = Some(vec!["Qwen3-Embedding-0.6B".into()]);
        let (r, s) = run(
            &f,
            with_tail(vec![
                Answer::Select(OLLAMA),
                Answer::Default,
                Answer::Yes,
                Answer::Select(1),
            ]),
        );
        assert_eq!(r.unwrap().embedder.unwrap().query_prefix, "");
        assert!(s.saw("select: Query prefix initial=0 [Keep the default / Instruct:"));
        assert!(s.saw("No prefix / ; Type one / ]"));
        assert!(!s.saw("input: Query prefix"));
    }

    #[test]
    fn advanced_keeps_an_existing_prefix_by_default() {
        let mut f = facts();
        let mut existing = embedder("http://127.0.0.1:8081", "my-model");
        existing.query_prefix = "Q: ".to_string();
        f.existing = Some(existing);
        let (r, _) = run(
            &f,
            with_tail(vec![
                Answer::Select(OTHER),
                text("http://127.0.0.1:8081"),
                text("my-model"),
                Answer::Yes,
                Answer::Default,
            ]),
        );
        assert_eq!(r.unwrap().embedder.unwrap().query_prefix, "Q: ");
    }

    #[test]
    fn other_models_get_no_prefix() {
        let (r, _) = run(&facts(), openai_paste_script());
        assert_eq!(r.unwrap().embedder.unwrap().query_prefix, "");
    }

    #[test]
    fn local_url_asks_no_key() {
        let (r, s) = run(
            &facts(),
            with_tail(vec![
                Answer::Select(OLLAMA),
                text(""),
                text("nomic-embed-text"),
                Answer::No,
            ]),
        );
        let a = r.unwrap();
        assert!(a.embedder.unwrap().token.is_none() && a.pasted.is_none());
        assert!(!s.saw("API key"));
    }

    #[test]
    fn local_found_asks_nothing_more() {
        let (r, s) = run(&facts(), with_tail(vec![Answer::Select(LOCAL)]));
        let a = r.unwrap();
        let e = a.embedder.unwrap();
        assert_eq!(e.url, "http://127.0.0.1:8737");
        assert_eq!(e.model, "qwen3-embedding-0.6b");
        assert_eq!(e.query_prefix, config::default_query_prefix(&e.model));
        assert!(e.token.is_none() && a.pasted.is_none() && a.dims.is_none());
        assert_eq!(a.local, Some(PathBuf::from("/bin/llama-server")));
        assert_eq!(a.timer, Some(15));
        assert!(!s.saw("input:") && !s.saw("API key") && !s.saw("advanced settings"));
        assert!(!s.saw("spin:"));
        assert!(s.saw("Local embedder, run by bilbo / llama-server found, 639 MB download"));
    }

    #[test]
    fn local_missing_asks_the_path() {
        let mut f = facts();
        f.local.llama_server = None;
        let (r, s) = run(&f, with_tail(vec![Answer::Select(LOCAL), text("/bin/sh")]));
        let a = r.unwrap();
        assert_eq!(a.local, Some(PathBuf::from("/bin/sh")));
        assert!(s.saw("llama-server not found, 639 MB download"));
        assert!(
            s.saw("note: llama-server not found\nbilbo runs llama-server but does not install it")
        );
        assert!(s.saw("brew install llama.cpp"));
        assert!(s.saw("input: Path to llama-server default="));
    }

    #[test]
    fn local_missing_empty_path_returns_to_the_list() {
        let mut f = facts();
        f.local.llama_server = None;
        let (r, s) = run(
            &f,
            vec![
                Answer::Select(LOCAL),
                text(""),
                Answer::Select(NONE),
                Answer::Multi(vec![0, 1]),
            ],
        );
        let a = r.unwrap();
        assert!(a.embedder.is_none() && a.local.is_none());
        let lists = s
            .shown
            .iter()
            .filter(|l| l.starts_with("select: Which embedder"))
            .count();
        assert_eq!(lists, 2);
    }

    #[test]
    fn local_missing_path_must_be_executable() {
        assert!(check_program("").is_ok());
        assert!(check_program("/bin/sh").is_ok());
        assert!(check_program("/nonexistent/llama-server").is_err());
        assert!(check_program("llama-server").is_err());
    }

    #[test]
    fn local_unavailable_says_why_and_returns_to_the_list() {
        let mut f = facts();
        f.local.unavailable = Some("port 8737 is in use".to_string());
        let (r, s) = run(
            &f,
            vec![
                Answer::Select(LOCAL),
                Answer::Select(NONE),
                Answer::Multi(vec![0, 1]),
            ],
        );
        assert!(r.unwrap().embedder.is_none());
        assert!(s.saw("Local embedder, run by bilbo / port 8737 is in use"));
        assert!(s.saw("warn: port 8737 is in use"));
        assert!(!s.saw("note: llama-server not found"));
    }

    #[test]
    fn local_preselected_on_rerun() {
        let mut f = facts();
        f.existing = Some(embedder("http://127.0.0.1:8737", "qwen3-embedding-0.6b"));
        let (r, s) = run(&f, with_tail(vec![Answer::Default]));
        assert!(s.saw("select: Which embedder should recall use? initial=1"));
        let a = r.unwrap();
        assert_eq!(a.local, Some(PathBuf::from("/bin/llama-server")));
        assert_eq!(a.embedder.unwrap().min_similarity, 0.5);
    }

    #[test]
    fn local_hint_names_the_download_size() {
        let mut local = facts().local;
        assert_eq!(local_hint(&local), "llama-server found, 639 MB download");
        local.llama_server = None;
        assert_eq!(
            local_hint(&local),
            "llama-server not found, 639 MB download"
        );
        local.unavailable = Some("no service manager".to_string());
        assert_eq!(local_hint(&local), "no service manager");
    }

    #[test]
    fn local_failed_retry_and_keyword_only() {
        let log = std::path::Path::new("/s/bilbo/embedder.log");
        let mut s = Script::new(vec![Answer::Select(0), Answer::Select(1)]);
        assert!(local_failed(&mut s, "server failed", log).unwrap());
        assert!(!local_failed(&mut s, "server failed", log).unwrap());
        assert!(s.saw("warn: server failed\nThe server's log is /s/bilbo/embedder.log."));
        assert!(s.saw("The local embedder failed. What now? initial=0 [Try again / ; Continue with keyword search only / ]"));
    }

    #[test]
    fn local_timer_is_still_asked() {
        let mut f = facts();
        f.timer_minutes = Some(30);
        let (r, s) = run(
            &f,
            vec![
                Answer::Select(LOCAL),
                Answer::Multi(vec![]),
                Answer::Default,
                Answer::Default,
            ],
        );
        assert_eq!(r.unwrap().timer, Some(30));
        assert!(s.saw("Keep the index fresh in the background? initial=1"));
    }

    #[test]
    fn progress_runs_the_work_under_the_bar() {
        let mut s = Script::new(vec![]);
        let seen = std::cell::Cell::new(0);
        let got = s.progress(
            "Downloading",
            10,
            |report| {
                report(10);
                seen.set(1);
                Ok(7)
            },
            |n| format!("done {n}"),
        );
        assert_eq!(got, Ok(7));
        assert_eq!(seen.get(), 1);
        assert!(s.saw("progress: Downloading of 10") && s.saw("progress done: done 7"));
        let err: Result<(), String> =
            s.progress("Downloading", 10, |_| Err("boom".into()), |_| String::new());
        assert_eq!(err, Err("boom".to_string()));
        assert!(s.saw("progress error: boom"));
    }

    #[test]
    fn unset_variable_warns_and_asks_again() {
        let mut script = Script::new(with_tail(vec![
            Answer::Select(OPENAI),
            text(""),
            Answer::No,
            Answer::Select(0),
            text("MISSING_KEY"),
            text("SET_KEY"),
        ]));
        let a = ask(&mut script, &facts(), ok, |n| n == "SET_KEY").unwrap();
        assert_eq!(
            a.embedder.unwrap().token,
            Some(Token::Var("SET_KEY".to_string()))
        );
        assert!(script.saw("warn: MISSING_KEY is not set or is empty in this shell."));
        assert!(script.saw("input: Variable name default=OPENAI_API_KEY"));
    }

    #[test]
    fn pasted_key_points_at_the_token_file() {
        let seen = std::cell::RefCell::new(Vec::new());
        let mut script = Script::new(openai_paste_script());
        let a = ask(
            &mut script,
            &facts(),
            |_: &Embedder, key: Option<&str>| {
                seen.borrow_mut().push(key.map(str::to_string));
                Ok(8)
            },
            |_| true,
        )
        .unwrap();
        assert_eq!(
            a.embedder.unwrap().token,
            Some(Token::File(PathBuf::from("/c/bilbo/token")))
        );
        assert_eq!(a.pasted.as_ref().map(|k| k.as_str()), Some("sk-abc123"));
        assert_eq!(*seen.borrow(), vec![Some("sk-abc123".to_string())]);
        assert!(script.saw("Paste it now / saved to /c/bilbo/token, readable only by you"));
    }

    #[test]
    fn key_file_expands_nothing_for_an_absolute_path() {
        let (r, _) = run(
            &facts(),
            with_tail(vec![
                Answer::Select(OPENAI),
                text(""),
                Answer::No,
                Answer::Select(1),
                text("/k/key"),
            ]),
        );
        assert_eq!(
            r.unwrap().embedder.unwrap().token,
            Some(Token::File(PathBuf::from("/k/key")))
        );
        assert_eq!(
            expand_home("~/k", Some(std::path::Path::new("/h"))),
            PathBuf::from("/h/k")
        );
    }

    #[test]
    fn existing_token_asks_before_replacing() {
        let mut f = facts();
        f.token_exists = true;
        let (r, s) = run(
            &f,
            with_tail(vec![
                Answer::Select(OPENAI),
                text(""),
                Answer::No,
                Answer::Select(2),
                Answer::No,
            ]),
        );
        let a = r.unwrap();
        assert!(a.pasted.is_none());
        assert_eq!(
            a.embedder.unwrap().token,
            Some(Token::File(PathBuf::from("/c/bilbo/token")))
        );
        assert!(s.saw("confirm: Replace the key in /c/bilbo/token?"));
        assert!(!s.saw("password:"));
    }

    #[test]
    fn check_failure_retry() {
        let calls = std::cell::Cell::new(0);
        let mut script = Script::new(with_tail(vec![
            Answer::Select(OPENAI),
            text(""),
            Answer::No,
            Answer::Select(3),
            Answer::Select(0),
        ]));
        let a = ask(
            &mut script,
            &facts(),
            |_: &Embedder, _: Option<&str>| {
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    Err("embedder https://api.openai.com answered 500".to_string())
                } else {
                    Ok(7)
                }
            },
            |_| true,
        )
        .unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(a.dims, Some(7));
        assert!(a.embedder.is_some());
    }

    #[test]
    fn check_failure_change_goes_back() {
        let calls = std::cell::Cell::new(0);
        let mut script = Script::new(with_tail(vec![
            Answer::Select(OPENAI),
            text(""),
            Answer::No,
            Answer::Select(3),
            Answer::Select(1),
            Answer::Select(0),
            Answer::Multi(vec![]),
        ]));
        let a = ask(
            &mut script,
            &facts(),
            |_: &Embedder, _: Option<&str>| {
                calls.set(calls.get() + 1);
                Err("embedder https://api.openai.com answered 500".to_string())
            },
            |_| true,
        )
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert!(a.embedder.is_none());
        assert_eq!(
            script
                .shown
                .iter()
                .filter(|l| l.starts_with("select: Which embedder"))
                .count(),
            2
        );
    }

    #[test]
    fn check_failure_keyword_only() {
        let mut script = Script::new(vec![
            Answer::Select(OPENAI),
            text(""),
            Answer::No,
            Answer::Select(3),
            Answer::Select(2),
            Answer::Multi(vec![0]),
        ]);
        let a = ask(
            &mut script,
            &facts(),
            |_: &Embedder, _: Option<&str>| {
                Err("embedder https://api.openai.com answered 500".to_string())
            },
            |_| true,
        )
        .unwrap();
        assert!(a.embedder.is_none() && a.dims.is_none() && a.timer.is_none());
        assert!(a.claude && !a.codex);
    }

    #[test]
    fn rejected_key_shows_url_and_status_not_the_key() {
        let mut script = Script::new(vec![
            Answer::Select(OPENAI),
            text(""),
            Answer::No,
            Answer::Select(2),
            Answer::Secret("sk-very-secret".to_string()),
            Answer::Select(2),
            Answer::Multi(vec![]),
        ]);
        let a = ask(
            &mut script,
            &facts(),
            |_: &Embedder, _: Option<&str>| {
                Err("embedder https://api.openai.com answered 401".to_string())
            },
            |_| true,
        )
        .unwrap();
        assert!(script.saw("embedder https://api.openai.com answered 401"));
        assert!(!script.saw("sk-very-secret"));
        assert!(a.pasted.is_none());
    }

    #[test]
    fn untick_codex() {
        let (r, _) = run(&facts(), vec![Answer::Select(NONE), Answer::Multi(vec![0])]);
        let a = r.unwrap();
        assert!(a.claude && !a.codex);
    }

    #[test]
    fn only_codex_found_maps_the_tick_back() {
        let mut f = facts();
        f.claude = None;
        let (r, s) = run(&f, vec![Answer::Select(NONE), Answer::Multi(vec![0])]);
        let a = r.unwrap();
        assert!(!a.claude && a.codex);
        assert!(s.saw("Codex / /bin/codex") && !s.saw("Claude Code / "));
    }

    #[test]
    fn no_tools_found_skips_the_question() {
        let mut f = facts();
        f.claude = None;
        f.codex = None;
        let (r, s) = run(&f, vec![Answer::Select(NONE)]);
        let a = r.unwrap();
        assert!(!a.claude && !a.codex);
        assert!(s.saw("info: Neither claude nor codex was found, so the plugin is skipped."));
        assert!(!s.saw("multiselect:"));
    }

    #[test]
    fn timer_choices() {
        let head = || {
            vec![
                Answer::Select(OPENAI),
                text(""),
                Answer::No,
                Answer::Select(3),
                Answer::Multi(vec![]),
            ]
        };
        let mut a15 = head();
        a15.push(Answer::Select(0));
        assert_eq!(run(&facts(), a15).0.unwrap().timer, Some(15));

        let mut a30 = head();
        a30.extend([Answer::Select(1), text("30")]);
        assert_eq!(run(&facts(), a30).0.unwrap().timer, Some(30));

        let mut none = head();
        none.push(Answer::Select(2));
        assert_eq!(run(&facts(), none).0.unwrap().timer, None);

        let mut dflt = head();
        dflt.extend([Answer::Select(1), text("")]);
        assert_eq!(run(&facts(), dflt).0.unwrap().timer, Some(15));
    }

    #[test]
    fn variable_key_warns_about_the_timer() {
        let (r, s) = run(
            &facts(),
            with_tail(vec![
                Answer::Select(OPENAI),
                text(""),
                Answer::No,
                Answer::Select(0),
                text(""),
            ]),
        );
        assert!(r.unwrap().embedder.is_some());
        let warn = s
            .shown
            .iter()
            .position(|l| l.starts_with("warn: The index timer cannot read OPENAI_API_KEY"))
            .unwrap();
        let ask = s
            .shown
            .iter()
            .position(|l| l.starts_with("select: Keep the index fresh"))
            .unwrap();
        assert!(warn < ask);
        assert!(s.saw("Pick \"No timer\""));
    }

    #[test]
    fn existing_config_defaults_reproduce_it() {
        let mut f = facts();
        let existing = Embedder {
            url: "https://api.example.com".to_string(),
            model: "m1".to_string(),
            token: Some(Token::File(PathBuf::from("/k/key"))),
            query_prefix: "p: ".to_string(),
            min_similarity: 0.7,
        };
        f.existing = Some(existing.clone());
        let (r, _) = run(&f, (0..8).map(|_| Answer::Default).collect());
        let a = r.unwrap();
        assert_eq!(a.embedder, Some(existing));
        assert!(a.pasted.is_none());
        assert!(a.claude && a.codex);
    }

    #[test]
    fn existing_loopback_ollama_keeps_its_url() {
        let mut existing = embedder("http://127.0.0.1:11434", "nomic-embed-text");
        existing.query_prefix = String::new();
        for models in [None, Some(vec!["llama3".into(), "nomic-embed-text".into()])] {
            let mut f = facts();
            f.ollama = models;
            f.existing = Some(existing.clone());
            let (r, _) = run(&f, (0..8).map(|_| Answer::Default).collect());
            assert_eq!(r.unwrap().embedder, Some(existing.clone()));
        }
    }

    #[test]
    fn timer_keeps_the_installed_interval() {
        let mut f = facts();
        f.timer_minutes = Some(30);
        let (r, s) = run(
            &f,
            vec![
                Answer::Select(OPENAI),
                text(""),
                Answer::No,
                Answer::Select(3),
                Answer::Multi(vec![]),
                Answer::Default,
                Answer::Default,
            ],
        );
        assert_eq!(r.unwrap().timer, Some(30));
        assert!(s.saw("Keep the index fresh in the background? initial=1"));
        assert!(s.saw("input: Minutes between runs (1 to 1440) default=30"));
    }

    #[test]
    fn managed_config_asks_no_embedder_question() {
        let mut f = facts();
        f.managed = Some("/nix/store/x-config".to_string());
        f.existing = Some(embedder("http://bagend:8081", "qwen3-embedding-0.6b"));
        let (r, s) = run(&f, tail());
        let a = r.unwrap();
        assert_eq!(a.embedder, f.existing);
        assert!(a.dims.is_none());
        assert!(s.saw("note: Config managed elsewhere"));
        assert!(s.saw("/c/bilbo/config links to /nix/store/x-config."));
        assert!(s.saw("embedder.url = http://bagend:8081"));
        assert!(!s.saw("Which embedder"));
        assert_eq!(a.timer, Some(15));
    }

    #[test]
    fn interrupt_at_every_prompt_aborts() {
        let prompts = openai_paste_script().len();
        assert_eq!(prompts, 7);
        for i in 0..prompts {
            let mut answers = openai_paste_script();
            answers[i] = Answer::Interrupt;
            let (r, _) = run(&facts(), answers);
            assert_eq!(
                r.err().map(|e| e.kind()),
                Some(io::ErrorKind::Interrupted),
                "prompt {i}"
            );
        }
    }

    #[test]
    fn end_of_input_aborts() {
        let (r, _) = run(&facts(), vec![]);
        assert_eq!(
            r.err().map(|e| e.kind()),
            Some(io::ErrorKind::UnexpectedEof)
        );
    }

    #[test]
    fn confirm_declined() {
        let summary = vec![
            "Keep the config /c".to_string(),
            "Search by keywords only".to_string(),
        ];
        let mut s = Script::new(vec![Answer::No, Answer::Yes]);
        assert!(!confirm(&mut s, &summary).unwrap());
        assert!(s.saw("note: Setup will\nKeep the config /c\nSearch by keywords only"));
        assert!(confirm(&mut s, &summary).unwrap());
        cancelled(&mut s);
        assert!(s.saw("cancel: Cancelled. Nothing changed."));
    }

    #[test]
    fn confirm_remove_defaults_to_no() {
        let mut s = Script::new(vec![Answer::Default]);
        assert!(!confirm_remove(&mut s, &["x".to_string()]).unwrap());
        assert!(s.saw("note: Setup will remove") && s.saw("Remove these? initial=false"));
    }

    #[test]
    fn first_index_runs_and_shows_the_line() {
        let mut s = Script::new(vec![Answer::Yes]);
        first_index(&mut s, 3, || Ok("indexed 3 notes".to_string())).unwrap();
        assert!(s.saw("confirm: Run the first index now? The store holds 3 notes."));
        assert!(s.saw("spin: Indexing 3 notes"));
        assert!(s.saw("spin done: indexed 3 notes"));
    }

    #[test]
    fn first_index_declined_runs_nothing() {
        let mut s = Script::new(vec![Answer::No]);
        let mut ran = false;
        first_index(&mut s, 3, || {
            ran = true;
            Ok(String::new())
        })
        .unwrap();
        assert!(!ran && !s.saw("spin:"));
    }

    #[test]
    fn first_index_failure_is_only_shown() {
        let mut s = Script::new(vec![Answer::Yes]);
        first_index(&mut s, 1, || Err("embedder gone".to_string())).unwrap();
        assert!(s.saw("spin error: embedder gone"));
    }

    #[test]
    fn finish_says_how_it_went() {
        let mut s = Script::default();
        finish(&mut s, false).unwrap();
        finish(&mut s, true).unwrap();
        assert!(s.saw("outro: Done. The report follows."));
        assert!(s.saw("outro: Some steps failed. The report follows."));
    }

    #[test]
    fn checks_reject_bad_values() {
        assert!(check_url("http://h:1").is_ok());
        assert_eq!(
            check_url("ftp://h").unwrap_err(),
            "Enter an http:// or https:// URL with a host"
        );
        assert_eq!(
            check_url("http://u:p@h").unwrap_err(),
            "Leave the user name and password out of the URL"
        );
        assert!(check_model(" ").is_err() && check_model("m").is_ok());
        assert!(check_var("A_1").is_ok() && check_var("1A").is_err());
        assert!(check_file("/a").is_ok() && check_file("a/b").is_err());
        assert!(check_minutes("1440").is_ok());
        assert!(check_minutes("0").is_err() && check_minutes("1441").is_err());
        assert!(check_minutes("+5").is_err() && check_minutes("x").is_err());
        assert!(check_key("sk-abc").is_ok());
        assert!(check_key("  ").is_err() && check_key("a b").is_err() && check_key("é").is_err());
    }
}
