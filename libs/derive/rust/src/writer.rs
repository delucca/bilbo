use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub instruction: String,
    pub material: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub answer_tokens: usize,
    pub timeout: Duration,
    pub response: u64,
    pub poll: Duration,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub read: Option<u64>,
    pub written: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    RateLimit,
    Overloaded,
    ServerError,
    Timeout,
    NoConnection,
    Tls,
    Unauthorized,
    Billing,
    Forbidden,
    NotFound,
    Rejected,
    NotStarted,
    Unreadable,
    TooLarge,
    StopReason,
    ToolUsed,
    ToolError,
    HoldsKey,
    Cut,
    NoNote,
}

impl What {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            What::RateLimit => "rate-limit",
            What::Overloaded => "overloaded",
            What::ServerError => "server-error",
            What::Timeout => "timeout",
            What::NoConnection => "no-connection",
            What::Tls => "tls",
            What::Unauthorized => "unauthorized",
            What::Billing => "billing",
            What::Forbidden => "forbidden",
            What::NotFound => "not-found",
            What::Rejected => "rejected",
            What::NotStarted => "not-started",
            What::Unreadable => "unreadable",
            What::TooLarge => "too-large",
            What::StopReason => "stop-reason",
            What::ToolUsed => "tool-used",
            What::ToolError => "tool-error",
            What::HoldsKey => "holds-key",
            What::Cut => "cut",
            What::NoNote => "no-note",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Whose {
    Window,
    Provider,
    Either,
}

impl What {
    #[must_use]
    pub fn whose(self) -> Whose {
        match self {
            What::NoNote | What::HoldsKey | What::ToolUsed => Whose::Window,
            What::RateLimit
            | What::Overloaded
            | What::ServerError
            | What::NoConnection
            | What::Tls
            | What::Unauthorized
            | What::Billing
            | What::Forbidden
            | What::NotFound
            | What::NotStarted => Whose::Provider,
            What::Timeout
            | What::Rejected
            | What::Unreadable
            | What::TooLarge
            | What::Cut
            | What::StopReason
            | What::ToolError => Whose::Either,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cause {
    pub what: What,
    pub status: Option<i32>,
    pub kind: Option<String>,
    pub code: Option<String>,
    pub request: Option<String>,
}

impl Cause {
    #[must_use]
    pub fn new(what: What) -> Cause {
        Cause {
            what,
            status: None,
            kind: None,
            code: None,
            request: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Answer {
        text: String,
        usage: Usage,
        model: Option<String>,
    },
    Cut(Usage),
    Refused {
        category: Option<String>,
        usage: Usage,
    },
    Busy {
        wait: Option<Duration>,
        cause: Cause,
    },
    Stopped(Cause),
    Failed {
        cause: Cause,
        usage: Usage,
    },
}

pub type Variables = Box<dyn Fn(&str) -> Option<String>>;

pub trait Writer {
    fn prepare(&mut self) -> Result<(), String>;
    fn write(&mut self, prompt: &Prompt, limits: &Limits) -> Outcome;
    fn model(&self) -> &str;
    fn describe(&self) -> Vec<String>;

    fn why(&self) -> Option<&str> {
        None
    }
}

const IDENTIFIER_BYTES: usize = 64;
const MODEL_CHARACTERS: usize = 200;

pub const MODEL_ALLOWED: &str = "a model's id of at most 200 characters with no space or control character, or the empty text for none";
pub const MODEL_REFUSED: &str = "must be a model's id of at most 200 characters with no space or control character, or the empty text for none";

#[must_use]
pub fn is_identifier(text: &str) -> bool {
    (1..=IDENTIFIER_BYTES).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

#[must_use]
pub fn identifier(text: &str) -> String {
    if is_identifier(text) {
        text.to_owned()
    } else {
        "unknown".to_owned()
    }
}

#[must_use]
pub fn is_model_name(text: &str) -> bool {
    text.chars().count() <= MODEL_CHARACTERS
        && !text.chars().any(|c| c.is_whitespace() || c.is_control())
}

#[must_use]
pub fn answered_model(text: &str) -> String {
    if !text.is_empty() && is_model_name(text) {
        text.to_owned()
    } else {
        "unknown".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{What, Whose, identifier};

    #[test]
    fn a_vendors_text_is_a_short_identifier_or_unknown() {
        let sixty_four = "a".repeat(64);
        for (text, kept) in [
            ("invalid_request_error", "invalid_request_error"),
            ("req_01A-b.c", "req_01A-b.c"),
            (sixty_four.as_str(), sixty_four.as_str()),
            ("", "unknown"),
            (&"a".repeat(65), "unknown"),
            ("a b", "unknown"),
            ("a\tb", "unknown"),
            ("a\nb", "unknown"),
            ("a\"b", "unknown"),
            ("é", "unknown"),
        ] {
            assert_eq!(identifier(text), kept, "{text:?}");
        }
    }

    #[test]
    fn each_cause_is_the_windows_the_providers_or_eithers() {
        let window = [What::NoNote, What::HoldsKey, What::ToolUsed];
        let provider = [
            What::RateLimit,
            What::Overloaded,
            What::ServerError,
            What::NoConnection,
            What::Tls,
            What::Unauthorized,
            What::Billing,
            What::Forbidden,
            What::NotFound,
            What::NotStarted,
        ];
        let either = [
            What::Timeout,
            What::Rejected,
            What::Unreadable,
            What::TooLarge,
            What::Cut,
            What::StopReason,
            What::ToolError,
        ];
        for (whose, causes) in [
            (Whose::Window, &window[..]),
            (Whose::Provider, &provider[..]),
            (Whose::Either, &either[..]),
        ] {
            for what in causes {
                assert_eq!(what.whose(), whose, "{}", what.word());
            }
        }
        assert_eq!(window.len() + provider.len() + either.len(), 20);
        assert_eq!(What::Tls.word(), "tls");
    }
}
