use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bilbo_llm::anthropic::tool::{self, Claude};
use bilbo_llm::model::{Cause, Limits, Model, Outcome, Prompt, Usage, Variables, What};
use bilbo_settings::file::File;
use bilbo_settings::key::Source;

use crate::Scratch;

const ANSWER: &str = include_str!("../../fixtures/anthropic/tool/answer.json");
const REFUSAL: &str = include_str!("../../fixtures/anthropic/tool/refusal.json");
const REFUSAL_TWO_TURNS: &str =
    include_str!("../../fixtures/anthropic/tool/refusal-two-turns.json");
const ERROR: &str = include_str!("../../fixtures/anthropic/tool/error.json");
const TWO_TURNS: &str = include_str!("../../fixtures/anthropic/tool/two-turns.json");
const DENIED: &str = include_str!("../../fixtures/anthropic/tool/denied.json");
const CUT: &str = include_str!("../../fixtures/anthropic/tool/cut.json");
const SEARCHED: &str = include_str!("../../fixtures/anthropic/tool/searched.json");
const FETCHED: &str = include_str!("../../fixtures/anthropic/tool/fetched.json");
const SEARCHED_BY_MODEL: &str =
    include_str!("../../fixtures/anthropic/tool/searched-by-model.json");
const SUBAGENT: &str = include_str!("../../fixtures/anthropic/tool/subagent.json");
const SUBAGENT_REFUSED: &str = include_str!("../../fixtures/anthropic/tool/subagent-refused.json");
const REFUSAL_SEARCHED: &str = include_str!("../../fixtures/anthropic/tool/refusal-searched.json");
const WITHOUT: [(&str, &str); 6] = [
    (
        include_str!("../../fixtures/anthropic/tool/no-turns.json"),
        "num_turns",
    ),
    (
        include_str!("../../fixtures/anthropic/tool/no-denials.json"),
        "permission_denials",
    ),
    (
        include_str!("../../fixtures/anthropic/tool/no-server-tools.json"),
        "usage.server_tool_use",
    ),
    (
        include_str!("../../fixtures/anthropic/tool/no-search-count.json"),
        "modelUsage.*.webSearchRequests",
    ),
    (
        include_str!("../../fixtures/anthropic/tool/no-subagents.json"),
        "subagent_stats",
    ),
    (
        include_str!("../../fixtures/anthropic/tool/no-stop-reason.json"),
        "stop_reason",
    ),
];

const AFTER: [&str; 12] = [
    "-p",
    "--model",
    "{model}",
    "--effort",
    "{effort}",
    "--tools",
    "",
    "--strict-mcp-config",
    "--no-session-persistence",
    "--safe-mode",
    "--system-prompt",
    "{instruction}",
];
const MARKER: &str = "zz-marker.md";

struct Rig {
    scratch: Scratch,
    state: PathBuf,
    work: PathBuf,
    out: PathBuf,
}

fn rig(name: &str) -> Rig {
    let scratch = Scratch::new(name);
    let state = scratch.path().join("state");
    let work = state.join("derive").join("work");
    let out = scratch.path().join("out");
    fs::create_dir_all(&out).expect("out");
    Rig {
        scratch,
        state,
        work,
        out,
    }
}

fn quoted(items: &[&str]) -> String {
    let items: Vec<String> = items.iter().map(|item| format!("{item:?}")).collect();
    format!("[{}]", items.join(", "))
}

fn config(program: &str, script: &Path, after: &[&str], tail: &str) -> String {
    let program = if program == "/bin/sh" {
        script.display().to_string()
    } else {
        program.to_owned()
    };
    format!(
        "[llm.anthropic]\nmodel = \"test-model-1\"\n\n[llm.anthropic.tool]\nprogram = {program:?}\narguments = {}\n{tail}",
        quoted(after)
    )
}

fn variables(home: Option<&Path>) -> Variables {
    let home = home.map(|home| home.display().to_string());
    Box::new(move |name| match name {
        "HOME" => home.clone(),
        "TEST_ONE" => Some("one".to_owned()),
        "TEST_TWO" => Some("inherited".to_owned()),
        _ => None,
    })
}

fn tool(text: &str, work: &Path, home: Option<&Path>) -> Claude {
    let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
    let section = tool::section(&Source {
        var: &|_| None,
        file: &file,
    })
    .expect("a section");
    Claude::new(section, variables(home), work.to_path_buf())
}

fn prompt() -> Prompt {
    Prompt {
        instruction: "Write paragraphs.".to_owned(),
        material: "#r1 hello".to_owned(),
    }
}

fn limits() -> Limits {
    Limits {
        answer_tokens: 16000,
        timeout: Duration::from_secs(10),
        response: 1 << 20,
        poll: Duration::from_millis(5),
    }
}

