use bilbo_store::id::Id;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Run,
    Empty,
    Refused,
    Failed,
    Busy,
    Stopped,
    Passed,
}

impl Verdict {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Verdict::Run => "run",
            Verdict::Empty => "empty",
            Verdict::Refused => "refused",
            Verdict::Failed => "failed",
            Verdict::Busy => "busy",
            Verdict::Stopped => "stopped",
            Verdict::Passed => "passed",
        }
    }
}

pub trait Log {
    fn append(&mut self, line: &str);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub at: u64,
    pub verdict: Verdict,
    pub kind: String,
    pub parent: Id,
    pub first: u64,
    pub last: u64,
    pub provider: String,
    pub asked: String,
    pub answered: Option<String>,
    pub attempt: Option<u32>,
    pub sent: u64,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub written: Option<u64>,
    pub kept: Option<u64>,
    pub range: Option<u64>,
    pub names: Option<u64>,
    pub millis: u64,
    pub reason: Option<String>,
    pub request: Option<String>,
}

fn text(field: &str) -> String {
    let clean: String = field
        .chars()
        .map(|c| {
            if matches!(c, '\t' | '\n' | '\r') {
                ' '
            } else {
                c
            }
        })
        .collect();
    if clean.is_empty() {
        "-".to_owned()
    } else {
        clean
    }
}

pub(crate) fn number<T: ToString>(field: Option<T>) -> String {
    field.map_or_else(|| "-".to_owned(), |number| number.to_string())
}

