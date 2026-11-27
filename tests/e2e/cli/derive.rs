use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use bilbo_provider::{Provider, Reply};
use serde_json::json;

use super::scratch::{NOTE, SESSION, Scratch, UUID, closed};

const KEY: &str = "sk-test-PLUM-7391";

fn message(text: &str) -> String {
    json!({
        "model": "claude-test-1",
        "id": "msg_TEST0001",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": text}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 100, "output_tokens": 20}
    })
    .to_string()
}

fn answer() -> Reply {
    Reply::answer(
        200,
        &message("r1-7\nThe user asked about the garden fence."),
    )
}

fn config(url: &str, extra: &str) -> String {
    format!(
        "derive.enabled = true\nllm.provider = \"anthropic\"\nderive.in = [\"projects\"]\nderive.quiet = \"0s\"\nderive.summary.enabled = false\nllm.anthropic.model = \"claude-test-1\"\nllm.anthropic.api.endpoint = \"{url}/v1/messages\"\n{extra}"
    )
}

fn put(scratch: &Scratch, text: &str) -> String {
    scratch.config(&closed(text))
}

fn sealed(test: &str) -> Scratch {
    let scratch = Scratch::new(test);
    let session = scratch.dir.join(format!(
        "home/.claude/projects/-home-me-src-garden/{UUID}.jsonl"
    ));
    fs::create_dir_all(session.parent().expect("a parent")).expect("folders");
    fs::write(&session, SESSION).expect("a session");
    let target = session.to_str().expect("UTF-8").to_owned();
    assert_eq!(scratch.run(&["remember", &target]).code, 0);
    scratch.backdate(120);
    assert_eq!(scratch.run(&["remember", &target]).code, 0);
    scratch
}

fn derive(scratch: &Scratch, args: &[&str]) -> super::Run {
    scratch.run_with(args, &[("ANTHROPIC_API_KEY", KEY)])
}

#[test]
fn derive_writes_notes_and_recall_answers_two_lists() {
    let scratch = sealed("e2e_derive_notes");
    let provider = Provider::start(vec![answer()]);
    put(&scratch, &config(&provider.url(), ""));
    let run = derive(&scratch, &["derive"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout
            .starts_with("entries 1, calls 1, notes 1, dropped 0, refused 0, failed 0\n"),
        "{}",
        run.stdout
    );
    let found = scratch.run(&["recall", "garden", "fence"]);
    assert_eq!(found.code, 0, "{}", found.stderr);
    let headers: Vec<&str> = found
        .stdout
        .lines()
        .filter(|l| l.starts_with("=== "))
        .collect();
    let entry = scratch.run(&["read", UUID, "--budget", "256"]);
    let entry = entry.stdout.lines().next().expect("a head").to_owned();
    let short = &entry["bilbo:".len()..][16..];
    let notes = scratch.run(&["recall", "--kind", "note", "garden", "fence"]);
    let note = notes
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("=== "))
        .expect("a note header")
        .split('#')
        .next()
        .expect("a ref")
        .to_owned();
    assert_eq!(
        headers,
        [
            format!("=== {short}#r3-20\t33 lines"),
            format!("=== {note}#r1\t2 lines\tnote"),
        ],
        "{}",
        found.stdout
    );
}