#[test]
fn the_tool_gets_the_default_arguments_and_environment() {
    let rig = rig("claude-defaults");
    let out = rig.out.display();
    let script = rig.scratch.executable(
        "tool.sh",
        &format!(
            "ls -A > {out}/ls\npwd > {out}/pwd\nprintf '%s\\n' \"$@\" > {out}/args\nenv > {out}/env\ncat > {out}/stdin\ncat {out}/../reply.txt"
        ),
    );
    fs::write(rig.scratch.path().join("reply.txt"), ANSWER).expect("reply");
    let home = rig.scratch.path().join("home");
    let text = format!(
        "[llm.anthropic]\nmodel = \"test-model-1\"\n\n[llm.anthropic.tool]\nprogram = {:?}\nproject_files = [\"{MARKER}\"]\n",
        script.display().to_string()
    );
    let mut writer = tool(&text, &rig.work, Some(&home));
    writer.prepare().expect("prepare");
    assert!(matches!(
        writer.call(&prompt(), &limits()),
        Outcome::Answer { .. }
    ));
    let read = |name: &str| fs::read_to_string(rig.out.join(name)).expect(name);
    assert_eq!(
        read("args").lines().collect::<Vec<_>>(),
        [
            "-p",
            "--model",
            "test-model-1",
            "--effort",
            "medium",
            "--output-format",
            "json",
            "--tools",
            "",
            "--disable-slash-commands",
            "--strict-mcp-config",
            "--setting-sources",
            "project",
            "--settings",
            "{\"disableAllHooks\": true}",
            "--no-session-persistence",
            "--safe-mode",
            "--system-prompt",
            "Write paragraphs.",
        ]
    );
    let mut env: Vec<String> = read("env")
        .lines()
        .filter(|line| {
            !["PWD=", "SHLVL=", "_=", "OLDPWD="]
                .iter()
                .any(|skip| line.starts_with(skip))
        })
        .map(str::to_owned)
        .collect();
    env.sort();
    assert_eq!(
        env,
        [
            "DISABLE_PROMPT_CACHING=1".to_owned(),
            format!("HOME={}", home.display()),
            "LANG=en_US.UTF-8".to_owned(),
        ]
    );
    assert_eq!(read("pwd").trim_end(), rig.work.display().to_string());
    assert_eq!(read("ls"), "");
    assert_eq!(read("stdin"), "#r1 hello");
}

fn marked(rig: &Rig, project_files: &str) -> (String, PathBuf) {
    let script = rig
        .scratch
        .executable("tool.sh", &format!("touch {}/MADE", rig.out.display()));
    let tail = format!("project_files = {project_files}\n");
    (
        config("/bin/sh", &script, &AFTER, &tail),
        rig.out.join("MADE"),
    )
}

fn cause(what: What, status: Option<i32>, kind: Option<&str>, code: Option<&str>) -> Cause {
    Cause {
        what,
        status,
        kind: kind.map(str::to_owned),
        code: code.map(str::to_owned),
        request: None,
    }
}

fn usage(read: u64, written: u64) -> Usage {
    Usage {
        read: Some(read),
        written: Some(written),
    }
}

fn printing_why(output: &str, exit: i32) -> (Outcome, Option<String>) {
    let rig = rig("claude-print");
    let file = rig.out.join("reply.txt");
    fs::write(&file, output).expect("reply");
    let script = rig
        .scratch
        .executable("tool.sh", &format!("cat {}\nexit {exit}", file.display()));
    let text = config(
        "/bin/sh",
        &script,
        &AFTER,
        &format!("project_files = [\"{MARKER}\"]\n"),
    );
    let mut writer = tool(&text, &rig.work, None);
    writer.prepare().expect("prepare");
    let outcome = writer.call(&prompt(), &limits());
    (outcome, writer.why().map(str::to_owned))
}

fn printing(output: &str, exit: i32) -> Outcome {
    printing_why(output, exit).0
}

