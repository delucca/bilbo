use bilbo_derive::instruction::Origin;
use bilbo_derive::plan::State;
use bilbo_derive::run::{Counts, Ended};
use bilbo_llm::model::{Cause, What};
use bilbo_store::id::Id;

use crate::console::message::{Detail, Message, group};
use crate::console::theme::Mark;
use crate::console::view::View;

pub fn counts(counts: &Counts) -> String {
    format!(
        "entries {}, calls {}, notes {}, dropped {}, refused {}, failed {}",
        counts.entries, counts.calls, counts.notes, counts.dropped, counts.refused, counts.failed
    )
}

pub fn plain(totals: &Counts, seconds: u64) -> String {
    format!(
        "{}\ntokens in {}, tokens out {}, seconds {seconds}\n",
        counts(totals),
        totals.tokens_in,
        totals.tokens_out
    )
}

pub fn human(totals: &Counts, seconds: u64, view: &View) -> String {
    let mark = if totals.failed > 0 || totals.refused > 0 {
        Mark::Warning
    } else if totals.notes > 0 {
        Mark::Done
    } else {
        Mark::Info
    };
    let text = if totals.notes > 0 {
        let noun = if totals.notes == 1 { "note" } else { "notes" };
        format!("Wrote {} {noun}", group(totals.notes))
    } else {
        "Wrote no notes".to_owned()
    };
    let message = Message {
        mark,
        text,
        details: vec![
            Detail::Text(format!(
                "{} entries, {} calls, {} dropped, {} refused, {} failed",
                group(totals.entries),
                group(totals.calls),
                group(totals.dropped),
                group(totals.refused),
                group(totals.failed),
            )),
            Detail::Text(format!(
                "{} tokens in, {} tokens out, {} seconds",
                group(totals.tokens_in),
                group(totals.tokens_out),
                group(seconds),
            )),
        ],
    };
    let mut text = message.human_lines(view).join("\n");
    text.push('\n');
    text
}

pub const NO_NOTE: &str = "the answer holds no note bilbo can keep";

pub fn sentence(what: What) -> &'static str {
    match what {
        What::RateLimit => "it limits the rate of requests",
        What::Overloaded => "it is overloaded",
        What::ServerError => "it had an error of its own",
        What::Timeout => "it gave no answer within derive.timeout",
        What::NoConnection => "it could not be reached",
        What::Tls => "the connection could not be made secure",
        What::Unauthorized => "it does not take the key",
        What::Billing => "the account has no credit or reached a spend limit",
        What::Forbidden => "it does not allow this request",
        What::NotFound => "it has no such model or address",
        What::Rejected => "it rejected the request",
        What::NotStarted => "its program could not be started",
        What::Unreadable => "bilbo cannot read what came back",
        What::TooLarge => "what came back is over derive.response",
        What::StopReason => "the answer ended for a reason bilbo does not take",
        What::ToolUsed => "the tool reached a tool of its own",
        What::ToolError => "the tool reported an error",
        What::HoldsKey => "what came back holds the key, and nothing of it was kept",
        What::Cut => "the answer was cut at derive.answer_tokens; raise it or lower derive.window",
    }
}

