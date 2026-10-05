//! A scripted `Prompter` for the identity domain's unit tests: answers in order, every shown text recorded.

use crate::host::prompt::{Choice, Prompter};
use std::collections::VecDeque;
use std::io;
use zeroize::Zeroizing;

pub enum Answer {
    Select(usize),
    Text(String),
    Yes,
    No,
    /// The prompt's own initial value or default.
    Default,
    Interrupt,
    /// The word the prompt (`Word 7`) asks for, read off the phrase the last `Recovery phrase` note showed.
    Shown,
    /// That word as its first 4 letters, in capitals, between spaces.
    ShownShort,
    /// A list word that is not that word.
    Wrong,
}

pub fn text(s: &str) -> Answer {
    Answer::Text(s.to_string())
}

#[derive(Default)]
pub struct Script {
    answers: VecDeque<Answer>,
    /// Every shown text, one entry per call, led by the method's name.
    pub shown: Vec<String>,
}

impl Script {
    pub fn new(answers: Vec<Answer>) -> Script {
        Script {
            answers: answers.into(),
            shown: Vec::new(),
        }
    }

    /// How many answers no prompt has taken.
    pub fn left(&self) -> usize {
        self.answers.len()
    }

    pub fn saw(&self, needle: &str) -> bool {
        self.shown.iter().any(|s| s.contains(needle))
    }

    fn next(&mut self) -> io::Result<Answer> {
        match self.answers.pop_front() {
            Some(Answer::Interrupt) => Err(io::ErrorKind::Interrupted.into()),
            Some(answer) => Ok(answer),
            None => Err(io::ErrorKind::UnexpectedEof.into()),
        }
    }

    /// The 12 words of the last `Recovery phrase` note shown, in order.
    pub fn phrase(&self) -> Option<Vec<String>> {
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

    /// What the answer `kind` types at the prompt `Word <n>`.
    fn typed(&self, kind: &Answer, prompt: &str) -> io::Result<String> {
        let n: usize = prompt
            .strip_prefix("Word ")
            .and_then(|n| n.parse().ok())
            .ok_or_else(Script::wrong)?;
        let word = self
            .phrase()
            .and_then(|words| words.get(n.checked_sub(1)?).cloned())
            .ok_or_else(Script::wrong)?;
        Ok(match kind {
            Answer::ShownShort => format!(" {} ", word[..4.min(word.len())].to_uppercase()),
            Answer::Wrong if word == "abandon" => "ability".to_string(),
            Answer::Wrong => "abandon".to_string(),
            _ => word,
        })
    }

    fn wrong() -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, "answer of the wrong kind")
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
    fn select(&mut self, prompt: &str, choices: &[Choice], initial: usize) -> io::Result<usize> {
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
        _choices: &[Choice],
        _initial: &[usize],
    ) -> io::Result<Vec<usize>> {
        self.log(format!("multiselect: {prompt}"));
        Err(Script::wrong())
    }
    /// Like the terminal, a value `check` refuses is logged and the prompt takes the next answer.
    fn input(
        &mut self,
        prompt: &str,
        default: &str,
        check: fn(&str) -> Result<(), String>,
    ) -> io::Result<String> {
        self.log(format!("input: {prompt} default={default}"));
        loop {
            let value = match self.next()? {
                Answer::Text(s) if s.is_empty() => default.to_string(),
                Answer::Text(s) => s,
                Answer::Default => default.to_string(),
                kind @ (Answer::Shown | Answer::ShownShort | Answer::Wrong) => {
                    self.typed(&kind, prompt)?
                }
                _ => return Err(Script::wrong()),
            };
            match check(&value) {
                Ok(()) => return Ok(value),
                Err(m) => self.log(format!("input refused: {prompt}: {m}")),
            }
        }
    }
    fn password(
        &mut self,
        prompt: &str,
        _check: fn(&str) -> Result<(), String>,
    ) -> io::Result<Zeroizing<String>> {
        self.log(format!("password: {prompt}"));
        Err(Script::wrong())
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
    /// Marks where the phrase's screen begins and ends in `shown`.
    fn screen<T>(&mut self, work: impl FnOnce(&mut Self) -> T) -> T {
        self.log("screen: begin".to_string());
        let out = work(self);
        self.log("screen: end".to_string());
        out
    }
}