#[test]
fn the_tool_is_started_as_the_settings_say() {
    let rig = rig("claude-started");
    let out = rig.out.display();
    let script = rig.scratch.executable(
        "tool.sh",
        &format!(
            "ls -A > {out}/ls\npwd > {out}/pwd\nprintf '%s\\n' \"$@\" > {out}/args\nenv > {out}/env\ncat > {out}/stdin\ncat {out}/../reply.txt"
        ),
    );
    fs::write(rig.scratch.path().join("reply.txt"), ANSWER).expect("reply");
    let tail = format!(
        "inherit = [\"HOME\", \"TEST_ONE\", \"TEST_TWO\", \"TEST_UNSET\"]\nproject_files = [\"{MARKER}\"]\n\n[llm.anthropic.tool.environment]\nDISABLE_PROMPT_CACHING = \"1\"\nTEST_TWO = \"table\"\n"
    );
    let home = rig.scratch.path().join("home");
    let mut writer = tool(
        &config("/bin/sh", &script, &AFTER, &tail),
        &rig.work,
        Some(&home),
    );
    writer.prepare().expect("prepare");
    assert!(matches!(
        writer.call(&prompt(), &limits()),
        Outcome::Answer { .. }
    ));
    let read = |name: &str| fs::read_to_string(rig.out.join(name)).expect(name);
    assert_eq!(
        read("args").lines().collect::<Vec<_>>(),
        [
            "-p",
            "--model",
            "test-model-1",
            "--effort",
            "medium",
            "--tools",
            "",
            "--strict-mcp-config",
            "--no-session-persistence",
            "--safe-mode",
            "--system-prompt",
            "Write paragraphs.",
        ]
    );
    let mut env: Vec<String> = read("env")
        .lines()
        .filter(|line| {
            !["PWD=", "SHLVL=", "_=", "OLDPWD="]
                .iter()
                .any(|skip| line.starts_with(skip))
        })
        .map(str::to_owned)
        .collect();
    env.sort();
    assert_eq!(
        env,
        [
            "DISABLE_PROMPT_CACHING=1".to_owned(),
            format!("HOME={}", home.display()),
            "TEST_ONE=one".to_owned(),
            "TEST_TWO=table".to_owned(),
        ]
    );
    assert_eq!(read("pwd").trim_end(), rig.work.display().to_string());
    assert_eq!(read("ls"), "");
    assert_eq!(read("stdin"), "#r1 hello");
}

#[test]
fn arguments_that_could_give_the_model_a_tool_start_no_child() {
    let no_tools: Vec<&str> = AFTER[..5].iter().chain(&AFTER[7..]).copied().collect();
    let mut with_mcp = AFTER.to_vec();
    with_mcp.extend(["--mcp-config", "x"]);
    for (after, text) in [
        (
            no_tools,
            "llm.anthropic.tool.arguments must keep the model without tools: it needs \"--tools\" followed by \"\"",
        ),
        (
            with_mcp,
            "llm.anthropic.tool.arguments must keep the model without tools: it must not hold \"--mcp-config\"",
        ),
    ] {
        let rig = rig("claude-nottools");
        let script = rig
            .scratch
            .executable("tool.sh", &format!("touch {}/MADE", rig.out.display()));
        let config = config(
            "/bin/sh",
            &script,
            &after,
            &format!("project_files = [\"{MARKER}\"]\n"),
        );
        let mut writer = tool(&config, &rig.work, None);
        assert_eq!(writer.prepare().unwrap_err(), text);
        assert_eq!(
            writer.call(&prompt(), &limits()),
            Outcome::Stopped(Cause::new(What::NotStarted))
        );
        assert!(!rig.out.join("MADE").exists());
    }
}

#[test]
fn a_project_file_above_the_work_folder_starts_no_child() {
    let rig = rig("claude-above");
    fs::write(rig.scratch.path().join("CLAUDE.md"), "x").expect("file");
    let script = rig
        .scratch
        .executable("tool.sh", &format!("touch {}/MADE", rig.out.display()));
    let mut writer = tool(&config("/bin/sh", &script, &AFTER, ""), &rig.work, None);
    let text = format!(
        "llm.anthropic.tool: {0}/CLAUDE.md is above the folder the tool runs in, and claude reads it as a project's; set paths.state outside {0}",
        rig.scratch.path().display()
    );
    assert_eq!(writer.prepare().unwrap_err(), text);
    assert_eq!(
        writer.call(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::NotStarted))
    );
    assert!(!rig.out.join("MADE").exists());
}

#[test]
fn a_project_file_above_a_work_folder_that_already_exists_starts_no_child() {
    let rig = rig("claude-above-existing");
    fs::create_dir_all(&rig.work).expect("work");
    fs::create_dir_all(rig.scratch.path().join(".claude")).expect("folder");
    fs::write(
        rig.scratch.path().join(".claude").join("settings.json"),
        "{}",
    )
    .expect("file");
    let script = rig
        .scratch
        .executable("tool.sh", &format!("touch {}/MADE", rig.out.display()));
    let mut writer = tool(&config("/bin/sh", &script, &AFTER, ""), &rig.work, None);
    let text = format!(
        "llm.anthropic.tool: {0}/.claude/settings.json is above the folder the tool runs in, and claude reads it as a project's; set paths.state outside {0}",
        rig.scratch.path().display()
    );
    assert_eq!(writer.prepare().unwrap_err(), text);
    assert_eq!(
        writer.call(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::NotStarted))
    );
    assert!(!rig.out.join("MADE").exists());
}

