use crate::terminal::View;
use crate::terminal::message::{Detail, Message};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Usage {
        reason: String,
        usage: Vec<String>,
        page: String,
    },
    Config(String),
    /// Exit 1 with no result: a rule refused the input.
    Refused(String),
    Output(String),
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
    use super::Failure;
    use crate::terminal::View;
    use crate::terminal::message::Detail;

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
    fn stderr_lines_follow_the_view() {
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
            reason: "missing verb".to_owned(),
            usage: vec!["usage: bilbo <verb> [<args>]...".to_owned()],
            page: "bilbo --help".to_owned(),
        };
        let message = failure.message();
        assert_eq!(message.text, "missing verb");
        assert_eq!(
            message.details,
            [
                Detail::Text("usage: bilbo <verb> [<args>]...".to_owned()),
                Detail::Command {
                    lead: "see",
                    command: "bilbo --help".to_owned()
                }
            ]
        );
    }
}
