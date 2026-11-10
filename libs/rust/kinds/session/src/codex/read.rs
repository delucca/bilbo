use bilbo_kinds::reader::Passage;

use crate::json::{self, Value};
use crate::render::{self, Outline, Tools};

const INJECTED: [&str; 4] = [
    "# AGENTS.md instructions",
    "<environment_context>",
    "<recommended_plugins>",
    "<skill>",
];
const TYPED: &str = "user.";
const NO_KIND: &str = "unknown";

fn field<'v>(value: &'v Value, name: &str) -> Option<&'v str> {
    value.get(name).and_then(Value::as_str)
}

fn injected(kind: Option<&str>, text: &str) -> bool {
    match kind.filter(|kind| *kind != NO_KIND) {
        Some(kind) => !kind.starts_with(TYPED),
        None => INJECTED.iter().any(|start| text.starts_with(start)),
    }
}

fn message(payload: &Value) -> Vec<String> {
    let Some(role) = field(payload, "role").filter(|role| matches!(*role, "user" | "assistant"))
    else {
        return Vec::new();
    };
    let kinds = payload
        .get("internal_chat_message_metadata_passthrough")
        .and_then(|meta| meta.get("content_item_kinds"));
    let kind = |at: usize| match kinds {
        Some(Value::List(kinds)) => kinds.get(at).and_then(Value::as_str),
        _ => None,
    };
    let piece = |(at, item): (usize, &Value)| {
        let dropped = |text: &str| role == "user" && injected(kind(at), text);
        match field(item, "type")? {
            "input_text" | "output_text" | "text" => {
                let text = render::strip(field(item, "text")?);
                (!text.is_empty() && !dropped(text)).then(|| format!("{role}: {text}"))
            }
            image @ "input_image" => (!dropped("")).then(|| format!("{role}: [{image}]")),
            _ => None,
        }
    };
    match payload.get("content") {
        Some(Value::List(items)) => items.iter().enumerate().filter_map(piece).collect(),
        Some(Value::Str(text)) => {
            let text = render::strip(text);
            let kept = !text.is_empty() && !(role == "user" && injected(None, text));
            kept.then(|| format!("{role}: {text}"))
                .into_iter()
                .collect()
        }
        _ => Vec::new(),
    }
}

/// A custom tool's `input` is text of its own (a script, a patch) and prints as written, JSON or not.
fn call(payload: &Value, given: &str, tools: &mut Tools) -> Option<String> {
    let name = field(payload, "name")?;
    tools.name(payload.get("call_id"), name.to_owned());
    let written = field(payload, given).unwrap_or_default();
    let object = (given == "arguments")
        .then(|| json::parse(written.as_bytes()))
        .flatten()
        .filter(Value::is_dict);
    let args = match object {
        Some(object) => render::input(&object),
        None => written.to_owned(),
    };
    Some(render::labelled(format!("tool_use {name}:"), &args))
}