#[test]
fn the_users_own_claude_folder_in_home_is_no_project() {
    let files = format!("[\".claude/{MARKER}\", \"{MARKER}\"]");
    let rig = rig("claude-home");
    let home = rig.scratch.path().join("home");
    let work = home.join("state").join("derive").join("work");
    fs::create_dir_all(home.join(".claude")).expect("folder");
    fs::write(home.join(".claude").join(MARKER), "x").expect("file");
    let (text, made) = marked(&rig, &files);
    let mut writer = tool(&text, &work, Some(&home));
    writer
        .prepare()
        .expect("the user's own folder is no project");
    writer.call(&prompt(), &limits());
    assert!(made.exists());

    fs::write(home.join(MARKER), "x").expect("file");
    let mut writer = tool(&text, &work, Some(&home));
    assert_eq!(
        writer.prepare().unwrap_err(),
        format!(
            "llm.anthropic.tool: {0}/{MARKER} is above the folder the tool runs in, and claude reads it as a project's; set paths.state outside {0}",
            home.display()
        )
    );

    fs::remove_file(home.join(MARKER)).expect("remove");
    let mut writer = tool(&text, &work, None);
    assert_eq!(
        writer.prepare().unwrap_err(),
        format!(
            "llm.anthropic.tool: {0}/.claude/{MARKER} is above the folder the tool runs in, and claude reads it as a project's; set paths.state outside {0}",
            home.display()
        )
    );
}

#[test]
fn the_project_files_are_the_settings() {
    let rig = rig("claude-files");
    fs::write(rig.scratch.path().join("CLAUDE.md"), "x").expect("file");
    let (text, _) = marked(&rig, "[\"marker.txt\"]");
    tool(&text, &rig.work, None)
        .prepare()
        .expect("CLAUDE.md is not in the list");
    fs::write(rig.scratch.path().join("marker.txt"), "x").expect("file");
    let error = tool(&text, &rig.work, None).prepare().unwrap_err();
    assert_eq!(
        error,
        format!(
            "llm.anthropic.tool: {0}/marker.txt is above the folder the tool runs in, and claude reads it as a project's; set paths.state outside {0}",
            rig.scratch.path().display()
        )
    );
}

#[test]
fn a_work_folder_that_is_a_link_starts_no_child() {
    let rig = rig("claude-link");
    let elsewhere = rig.scratch.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).expect("elsewhere");
    fs::write(elsewhere.join("keep"), "x").expect("file");
    fs::create_dir_all(rig.state.join("derive")).expect("derive");
    symlink(&elsewhere, &rig.work).expect("link");
    let (text, made) = marked(&rig, &format!("[\"{MARKER}\"]"));
    let mut writer = tool(&text, &rig.work, None);
    assert_eq!(
        writer.prepare().unwrap_err(),
        format!(
            "{} is a link: bilbo will not run a program there",
            rig.work.display()
        )
    );
    assert!(elsewhere.join("keep").exists());
    assert!(!made.exists());
}

#[test]
fn an_answer_is_the_result_with_the_tools_counts_and_its_model() {
    assert_eq!(
        printing(ANSWER, 0),
        Outcome::Answer {
            text: "#r1-r2 The user asked for the carrot rows to be thinned.".to_owned(),
            usage: usage(108, 40),
            model: Some("test-model-1".to_owned()),
        }
    );
}

#[test]
fn a_refusal_is_read_before_the_error_flag() {
    assert_eq!(
        printing(REFUSAL, 1),
        Outcome::Refused {
            category: None,
            usage: usage(58, 0)
        }
    );
}

#[test]
fn a_refusal_with_two_turns_is_a_failed_call() {
    assert_eq!(
        printing(REFUSAL_TWO_TURNS, 0),
        Outcome::Failed {
            cause: cause(What::ToolUsed, Some(0), None, None),
            usage: usage(58, 0)
        }
    );
}

#[test]
fn more_than_one_turn_or_a_denied_permission_is_a_failed_call() {
    for body in [TWO_TURNS, DENIED] {
        assert!(matches!(
            printing(body, 0),
            Outcome::Failed {
                cause: Cause {
                    what: What::ToolUsed,
                    ..
                },
                ..
            }
        ));
    }
}

#[test]
fn a_failed_reply_carries_its_token_counts() {
    assert_eq!(
        printing(TWO_TURNS, 0),
        Outcome::Failed {
            cause: cause(What::ToolUsed, Some(0), None, None),
            usage: usage(1400, 300)
        }
    );
}

