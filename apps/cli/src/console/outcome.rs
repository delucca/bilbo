use std::fmt;

use bilbo_store::entries::StoreError;

use crate::console::message::{Detail, Message};
use crate::console::settings::{CACHE, ROOT, STATE};
use crate::console::view::View;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Success,
    NothingMatched,
    Refused,
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
            Exit::NothingMatched | Exit::Refused => 1,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Usage {
        reason: String,
        usage: Vec<String>,
        page: String,
    },
    Config(String),
    Refused(String),
    Output(String),
}

impl From<bilbo_settings::error::Error> for Failure {
    fn from(error: bilbo_settings::error::Error) -> Self {
        Self::Config(crate::console::message::sanitize(&error.to_string()))
    }
}

/// A 0.20 root is a setting to change, so it is a config error like any other.
impl From<&StoreError> for Failure {
    fn from(error: &StoreError) -> Self {
        match error {
            StoreError::Older(_) => Self::Config(crate::console::message::sanitize(&format!(
                "{error}; set {} or {} to another folder",
                ROOT.about.variable.unwrap_or_default(),
                ROOT.about.name
            ))),
            StoreError::Inside { folder, .. } => {
                let key = if *folder == "state" { &STATE } else { &CACHE };
                Self::Config(crate::console::message::sanitize(&format!(
                    "{error}; set {} to a folder outside the root",
                    key.about.name
                )))
            }
            _ => Self::Refused(crate::console::message::sanitize(&error.to_string())),
        }
    }
}

impl Failure {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Refused(_) | Self::Output(_) => 1,
            Self::Usage { .. } | Self::Config(_) => 2,
        }
    }

    pub fn message(&self) -> Message {
        match self {
            Self::Usage {
                reason,
                usage,
                page,
            } => {
                let mut message = Message::error(reason.clone());
                message.details = usage.iter().cloned().map(Detail::Text).collect();
                message.details.push(Detail::Command {
                    lead: "see",
                    command: page.clone(),
                });
                message
            }
            Self::Config(text) | Self::Refused(text) | Self::Output(text) => {
                Message::error(text.clone())
            }
        }
    }

    pub fn stderr_lines(&self, view: &View) -> Vec<String> {
        self.message().lines(view)
    }
}

#[cfg(test)]
mod tests {
    use bilbo_settings::error::{At, Error};
    use bilbo_store::entries::StoreError;

    use super::{Exit, Failure, Outcome};
    use crate::console::message::{Detail, Message};
    use crate::console::theme::Mark;
    use crate::console::view::View;

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

    #[test]
    fn usage_and_config_exit_2_refused_and_output_exit_1() {
        let usage = Failure::Usage {
            reason: String::new(),
            usage: Vec::new(),
            page: String::new(),
        };
        assert_eq!(usage.exit_code(), 2);
        assert_eq!(Failure::Config(String::new()).exit_code(), 2);
        assert_eq!(Failure::Refused(String::new()).exit_code(), 1);
        assert_eq!(Failure::Output(String::new()).exit_code(), 1);
    }

    #[test]
    fn a_refused_run_exits_1_without_a_failure() {
        let outcome = Outcome::new("x").with_exit(Exit::Refused);
        assert_eq!(outcome.exit_code(), 1);
        assert_eq!(outcome.stdout(&View::PLAIN), "x");
    }

    #[test]
    fn a_config_error_is_a_config_failure() {
        let failure = Failure::from(Error::Relative {
            at: At::Variable("BILBO_HOME"),
            value: "x".into(),
        });
        assert_eq!(
            failure,
            Failure::Config("BILBO_HOME is not an absolute path: x".to_owned())
        );
        assert_eq!(failure.exit_code(), 2);
    }

    #[test]
    fn a_0_20_root_is_a_config_error_that_names_the_settings_to_change() {
        let failure = Failure::from(&StoreError::Older("/r/notes".into()));
        assert_eq!(
            failure,
            Failure::Config(
                "/r/notes belongs to bilbo 0.20; set BILBO_HOME or paths.root to another folder"
                    .to_owned()
            )
        );
        assert_eq!(failure.exit_code(), 2);
    }

    #[test]
    fn a_state_or_cache_folder_inside_the_root_is_a_config_error_that_names_its_key() {
        for (folder, key) in [("state", "paths.state"), ("cache", "paths.cache")] {
            let failure = Failure::from(&StoreError::Inside {
                folder,
                path: format!("/r/{folder}").into(),
                root: "/r".into(),
            });
            assert_eq!(
                failure,
                Failure::Config(format!(
                    "the {folder} folder /r/{folder} is inside the store root /r, which holds only segments; set {key} to a folder outside the root"
                ))
            );
            assert_eq!(failure.exit_code(), 2);
        }
    }

    #[test]
    fn any_other_store_error_is_refused_and_exits_1() {
        let failure = Failure::from(&StoreError::Unsafe("/r\n".into()));
        assert_eq!(
            failure,
            Failure::Refused("/r\\n can be written by others; bilbo will not use it".to_owned())
        );
        assert_eq!(failure.exit_code(), 1);
    }

    #[test]
    fn a_failures_stderr_lines_follow_the_view() {
        let failure = Failure::Refused("no".to_owned());
        assert_eq!(failure.stderr_lines(&View::PLAIN), ["bilbo: no"]);
        assert_eq!(
            failure.stderr_lines(&View::HUMAN),
            ["\x1b[31m■\x1b[39m  no"]
        );
    }

    #[test]
    fn a_usage_failure_is_the_reason_the_usage_lines_and_the_page() {
        let failure = Failure::Usage {
            reason: "missing <file|folder>".to_owned(),
            usage: vec!["usage: bilbo remember <file|folder>".to_owned()],
            page: "bilbo remember --help".to_owned(),
        };
        let message = failure.message();
        assert_eq!(message.text, "missing <file|folder>");
        assert_eq!(
            message.details,
            [
                Detail::Text("usage: bilbo remember <file|folder>".to_owned()),
                Detail::Command {
                    lead: "see",
                    command: "bilbo remember --help".to_owned()
                }
            ]
        );
    }

    #[test]
    fn a_usage_failure_without_usage_lines_is_the_reason_and_the_page() {
        let failure = Failure::Usage {
            reason: "missing verb; the verbs are remember, config".to_owned(),
            usage: Vec::new(),
            page: "bilbo --help".to_owned(),
        };
        assert_eq!(
            failure.stderr_lines(&View::PLAIN),
            [
                "bilbo: missing verb; the verbs are remember, config",
                "bilbo: see 'bilbo --help'"
            ]
        );
    }
}
