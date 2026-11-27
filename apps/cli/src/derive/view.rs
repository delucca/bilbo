use bilbo_derive::instruction::Origin;
use bilbo_derive::plan::{State, SummaryState, TagsState};
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
pub const NO_TAG: &str = "the answer names no tag bilbo can keep";
pub const NO_SUMMARY: &str = "the answer holds no summary bilbo can keep";
pub const NONE_TO_PICK: &str = "not due: none to pick from";
pub const BOTH: &str = "the answer picks a tag and says none fits";

pub fn sentence(what: What) -> &'static str {
    match what {
        What::RateLimit => "it limits the rate of requests",
        What::Overloaded => "it is overloaded",
        What::ServerError => "it had an error of its own",
        What::Timeout => "it gave no answer within llm.timeout",
        What::NoConnection => "it could not be reached",
        What::Tls => "the connection could not be made secure",
        What::Unauthorized => "it does not take the key",
        What::Billing => "the account has no credit or reached a spend limit",
        What::Forbidden => "it does not allow this request",
        What::NotFound => "it has no such model or address",
        What::Rejected => "it rejected the request",
        What::NotStarted => "its program could not be started",
        What::Unreadable => "bilbo cannot read what came back",
        What::TooLarge => "what came back is over llm.response",
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

pub fn tags_what(cause: &Cause, api: bool) -> String {
    if cause.what != What::Cut {
        return what(cause, api);
    }
    let sentence = if api {
        "the answer was cut at derive.answer_tokens; raise it or lower derive.tags.notes"
    } else {
        "the answer was cut by the tool; lower derive.tags.notes"
    };
    format!("{sentence}{}", tail(cause, api))
}

pub fn summary_what(cause: &Cause, api: bool) -> String {
    if cause.what != What::Cut {
        return what(cause, api);
    }
    let sentence = if api {
        "the answer was cut at derive.answer_tokens; raise it or lower derive.summary.window"
    } else {
        "the answer was cut by the tool; lower derive.summary.window"
    };
    format!("{sentence}{}", tail(cause, api))
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
    let own = matches!(ended, Ended::Stopped(cause, _) if cause.what == What::NotStarted);
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
pub struct TagsDry {
    pub due: u64,
    pub calls: u64,
    pub tokens: u64,
    pub pick: usize,
    pub model: Option<String>,
    pub instruction: Option<(Origin, String)>,
    pub lines: Vec<String>,
}

pub fn tags_state(state: Option<TagsState>, entry: State) -> String {
    match (state, entry) {
        (Some(TagsState::Due), _) => "due".to_owned(),
        (Some(TagsState::Asked), _) => "asked".to_owned(),
        (Some(TagsState::PassedOver), _) => "passed over".to_owned(),
        (Some(TagsState::NotComplete), _) => "not due: notes not complete".to_owned(),
        (Some(TagsState::NoNote), _) => "not due: no note".to_owned(),
        (None, state) => format!("not due: {}", state_text(state)),
    }
}

pub fn tags_ref_line(entry: Id, state: &str, held: &[String]) -> String {
    let held = if held.is_empty() {
        "-".to_owned()
    } else {
        held.join(",")
    };
    format!("tags\t{entry}\t{state}\t{held}")
}

pub fn tags_line(tags: &TagsDry) -> String {
    if tags.pick == 0 {
        return "tags: none to pick from: derive.tags.pick is empty".to_owned();
    }
    let mut line = format!(
        "tags: {} entries due; this run {} calls, {} tokens of notes to send; {} tags to pick from",
        tags.due, tags.calls, tags.tokens, tags.pick
    );
    if let Some((held, id)) = &tags.instruction {
        line.push_str(&format!("; instruction ({}, {id})", origin(held)));
    }
    if let Some(model) = &tags.model {
        line.push_str(&format!("; model {model}"));
    }
    line
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummariesDry {
    pub due: u64,
    pub calls: u64,
    pub tokens: u64,
    pub instruction: (Origin, String),
    pub lines: Vec<String>,
}

pub fn summary_state(state: SummaryState) -> String {
    match state {
        SummaryState::Due => "due".to_owned(),
        SummaryState::Written => "written".to_owned(),
        SummaryState::PassedOver => "passed over".to_owned(),
        SummaryState::NotComplete => "not due: notes not complete".to_owned(),
        SummaryState::NoNote => "not due: no note".to_owned(),
        SummaryState::Short => "not due: short".to_owned(),
        SummaryState::NoReader => "not due: no reader reads its kind".to_owned(),
        SummaryState::Forgotten => "not due: forgotten".to_owned(),
        SummaryState::Out(state) => format!("not due: {}", state_text(state)),
    }
}

pub fn summary_ref_line(entry: Id, state: &str) -> String {
    format!("summary\t{entry}\t{state}")
}

pub fn summaries_line(summaries: &SummariesDry) -> String {
    let (held, id) = &summaries.instruction;
    format!(
        "summaries: {} entries due; this run {} calls, {} tokens to send; instruction ({}, {id})",
        summaries.due,
        summaries.calls,
        summaries.tokens,
        origin(held)
    )
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
    pub tags: Option<TagsDry>,
    pub summaries: Option<SummariesDry>,
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
    lines.extend(dry.tags.iter().map(tags_line));
    lines.extend(dry.summaries.iter().map(summaries_line));
    lines.extend(dry.provider.iter().cloned());
    lines.extend(dry.entry_lines.iter().cloned());
    lines.extend(dry.tags.iter().flat_map(|tags| tags.lines.iter().cloned()));
    lines.extend(
        dry.summaries
            .iter()
            .flat_map(|summaries| summaries.lines.iter().cloned()),
    );
    lines
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bilbo_derive::instruction::Origin;
    use bilbo_derive::plan::State;
    use bilbo_derive::run::{Counts, Ended, Which};
    use bilbo_llm::model::{Cause, What};
    use bilbo_store::id::Id;

    use super::{
        Dry, KindRow, NotDue, Noted, counts, entry_line, lines, plain, sentence, stopped,
        summary_what, tail, what, why_after,
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
            (What::Timeout, "it gave no answer within llm.timeout"),
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
            (What::TooLarge, "what came back is over llm.response"),
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
    fn a_cut_summary_says_which_key_to_lower() {
        let cut = Cause::new(What::Cut);
        assert_eq!(
            summary_what(&cut, true),
            "the answer was cut at derive.answer_tokens; raise it or lower derive.summary.window"
        );
        assert_eq!(
            summary_what(&cut, false),
            "the answer was cut by the tool; lower derive.summary.window"
        );
        let other = Cause::new(What::Timeout);
        assert_eq!(summary_what(&other, true), what(&other, true));
        assert_eq!(summary_what(&other, false), what(&other, false));
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
            why_after(&Ended::Stopped(unauthorized, Which::Writer), Some("x")),
            Some("x".to_owned())
        );
    }

    #[test]
    fn why_a_call_gave_no_answer_is_said_after_the_runs_own_lines() {
        let not_started = Cause::new(What::NotStarted);
        let ended = Ended::Stopped(not_started.clone(), Which::Writer);
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
            why_after(
                &Ended::Failures(Which::Writer),
                Some("a reply lacks num_turns")
            ),
            Some("a reply lacks num_turns".to_owned())
        );
        assert_eq!(why_after(&Ended::Failures(Which::Writer), None), None);
    }

    #[test]
    fn the_dry_runs_tags_line_has_its_two_forms_its_instruction_and_its_model() {
        use super::{TagsDry, tags_line};
        let mut tags = TagsDry {
            due: 37,
            calls: 16,
            tokens: 17212,
            pick: 5,
            instruction: Some((Origin::Shipped, "a5d31572b3105b1b".to_owned())),
            ..TagsDry::default()
        };
        assert_eq!(
            tags_line(&tags),
            "tags: 37 entries due; this run 16 calls, 17212 tokens of notes to send; 5 tags to pick from; instruction (built in, a5d31572b3105b1b)"
        );
        tags.instruction = Some((
            Origin::File(PathBuf::from("/home/me/notes/tags.txt")),
            "4c1a70b9e2d355f0".to_owned(),
        ));
        tags.model = Some("claude-haiku-5-5".to_owned());
        assert_eq!(
            tags_line(&tags),
            "tags: 37 entries due; this run 16 calls, 17212 tokens of notes to send; 5 tags to pick from; instruction (/home/me/notes/tags.txt, 4c1a70b9e2d355f0); model claude-haiku-5-5"
        );
        assert_eq!(
            tags_line(&TagsDry::default()),
            "tags: none to pick from: derive.tags.pick is empty"
        );
    }

    #[test]
    fn a_refs_tags_line_is_four_fields_by_tabs() {
        use super::{tags_ref_line, tags_state};
        use bilbo_derive::plan::TagsState;
        let entry = Id::from_bytes([1; 16]);
        for (state, entry_state, words) in [
            (Some(TagsState::Due), State::Due, "due"),
            (Some(TagsState::Asked), State::Covered, "asked"),
            (Some(TagsState::PassedOver), State::Covered, "passed over"),
            (
                Some(TagsState::NotComplete),
                State::Covered,
                "not due: notes not complete",
            ),
            (Some(TagsState::NoNote), State::Covered, "not due: no note"),
            (None, State::LeftOut, "not due: left out: derive.not_in"),
            (None, State::StillGrowing, "not due: still growing"),
        ] {
            assert_eq!(tags_state(state, entry_state), words);
        }
        assert_eq!(
            tags_ref_line(entry, "due", &["a".to_owned(), "b".to_owned()]),
            format!("tags\t{entry}\tdue\ta,b")
        );
        assert_eq!(
            tags_ref_line(entry, "asked", &[]),
            format!("tags\t{entry}\tasked\t-")
        );
        assert_eq!(
            tags_ref_line(entry, super::NONE_TO_PICK, &["garden".to_owned()]),
            format!("tags\t{entry}\tnot due: none to pick from\tgarden")
        );
    }

    #[test]
    fn each_state_of_a_summary_has_its_words() {
        use super::summary_state;
        use bilbo_derive::plan::SummaryState;
        for (state, words) in [
            (SummaryState::Due, "due"),
            (SummaryState::Written, "written"),
            (SummaryState::PassedOver, "passed over"),
            (SummaryState::NotComplete, "not due: notes not complete"),
            (SummaryState::NoNote, "not due: no note"),
            (SummaryState::Short, "not due: short"),
            (SummaryState::NoReader, "not due: no reader reads its kind"),
            (SummaryState::Forgotten, "not due: forgotten"),
            (
                SummaryState::Out(State::LabelsNotCurrent),
                "not due: left out: labels not current",
            ),
            (
                SummaryState::Out(State::BelongsToAnother),
                "not due: left out: belongs to another entry",
            ),
            (
                SummaryState::Out(State::NotNamed),
                "not due: left out: not named by derive.in",
            ),
            (
                SummaryState::Out(State::LeftOut),
                "not due: left out: derive.not_in",
            ),
            (
                SummaryState::Out(State::StillGrowing),
                "not due: still growing",
            ),
        ] {
            assert_eq!(summary_state(state), words);
        }
    }

    #[test]
    fn the_summaries_line_counts_what_is_due_and_names_the_instruction() {
        use super::{SummariesDry, summaries_line, summary_ref_line};
        let mut summaries = SummariesDry {
            due: 731,
            calls: 6,
            tokens: 14210,
            instruction: (Origin::Shipped, "7fdf9f0d93f443e3".to_owned()),
            lines: Vec::new(),
        };
        assert_eq!(
            summaries_line(&summaries),
            "summaries: 731 entries due; this run 6 calls, 14210 tokens to send; instruction (built in, 7fdf9f0d93f443e3)"
        );
        summaries.instruction = (
            Origin::File(PathBuf::from("/home/me/notes/summary.txt")),
            "4c1a70b9e2d355f0".to_owned(),
        );
        assert_eq!(
            summaries_line(&summaries),
            "summaries: 731 entries due; this run 6 calls, 14210 tokens to send; instruction (/home/me/notes/summary.txt, 4c1a70b9e2d355f0)"
        );
        let entry = Id::from_bytes([1; 16]);
        assert_eq!(
            summary_ref_line(entry, "due"),
            format!("summary\t{entry}\tdue")
        );
    }

    #[test]
    fn the_summarys_lines_follow_the_tags_and_are_absent_with_none() {
        use super::{SummariesDry, TagsDry};
        let summaries = SummariesDry {
            due: 1,
            calls: 1,
            tokens: 0,
            instruction: (Origin::Shipped, "7fdf9f0d93f443e3".to_owned()),
            lines: vec!["summary\tS\tdue".to_owned()],
        };
        let mut dry = Dry {
            limit: 20,
            provider: vec!["provider: p".to_owned()],
            entry_lines: vec!["entry\tE".to_owned()],
            ..Dry::default()
        };
        let before = [
            "kinds: none",
            "named: 0 entries by derive.in, 0 of them left out by derive.not_in",
            "due: 0 entries",
            "this run: 0 calls for 0 entries, 0 tokens of records to send (derive.calls 20)",
            "provider: p",
            "entry\tE",
        ];
        assert_eq!(lines(&dry), before);
        dry.summaries = Some(summaries.clone());
        let found = lines(&dry);
        let at = found
            .iter()
            .position(|line| line.starts_with("this run:"))
            .expect("the run's line");
        assert!(found[at + 1].starts_with("summaries: "));
        assert_eq!(found.last().map(String::as_str), Some("summary\tS\tdue"));
        dry.tags = Some(TagsDry {
            pick: 1,
            lines: vec!["tags\tT\tasked\t-".to_owned()],
            ..TagsDry::default()
        });
        let both = lines(&dry);
        let tags_at = both
            .iter()
            .position(|line| line.starts_with("tags: "))
            .expect("the tags' line");
        assert!(both[tags_at + 1].starts_with("summaries: "));
        let tail: Vec<&str> = both.iter().rev().take(2).map(String::as_str).collect();
        assert_eq!(tail, ["summary\tS\tdue", "tags\tT\tasked\t-"]);
        dry.summaries = None;
        dry.tags = None;
        assert_eq!(lines(&dry), before);
    }
}