pub fn tail(cause: &Cause, api: bool) -> String {
    let lead = if api { "HTTP" } else { "exit" };
    let mut head: Vec<String> = Vec::new();
    if let Some(status) = cause.status {
        head.push(status.to_string());
    }
    head.extend(cause.kind.iter().cloned());
    head.extend(cause.code.iter().cloned());
    let mut parts = Vec::new();
    if !head.is_empty() {
        parts.push(format!("{lead} {}", head.join(" ")));
    }
    if let Some(request) = &cause.request {
        parts.push(format!("request {request}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

pub fn what(cause: &Cause, api: bool) -> String {
    if cause.what == What::Cut && !api {
        return format!(
            "the answer was cut by the tool; lower derive.window{}",
            tail(cause, api)
        );
    }
    if cause.what == What::Unauthorized && !api {
        return format!(
            "the tool is not logged in, or its login is no longer taken{}",
            tail(cause, api)
        );
    }
    if cause.what == What::Billing && !api {
        return format!(
            "the plan's usage limit is reached, or the account has no credit{}",
            tail(cause, api)
        );
    }
    format!("{}{}", sentence(cause.what), tail(cause, api))
}

pub fn stopped(cause: &Cause, api: bool, why: Option<&str>) -> String {
    match (cause.what, why) {
        (What::NotStarted, Some(why)) => {
            format!("{}: {why}{}", sentence(cause.what), tail(cause, api))
        }
        _ => what(cause, api),
    }
}

pub fn why_after(ended: &Ended, why: Option<&str>) -> Option<String> {
    let own = matches!(ended, Ended::Stopped(cause) if cause.what == What::NotStarted);
    why.filter(|_| !own).map(str::to_owned)
}

pub fn place(extent: &bilbo_derive::tries::Extent) -> String {
    format!("{} #r{}-{}", extent.parent, extent.first, extent.last)
}

pub fn state_text(state: State) -> &'static str {
    match state {
        State::Due => "due",
        State::Covered => "covered",
        State::PassedOver => "passed over",
        State::StillGrowing => "still growing",
        State::LabelsNotCurrent => "left out: labels not current",
        State::NoInstruction => "left out: no instruction",
        State::BelongsToAnother => "left out: belongs to another entry",
        State::NotNamed => "left out: not named by derive.in",
        State::LeftOut => "left out: derive.not_in",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Noted {
    pub entry: Id,
    pub note_entry: Option<Id>,
    pub notes: u64,
}

pub fn entry_line(state: State, noted: &Noted) -> String {
    let held = noted
        .note_entry
        .map_or_else(|| "-".to_owned(), |id| id.to_string());
    format!(
        "entry\t{}\t{}\t{held}\t{}",
        noted.entry,
        state_text(state),
        noted.notes
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindRow {
    pub kind: String,
    pub origin: Origin,
    pub instruction: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotDue {
    pub growing: u64,
    pub covered: u64,
    pub belongs: u64,
    pub passed: u64,
    pub labels: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dry {
    pub off: Vec<String>,
    pub kinds: Vec<KindRow>,
    pub named: u64,
    pub left_out: u64,
    pub due: u64,
    pub not_due: NotDue,
    pub calls: u64,
    pub entries: u64,
    pub tokens: u64,
    pub limit: usize,
    pub provider: Vec<String>,
    pub entry_lines: Vec<String>,
}

fn origin(origin: &Origin) -> String {
    match origin {
        Origin::Shipped => "built in".to_owned(),
        Origin::File(path) => path.display().to_string(),
    }
}

pub fn lines(dry: &Dry) -> Vec<String> {
    let mut lines: Vec<String> = dry
        .off
        .iter()
        .map(|text| format!("off: {text}; a run would send nothing"))
        .collect();
    let kinds: Vec<String> = dry
        .kinds
        .iter()
        .map(|row| {
            format!(
                "{} ({}, {})",
                row.kind,
                origin(&row.origin),
                row.instruction
            )
        })
        .collect();
    let kinds = if kinds.is_empty() {
        "none".to_owned()
    } else {
        kinds.join(", ")
    };
    lines.push(format!("kinds: {kinds}"));
    lines.push(format!(
        "named: {} entries by derive.in, {} of them left out by derive.not_in",
        dry.named, dry.left_out
    ));
    lines.push(format!("due: {} entries", dry.due));
    let due = &dry.not_due;
    let items: Vec<String> = [
        (due.growing, "still growing"),
        (due.covered, "covered"),
        (due.belongs, "belong to another entry"),
        (due.passed, "passed over"),
        (due.labels, "with labels not current"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, what)| format!("{count} {what}"))
    .collect();
    if !items.is_empty() {
        lines.push(format!("not due: {}", items.join(", ")));
    }
    lines.push(format!(
        "this run: {} calls for {} entries, {} tokens of records to send (derive.calls {})",
        dry.calls, dry.entries, dry.tokens, dry.limit
    ));
    lines.extend(dry.provider.iter().cloned());
    lines.extend(dry.entry_lines.iter().cloned());
    lines
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bilbo_derive::instruction::Origin;
    use bilbo_derive::plan::State;
    use bilbo_derive::run::{Counts, Ended};
    use bilbo_llm::model::{Cause, What};
    use bilbo_store::id::Id;

    use super::{
        Dry, KindRow, NotDue, Noted, counts, entry_line, lines, plain, sentence, stopped, tail,
        what, why_after,
    };

    #[test]
    fn the_counts_are_two_lines_of_plain_digits() {
        let totals = Counts {
            entries: 3,
            calls: 4,
            notes: 31,
            dropped: 2,
            refused: 0,
            failed: 0,
            passed: 5,
            tokens_in: 91806,
            tokens_out: 7340,
        };
        assert_eq!(
            plain(&totals, 48),
            "entries 3, calls 4, notes 31, dropped 2, refused 0, failed 0\n\
             tokens in 91806, tokens out 7340, seconds 48\n"
        );
        assert_eq!(
            plain(&Counts::default(), 0),
            "entries 0, calls 0, notes 0, dropped 0, refused 0, failed 0\n\
             tokens in 0, tokens out 0, seconds 0\n"
        );
        assert_eq!(counts(&totals).lines().count(), 1);
    }

    #[test]
    fn each_what_has_its_sentence_and_the_tail_holds_the_parts_there_are() {
        let all = [
            (What::RateLimit, "it limits the rate of requests"),
            (What::Overloaded, "it is overloaded"),
            (What::ServerError, "it had an error of its own"),
            (What::Timeout, "it gave no answer within derive.timeout"),
            (What::NoConnection, "it could not be reached"),
            (What::Tls, "the connection could not be made secure"),
            (What::Unauthorized, "it does not take the key"),
            (
                What::Billing,
                "the account has no credit or reached a spend limit",
            ),
            (What::Forbidden, "it does not allow this request"),
            (What::NotFound, "it has no such model or address"),
            (What::Rejected, "it rejected the request"),
            (What::NotStarted, "its program could not be started"),
            (What::Unreadable, "bilbo cannot read what came back"),
            (What::TooLarge, "what came back is over derive.response"),
            (
                What::StopReason,
                "the answer ended for a reason bilbo does not take",
            ),
            (What::ToolUsed, "the tool reached a tool of its own"),
            (What::ToolError, "the tool reported an error"),
            (
                What::HoldsKey,
                "what came back holds the key, and nothing of it was kept",
            ),
            (
                What::Cut,
                "the answer was cut at derive.answer_tokens; raise it or lower derive.window",
            ),
        ];
        let said: Vec<(What, &str)> = all
            .iter()
            .map(|(what, _)| (*what, sentence(*what)))
            .collect();
        assert_eq!(said, all);
        let full = Cause {
            what: What::NotFound,
            status: Some(404),
            kind: Some("not_found_error".to_owned()),
            code: Some("model_gone".to_owned()),
            request: Some("req_test1".to_owned()),
        };
        assert_eq!(
            tail(&full, true),
            " (HTTP 404 not_found_error model_gone, request req_test1)"
        );
        assert_eq!(
            tail(&full, false),
            " (exit 404 not_found_error model_gone, request req_test1)"
        );
        let some = Cause {
            kind: None,
            code: None,
            request: None,
            ..full.clone()
        };
        assert_eq!(tail(&some, true), " (HTTP 404)");
        let request = Cause {
            status: None,
            kind: None,
            code: None,
            ..full
        };
        assert_eq!(tail(&request, false), " (request req_test1)");
        assert_eq!(tail(&Cause::new(What::Cut), true), "");
    }

    #[test]
    fn the_dry_runs_lines_come_in_their_order_and_an_empty_one_is_absent() {
        let mut dry = Dry {
            kinds: vec![
                KindRow {
                    kind: "session".to_owned(),
                    origin: Origin::Shipped,
                    instruction: "9f86d081884c7d65".to_owned(),
                },
                KindRow {
                    kind: "markdown".to_owned(),
                    origin: Origin::File(PathBuf::from("/home/me/notes/markdown.txt")),
                    instruction: "4c1a70b9e2d355f0".to_owned(),
                },
            ],
            named: 412,
            left_out: 14,
            due: 37,
            not_due: NotDue {
                growing: 12,
                covered: 340,
                belongs: 9,
                passed: 2,
                labels: 1,
            },
            calls: 20,
            entries: 14,
            tokens: 380_122,
            limit: 20,
            provider: vec!["provider: anthropic, via api, model m, at https://x".to_owned()],
            entry_lines: vec!["entry\tA\tcovered\t-\t0".to_owned()],
            ..Dry::default()
        };
        assert_eq!(
            lines(&dry),
            [
                "kinds: session (built in, 9f86d081884c7d65), markdown (/home/me/notes/markdown.txt, 4c1a70b9e2d355f0)",
                "named: 412 entries by derive.in, 14 of them left out by derive.not_in",
                "due: 37 entries",
                "not due: 12 still growing, 340 covered, 9 belong to another entry, 2 passed over, 1 with labels not current",
                "this run: 20 calls for 14 entries, 380122 tokens of records to send (derive.calls 20)",
                "provider: anthropic, via api, model m, at https://x",
                "entry\tA\tcovered\t-\t0",
            ]
        );
        dry.off = vec!["derive.enabled is false".to_owned()];
        dry.not_due = NotDue::default();
        dry.kinds.clear();
        dry.provider.clear();
        dry.entry_lines.clear();
        assert_eq!(
            lines(&dry),
            [
                "off: derive.enabled is false; a run would send nothing",
                "kinds: none",
                "named: 412 entries by derive.in, 14 of them left out by derive.not_in",
                "due: 37 entries",
                "this run: 20 calls for 14 entries, 380122 tokens of records to send (derive.calls 20)",
            ]
        );
    }

    #[test]
    fn an_entry_line_is_five_fields_by_tabs() {
        let parent = Id::from_bytes([1; 16]);
        let notes = Id::from_bytes([2; 16]);
        for (state, text) in [
            (State::Due, "due"),
            (State::Covered, "covered"),
            (State::PassedOver, "passed over"),
            (State::LabelsNotCurrent, "left out: labels not current"),
            (State::NoInstruction, "left out: no instruction"),
            (
                State::BelongsToAnother,
                "left out: belongs to another entry",
            ),
            (State::NotNamed, "left out: not named by derive.in"),
            (State::LeftOut, "left out: derive.not_in"),
        ] {
            let line = entry_line(
                state,
                &Noted {
                    entry: parent,
                    note_entry: Some(notes),
                    notes: 12,
                },
            );
            assert_eq!(line, format!("entry\t{parent}\t{text}\t{notes}\t12"));
            assert_eq!(line.split('\t').count(), 5);
        }
        assert_eq!(
            entry_line(
                State::Covered,
                &Noted {
                    entry: parent,
                    note_entry: None,
                    notes: 0
                }
            ),
            format!("entry\t{parent}\tcovered\t-\t0")
        );
    }

    #[test]
    fn a_cut_answer_of_the_tool_names_the_window() {
        let cut = Cause::new(What::Cut);
        assert_eq!(
            what(&cut, true),
            "the answer was cut at derive.answer_tokens; raise it or lower derive.window"
        );
        assert_eq!(
            what(&cut, false),
            "the answer was cut by the tool; lower derive.window"
        );
    }

    #[test]
    fn a_lost_login_and_a_spent_plan_of_the_tool_have_their_own_words() {
        let unauthorized = Cause::new(What::Unauthorized);
        assert_eq!(
            what(&unauthorized, false),
            "the tool is not logged in, or its login is no longer taken"
        );
        assert_eq!(what(&unauthorized, true), "it does not take the key");
        let coded = Cause {
            status: Some(1),
            code: Some("401".to_owned()),
            ..Cause::new(What::Unauthorized)
        };
        assert_eq!(
            what(&coded, false),
            "the tool is not logged in, or its login is no longer taken (exit 1 401)"
        );
        let billing = Cause::new(What::Billing);
        assert_eq!(
            what(&billing, false),
            "the plan's usage limit is reached, or the account has no credit"
        );
        assert_eq!(
            what(&billing, true),
            "the account has no credit or reached a spend limit"
        );
        assert_eq!(
            stopped(&unauthorized, false, Some("x")),
            "the tool is not logged in, or its login is no longer taken"
        );
        assert_eq!(
            why_after(&Ended::Stopped(unauthorized), Some("x")),
            Some("x".to_owned())
        );
    }

    #[test]
    fn why_a_call_gave_no_answer_is_said_after_the_runs_own_lines() {
        let not_started = Cause::new(What::NotStarted);
        let ended = Ended::Stopped(not_started.clone());
        assert_eq!(
            stopped(&not_started, false, Some("the work folder is not empty")),
            "its program could not be started: the work folder is not empty"
        );
        assert_eq!(
            stopped(&not_started, false, None),
            "its program could not be started"
        );
        assert_eq!(
            why_after(&ended, Some("the work folder is not empty")),
            None
        );
        assert_eq!(
            why_after(&Ended::Failures, Some("a reply lacks num_turns")),
            Some("a reply lacks num_turns".to_owned())
        );
        assert_eq!(why_after(&Ended::Failures, None), None);
    }
}
