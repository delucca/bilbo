//! Renders as the eval's bm25 `build.py` does (`render_session`, `render_entry`); where that script would stop on
//! an unexpected shape, the block or the entry renders nothing.

use bilbo_kinds::reader::{EntryLabels, Passage};

use crate::json::{self, Value};
use crate::render::{self, Outline, Tools};

/// `None` where the eval's `media()` would stop: a `source` that is not an object.
fn media(block: &Value) -> Option<String> {
    let kind = block.get("type")?.python_str();
    let media_type = match block.get("source") {
        None => String::new(),
        Some(source @ Value::Dict(_)) => source
            .get("media_type")
            .map(Value::python_str)
            .unwrap_or_default(),
        Some(_) => return None,
    };
    Some(format!("[{kind} {media_type}]").replace(" ]", "]"))
}

/// `None` where `render_result` would stop: content that is a number or `true`, or a text item whose text is no string.
fn render_result(content: Option<&Value>) -> Option<String> {
    let items = match content {
        Some(Value::Str(text)) => return Some(text.clone()),
        Some(Value::List(items)) => items,
        // Iterating a dict gives its keys, which are no items; a falsy value gives none.
        Some(Value::Dict(_)) => return Some(String::new()),
        Some(other) if other.truthy() => return None,
        _ => return Some(String::new()),
    };
    let mut parts = Vec::new();
    for item in items {
        let kind = if item.is_dict() {
            item.get("type")
        } else {
            None
        };
        match kind {
            Some(Value::Str(kind)) if kind == "text" => match item.get("text") {
                None => parts.push(String::new()),
                Some(Value::Str(text)) => parts.push(text.clone()),
                Some(_) => return None,
            },
            Some(Value::Str(kind)) if kind == "image" => parts.push(media(item)?),
            Some(Value::Str(kind)) if kind == "tool_reference" => {
                let name = item
                    .get("tool_name")
                    .map(Value::python_str)
                    .unwrap_or_default();
                parts.push(format!("[tool_reference {name}]"));
            }
            Some(kind) if kind.truthy() => parts.push(format!("[{}]", kind.python_str())),
            _ => {}
        }
    }
    Some(parts.join("\n"))
}

fn piece(block: &Value, label: &str, tools: &mut Tools) -> Option<String> {
    let name = || block.get("name").map(Value::python_str).unwrap_or_default();
    match block.get("type")?.as_str()? {
        "text" => {
            let text = match block.get("text") {
                None => "",
                Some(text) => render::strip(text.as_str()?),
            };
            (!text.is_empty()).then(|| format!("{label}: {text}"))
        }
        "tool_use" => {
            if !tools.name(block.get("id"), name()) {
                return None;
            }
            let args = match block.get("input").filter(|input| input.truthy()) {
                Some(input) => render::input(input),
                None => String::new(),
            };
            Some(render::labelled(format!("tool_use {}:", name()), &args))
        }
        "tool_result" => {
            let name = tools.named(block.get("tool_use_id"))?;
            let error = block.get("is_error").is_some_and(Value::truthy);
            let head = format!(
                "{} {name}:",
                if error { "tool_error" } else { "tool_result" }
            );
            let body = render_result(block.get("content"))?;
            Some(render::labelled(head, render::strip(&body)))
        }
        "image" | "document" => Some(format!("{label}: {}", media(block)?)),
        _ => None,
    }
}

fn render_entry(entry: &Value, role: &str, tools: &mut Tools) -> Vec<String> {
    let content = match entry.get("message") {
        Some(message @ Value::Dict(_)) => message.get("content"),
        _ => None,
    };
    let text;
    let blocks: &[Value] = match content {
        Some(Value::Str(content)) => {
            text = [Value::Dict(vec![
                ("type".to_owned(), Value::Str("text".to_owned())),
                ("text".to_owned(), Value::Str(content.clone())),
            ])];
            &text
        }
        Some(Value::List(blocks)) => blocks,
        _ => return Vec::new(),
    };
    let summary = entry.get("isCompactSummary").is_some_and(Value::truthy);
    let label = if summary { "compact summary" } else { role };
    blocks
        .iter()
        .filter_map(|block| piece(block, label, tools))
        .collect()
}