#[test]
fn derive_is_off_by_default_and_sends_nothing() {
    let scratch = sealed("e2e_derive_off");
    let provider = Provider::start(vec![answer()]);
    put(
        &scratch,
        &format!(
            "llm.anthropic.api.endpoint = \"{}/v1/messages\"\n",
            provider.url()
        ),
    );
    let run = derive(&scratch, &["derive"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stderr,
        "bilbo: derive is off: derive.enabled is false\n"
    );
    assert_eq!(run.stdout, "");
    assert!(provider.requests().is_empty());
}

#[test]
fn a_dry_run_prints_its_lines_and_sends_nothing() {
    let scratch = sealed("e2e_derive_dry");
    let provider = Provider::start(vec![answer()]);
    put(&scratch, &config(&provider.url(), ""));
    let run = derive(&scratch, &["derive", "--dry-run"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert!(
        lines[0].starts_with("kinds: session (built in, "),
        "{}",
        run.stdout
    );
    assert_eq!(
        lines[1],
        "named: 1 entries by derive.in, 0 of them left out by derive.not_in"
    );
    assert_eq!(lines[2], "due: 1 entries");
    assert!(lines[4].starts_with("provider: anthropic, via api, model claude-test-1, at "));
    assert!(provider.requests().is_empty());
}

#[test]
fn a_plain_http_endpoint_is_reached_with_no_proxy_whatever_the_environment_names() {
    let scratch = sealed("e2e_derive_proxy");
    let provider = Provider::start(vec![answer()]);
    put(&scratch, &config(&provider.url(), ""));
    let run = scratch.run_with(
        &["derive"],
        &[
            ("ANTHROPIC_API_KEY", KEY),
            ("HTTP_PROXY", "http://127.0.0.1:9"),
            ("http_proxy", "http://127.0.0.1:9"),
            ("ALL_PROXY", "http://127.0.0.1:9"),
        ],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(provider.requests().len(), 1);
}

fn files_under(root: &Path, found: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
    for entry in fs::read_dir(root).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, found);
        } else {
            found.push((path.clone(), fs::read(&path).unwrap_or_default()));
        }
    }
}

#[test]
fn the_key_comes_from_a_program_and_is_in_nothing_the_binary_wrote() {
    let scratch = sealed("e2e_derive_key");
    let provider = Provider::start(vec![answer()]);
    let program = scratch.dir.join("key.sh");
    fs::write(&program, format!("#!/bin/sh\necho {KEY}\n")).expect("a script");
    put(
        &scratch,
        &config(
            &provider.url(),
            &format!(
                "llm.anthropic.api.key_command = [\"/bin/sh\", \"{}\"]\n",
                program.display()
            ),
        ),
    );
    let run = scratch.run(&["derive"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(provider.requests().len(), 1);
    assert_eq!(provider.requests()[0].header("x-api-key"), [KEY]);
    assert!(!run.stdout.contains(KEY) && !run.stderr.contains(KEY));
    let found = scratch.run(&["recall", "garden", "fence"]);
    let reference = found
        .stdout
        .lines()
        .filter(|line| line.starts_with("=== ") && line.ends_with("\tnote"))
        .find_map(|line| line.strip_prefix("=== ")?.split('\t').next())
        .expect("a note hit")
        .to_owned();
    let entry = reference.split('#').next().expect("an entry").to_owned();
    let printed = scratch.run(&["read", &entry, "--budget", "1000"]);
    assert!(!printed.stdout.contains(KEY));
    let mut files = Vec::new();
    for folder in ["store", "state", "cache", "home", "config"] {
        files_under(&scratch.dir.join(folder), &mut files);
    }
    for (path, bytes) in files {
        assert!(
            !bytes
                .windows(KEY.len())
                .any(|window| window == KEY.as_bytes()),
            "{}",
            path.display()
        );
    }
}

#[test]
fn a_failed_call_exits_1_and_a_config_error_exits_2() {
    let scratch = sealed("e2e_derive_exits");
    let cut = Reply::answer(
        200,
        &json!({
            "model": "claude-test-1",
            "content": [{"type": "thinking", "thinking": "", "signature": "c2ln"}],
            "stop_reason": "max_tokens",
            "usage": {"input_tokens": 100, "output_tokens": 16000}
        })
        .to_string(),
    );
    let provider = Provider::start(vec![cut]);
    put(&scratch, &config(&provider.url(), ""));
    let run = derive(&scratch, &["derive"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    put(
        &scratch,
        &config(&provider.url(), "").replace("llm.anthropic.model = \"claude-test-1\"\n", ""),
    );
    let run = derive(&scratch, &["derive"]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert_eq!(run.stderr, "bilbo: llm.anthropic.model is not set\n");
    scratch.config(&closed(""));
}

#[test]
fn the_help_page_lists_the_kinds_and_the_providers() {
    let scratch = Scratch::new("e2e_derive_help");
    let run = scratch.run(&["derive", "--help"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout.contains(
            "Kinds:\n  session  bilbo ships an instruction for it\nProviders:\n  anthropic  Anthropic's models: via api, with a key; via tool, the tool claude\n  openai     OpenAI's models: via api, with a key; via tool, the tool codex\n"
        ),
        "{}",
        run.stdout
    );
}

#[test]
fn derive_through_a_tool_needs_no_key() {
    let scratch = sealed("e2e_derive_tool");
    let script = scratch.dir.join("tool.sh");
    let reply = json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "stop_reason": "end_turn",
        "num_turns": 1,
        "permission_denials": [],
        "result": "r1-7\nThe user asked about the garden fence.",
        "usage": {
            "input_tokens": 100,
            "output_tokens": 20,
            "server_tool_use": {"web_search_requests": 0, "web_fetch_requests": 0}
        },
        "modelUsage": {"claude-test-1": {"webSearchRequests": 0}},
        "subagent_stats": {"spawned": 0}
    });
    fs::write(scratch.dir.join("reply.json"), reply.to_string()).expect("a reply");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ncat > /dev/null\ncat {}/reply.json\n",
            scratch.dir.display()
        ),
    )
    .expect("a script");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("a mode");
    put(
        &scratch,
        &format!(
            "derive.enabled = true\nllm.provider = \"anthropic\"\nderive.in = [\"projects\"]\nderive.quiet = \"0s\"\nderive.summary.enabled = false\nllm.anthropic.via = \"tool\"\nllm.anthropic.model = \"claude-test-1\"\n\n[llm.anthropic.tool]\nprogram = {:?}\nproject_files = [\"zz-marker.md\"]\narguments = [\"-p\", \"--tools\", \"\", \"--strict-mcp-config\", \"--safe-mode\", \"--no-session-persistence\", \"--system-prompt\", \"{{instruction}}\"]\n",
            script.display().to_string()
        ),
    );
    let run = scratch.run(&["derive"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout
            .starts_with("entries 1, calls 1, notes 1, dropped 0, refused 0, failed 0\n"),
        "{}",
        run.stdout
    );
    let found = scratch.run(&["recall", "garden", "fence"]);
    assert!(
        found
            .stdout
            .contains("The user asked about the garden fence.")
    );
}

#[test]
fn derive_through_codex_needs_no_key() {
    let scratch = sealed("e2e_derive_codex");
    let script = scratch.dir.join("tool.sh");
    let events: String = [
        json!({"type": "thread.started", "thread_id": "00000000-0000-7000-8000-000000000001"}),
        json!({"type": "turn.started"}),
        json!({"type": "item.completed", "item": {
            "id": "item_0",
            "type": "agent_message",
            "text": "r1-7\nThe user asked about the garden fence."
        }}),
        json!({"type": "turn.completed", "usage": {
            "input_tokens": 1200,
            "cached_input_tokens": 0,
            "cache_write_input_tokens": 0,
            "output_tokens": 80,
            "reasoning_output_tokens": 30
        }}),
    ]
    .iter()
    .map(|line| format!("{line}\n"))
    .collect();
    let models = json!({"models": [{
        "slug": "model-a",
        "display_name": "Model A",
        "supported_reasoning_levels": [{"effort": "medium", "description": "b"}],
        "tool_mode": "code_mode_only",
        "apply_patch_tool_type": "freeform",
        "experimental_supported_tools": ["clock"],
        "supports_search_tool": true
    }]});
    fs::write(scratch.dir.join("reply.jsonl"), events).expect("a reply");
    fs::write(scratch.dir.join("models.json"), models.to_string()).expect("a catalog");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'codex-cli 0.169.0' ;;\n  debug) cat {dir}/models.json ;;\n  exec) cat > /dev/null; cat {dir}/reply.jsonl ;;\nesac\n",
            dir = scratch.dir.display()
        ),
    )
    .expect("a script");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("a mode");
    let home = scratch.dir.join("state/bilbo/derive/codex");
    fs::create_dir_all(&home).expect("a home");
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).expect("a mode");
    fs::write(home.join("auth.json"), "").expect("a login");
    put(
        &scratch,
        &format!(
            "derive.enabled = true\nllm.provider = \"openai\"\nderive.in = [\"projects\"]\nderive.quiet = \"0s\"\nderive.summary.enabled = false\nllm.openai.via = \"tool\"\nllm.openai.model = \"model-a\"\n\n[llm.openai.tool]\nprogram = {:?}\nsystem_files = []\n",
            script.display().to_string()
        ),
    );
    let run = scratch.run(&["derive"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout
            .starts_with("entries 1, calls 1, notes 1, dropped 0, refused 0, failed 0\n"),
        "{}",
        run.stdout
    );
    let found = scratch.run(&["recall", "garden", "fence"]);
    assert!(
        found
            .stdout
            .contains("The user asked about the garden fence.")
    );
}