fn output(payload: &Value, tools: &Tools) -> Option<String> {
    let name = tools.named(payload.get("call_id"))?;
    let body = match payload.get("output") {
        Some(Value::Str(text)) => text.clone(),
        Some(Value::List(items)) => items
            .iter()
            .filter_map(|item| match field(item, "type")? {
                "input_text" | "output_text" | "text" => field(item, "text").map(str::to_owned),
                other => Some(format!("[{other}]")),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    Some(render::labelled(
        format!("tool_result {name}:"),
        render::strip(&body),
    ))
}

/// A subagent's rollout has no user message; its task arrives as one of these.
fn agent_message(payload: &Value) -> Option<String> {
    let author = field(payload, "author")?;
    let recipient = field(payload, "recipient")?;
    let Some(Value::List(items)) = payload.get("content") else {
        return None;
    };
    let text = items
        .iter()
        .filter(|item| field(item, "type") == Some("input_text"))
        .filter_map(|item| field(item, "text"))
        .collect::<Vec<_>>()
        .join("\n");
    let text = render::strip(&text);
    (!text.is_empty()).then(|| format!("agent_message {author} to {recipient}: {text}"))
}

pub fn passages(records: &[&[u8]]) -> Vec<Passage> {
    let mut tools = Tools::default();
    let mut passages = Vec::new();
    for (record, number) in records.iter().zip(1u64..) {
        let Some(line) = json::parse(record) else {
            continue;
        };
        let Some(payload) = line.get("payload").filter(|payload| payload.is_dict()) else {
            continue;
        };
        let (pieces, prompt) = match (field(&line, "type"), field(payload, "type")) {
            (Some("response_item"), Some("message")) => {
                (message(payload), field(payload, "role") == Some("user"))
            }
            (Some("response_item"), Some("function_call")) => (
                call(payload, "arguments", &mut tools).into_iter().collect(),
                false,
            ),
            (Some("response_item"), Some("custom_tool_call")) => (
                call(payload, "input", &mut tools).into_iter().collect(),
                false,
            ),
            (Some("response_item"), Some("function_call_output" | "custom_tool_call_output")) => {
                (output(payload, &tools).into_iter().collect(), false)
            }
            (Some("response_item"), Some("agent_message")) => {
                (agent_message(payload).into_iter().collect(), true)
            }
            (Some("compacted"), _) => {
                let summary = field(payload, "message").map(render::strip);
                let piece = summary
                    .filter(|summary| !summary.is_empty())
                    .map(|summary| format!("compact summary: {summary}"));
                (piece.into_iter().collect(), true)
            }
            _ => (Vec::new(), false),
        };
        let at = render::time(line.get("timestamp"));
        passages.extend(render::passage(number, &pieces, at, Outline::of(prompt)));
    }
    passages
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use bilbo_kinds::reader::Passage;

    use super::passages;

    const ROLLOUT: &str = include_str!(
        "../../fixtures/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl"
    );
    const OLDER: &str = include_str!(
        "../../fixtures/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl"
    );

    fn read(text: &str) -> Vec<Passage> {
        let records: Vec<&[u8]> = text.split_inclusive('\n').map(str::as_bytes).collect();
        passages(&records)
    }

    fn texts(text: &str) -> Vec<(u64, String)> {
        read(text)
            .into_iter()
            .map(|passage| (passage.range.first, passage.text))
            .collect()
    }

    fn item(payload: &str) -> String {
        format!("{{\"type\":\"response_item\",\"payload\":{payload}}}\n")
    }

    #[test]
    fn a_rollout_reads_in_the_labels_of_the_claude_dialect() {
        let expected = [
            (
                5,
                "user: Prune the roses and show me the result.\nuser: [input_image]",
            ),
            (7, "assistant: I'll look at the rose bed first."),
            (
                8,
                "tool_use shell:\ncommand: [\"ls\", \"-la\", \"beds\"]\nworkdir: /home/me/src/garden\ntimeout_ms: 10000",
            ),
            (9, "tool_result shell:\nroses.csv\ntulips.csv"),
            (
                11,
                "tool_use apply_patch:\n*** Begin Patch\n*** Update File: beds/roses.csv\n@@\n-rose,unpruned\n+rose,pruned\n*** End Patch",
            ),
            (
                12,
                "tool_result apply_patch:\nSuccess. Updated the following files:\nM beds/roses.csv\n[input_image]",
            ),
            (13, "tool_use wait:\nnot json"),
            (14, "tool_result unknown:\ndone"),
            (
                15,
                "agent_message /root/pruner_1 to /root: The side bed is pruned too.",
            ),
            (
                18,
                "compact summary: The roses were pruned and beds/roses.csv was updated.",
            ),
            (21, "user: Now water them."),
            (22, "assistant: The roses are pruned and watered."),
        ];
        let found = texts(ROLLOUT);
        assert_eq!(found.len(), expected.len(), "{found:#?}");
        for ((record, text), (expected_record, expected_text)) in found.iter().zip(expected) {
            assert_eq!((*record, text.as_str()), (expected_record, expected_text));
        }
    }

    #[test]
    fn what_the_harness_injects_renders_nothing() {
        let rendered: Vec<u64> = texts(ROLLOUT).iter().map(|(record, _)| *record).collect();
        for (record, what) in [
            (1, "the session meta"),
            (2, "a turn context"),
            (3, "a developer message"),
            (4, "the AGENTS.md and environment user message"),
            (6, "reasoning"),
            (10, "an event"),
            (16, "inter-agent metadata"),
            (17, "tool definitions"),
            (19, "a compaction with no summary"),
            (20, "an environment message with no item kinds"),
            (23, "a token usage record"),
            (24, "the world state"),
            (25, "the task's end event"),
        ] {
            assert!(!rendered.contains(&record), "{what}");
        }
        let skill = read(ROLLOUT);
        assert!(
            !skill[0].text.contains("<skill>"),
            "a selected skill's text is injected too"
        );
    }

    #[test]
    fn prompts_agent_messages_and_summaries_are_listed_and_each_passage_has_its_time() {
        let found = read(ROLLOUT);
        let listed: Vec<u64> = found
            .iter()
            .filter(|passage| passage.outline.is_some())
            .map(|passage| passage.range.first)
            .collect();
        assert_eq!(listed, [5, 15, 18, 21]);
        let nine = SystemTime::UNIX_EPOCH + Duration::from_secs(1_794_042_000);
        assert_eq!(found[0].at, Some(nine + Duration::from_secs(2 * 60)));
        assert_eq!(found[0].pieces.len(), 2);
        assert_eq!(found[0].pieces[1].start, 46);
        assert_eq!(
            found[8].pieces[0].label,
            "agent_message /root/pruner_1 to /root:"
        );
        assert_eq!(found[2].pieces[0].label, "tool_use shell:");
    }

    #[test]
    fn the_older_fixture_reads_too() {
        assert_eq!(
            texts(OLDER),
            [
                (
                    3,
                    "user: Water the beans, then show me the bed.\nuser: [input_image]".to_owned()
                ),
                (
                    5,
                    "tool_use shell:\ncommand: [\"git\", \"rev-parse\", \"HEAD\"]".to_owned()
                ),
                (
                    6,
                    "tool_result unknown:\nb1bf7b8e94bd4c1069fa71fdbfa4350794835730".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn a_user_item_is_injected_by_its_kind_or_by_how_its_text_starts() {
        let message = |kinds: &str, text: &str| {
            item(&format!(
                "{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"{text}\"}}]{kinds}}}"
            ))
        };
        let kinds = |kind: &str| {
            format!(
                ",\"internal_chat_message_metadata_passthrough\":{{\"content_item_kinds\":[\"{kind}\"]}}"
            )
        };
        for (kinds, text, shown) in [
            (String::new(), "plain", true),
            (String::new(), "# AGENTS.md instructions for /x", false),
            (String::new(), "<environment_context>x", false),
            (String::new(), "<recommended_plugins>x", false),
            (String::new(), "<skill>x", false),
            (String::new(), "  <environment_context>x", false),
            (String::new(), "see <environment_context>", true),
            (String::new(), "<other_tag>x", true),
            (
                kinds("user.text"),
                "<environment_context> typed by a person",
                true,
            ),
            (kinds("unknown"), "<environment_context>x", false),
            (kinds("unknown"), "plain", true),
            (kinds("agents_md.instructions"), "plain", false),
            (kinds("hooks.additional_context"), "plain", false),
            (
                ",\"internal_chat_message_metadata_passthrough\":{\"content_item_kinds\":[]}"
                    .to_owned(),
                "plain",
                true,
            ),
            (
                ",\"internal_chat_message_metadata_passthrough\":{\"content_item_kinds\":[7]}"
                    .to_owned(),
                "plain",
                true,
            ),
            (
                ",\"internal_chat_message_metadata_passthrough\":null".to_owned(),
                "plain",
                true,
            ),
        ] {
            assert_eq!(
                texts(&message(&kinds, text)).len() == 1,
                shown,
                "{kinds} {text}"
            );
        }
        let assistant = item(
            "{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"<environment_context> quoted\"}]}",
        );
        assert_eq!(
            texts(&assistant).len(),
            1,
            "only a user item is ever injected"
        );
    }

    #[test]
    fn a_result_names_the_tool_of_its_call_by_call_id() {
        let lines = [
            item(r#"{"type":"function_call","name":"shell","arguments":"{}","call_id":"c1"}"#),
            item(r#"{"type":"custom_tool_call","name":"exec","input":"ls","call_id":"c2"}"#),
            item(r#"{"type":"custom_tool_call_output","call_id":"c2","output":"out"}"#),
            item(r#"{"type":"function_call_output","call_id":"c1","output":[]}"#),
            item(r#"{"type":"function_call_output","call_id":"c3","output":null}"#),
        ];
        assert_eq!(
            texts(&lines.concat()),
            [
                (1, "tool_use shell:".to_owned()),
                (2, "tool_use exec:\nls".to_owned()),
                (3, "tool_result exec:\nout".to_owned()),
                (4, "tool_result shell:".to_owned()),
                (5, "tool_result unknown:".to_owned()),
            ]
        );
    }

    #[test]
    fn arguments_that_are_a_json_object_print_a_line_per_key_and_anything_else_as_written() {
        let call = |arguments: &str| {
            let line = format!(
                "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call\",\"name\":\"t\",\"arguments\":{arguments}}}}}\n"
            );
            texts(&line)[0].1.clone()
        };
        assert_eq!(
            call(r#""{\"a\": \"x\", \"b\": [1, 2.50]}""#),
            "tool_use t:\na: x\nb: [1, 2.5]"
        );
        assert_eq!(call(r#""[1, 2]""#), "tool_use t:\n[1, 2]");
        assert_eq!(call(r#""plain words""#), "tool_use t:\nplain words");
        assert_eq!(call(r#""""#), "tool_use t:");
        assert_eq!(call(r#""{}""#), "tool_use t:");
        assert_eq!(call("null"), "tool_use t:");
        assert_eq!(call("{\"a\":1}"), "tool_use t:", "only a string is read");
    }

    #[test]
    fn a_custom_tools_input_prints_as_written_even_when_it_is_json() {
        let line = item(r#"{"type":"custom_tool_call","name":"exec","input":"{\"a\":1}"}"#);
        assert_eq!(texts(&line), [(1, "tool_use exec:\n{\"a\":1}".to_owned())]);
    }

    #[test]
    fn a_line_of_any_other_shape_renders_nothing_and_never_panics() {
        let lines = [
            "\n".to_owned(),
            "not json\n".to_owned(),
            "[]\n".to_owned(),
            "{\"type\":\"response_item\"}\n".to_owned(),
            "{\"type\":\"response_item\",\"payload\":null}\n".to_owned(),
            "{\"type\":\"response_item\",\"payload\":[1]}\n".to_owned(),
            "{\"type\":5,\"payload\":{\"type\":\"message\"}}\n".to_owned(),
            "{\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"x\"}}\n".to_owned(),
            item("{}"),
            item("{\"type\":5}"),
            item("{\"type\":\"message\"}"),
            item("{\"type\":\"message\",\"role\":5,\"content\":[]}"),
            item("{\"type\":\"message\",\"role\":\"user\"}"),
            item("{\"type\":\"message\",\"role\":\"user\",\"content\":null}"),
            item("{\"type\":\"message\",\"role\":\"user\",\"content\":[null,5,\"x\",{},{\"type\":\"input_text\"},{\"type\":\"input_text\",\"text\":5},{\"type\":\"input_text\",\"text\":\"  \"},{\"type\":\"other\",\"text\":\"x\"}]}"),
            item("{\"type\":\"message\",\"role\":\"developer\",\"content\":[{\"type\":\"input_text\",\"text\":\"x\"}]}"),
            item("{\"type\":\"message\",\"role\":\"system\",\"content\":[{\"type\":\"input_text\",\"text\":\"x\"}]}"),
            item("{\"type\":\"function_call\",\"arguments\":\"{}\"}"),
            item("{\"type\":\"function_call\",\"name\":5}"),
            item("{\"type\":\"function_call_output\",\"call_id\":[],\"output\":\"x\"}"),
            item("{\"type\":\"agent_message\",\"author\":\"/root\"}"),
            item("{\"type\":\"agent_message\",\"author\":\"/a\",\"recipient\":\"/b\",\"content\":\"x\"}"),
            item("{\"type\":\"agent_message\",\"author\":\"/a\",\"recipient\":\"/b\",\"content\":[{\"type\":\"encrypted_content\",\"encrypted_content\":\"x\"}]}"),
            item("{\"type\":\"reasoning\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"x\"}]}"),
            item("{\"type\":\"configuration_update\",\"reasoning\":{\"effort\":\"high\"}}"),
            item("{\"type\":\"something_new\",\"text\":\"x\"}"),
            "{\"type\":\"compacted\",\"payload\":{\"message\":5}}\n".to_owned(),
            "{\"type\":\"compacted\",\"payload\":{\"message\":\"  \"}}\n".to_owned(),
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"x\"}]}}\n".to_owned(),
        ];
        assert_eq!(texts(&lines.concat()), []);
        let string = item("{\"type\":\"message\",\"role\":\"user\",\"content\":\" a string \"}");
        assert_eq!(texts(&string), [(1, "user: a string".to_owned())]);
        assert!(passages(&[b"\xff\n"]).is_empty());
    }
}