fn role(entry: &Value) -> Option<&str> {
    let kind = match entry.get("type") {
        None | Some(Value::Null) => entry.get("role"),
        kind => kind,
    };
    kind.and_then(Value::as_str)
        .filter(|role| matches!(*role, "user" | "assistant"))
}

fn flag(entry: &Value, name: &str) -> bool {
    entry.get(name).is_some_and(Value::truthy)
}

pub fn renders(entry: &Value) -> bool {
    role(entry).is_some_and(|role| {
        !flag(entry, "isMeta") && !render_entry(entry, role, &mut Tools::default()).is_empty()
    })
}

/// The thread is the `isSidechain` value of the first entry that renders; entries of the other value are left out,
/// as in the eval. A subagent's file marks every entry, so it reads as its own. An entry left out for another reason
/// (`isMeta`, nothing to say) never chooses the thread.
pub fn passages(records: &[&[u8]]) -> Vec<Passage> {
    let mut tools = Tools::default();
    let mut thread = None;
    let mut passages = Vec::new();
    for (record, number) in records.iter().zip(1u64..) {
        let Some(entry) = json::parse(record).filter(Value::is_dict) else {
            continue;
        };
        let Some(role) = role(&entry) else {
            continue;
        };
        let sidechain = flag(&entry, "isSidechain");
        if thread.is_some_and(|thread| thread != sidechain) || flag(&entry, "isMeta") {
            continue;
        }
        let pieces = render_entry(&entry, role, &mut tools);
        if !pieces.is_empty() {
            thread = Some(sidechain);
        }
        let prompt = pieces
            .first()
            .is_some_and(|piece| matches!(render::head_of(piece), "user:" | "compact summary:"));
        let at = render::time(entry.get("timestamp"));
        passages.extend(render::passage(number, &pieces, at, Outline::of(prompt)));
    }
    passages
}

