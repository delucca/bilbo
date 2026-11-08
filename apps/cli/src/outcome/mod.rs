pub mod failure;
pub mod route;

use std::fmt;

use crate::terminal::View;
use crate::terminal::message::Message;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Success,
    /// Exit 1 with a result: nothing matched or problems found; stdout and diagnostics still print.
    NothingMatched,
}

type Human = Box<dyn Fn(&View) -> String>;

pub struct Outcome {
    plain: String,
    human: Option<Human>,
    diagnostics: Vec<Message>,
    exit: Exit,
}

impl Outcome {
    pub fn new(plain: impl Into<String>) -> Self {
        Self {
            plain: plain.into(),
            human: None,
            diagnostics: Vec::new(),
            exit: Exit::Success,
        }
    }

    #[must_use]
    pub fn with_human(mut self, human: impl Fn(&View) -> String + 'static) -> Self {
        self.human = Some(Box::new(human));
        self
    }

    #[must_use]
    pub fn with_diagnostics(mut self, diagnostics: Vec<Message>) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    #[must_use]
    pub fn with_exit(mut self, exit: Exit) -> Self {
        self.exit = exit;
        self
    }

    pub fn stdout(&self, view: &View) -> String {
        match &self.human {
            Some(human) if view.human => human(view),
            _ => self.plain.clone(),
        }
    }

    pub fn stderr_lines(&self, view: &View) -> Vec<String> {
        self.diagnostics
            .iter()
            .flat_map(|message| message.lines(view))
            .collect()
    }

    pub fn exit_code(&self) -> u8 {
        match self.exit {
            Exit::Success => 0,
            Exit::NothingMatched => 1,
        }
    }
}

impl fmt::Debug for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Outcome")
            .field("plain", &self.plain)
            .field("diagnostics", &self.diagnostics)
            .field("exit", &self.exit)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::{Exit, Outcome};
    use crate::terminal::View;
    use crate::terminal::message::{Detail, Mark, Message};

    fn warning(text: &str) -> Message {
        Message {
            mark: Mark::Warning,
            text: text.to_owned(),
            details: vec![Detail::Text("detail".to_owned())],
        }
    }

    #[test]
    fn stdout_is_the_plain_text_unless_the_human_view_says_otherwise() {
        let same = Outcome::new("hits\n");
        assert_eq!(same.stdout(&View::PLAIN), "hits\n");
        assert_eq!(same.stdout(&View::HUMAN), "hits\n");
        let differ = Outcome::new("hits\n").with_human(|view| {
            if view.paint {
                "\x1b[1mhits\x1b[22m\n".to_owned()
            } else {
                "hits (terminal)\n".to_owned()
            }
        });
        assert_eq!(differ.stdout(&View::PLAIN), "hits\n");
        assert_eq!(differ.stdout(&View::HUMAN), "\x1b[1mhits\x1b[22m\n");
        let unpainted = View {
            paint: false,
            ..View::HUMAN
        };
        assert_eq!(differ.stdout(&unpainted), "hits (terminal)\n");
    }

    #[test]
    fn diagnostics_are_written_in_the_view_of_stderr() {
        let outcome = Outcome::new("").with_diagnostics(vec![warning("first"), warning("second")]);
        assert_eq!(
            outcome.stderr_lines(&View::PLAIN),
            [
                "bilbo: first",
                "bilbo: detail",
                "bilbo: second",
                "bilbo: detail"
            ]
        );
        assert_eq!(
            outcome.stderr_lines(&View::HUMAN),
            [
                "\x1b[33m▲\x1b[39m  first",
                "   \x1b[2mdetail\x1b[22m",
                "\x1b[33m▲\x1b[39m  second",
                "   \x1b[2mdetail\x1b[22m"
            ]
        );
    }

    #[test]
    fn a_result_has_no_diagnostics_and_exits_0_unless_told_otherwise() {
        let outcome = Outcome::new("x");
        assert!(outcome.stderr_lines(&View::PLAIN).is_empty());
        assert_eq!(outcome.exit_code(), 0);
    }

    #[test]
    fn nothing_matched_exits_1_without_a_failure() {
        let outcome = Outcome::new("")
            .with_diagnostics(vec![warning("no note matched")])
            .with_exit(Exit::NothingMatched);
        assert_eq!(outcome.exit_code(), 1);
        assert_eq!(outcome.stdout(&View::PLAIN), "");
        assert_eq!(
            outcome.stderr_lines(&View::PLAIN)[0],
            "bilbo: no note matched"
        );
    }
}