#[test]
fn an_error_that_is_no_refusal_is_a_failed_call_and_its_text_goes_nowhere() {
    let outcome = printing(ERROR, 0);
    assert_eq!(
        outcome,
        Outcome::Failed {
            cause: cause(
                What::ToolError,
                Some(0),
                Some("error_during_execution"),
                None
            ),
            usage: usage(68, 7)
        }
    );
    assert!(!format!("{outcome:?}").contains("CANARY"));
}

#[test]
fn an_exit_that_is_not_0_is_a_failed_call_with_its_code() {
    assert_eq!(
        printing(ANSWER, 3),
        Outcome::Failed {
            cause: cause(What::ToolError, Some(3), Some("success"), None),
            usage: usage(108, 40)
        }
    );
}

#[test]
fn a_cut_answer_is_cut() {
    assert_eq!(printing(CUT, 0), Outcome::Cut(usage(108, 40)));
}

#[test]
fn output_that_is_no_reply_is_a_failed_call() {
    for body in ["plain text", "[]"] {
        assert_eq!(
            printing(body, 0),
            Outcome::Failed {
                cause: cause(What::Unreadable, Some(0), None, None),
                usage: Usage::default()
            },
            "{body}"
        );
    }
    let number = ANSWER.replace(
        "\"result\":\"#r1-r2 The user asked for the carrot rows to be thinned.\"",
        "\"result\":5",
    );
    let number = number.as_str();
    assert_eq!(
        printing(number, 0),
        Outcome::Failed {
            cause: cause(What::Unreadable, Some(0), None, None),
            usage: usage(108, 40)
        }
    );
}

#[test]
fn a_tool_that_does_not_end_in_time_is_busy_and_is_stopped() {
    let rig = rig("claude-timeout");
    let script = rig.scratch.executable(
        "tool.sh",
        &format!("echo $$ > {}/pid\nsleep 30", rig.out.display()),
    );
    let text = config(
        "/bin/sh",
        &script,
        &AFTER,
        &format!("project_files = [\"{MARKER}\"]\n"),
    );
    let mut writer = tool(&text, &rig.work, None);
    writer.prepare().expect("prepare");
    let outcome = writer.call(
        &prompt(),
        &Limits {
            timeout: Duration::from_secs(1),
            ..limits()
        },
    );
    assert_eq!(
        outcome,
        Outcome::Busy {
            wait: None,
            cause: Cause::new(What::Timeout)
        }
    );
    let pid = fs::read_to_string(rig.out.join("pid")).expect("pid");
    assert!(
        crate::gone(pid.trim().parse().expect("a pid")),
        "the tool still lives"
    );
}

#[test]
fn output_over_the_limit_is_a_failed_call() {
    let rig = rig("claude-limit");
    let script = rig.scratch.executable("tool.sh", "printf '%02000d' 0");
    let text = config(
        "/bin/sh",
        &script,
        &AFTER,
        &format!("project_files = [\"{MARKER}\"]\n"),
    );
    let mut writer = tool(&text, &rig.work, None);
    writer.prepare().expect("prepare");
    assert_eq!(
        writer.call(
            &prompt(),
            &Limits {
                response: 100,
                ..limits()
            }
        ),
        Outcome::Failed {
            cause: Cause::new(What::TooLarge),
            usage: Usage::default()
        }
    );
}

#[test]
fn a_program_that_is_not_there_stops_the_run() {
    let rig = rig("claude-missing");
    let missing = rig.scratch.path().join("missing");
    let text = config(
        &missing.display().to_string(),
        &rig.scratch.path().join("none"),
        &AFTER,
        &format!("project_files = [\"{MARKER}\"]\n"),
    );
    let mut writer = tool(&text, &rig.work, None);
    writer.prepare().expect("prepare starts nothing");
    assert_eq!(
        writer.call(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::NotStarted))
    );
}

#[test]
fn the_work_folder_is_made_private_and_emptied_before_each_run() {
    let rig = rig("claude-private");
    let out = rig.out.display();
    let script = rig.scratch.executable(
        "tool.sh",
        &format!("ls -A >> {out}/ls\ntouch left\ncat {out}/../reply.txt"),
    );
    fs::write(rig.scratch.path().join("reply.txt"), ANSWER).expect("reply");
    let text = config(
        "/bin/sh",
        &script,
        &AFTER,
        &format!("project_files = [\"{MARKER}\"]\n"),
    );
    let mut writer = tool(&text, &rig.work, None);
    writer.prepare().expect("prepare");
    assert_eq!(
        fs::metadata(&rig.work).expect("work").permissions().mode() & 0o777,
        0o700
    );
    writer.call(&prompt(), &limits());
    assert!(rig.work.join("left").exists());
    fs::write(rig.work.join("extra"), "x").expect("extra");
    writer.call(&prompt(), &limits());
    assert_eq!(fs::read_to_string(rig.out.join("ls")).expect("ls"), "");
}