pub fn labels(records: &[&[u8]]) -> Vec<String> {
    let mut found = EntryLabels::default();
    for record in records {
        let Some(record) = json::parse(record).filter(Value::is_dict) else {
            continue;
        };
        if let Some(cwd) = record.get("cwd").and_then(Value::as_str) {
            found.add(cwd);
        }
        let environment = record.get("type").and_then(Value::as_str) == Some("attachment")
            && record
                .get("attachment")
                .and_then(|attachment| attachment.get("type"))
                .and_then(Value::as_str)
                == Some("environment");
        if !environment {
            continue;
        }
        let added = record
            .get("attachment")
            .and_then(|attachment| attachment.get("snapshot"))
            .and_then(|snapshot| snapshot.get("additionalWorkingDirectories"));
        if let Some(Value::List(folders)) = added {
            for folder in folders.iter().filter_map(Value::as_str) {
                found.add(folder);
            }
        }
    }
    found.into_texts()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use bilbo_kinds::reader::{Passage, Piece};

    use super::passages;

    const SESSION: &str = include_str!("../../fixtures/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl");
    const OLDER: &str = include_str!("../../fixtures/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl");
    const SUBAGENT: &str = include_str!("../../fixtures/agent-ad63c2b3c9e7a4b3d.jsonl");
    const CURSOR: &str = include_str!("../../fixtures/cursor-session.jsonl");

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

    fn user(content: &str) -> String {
        format!("{{\"type\":\"user\",\"message\":{{\"content\":{content}}}}}\n")
    }

    fn assistant(content: &str) -> String {
        format!("{{\"type\":\"assistant\",\"message\":{{\"content\":{content}}}}}\n")
    }

    fn rendered(lines: &[String]) -> Vec<(u64, String)> {
        texts(&lines.concat())
    }

    #[test]
    fn a_session_reads_as_the_evals_render_printed_it() {
        let expected = [
            (
                3,
                "user: Thin the carrot rows, then tell me how many seedlings are left.\n\nUse the north bed only.",
            ),
            (
                5,
                "assistant: I'll count the rows first — então desbasto.\ntool_use Bash:\ncommand: wc -l beds/north.csv\ndescription: Count the rows\ntimeout: 5000\nrun_in_background: false",
            ),
            (6, "tool_result Bash:\n42 beds/north.csv"),
            (
                8,
                "tool_use Edit:\nfile_path: /home/me/src/garden/beds/north.csv\nold_string: carrot,42\nnew_string: carrot,30\nreplace_all: false\nedits: [{\"row\": 3, \"spacing\": 2.5}, {\"row\": 4, \"spacing\": 3.0}]\nnote: null\ntool_use TaskList:",
            ),
            (
                9,
                "tool_error Edit:\nThe file is read-only.\ntool_result TaskList:\n2 tasks\n[image image/png]\n[tool_reference TaskGet]",
            ),
            (
                17,
                "compact summary: This session is being continued. The carrots in the north bed were thinned from 42 to 30.",
            ),
            (
                18,
                "user: Here is the bed. What is the weed in the corner?\nuser: [image image/jpeg]\nuser: [document application/pdf]",
            ),
            (19, "assistant: That is purslane; it is edible."),
            (
                20,
                "tool_use Read:\nfile_path: /home/me/src/garden/seed.log",
            ),
        ];
        let log: String = (1..=60)
            .map(|row| format!("\nrow {row:03} of the seed log: sown, watered, thinned"))
            .collect();
        let found = texts(SESSION);
        assert_eq!(found.len(), expected.len() + 1, "{found:#?}");
        for ((record, text), (expected_record, expected_text)) in found.iter().zip(expected) {
            assert_eq!((*record, text.as_str()), (expected_record, expected_text));
        }
        assert_eq!(found[9], (21, format!("tool_result Read:{log}")));
    }

    #[test]
    fn each_piece_has_its_label_line_and_the_byte_it_starts_at() {
        let found = read(SESSION);
        let pieces = |record: u64| {
            let passage = found
                .iter()
                .find(|passage| passage.range.first == record)
                .expect("the record renders");
            assert_eq!(passage.range.count, 1);
            assert_eq!(passage.ends, [passage.text.len()]);
            passage
                .pieces
                .iter()
                .map(|Piece { start, label }| (*start, label.as_str()))
                .collect::<Vec<_>>()
        };
        assert_eq!(pieces(3), [(0, "user:")]);
        assert_eq!(pieces(5), [(0, "assistant:"), (58, "tool_use Bash:")]);
        assert_eq!(
            pieces(8),
            [(0, "tool_use Edit:"), (199, "tool_use TaskList:")]
        );
        assert_eq!(
            pieces(9),
            [(0, "tool_error Edit:"), (40, "tool_result TaskList:")]
        );
        assert_eq!(pieces(18), [(0, "user:"), (55, "user:"), (80, "user:")]);
    }

    #[test]
    fn prompts_and_compact_summaries_are_listed_and_each_passage_has_its_time() {
        let found = read(SESSION);
        let listed: Vec<u64> = found
            .iter()
            .filter(|passage| passage.outline.is_some())
            .map(|passage| passage.range.first)
            .collect();
        assert_eq!(listed, [3, 17, 18]);
        let eight = SystemTime::UNIX_EPOCH + Duration::from_secs(1_794_038_400);
        assert_eq!(found[0].at, Some(eight + Duration::from_secs(60)));
        assert_eq!(found[7].at, Some(eight + Duration::from_secs(14 * 60)));
        assert!(read(CURSOR).iter().all(|passage| passage.at.is_none()));
    }

    #[test]
    fn thinking_meta_and_other_entry_types_render_nothing() {
        let rendered: Vec<u64> = texts(SESSION).iter().map(|(record, _)| *record).collect();
        for (record, what) in [
            (1, "a permission mode"),
            (2, "a file history snapshot"),
            (4, "an assistant entry of thinking only"),
            (7, "an isMeta entry"),
            (12, "a system entry"),
            (13, "a blank line"),
            (14, "a line that is not JSON"),
            (15, "a line that is not an object"),
            (16, "a compact boundary"),
            (22, "a title"),
            (23, "the last prompt"),
        ] {
            assert!(!rendered.contains(&record), "{what}");
        }
    }

    #[test]
    fn entries_of_another_thread_are_left_out_of_a_sessions_file() {
        let rendered: Vec<u64> = texts(SESSION).iter().map(|(record, _)| *record).collect();
        assert!(!rendered.contains(&10) && !rendered.contains(&11));
    }

    #[test]
    fn a_subagents_file_is_its_own_thread() {
        assert_eq!(
            texts(SUBAGENT),
            [
                (1, "user: Count the seed packets in the shed.".to_owned()),
                (2, "assistant: Twelve packets.".to_owned()),
            ]
        );
        let then_main = [SUBAGENT, &user("\"from the main thread\"")].concat();
        assert_eq!(
            texts(&then_main).len(),
            2,
            "the thread is the first entry's"
        );
        assert_eq!(
            read(SUBAGENT)[0].outline.as_deref(),
            Some("user: Count the seed packets in the shed.")
        );
    }

    #[test]
    fn an_entry_that_is_left_out_never_chooses_the_thread() {
        let side = |extra: &str, content: &str| {
            format!(
                "{{\"type\":\"user\",\"isSidechain\":true{extra},\"message\":{{\"content\":{content}}}}}\n"
            )
        };
        let main = user("\"main\"");
        for (first, why) in [
            (side(",\"isMeta\":true", "\"skip\""), "a meta entry"),
            (side("", "\"  \""), "an entry with nothing to say"),
            (
                side("", "[{\"type\":\"thinking\",\"thinking\":\"x\"}]"),
                "thinking only",
            ),
            (
                "{\"type\":\"system\",\"isSidechain\":true}\n".to_owned(),
                "another type",
            ),
        ] {
            assert_eq!(
                rendered(&[first, main.clone()]),
                [(2, "user: main".to_owned())],
                "{why}"
            );
        }
        let found = rendered(&[side("", "\"side\""), main]);
        assert_eq!(
            found,
            [(1, "user: side".to_owned())],
            "an entry that renders does"
        );
    }

    #[test]
    fn cursors_entries_carry_role_and_no_type() {
        assert_eq!(
            texts(CURSOR),
            [
                (1, "user: Rename the trellis module.".to_owned()),
                (2, "assistant: Renamed it to support.".to_owned()),
            ]
        );
        let null_type = "{\"type\":null,\"role\":\"user\",\"message\":{\"content\":\"hi\"}}\n";
        assert_eq!(texts(null_type), [(1, "user: hi".to_owned())]);
        let typed = "{\"type\":\"system\",\"role\":\"user\",\"message\":{\"content\":\"hi\"}}\n";
        assert!(texts(typed).is_empty(), "the type wins over the role");
    }

    #[test]
    fn a_result_names_the_tool_of_its_call_from_any_earlier_record() {
        let found = texts(OLDER);
        assert_eq!(
            found[1].1,
            "tool_use Bash:\ncommand: git log -1 --format=%H\ndescription: Show the last commit"
        );
        assert_eq!(
            found[2].1, "tool_result unknown:\nb1bf7b8e94bd4c1069fa71fdbfa4350794835730",
            "no call gave this id"
        );
        let lines = [
            assistant(r#"[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"a"}}]"#),
            user("\"between\""),
            "{\"type\":\"system\"}\n".to_owned(),
            user(r#"[{"type":"tool_result","tool_use_id":"t1","content":"body"}]"#),
        ];
        assert_eq!(
            rendered(&lines)[2],
            (4, "tool_result Read:\nbody".to_owned())
        );
    }

    #[test]
    fn nesting_past_128_levels_and_lone_surrogates_read_as_the_plan_says() {
        let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
        let call = format!(
            "[{{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"Read\",\"input\":{{\"a\":{deep}}}}}]"
        );
        let result = user(r#"[{"type":"tool_result","tool_use_id":"t1","content":"x"}]"#);
        assert_eq!(
            rendered(&[assistant(&call), result]),
            [(2, "tool_result unknown:\nx".to_owned())],
            "the eval rendered the call, and named it in the result"
        );

        let call = r#"[{"type":"tool_use","id":"\ud800","name":"Bash","input":{}}]"#;
        let result = user(r#"[{"type":"tool_result","tool_use_id":"\udc00","content":"x"}]"#);
        assert_eq!(
            rendered(&[assistant(call), result])[1],
            (2, "tool_result Bash:\nx".to_owned()),
            "the eval kept the two lone surrogates apart: unknown"
        );
    }

    #[test]
    fn a_call_in_an_entry_that_is_left_out_names_nothing() {
        let call = r#"[{"type":"tool_use","id":"t1","name":"Read","input":{}}]"#;
        let result = user(r#"[{"type":"tool_result","tool_use_id":"t1","content":"x"}]"#);
        let meta = format!(
            "{{\"type\":\"assistant\",\"isMeta\":true,\"message\":{{\"content\":{call}}}}}\n"
        );
        assert_eq!(
            rendered(&[user("\"go\""), meta, result]),
            [
                (1, "user: go".to_owned()),
                (3, "tool_result unknown:\nx".to_owned())
            ]
        );
    }

    #[test]
    fn truth_is_pythons_for_every_flag() {
        let entry = |flags: &str| {
            format!("{{\"type\":\"user\",{flags},\"message\":{{\"content\":\"x\"}}}}\n")
        };
        let first = user("\"first\"");
        for (flags, shown) in [
            ("\"isMeta\":false", true),
            ("\"isMeta\":0", true),
            ("\"isMeta\":\"\"", true),
            ("\"isMeta\":null", true),
            ("\"isMeta\":[]", true),
            ("\"isMeta\":\"false\"", false),
            ("\"isMeta\":1", false),
            ("\"isMeta\":[0]", false),
            ("\"isSidechain\":\"no\"", false),
            ("\"isSidechain\":0.0", true),
        ] {
            let found = rendered(&[first.clone(), entry(flags)]);
            assert_eq!(found.len() == 2, shown, "{flags}");
        }
        let summary = entry("\"isCompactSummary\":\"yes\"");
        assert_eq!(rendered(&[summary])[0].1, "compact summary: x");
        let error = user(r#"[{"type":"tool_result","content":"x","is_error":"false"}]"#);
        assert_eq!(rendered(&[error])[0].1, "tool_error unknown:\nx");
    }

    #[test]
    fn an_input_prints_each_value_as_python_dumps_it() {
        let input = |input: &str| {
            let call = format!(
                "[{{\"type\":\"tool_use\",\"id\":\"t\",\"name\":\"T\",\"input\":{input}}}]"
            );
            rendered(&[assistant(&call)])[0].1.clone()
        };
        assert_eq!(input("{}"), "tool_use T:");
        assert_eq!(input("null"), "tool_use T:");
        assert_eq!(input("0"), "tool_use T:");
        assert_eq!(input("\"\""), "tool_use T:");
        assert_eq!(input("[]"), "tool_use T:");
        assert_eq!(input("\"text\""), "tool_use T:\n\"text\"");
        assert_eq!(input("[1,\"a\"]"), "tool_use T:\n[1, \"a\"]");
        assert_eq!(input("7"), "tool_use T:\n7");
        assert_eq!(
            input(
                r#"{"b":"raw \"text\"\nline","a":{"z":1.0,"y":[true,null]},"n":-0,"big":12345678901234567890123,"f":1e22,"s":"é","e":""}"#
            ),
            "tool_use T:\nb: raw \"text\"\nline\na: {\"z\": 1.0, \"y\": [true, null]}\nn: 0\nbig: 12345678901234567890123\nf: 1e+22\ns: é\ne: "
        );
        assert_eq!(input(r#"{"a":1,"b":2,"a":3}"#), "tool_use T:\na: 3\nb: 2");
        assert_eq!(input(r#"{"":""}"#), "tool_use T:\n: ");
    }

    #[test]
    fn a_name_that_is_not_a_string_prints_as_pythons_str() {
        let call = |name: &str| {
            let block = format!("[{{\"type\":\"tool_use\",\"id\":\"t\",{name}\"input\":{{}}}}]");
            let result = user(r#"[{"type":"tool_result","tool_use_id":"t","content":""}]"#);
            rendered(&[assistant(&block), result])
        };
        for (name, printed) in [
            ("", ""),
            ("\"name\":null,", "None"),
            ("\"name\":true,", "True"),
            ("\"name\":12,", "12"),
            ("\"name\":1.50,", "1.5"),
            ("\"name\":[\"a\"],", "['a']"),
            ("\"name\":\"a:b\",", "a:b"),
        ] {
            let found = call(name);
            assert_eq!(found[0].1, format!("tool_use {printed}:"), "{name}");
            assert_eq!(found[1].1, format!("tool_result {printed}:"), "{name}");
        }
        let colon = read(&assistant(
            r#"[{"type":"tool_use","id":"t","name":"a:b","input":{}}]"#,
        ));
        assert_eq!(colon[0].pieces[0].label, "tool_use a:");
    }

    #[test]
    fn a_result_prints_its_text_images_references_and_other_items() {
        let result = |content: &str| {
            let block = format!("[{{\"type\":\"tool_result\",\"content\":{content}}}]");
            rendered(&[user(&block)])[0].1.clone()
        };
        let head = "tool_result unknown:";
        assert_eq!(result("\"  body \\n\""), format!("{head}\nbody"));
        assert_eq!(result("\"\\u001c \""), head);
        assert_eq!(result("null"), head);
        assert_eq!(result("[]"), head);
        assert_eq!(result("0"), head);
        assert_eq!(result("false"), head);
        assert_eq!(
            result("{\"type\":\"text\",\"text\":\"keys are no items\"}"),
            head
        );
        assert_eq!(
            result(r#"[{"type":"text","text":"a"},{"type":"text"},{"type":"text","text":"b"}]"#),
            format!("{head}\na\n\nb")
        );
        assert_eq!(
            result(
                r#"[{"type":"image"},{"type":"image","source":{}},{"type":"image","source":{"media_type":null}},{"type":"image","source":{"media_type":"a ]b"}}]"#
            ),
            format!("{head}\n[image]\n[image]\n[image None]\n[image a]b]")
        );
        assert_eq!(
            result(
                r#"[{"type":"tool_reference"},{"type":"tool_reference","tool_name":7},{"type":"resource"},{"type":5},{"type":""},{"type":null},"text",3,{"no":"type"}]"#
            ),
            format!("{head}\n[tool_reference ]\n[tool_reference 7]\n[resource]\n[5]")
        );
    }

    #[test]
    fn a_block_the_evals_render_would_stop_on_renders_nothing_and_the_rest_is_kept() {
        let kept = r#"{"type":"text","text":"kept"}"#;
        for block in [
            r#"{"type":"text","text":null}"#,
            r#"{"type":"text","text":5}"#,
            r#"{"type":"text","text":["a"]}"#,
            r#"{"type":"tool_use","id":["a"],"name":"T"}"#,
            r#"{"type":"tool_use","id":{},"name":"T"}"#,
            r#"{"type":"tool_result","tool_use_id":[],"content":"x"}"#,
            r#"{"type":"tool_result","content":5}"#,
            r#"{"type":"tool_result","content":true}"#,
            r#"{"type":"tool_result","content":1.5}"#,
            r#"{"type":"tool_result","content":[{"type":"text","text":null}]}"#,
            r#"{"type":"tool_result","content":[{"type":"text","text":5}]}"#,
            r#"{"type":"tool_result","content":[{"type":"image","source":null}]}"#,
            r#"{"type":"image","source":null}"#,
            r#"{"type":"image","source":"data"}"#,
            r#"{"type":"document","source":[]}"#,
            r#"{"type":"thinking","thinking":"hidden"}"#,
            r#"{"type":"server_tool_use","id":"s"}"#,
            r#"{"type":5}"#,
            r#"{"no":"type"}"#,
            r#""a string""#,
            "null",
            "7",
        ] {
            let found = rendered(&[user(&format!("[{block},{kept}]"))]);
            assert_eq!(found, [(1, "user: kept".to_owned())], "{block}");
        }
    }

    #[test]
    fn a_record_of_any_other_shape_renders_nothing_and_never_panics() {
        let deep = format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":{}\"x\"{}}}}}\n",
            "[".repeat(5000),
            "]".repeat(5000)
        );
        let lines = [
            user("\"first\""),
            "\n".to_owned(),
            " \t\r\n".to_owned(),
            "not json\n".to_owned(),
            "{\"type\":\"user\"\n".to_owned(),
            "[1,2]\n".to_owned(),
            "\"user\"\n".to_owned(),
            "7\n".to_owned(),
            "null\n".to_owned(),
            "{}\n".to_owned(),
            "{\"type\":5,\"role\":\"user\"}\n".to_owned(),
            "{\"type\":[\"user\"]}\n".to_owned(),
            "{\"role\":\"system\",\"message\":{\"content\":\"x\"}}\n".to_owned(),
            "{\"type\":\"user\"}\n".to_owned(),
            "{\"type\":\"user\",\"message\":null}\n".to_owned(),
            "{\"type\":\"user\",\"message\":\"text\"}\n".to_owned(),
            "{\"type\":\"user\",\"message\":[1]}\n".to_owned(),
            "{\"type\":\"user\",\"message\":7}\n".to_owned(),
            "{\"type\":\"user\",\"message\":{}}\n".to_owned(),
            user("null"),
            user("7"),
            user("{\"type\":\"text\",\"text\":\"an object is no list\"}"),
            user("\"   \""),
            user("[]"),
            deep,
            "{\"type\":\"user\",\"message\":{\"content\":\"\\ud83d\"}}\n".to_owned(),
            user("\"last\""),
        ];
        let found = rendered(&lines);
        assert_eq!(found.len(), 3, "{found:?}");
        assert_eq!(found[0], (1, "user: first".to_owned()));
        assert_eq!(found[1], (26, "user: \u{fffd}".to_owned()));
        assert_eq!(found[2], (27, "user: last".to_owned()));
        let mut bytes = lines.concat().into_bytes();
        bytes.extend_from_slice(
            b"\xff\xfe\n{\"type\":\"user\",\"message\":{\"content\":\"\xc3\"}}\n",
        );
        let records: Vec<&[u8]> = bytes.split_inclusive(|byte| *byte == b'\n').collect();
        assert_eq!(passages(&records).len(), 3);
        assert!(passages(&[]).is_empty());
    }

    #[test]
    fn text_is_stripped_as_python_strips_it() {
        let found = rendered(&[
            user("\"\\u001c\\u001f  x \\u00a0\\u3000\""),
            user("\"\\u200b x \\ufeff\""),
            user("[{\"type\":\"text\",\"text\":\" \\u001d \"},{\"type\":\"text\",\"text\":\"y\"}]"),
        ]);
        assert_eq!(
            found,
            [
                (1, "user: x".to_owned()),
                (2, "user: \u{200b} x \u{feff}".to_owned()),
                (3, "user: y".to_owned()),
            ]
        );
    }
}