#[must_use]
pub fn line(call: &Call) -> String {
    let fields = [
        "d1".to_owned(),
        call.at.to_string(),
        call.verdict.word().to_owned(),
        text(&call.kind),
        call.parent.to_string(),
        format!("{}-{}", call.first, call.last),
        text(&call.provider),
        text(&call.asked),
        text(call.answered.as_deref().unwrap_or("")),
        number(call.attempt),
        call.sent.to_string(),
        number(call.tokens_in),
        number(call.tokens_out),
        number(call.written),
        number(call.kept),
        number(call.range),
        number(call.names),
        call.millis.to_string(),
        text(call.reason.as_deref().unwrap_or("")),
        text(call.request.as_deref().unwrap_or("")),
    ];
    fields.join("\t")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagVerdict {
    Tagged,
    None,
    Refused,
    Failed,
    Busy,
    Stopped,
    Passed,
}

impl TagVerdict {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            TagVerdict::Tagged => "tagged",
            TagVerdict::None => "none",
            TagVerdict::Refused => "refused",
            TagVerdict::Failed => "failed",
            TagVerdict::Busy => "busy",
            TagVerdict::Stopped => "stopped",
            TagVerdict::Passed => "passed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tagged {
    pub at: u64,
    pub verdict: TagVerdict,
    pub kind: String,
    pub entry: Id,
    pub provider: String,
    pub asked: String,
    pub answered: Option<String>,
    pub attempt: Option<u32>,
    pub sent: Option<u64>,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub picks: Option<u64>,
    pub dropped: Option<u64>,
    pub millis: u64,
    pub reason: Option<String>,
    pub request: Option<String>,
}

#[must_use]
pub fn tags_line(call: &Tagged) -> String {
    let fields = [
        "t1".to_owned(),
        call.at.to_string(),
        call.verdict.word().to_owned(),
        text(&call.kind),
        call.entry.to_string(),
        text(&call.provider),
        text(&call.asked),
        text(call.answered.as_deref().unwrap_or("")),
        number(call.attempt),
        number(call.sent),
        number(call.tokens_in),
        number(call.tokens_out),
        number(call.picks),
        number(call.dropped),
        call.millis.to_string(),
        text(call.reason.as_deref().unwrap_or("")),
        text(call.request.as_deref().unwrap_or("")),
    ];
    fields.join("\t")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryVerdict {
    Written,
    Refused,
    Failed,
    Busy,
    Stopped,
    Passed,
    Retired,
}

impl SummaryVerdict {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            SummaryVerdict::Written => "written",
            SummaryVerdict::Refused => "refused",
            SummaryVerdict::Failed => "failed",
            SummaryVerdict::Busy => "busy",
            SummaryVerdict::Stopped => "stopped",
            SummaryVerdict::Passed => "passed",
            SummaryVerdict::Retired => "retired",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summarised {
    pub at: u64,
    pub verdict: SummaryVerdict,
    pub kind: String,
    pub entry: Id,
    pub provider: String,
    pub asked: String,
    pub answered: Option<String>,
    pub attempt: Option<u32>,
    pub sent: Option<u64>,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub characters: Option<u64>,
    pub millis: u64,
    pub reason: Option<String>,
    pub request: Option<String>,
}

#[must_use]
pub fn summary_line(call: &Summarised) -> String {
    let fields = [
        "u1".to_owned(),
        call.at.to_string(),
        call.verdict.word().to_owned(),
        text(&call.kind),
        call.entry.to_string(),
        text(&call.provider),
        text(&call.asked),
        text(call.answered.as_deref().unwrap_or("")),
        number(call.attempt),
        number(call.sent),
        number(call.tokens_in),
        number(call.tokens_out),
        number(call.characters),
        call.millis.to_string(),
        text(call.reason.as_deref().unwrap_or("")),
        text(call.request.as_deref().unwrap_or("")),
    ];
    fields.join("\t")
}

#[cfg(test)]
mod tests {
    use bilbo_store::id::Id;

    use super::{Call, Summarised, SummaryVerdict, Verdict, line, summary_line};

    const PARENT: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DB";

    fn call(verdict: Verdict) -> Call {
        Call {
            at: 1_794_107_564,
            verdict,
            kind: "session".to_owned(),
            parent: Id::parse(PARENT).expect("id"),
            first: 1,
            last: 811,
            provider: "anthropic".to_owned(),
            asked: "model-a".to_owned(),
            answered: None,
            attempt: Some(1),
            sent: 19_874,
            tokens_in: None,
            tokens_out: None,
            written: None,
            kept: None,
            range: None,
            names: None,
            millis: 1500,
            reason: None,
            request: None,
        }
    }

    #[test]
    fn a_log_line_is_d1_and_twenty_fields() {
        let mut run = call(Verdict::Run);
        run.answered = Some("model-b".to_owned());
        run.tokens_in = Some(29_411);
        run.tokens_out = Some(2210);
        run.written = Some(14);
        run.kept = Some(12);
        run.range = Some(1);
        run.names = Some(1);
        assert_eq!(
            line(&run),
            format!(
                "d1\t1794107564\trun\tsession\t{PARENT}\t1-811\tanthropic\tmodel-a\tmodel-b\t1\t19874\t29411\t2210\t14\t12\t1\t1\t1500\t-\t-"
            )
        );
        let mut refused = call(Verdict::Refused);
        refused.reason = Some("refusal".to_owned());
        assert_eq!(
            line(&refused),
            format!(
                "d1\t1794107564\trefused\tsession\t{PARENT}\t1-811\tanthropic\tmodel-a\t-\t1\t19874\t-\t-\t-\t-\t-\t-\t1500\trefusal\t-"
            )
        );
        let mut busy = call(Verdict::Busy);
        busy.attempt = Some(2);
        busy.reason = Some("rate-limit 429 rate_limit_error".to_owned());
        busy.request = Some("req_01".to_owned());
        assert_eq!(
            line(&busy),
            format!(
                "d1\t1794107564\tbusy\tsession\t{PARENT}\t1-811\tanthropic\tmodel-a\t-\t2\t19874\t-\t-\t-\t-\t-\t-\t1500\trate-limit 429 rate_limit_error\treq_01"
            )
        );
        let mut passed = call(Verdict::Passed);
        passed.attempt = None;
        passed.sent = 0;
        passed.millis = 0;
        assert_eq!(
            line(&passed),
            format!(
                "d1\t1794107564\tpassed\tsession\t{PARENT}\t1-811\tanthropic\tmodel-a\t-\t-\t0\t-\t-\t-\t-\t-\t-\t0\t-\t-"
            )
        );
        assert_eq!(line(&run).split('\t').count(), 20);
    }

    #[test]
    fn a_field_with_nothing_to_say_is_a_dash() {
        let mut empty = call(Verdict::Failed);
        empty.provider = String::new();
        empty.asked = "a\tb\nc".to_owned();
        empty.reason = Some(String::new());
        let found = line(&empty);
        let fields: Vec<&str> = found.split('\t').collect();
        assert_eq!(fields.len(), 20);
        assert_eq!(fields[6], "-");
        assert_eq!(fields[7], "a b c");
        assert_eq!(fields[18], "-");
        assert!(!found.contains('\n'));
    }

    #[test]
    fn a_tags_line_is_t1_and_seventeen_fields() {
        let entry = Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DB").expect("id");
        let tagged = super::Tagged {
            at: 1794107564,
            verdict: super::TagVerdict::Tagged,
            kind: "session".to_owned(),
            entry,
            provider: "anthropic".to_owned(),
            asked: "model-a".to_owned(),
            answered: Some("model-b".to_owned()),
            attempt: Some(1),
            sent: Some(2890),
            tokens_in: Some(3120),
            tokens_out: Some(12),
            picks: Some(1),
            dropped: Some(0),
            millis: 1500,
            reason: None,
            request: None,
        };
        assert_eq!(
            super::tags_line(&tagged),
            "t1\t1794107564\ttagged\tsession\t01JC5Q3V7XKM4P8R2T6W9YB0DB\tanthropic\tmodel-a\tmodel-b\t1\t2890\t3120\t12\t1\t0\t1500\t-\t-"
        );
        let failed = super::Tagged {
            verdict: super::TagVerdict::Failed,
            tokens_out: Some(40),
            picks: None,
            dropped: None,
            reason: Some("no-tag strays".to_owned()),
            ..tagged
        };
        let line = super::tags_line(&failed);
        assert_eq!(
            line,
            "t1\t1794107564\tfailed\tsession\t01JC5Q3V7XKM4P8R2T6W9YB0DB\tanthropic\tmodel-a\tmodel-b\t1\t2890\t3120\t40\t-\t-\t1500\tno-tag strays\t-"
        );
        assert_eq!(line.split('\t').count(), 17);
    }

    #[test]
    fn a_summarys_line_is_u1_and_sixteen_fields() {
        let call = |verdict| Summarised {
            at: 1_794_107_564,
            verdict,
            kind: "session".to_owned(),
            entry: Id::parse(PARENT).expect("id"),
            provider: "anthropic".to_owned(),
            asked: "model-a".to_owned(),
            answered: Some("model-b".to_owned()),
            attempt: Some(1),
            sent: Some(412),
            tokens_in: Some(790),
            tokens_out: Some(61),
            characters: Some(74),
            millis: 1830,
            reason: None,
            request: None,
        };
        assert_eq!(
            summary_line(&call(SummaryVerdict::Written)),
            format!(
                "u1\t1794107564\twritten\tsession\t{PARENT}\tanthropic\tmodel-a\tmodel-b\t1\t412\t790\t61\t74\t1830\t-\t-"
            )
        );
        let idle = |verdict| Summarised {
            asked: String::new(),
            answered: None,
            attempt: None,
            sent: None,
            tokens_in: None,
            tokens_out: None,
            characters: None,
            millis: 0,
            ..call(verdict)
        };
        let passed = summary_line(&idle(SummaryVerdict::Passed));
        assert_eq!(
            passed,
            format!(
                "u1\t1794107564\tpassed\tsession\t{PARENT}\tanthropic\t-\t-\t-\t-\t-\t-\t-\t0\t-\t-"
            )
        );
        assert_eq!(
            summary_line(&idle(SummaryVerdict::Retired)),
            passed.replacen("passed", "retired", 1)
        );
        let mut spaced = call(SummaryVerdict::Failed);
        spaced.asked = "a\tb\nc".to_owned();
        assert_eq!(summary_line(&spaced).split('\t').count(), 16);
        assert!(summary_line(&spaced).contains("\ta b c\t"));
    }
}