#[test]
fn the_lines_for_the_dry_run_show_the_arguments_as_written() {
    let file = File::parse(
        Path::new("/c/config.toml"),
        b"[llm.anthropic]\nmodel = \"m\"\n",
    )
    .expect("toml");
    let section = tool::section(&Source {
        var: &|_| None,
        file: &file,
    })
    .expect("section");
    let writer = Claude::new(section, variables(None), PathBuf::from("/s/derive/work"));
    assert_eq!(
        writer.describe(),
        [
            "provider: anthropic, via tool, model m, effort medium, program claude",
            r#"arguments: ["-p", "--model", "{model}", "--effort", "{effort}", "--output-format", "json", "--tools", "", "--disable-slash-commands", "--strict-mcp-config", "--setting-sources", "project", "--settings", "{\"disableAllHooks\": true}", "--no-session-persistence", "--safe-mode", "--system-prompt", "{instruction}"]"#,
            r#"inherit: ["HOME", "USER", "PATH", "TMPDIR"]"#,
            r#"environment: { DISABLE_PROMPT_CACHING = "1", LANG = "en_US.UTF-8" }"#,
            r#"project_files: ["CLAUDE.md", "CLAUDE.local.md", "AGENTS.md", ".mcp.json", ".claude/CLAUDE.md", ".claude/rules", ".claude/skills", ".claude/commands", ".claude/settings.json", ".claude/settings.local.json"]"#,
        ]
    );
}

#[test]
fn a_placeholder_that_makes_a_flag_starts_no_child() {
    let rig = rig("claude-placeholder");
    let script = rig
        .scratch
        .executable("tool.sh", &format!("touch {}/MADE", rig.out.display()));
    let mut after = AFTER.to_vec();
    after.push("--{model}");
    let text = config(
        "/bin/sh",
        &script,
        &after,
        &format!("project_files = [\"{MARKER}\"]\n"),
    )
    .replace("test-model-1", "chrome");
    let mut writer = tool(&text, &rig.work, None);
    assert_eq!(
        writer.prepare().unwrap_err(),
        "llm.anthropic.tool.arguments must keep the model without tools: it must not hold \"--{model}\""
    );
    assert_eq!(
        writer.call(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::NotStarted))
    );
    assert!(!rig.out.join("MADE").exists());
}

#[test]
fn a_project_file_above_the_work_folders_real_path_starts_no_child() {
    for (file, refused) in [(true, true), (false, false)] {
        let rig = rig("claude-real");
        let root = rig.scratch.path();
        fs::create_dir_all(root.join("safe")).expect("safe");
        fs::create_dir_all(root.join("project").join("state")).expect("project");
        symlink(
            root.join("project").join("state"),
            root.join("safe").join("state"),
        )
        .expect("link");
        if file {
            fs::write(root.join("project").join("CLAUDE.md"), "x").expect("file");
        }
        let work = root.join("safe").join("state").join("derive").join("work");
        let (text, made) = marked(&rig, "[\"CLAUDE.md\"]");
        let mut writer = tool(&text, &work, None);
        if refused {
            assert_eq!(
                writer.prepare().unwrap_err(),
                format!(
                    "llm.anthropic.tool: {0}/project/CLAUDE.md is above the folder the tool runs in, and claude reads it as a project's; set paths.state outside {0}/project",
                    root.display()
                )
            );
            assert_eq!(
                writer.call(&prompt(), &limits()),
                Outcome::Stopped(Cause::new(What::NotStarted))
            );
            assert!(!made.exists());
        } else {
            writer.prepare().expect("nothing above");
            writer.call(&prompt(), &limits());
            assert!(made.exists());
        }
    }
}

#[test]
fn the_claude_folder_exception_is_for_the_home_the_child_gets() {
    let files = "[\".claude/settings.json\"]";
    let rig = rig("claude-childhome");
    let home = rig.scratch.path().join("home");
    let work = home.join("state").join("derive").join("work");
    fs::create_dir_all(home.join(".claude")).expect("folder");
    fs::write(home.join(".claude").join("settings.json"), "{}").expect("file");
    let (text, made) = marked(&rig, files);
    let refused = format!(
        "llm.anthropic.tool: {0}/.claude/settings.json is above the folder the tool runs in, and claude reads it as a project's; set paths.state outside {0}",
        home.display()
    );
    let elsewhere = format!(
        "{text}\n[llm.anthropic.tool.environment]\nHOME = \"{}\"\n",
        rig.scratch.path().join("elsewhere").display()
    );
    assert_eq!(
        tool(&elsewhere, &work, Some(&home)).prepare().unwrap_err(),
        refused
    );
    let out_of_inherit = format!("{text}inherit = [\"TEST_ONE\"]\n");
    assert_eq!(
        tool(&out_of_inherit, &work, Some(&home))
            .prepare()
            .unwrap_err(),
        refused
    );
    let mut writer = tool(&text, &work, Some(&home));
    writer.prepare().expect("the child's own home");
    writer.call(&prompt(), &limits());
    assert!(made.exists());
}