#[test]
fn a_session_is_noted_and_tagged_and_found_by_its_tag() {
    let scratch = sealed("e2e_derive_tags");
    let tags = Reply::answer(200, &message("garden"));
    let provider = Provider::start(vec![answer(), tags]);
    put(
        &scratch,
        &config(
            &provider.url(),
            "derive.tags.enabled = true\nderive.tags.pick = [\"garden\", \"bilbo\"]\n",
        ),
    );
    let run = derive(&scratch, &["derive"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout
            .starts_with("entries 1, calls 2, notes 1, dropped 0, refused 0, failed 0\n"),
        "{}",
        run.stdout
    );
    let found = scratch.run(&["recall", "--in", "tag:garden", "garden", "fence"]);
    assert_eq!(found.code, 0, "{}", found.stderr);
    let headers: Vec<&str> = found
        .stdout
        .lines()
        .filter(|l| l.starts_with("=== "))
        .collect();
    assert_eq!(headers.len(), 2, "{}", found.stdout);
    assert!(
        headers.iter().any(|line| line.ends_with("\tnote")),
        "{headers:?}"
    );
    let none = scratch.run(&["recall", "--in", "tag:bilbo", "garden", "fence"]);
    assert!(!none.stdout.contains("=== "), "{}", none.stdout);
}

fn summarising(url: &str) -> String {
    format!(
        "derive.enabled = true\nllm.provider = \"anthropic\"\nderive.in = [\"projects\", \"notes\"]\nderive.quiet = \"0s\"\nderive.summary.enabled = true\nderive.summary.least = 0\nllm.anthropic.model = \"claude-test-1\"\nllm.anthropic.api.endpoint = \"{url}/v1/messages\"\ndigest.markdown.most = 6\ndigest.session.most = 6\ndigest.note.most = 6\ndigest.markdown.floor = 0\ndigest.session.floor = 0\ndigest.note.floor = 0\ndigest.session.old_after = \"36500d\"\ndigest.session.subagents = false\ndigest.session.parts = false\ndigest.note.width = 300\ndigest.summary.enabled = true\n"
    )
}

const FILE_SUMMARY: &str = "The vegetable beds since March and what was sown in each.";
const SESSION_SUMMARY: &str = "The garden fence was asked about.";

#[test]
fn a_session_and_a_file_get_summaries_and_digest_and_read_show_them() {
    let scratch = Scratch::new("e2e_derive_summaries");
    let session = scratch.dir.join(format!(
        "home/.claude/projects/-home-me-src-garden/{UUID}.jsonl"
    ));
    fs::create_dir_all(session.parent().expect("a parent")).expect("folders");
    fs::write(&session, SESSION).expect("a session");
    let notes = scratch.dir.join("home/notes");
    fs::create_dir_all(&notes).expect("folders");
    fs::write(notes.join("sections.md"), NOTE).expect("a note");
    let target = session.to_str().expect("UTF-8").to_owned();
    for _ in 0..2 {
        for place in [target.clone(), scratch.path("home/notes")] {
            assert_eq!(scratch.run(&["remember", &place]).code, 0);
        }
        scratch.backdate(120);
    }
    let provider = Provider::start(vec![
        answer(),
        Reply::answer(200, &message(SESSION_SUMMARY)),
        Reply::answer(200, &message(FILE_SUMMARY)),
    ]);
    put(&scratch, &summarising(&provider.url()));
    let kept = derive(&scratch, &["derive", UUID]);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    assert_eq!(kept.stderr.trim_end(), "bilbo: summarised 1 entries");
    let file = scratch
        .run(&["read", "01JC5Q3V7XKM4P8R2T6W9YB0DG"])
        .stdout
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("bilbo:"))
        .expect("the file's id")
        .to_owned();
    let kept = derive(&scratch, &["derive", &file]);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    assert_eq!(kept.stderr.trim_end(), "bilbo: summarised 1 entries");
    let digest = scratch.run_with_input(
        &["digest"],
        br#"{"session_id":"11111111-2222-4333-8444-555555555555","prompt":"garden beds carrots fence"}"#,
    );
    assert_eq!(digest.code, 0, "{}", digest.stderr);
    assert!(
        digest
            .stdout
            .lines()
            .any(|line| line.contains("sections.md")
                && line.ends_with(&format!("\tsummary: {FILE_SUMMARY}"))),
        "{}",
        digest.stdout
    );
    assert!(
        digest
            .stdout
            .lines()
            .any(|line| line.contains("#r")
                && line.ends_with(&format!("\tsummary: {SESSION_SUMMARY}"))),
        "{}",
        digest.stdout
    );
    let read = scratch.run(&["read", &file]);
    assert_eq!(read.code, 0, "{}", read.stderr);
    assert!(
        read.stdout
            .contains(&format!("\nsummary: {FILE_SUMMARY}\noutline:\n")),
        "{}",
        read.stdout
    );
}

#[test]
fn a_dry_run_with_the_summary_on_prints_its_line_and_sends_nothing() {
    let scratch = sealed("e2e_derive_summary_dry");
    let provider = Provider::start(vec![answer()]);
    put(&scratch, &summarising(&provider.url()));
    let run = derive(&scratch, &["derive", "--dry-run"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let lines: Vec<&str> = run
        .stdout
        .lines()
        .filter(|line| line.starts_with("summaries:"))
        .collect();
    assert_eq!(lines.len(), 1, "{}", run.stdout);
    assert!(
        lines[0].starts_with(
            "summaries: 1 entries due; this run 1 calls, 0 tokens to send; instruction (built in, "
        ) && lines[0].ends_with(')'),
        "{}",
        lines[0]
    );
    assert!(provider.requests().is_empty());
}