#[test]
fn a_skills_folder_above_the_work_folder_starts_no_child() {
    for folder in ["skills", "commands"] {
        let rig = rig("claude-skills");
        fs::create_dir_all(rig.scratch.path().join(".claude").join(folder)).expect("folder");
        let script = rig
            .scratch
            .executable("tool.sh", &format!("touch {}/MADE", rig.out.display()));
        let mut writer = tool(&config("/bin/sh", &script, &AFTER, ""), &rig.work, None);
        assert_eq!(
            writer.prepare().unwrap_err(),
            format!(
                "llm.anthropic.tool: {0}/.claude/{folder} is above the folder the tool runs in, and claude reads it as a project's; set paths.state outside {0}",
                rig.scratch.path().display()
            )
        );
        assert_eq!(
            writer.call(&prompt(), &limits()),
            Outcome::Stopped(Cause::new(What::NotStarted))
        );
        assert!(!rig.out.join("MADE").exists());
    }
}

#[test]
fn a_count_of_a_server_tool_or_of_a_subagent_is_a_failed_call() {
    for (name, body) in [
        ("searched", SEARCHED),
        ("fetched", FETCHED),
        ("searched-by-model", SEARCHED_BY_MODEL),
        ("subagent", SUBAGENT),
        ("subagent-refused", SUBAGENT_REFUSED),
        ("refusal-searched", REFUSAL_SEARCHED),
    ] {
        assert_eq!(
            printing(body, 0),
            Outcome::Failed {
                cause: cause(What::ToolUsed, Some(0), None, None),
                usage: usage(108, 40)
            },
            "{name}"
        );
    }
}

#[test]
fn a_reply_that_lacks_a_field_the_guard_reads_is_unreadable() {
    let unreadable = Outcome::Failed {
        cause: cause(What::Unreadable, Some(0), None, None),
        usage: usage(108, 40),
    };
    for (body, path) in WITHOUT {
        let (outcome, why) = printing_why(body, 0);
        assert_eq!(outcome, unreadable, "{path}");
        assert_eq!(
            why.as_deref(),
            Some(
                format!("the reply holds no \"{path}\": bilbo reads the reply as claude 2.1.327 prints it")
                    .as_str()
            ),
            "{path}"
        );
    }
    let stats_at = ANSWER.find("\"subagent_stats\":").expect("stats");
    let other_types = [
        (ANSWER.replace("\"num_turns\":1", "\"num_turns\":\"x\""), "num_turns"),
        (
            ANSWER.replace("\"permission_denials\":[]", "\"permission_denials\":\"x\""),
            "permission_denials",
        ),
        (
            ANSWER.replace(
                "\"server_tool_use\":{\"web_search_requests\":0,\"web_fetch_requests\":0}",
                "\"server_tool_use\":\"x\"",
            ),
            "usage.server_tool_use",
        ),
        (
            ANSWER.replace(
                "\"modelUsage\":{\"test-model-1\":{\"inputTokens\":100,\"outputTokens\":40,\"webSearchRequests\":0}}",
                "\"modelUsage\":\"x\"",
            ),
            "modelUsage",
        ),
        (
            format!("{}\"subagent_stats\":\"x\"}}", &ANSWER[..stats_at]),
            "subagent_stats",
        ),
        (ANSWER.replace("\"stop_reason\":\"end_turn\"", "\"stop_reason\":5"), "stop_reason"),
    ];
    let wanted = [
        "a whole number",
        "a list",
        "an object",
        "an object",
        "an object",
        "a text",
    ];
    for ((body, path), wanted) in other_types.iter().zip(wanted) {
        let (outcome, why) = printing_why(body, 0);
        assert_eq!(outcome, unreadable, "{path}");
        assert_eq!(
            why.as_deref(),
            Some(
                format!("the reply's \"{path}\" is not {wanted}: bilbo reads the reply as claude 2.1.327 prints it")
                    .as_str()
            ),
            "{path}"
        );
    }
}

#[test]
fn an_empty_model_is_refused_before_any_child() {
    let rig = rig("claude-nomodel");
    let (text, made) = marked(&rig, &format!("[\"{MARKER}\"]"));
    let text = text.replace("test-model-1", "");
    let mut writer = tool(&text, &rig.work, None);
    assert_eq!(
        writer.prepare().unwrap_err(),
        "llm.anthropic.model is not set"
    );
    assert!(!made.exists());
}

#[test]
fn a_tool_killed_after_it_printed_is_a_failed_call() {
    let rig = rig("claude-killed");
    let file = rig.out.join("reply.txt");
    fs::write(&file, ANSWER).expect("reply");
    let script = rig
        .scratch
        .executable("tool.sh", &format!("cat {}\nkill -9 $$", file.display()));
    let text = config(
        "/bin/sh",
        &script,
        &AFTER,
        &format!("project_files = [\"{MARKER}\"]\n"),
    );
    let mut writer = tool(&text, &rig.work, None);
    writer.prepare().expect("prepare");
    assert_eq!(
        writer.call(&prompt(), &limits()),
        Outcome::Failed {
            cause: cause(What::ToolError, None, Some("success"), None),
            usage: usage(108, 40)
        }
    );
}

#[test]
fn why_the_tool_was_not_started_is_kept_for_the_run() {
    let rig = rig("claude-why");
    let reply = rig.out.join("reply.txt");
    fs::write(&reply, ANSWER).expect("reply");
    let script = rig
        .scratch
        .executable("tool.sh", &format!("cat {}", reply.display()));
    let text = config(
        "/bin/sh",
        &script,
        &AFTER,
        &format!("project_files = [\"{MARKER}\"]\n"),
    );
    let mut writer = tool(&text, &rig.work, None);
    writer.prepare().expect("prepare");
    assert!(matches!(
        writer.call(&prompt(), &limits()),
        Outcome::Answer { .. }
    ));
    assert_eq!(writer.why(), None);
    let (refused, why) = printing_why(REFUSAL, 1);
    assert!(matches!(refused, Outcome::Refused { .. }));
    assert_eq!(why, None);

    fs::create_dir_all(rig.work.join("inner")).expect("inner");
    fs::write(rig.work.join("inner").join("file"), "x").expect("file");
    assert_eq!(
        writer.call(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::NotStarted))
    );
    assert_eq!(
        writer.why(),
        Some(
            format!(
                "{} holds a folder that is not empty: inner",
                rig.work.display()
            )
            .as_str()
        )
    );
    fs::remove_dir_all(rig.work.join("inner")).expect("remove");

    fs::write(rig.scratch.path().join(MARKER), "x").expect("marker");
    assert_eq!(
        writer.call(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::NotStarted))
    );
    assert_eq!(
        writer.why(),
        Some(
            format!(
                "llm.anthropic.tool: {0}/{MARKER} is above the folder the tool runs in, and claude reads it as a project's; set paths.state outside {0}",
                rig.scratch.path().display()
            )
            .as_str()
        )
    );
    fs::remove_file(rig.scratch.path().join(MARKER)).expect("remove");

    fs::remove_file(&script).expect("remove the program");
    assert_eq!(
        writer.call(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::NotStarted))
    );
    assert_eq!(
        writer.why(),
        Some(
            format!(
                "{}: No such file or directory (os error 2)",
                script.display()
            )
            .as_str()
        )
    );
}

#[test]
fn a_name_of_the_reply_reaches_no_sentence() {
    let stats_at = ANSWER.find("\"subagent_stats\":").expect("stats");
    let cases = [
        (
            ANSWER.replace(
                "\"test-model-1\":{\"inputTokens\":100,\"outputTokens\":40,\"webSearchRequests\":0}",
                "\"CANARY-MODEL\":{\"inputTokens\":100}",
            ),
            "the reply holds no \"modelUsage.*.webSearchRequests\": bilbo reads the reply as claude 2.1.327 prints it",
        ),
        (
            ANSWER.replace(
                "\"server_tool_use\":{\"web_search_requests\":0,\"web_fetch_requests\":0}",
                "\"server_tool_use\":{\"CANARY-TOOL\":\"x\"}",
            ),
            "the reply's \"usage.server_tool_use.*\" is not a whole number: bilbo reads the reply as claude 2.1.327 prints it",
        ),
        (
            format!(
                "{}\"subagent_stats\":{{\"outer\":{{\"CANARY-DEEP\":\"x\"}}}}}}",
                &ANSWER[..stats_at]
            ),
            "the reply's \"subagent_stats.*\" is not a whole number: bilbo reads the reply as claude 2.1.327 prints it",
        ),
    ];
    for (body, sentence) in cases {
        assert_ne!(body, ANSWER);
        let (outcome, why) = printing_why(&body, 0);
        assert_eq!(
            outcome,
            Outcome::Failed {
                cause: cause(What::Unreadable, Some(0), None, None),
                usage: usage(108, 40)
            }
        );
        assert_eq!(why.as_deref(), Some(sentence));
        assert!(!format!("{outcome:?}{why:?}").contains("CANARY"));
    }
}
