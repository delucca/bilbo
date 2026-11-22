use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bilbo_llm::openai::tool::{self, Codex};
use bilbo_llm::writer::{Cause, Limits, Outcome, Prompt, Usage, Variables, What, Writer};
use bilbo_settings::file::File;
use bilbo_settings::key::Source;

use crate::Scratch;

const ANSWER_TEXT: &str = "#r1-r2 The user asked for the carrot rows to be thinned.";
const MODELS: &str = include_str!("../../fixtures/openai/tool/models.json");
const ANSWER: &str = include_str!("../../fixtures/openai/tool/answer.jsonl");
const FAILED_LIMIT: &str = include_str!("../../fixtures/openai/tool/failed-limit.jsonl");
const FAILED_401: &str = include_str!("../../fixtures/openai/tool/failed-401.jsonl");
const FAILED_CUT: &str = include_str!("../../fixtures/openai/tool/failed-cut.jsonl");
const FAILED_FILTER: &str = include_str!("../../fixtures/openai/tool/failed-filter.jsonl");
const TOOL_COMMAND: &str = include_str!("../../fixtures/openai/tool/tool-command.jsonl");

const DEFAULT: [&str; 74] = [
    "exec",
    "--json",
    "--ephemeral",
    "--ignore-user-config",
    "--ignore-rules",
    "--skip-git-repo-check",
    "--strict-config",
    "-s",
    "read-only",
    "-m",
    "{model}",
    "-c",
    "model_catalog_json=\"{catalog}\"",
    "-c",
    "model_instructions_file=\"{instruction}\"",
    "-c",
    "model_reasoning_effort=\"{effort}\"",
    "--disable",
    "shell_tool",
    "--disable",
    "unified_exec",
    "--disable",
    "view_image",
    "--disable",
    "sleep_tool",
    "--disable",
    "apps",
    "--disable",
    "plugins",
    "--disable",
    "tool_suggest",
    "--disable",
    "hooks",
    "--disable",
    "image_generation",
    "--disable",
    "goals",
    "--disable",
    "shell_snapshot",
    "--disable",
    "incremental_tools",
    "--disable",
    "unbounded_connection_retries",
    "-c",
    "agents.enabled=false",
    "-c",
    "tools.experimental_request_user_input.enabled=false",
    "-c",
    "web_search=\"disabled\"",
    "-c",
    "cloud.skills.enabled=false",
    "-c",
    "skills.include_instructions=false",
    "-c",
    "skills.bundled.enabled=false",
    "-c",
    "include_environment_context=false",
    "-c",
    "include_permissions_instructions=false",
    "-c",
    "include_apps_instructions=false",
    "-c",
    "include_collaboration_mode_instructions=false",
    "-c",
    "project_doc_max_bytes=0",
    "-c",
    "project_root_markers=[]",
    "-c",
    "history.persistence=\"none\"",
    "-c",
    "analytics.enabled=false",
    "-c",
    "otel.metrics_exporter=\"none\"",
    "-",
];

const SETUP: [(&str, bool); 10] = [
    ("AGENTS.md", false),
    ("AGENTS.override.md", false),
    ("config.toml", false),
    (".env", false),
    ("environments.toml", false),
    ("rules", true),
    ("skills", true),
    ("hooks.json", false),
    ("agents", true),
    ("memories", true),
];

const VERSION: &str = "echo \"codex-cli 0.169.0\"";

struct Rig {
    scratch: Scratch,
    state: PathBuf,
    work: PathBuf,
    home: PathBuf,
    out: PathBuf,
}

fn rig(name: &str) -> Rig {
    let scratch = Scratch::new(name);
    let state = scratch.path().join("state");
    let work = state.join("derive").join("work");
    let home = state.join("derive").join("codex");
    let out = scratch.path().join("out");
    fs::create_dir_all(&out).expect("out");
    login(&home);
    fs::write(scratch.path().join("models.json"), MODELS).expect("models");
    fs::write(scratch.path().join("reply.txt"), ANSWER).expect("reply");
    Rig {
        scratch,
        state,
        work,
        home,
        out,
    }
}

fn login(home: &Path) {
    fs::create_dir_all(home).expect("a home");
    fs::set_permissions(home, fs::Permissions::from_mode(0o700)).expect("a mode");
    fs::write(home.join("auth.json"), "").expect("a login");
}

fn models(rig: &Rig) -> String {
    format!("cat {}/models.json", rig.scratch.path().display())
}

fn standard(rig: &Rig, exit: i32) -> String {
    let out = rig.out.display();
    let reply = rig.scratch.path().join("reply.txt");
    format!(
        "ls -A > {out}/ls\npwd > {out}/pwd\nprintf '%s\\n' \"$@\" > {out}/args\nenv > {out}/env\ncat > {out}/stdin\ncp catalog.json instruction.txt {out}/\ncat {}\nexit {exit}",
        reply.display()
    )
}

fn script_of(rig: &Rig, name: &str, version: &str, models: &str, exec: &str) -> PathBuf {
    let out = rig.out.display();
    rig.scratch.executable(
        name,
        &format!(
            "echo \"$0\" >> {out}/self\necho \"$1\" >> {out}/calls\ncase \"$1\" in\n  --version) {version} ;;\n  debug) {models} ;;\n  exec) {exec} ;;\nesac"
        ),
    )
}

fn script(rig: &Rig) -> PathBuf {
    script_of(rig, "tool.sh", VERSION, &models(rig), &standard(rig, 0))
}

fn config_in(model: Option<&str>, program: &str, system: &str, tail: &str) -> String {
    let model = model.map_or_else(String::new, |model| {
        format!("derive.openai.model = {model:?}\n\n")
    });
    format!("{model}[derive.openai.tool]\nprogram = {program:?}\nsystem_files = {system}\n{tail}")
}

fn config(script: &Path, tail: &str) -> String {
    config_in(Some("model-a"), &script.display().to_string(), "[]", tail)
}

fn quoted(items: &[&str]) -> String {
    let items: Vec<String> = items.iter().map(|item| format!("{item:?}")).collect();
    format!("[{}]", items.join(", "))
}

fn vars(path: Option<&str>, home: Option<&str>, codex_home: Option<&str>) -> Variables {
    let path = path.map(str::to_owned);
    let home = home.map(str::to_owned);
    let codex_home = codex_home.map(str::to_owned);
    Box::new(move |name| match name {
        "PATH" => path.clone(),
        "HOME" => home.clone(),
        "CODEX_HOME" => codex_home.clone(),
        "TEST_ONE" => Some("one".to_owned()),
        "TEST_TWO" => Some("inherited".to_owned()),
        _ => None,
    })
}

fn test_path() -> String {
    "/usr/bin:/bin".to_owned()
}

fn variables(rig: &Rig) -> Variables {
    let user = rig.scratch.path().join("user");
    vars(Some(&test_path()), Some(&user.display().to_string()), None)
}

fn adapter_at(text: &str, work: &Path, variables: Variables) -> Codex {
    constructed(text, work, variables, Duration::from_secs(10), 1 << 20)
}

fn constructed(
    text: &str,
    work: &Path,
    variables: Variables,
    timeout: Duration,
    limit: usize,
) -> Codex {
    let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
    let section = tool::section(&Source {
        var: &|_| None,
        file: &file,
    })
    .expect("a section");
    Codex::new(
        section,
        variables,
        work.to_path_buf(),
        timeout,
        limit,
        Duration::from_millis(5),
    )
}

fn adapter_with(text: &str, rig: &Rig, variables: Variables) -> Codex {
    adapter_at(text, &rig.work, variables)
}

fn adapter(text: &str, rig: &Rig) -> Codex {
    adapter_with(text, rig, variables(rig))
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

fn no_call(rig: &Rig) {
    assert!(
        !rig.out.join("calls").exists(),
        "the tool was started: {:?}",
        calls(rig)
    );
}

fn calls(rig: &Rig) -> Vec<String> {
    fs::read_to_string(rig.out.join("calls"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn read(rig: &Rig, name: &str) -> String {
    fs::read_to_string(rig.out.join(name)).expect(name)
}

fn cause(what: What, status: Option<i32>, code: Option<&str>) -> Cause {
    Cause {
        what,
        status,
        kind: None,
        code: code.map(str::to_owned),
        request: None,
    }
}

fn answered(outcome: &Outcome) -> &str {
    match outcome {
        Outcome::Answer { text, .. } => text,
        other => panic!("not an answer: {other:?}"),
    }
}

fn start(text: &str, rig: &Rig) -> Codex {
    let mut writer = adapter(text, rig);
    writer.prepare().expect("prepare");
    writer
}

fn refused(text: &str, rig: &Rig) -> String {
    adapter(text, rig).prepare().expect_err("a refusal")
}

fn t5e(home: &Path, name: &str) -> String {
    format!(
        "derive.openai.tool.home: {}/{name} is the user's setup for codex, and bilbo's codex home holds none",
        home.display()
    )
}

fn t6(file: &Path) -> String {
    format!(
        "derive.openai.tool: {} configures codex whatever its arguments are; read it, then take it out of derive.openai.tool.system_files to run with it",
        file.display()
    )
}

fn t6b(path: &Path) -> String {
    format!(
        "derive.openai.tool: {} is there and cannot be read ({}), so bilbo cannot tell whether a file there configures codex: name the files you can read in derive.openai.tool.system_files instead",
        path.display(),
        std::io::Error::from_raw_os_error(13)
    )
}

fn t5h(home: &Path, own: &Path) -> String {
    format!(
        "derive.openai.tool.home: {} and your own codex home, {}, are one folder or one is inside the other: bilbo's is a folder apart",
        home.display(),
        own.display()
    )
}

fn t5i(home: &Path, work: &Path) -> String {
    format!(
        "derive.openai.tool.home: {} and the folder the tool runs in, {}, are one folder or one is inside the other",
        home.display(),
        work.display()
    )
}

fn t8b(program: &Path) -> String {
    format!(
        "derive.openai.tool.program: {} is not codex 0.169.0, the release bilbo's arguments are read from",
        program.display()
    )
}

fn t8c(program: &str) -> String {
    format!("derive.openai.tool.program: {program} is not on PATH")
}

fn t8d(file: &Path) -> String {
    format!(
        "derive.openai.tool.program: {} changed since bilbo asked it its release",
        file.display()
    )
}

fn t9a(program: &Path) -> String {
    format!(
        "derive.openai.tool.program: {} printed no catalog of its models",
        program.display()
    )
}

fn t12(home: &Path) -> String {
    format!(
        "log in again with CODEX_HOME={} codex login",
        home.display()
    )
}

const T11C: &str =
    "codex reached a tool of its own: bilbo's arguments did not keep it without tools";

const TOOL_LINE: &str =
    "{\"type\":\"item.started\",\"item\":{\"id\":\"item_0\",\"type\":\"KIND\"}}";

fn t5o(home: &Path) -> String {
    format!(
        "derive.openai.tool.home: {0} can be written by others: make it yours alone with chmod 700 {0}",
        home.display()
    )
}

fn t8e(program: &Path, what: &str) -> String {
    format!(
        "derive.openai.tool.program: {} did not end within derive.timeout when asked {what}",
        program.display()
    )
}

fn t8f(program: &Path, what: &str) -> String {
    format!(
        "derive.openai.tool.program: {} printed more than derive.response allows for {what}: raise derive.response",
        program.display()
    )
}

struct Modes(Vec<(PathBuf, u32)>);

impl Drop for Modes {
    fn drop(&mut self) {
        for (path, mode) in &self.0 {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(*mode));
        }
    }
}

#[test]
fn the_tool_gets_the_default_arguments_its_two_files_and_its_variables() {
    let rig = rig("openai-defaults");
    let script = script(&rig);
    let mut writer = start(&config(&script, ""), &rig);
    let outcome = writer.write(&prompt(), &limits());
    assert_eq!(answered(&outcome), ANSWER_TEXT);
    assert_eq!(calls(&rig), ["--version", "debug", "--version", "exec"]);
    let catalog = rig.work.join("catalog.json");
    let instruction = rig.work.join("instruction.txt");
    let expected: Vec<String> = DEFAULT
        .iter()
        .map(|item| {
            item.replace("{model}", "model-a")
                .replace("{effort}", "medium")
                .replace("{catalog}", &catalog.display().to_string())
                .replace("{instruction}", &instruction.display().to_string())
        })
        .collect();
    assert_eq!(read(&rig, "args").lines().collect::<Vec<_>>(), expected);
    assert_eq!(read(&rig, "pwd").trim_end(), rig.work.display().to_string());
    assert_eq!(read(&rig, "stdin"), "#r1 hello");
    assert_eq!(read(&rig, "instruction.txt"), "Write paragraphs.");
    assert_eq!(
        fs::read(rig.out.join("catalog.json")).expect("catalog"),
        tool::catalog("x", MODELS.as_bytes(), "model-a", "medium").expect("a catalog")
    );
    let mut env: Vec<String> = read(&rig, "env")
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
            format!("CODEX_HOME={}", rig.home.display()),
            format!("HOME={}", rig.work.display()),
            format!("PATH={}", test_path()),
        ]
    );
    assert_eq!(read(&rig, "ls"), "catalog.json\ninstruction.txt\n");

    let lnk = rig.scratch.path().join("lnk");
    symlink(&rig.state, &lnk).expect("link");
    let through = lnk.join("derive").join("work");
    let mut writer = adapter_at(&config(&script, ""), &through, variables(&rig));
    writer.prepare().expect("prepare");
    let outcome = writer.write(&prompt(), &limits());
    assert_eq!(answered(&outcome), ANSWER_TEXT);
    let args = read(&rig, "args");
    assert!(args.contains(&format!("{}\"", catalog.display())), "{args}");
    assert!(
        args.contains(&format!("{}\"", instruction.display())),
        "{args}"
    );
    assert!(!args.contains(&lnk.display().to_string()), "{args}");
    assert_eq!(read(&rig, "pwd").trim_end(), rig.work.display().to_string());
}

#[test]
fn the_variables_are_the_settings_and_bilbos_two_come_last() {
    let rig = rig("openai-variables");
    let script = script(&rig);
    let tail = "inherit = [\"TEST_ONE\"]\n\n[derive.openai.tool.environment]\nTEST_TWO = \"two\"\n";
    let env_of = |rig: &Rig| {
        let mut env: Vec<String> = read(rig, "env")
            .lines()
            .filter(|line| {
                !["PWD=", "SHLVL=", "_=", "OLDPWD="]
                    .iter()
                    .any(|skip| line.starts_with(skip))
            })
            .map(str::to_owned)
            .collect();
        env.sort();
        env
    };
    let mut writer = start(&config(&script, tail), &rig);
    writer.write(&prompt(), &limits());
    assert_eq!(
        env_of(&rig),
        [
            format!("CODEX_HOME={}", rig.home.display()),
            format!("HOME={}", rig.work.display()),
            "TEST_ONE=one".to_owned(),
            "TEST_TWO=two".to_owned(),
        ]
    );
    let other = rig.scratch.path().join("other");
    login(&other);
    let tail = format!("home = {:?}\n{tail}", other.display().to_string());
    let mut writer = start(&config(&script, &tail), &rig);
    writer.write(&prompt(), &limits());
    assert_eq!(
        env_of(&rig),
        [
            format!("CODEX_HOME={}", other.display()),
            format!("HOME={}", rig.work.display()),
            "TEST_ONE=one".to_owned(),
            "TEST_TWO=two".to_owned(),
        ]
    );
}

#[test]
fn a_home_that_is_missing_a_link_or_no_folder_starts_no_child() {
    let rig = rig("openai-home-kinds");
    let script = script(&rig);
    let root = rig.scratch.path();
    let missing = root.join("nowhere");
    let linked = root.join("linked");
    symlink(&rig.home, &linked).expect("link");
    let file = root.join("a-file");
    fs::write(&file, "").expect("file");
    for (home, expected) in [
        (
            &missing,
            format!(
                "derive.openai.tool.home: {0} does not exist: make it, then log in with CODEX_HOME={0} codex login",
                missing.display()
            ),
        ),
        (
            &linked,
            format!("derive.openai.tool.home: {} is a link", linked.display()),
        ),
        (
            &file,
            format!(
                "derive.openai.tool.home: {} is not a folder",
                file.display()
            ),
        ),
    ] {
        let tail = format!("home = {:?}\n", home.display().to_string());
        assert_eq!(refused(&config(&script, &tail), &rig), expected);
        no_call(&rig);
    }
}

#[test]
fn a_home_with_the_users_setup_starts_no_child() {
    for (name, folder) in SETUP {
        let rig = rig("openai-setup");
        let script = script(&rig);
        let entry = rig.home.join(name);
        if folder {
            fs::create_dir(&entry).expect("a folder");
        } else {
            fs::write(&entry, "x").expect("a file");
        }
        assert_eq!(
            refused(&config(&script, ""), &rig),
            t5e(&rig.home, name),
            "{name}"
        );
        no_call(&rig);
    }
}

#[test]
fn a_home_with_no_login_starts_no_child() {
    let rig = rig("openai-no-login");
    let script = script(&rig);
    fs::remove_file(rig.home.join("auth.json")).expect("remove");
    assert_eq!(
        refused(&config(&script, ""), &rig),
        format!(
            "derive.openai.tool.home: {0} holds no login: run CODEX_HOME={0} codex login",
            rig.home.display()
        )
    );
    no_call(&rig);
}

#[test]
fn a_file_of_the_machine_that_configures_codex_starts_no_child() {
    let attempt = |rig: &Rig, script: &Path, item: &Path| {
        let text = config_in(
            Some("model-a"),
            &script.display().to_string(),
            &quoted(&[item.display().to_string().as_str()]),
            "",
        );
        adapter(&text, rig).prepare()
    };

    let rig = rig("openai-system-file");
    let script = script(&rig);
    let etc = rig.scratch.path().join("etc");
    fs::create_dir_all(&etc).expect("etc");
    let managed = etc.join("managed.toml");
    assert_eq!(attempt(&rig, &script, &managed), Ok(()));
    let started = calls(&rig).len();
    fs::write(&managed, "x").expect("file");
    assert_eq!(attempt(&rig, &script, &managed).unwrap_err(), t6(&managed));
    fs::remove_file(&managed).expect("remove");
    symlink(rig.scratch.path().join("nowhere"), &managed).expect("link");
    assert_eq!(attempt(&rig, &script, &managed).unwrap_err(), t6(&managed));
    assert_eq!(calls(&rig).len(), started);

    fs::remove_file(&managed).expect("remove");
    let star = etc.join("*");
    let started = calls(&rig).len();
    let config_toml = etc.join("config.toml");
    fs::write(&config_toml, "x").expect("file");
    assert_eq!(attempt(&rig, &script, &star).unwrap_err(), t6(&config_toml));
    fs::remove_file(&config_toml).expect("remove");
    let dangling = etc.join("dangling");
    symlink(rig.scratch.path().join("nowhere"), &dangling).expect("link");
    assert_eq!(attempt(&rig, &script, &star).unwrap_err(), t6(&dangling));
    fs::remove_file(&dangling).expect("remove");
    assert_eq!(calls(&rig).len(), started);
    assert_eq!(attempt(&rig, &script, &star), Ok(()));
    fs::write(&star, "x").expect("a file named star");
    assert_eq!(attempt(&rig, &script, &star).unwrap_err(), t6(&star));

    let rig = self::rig("openai-system-wild");
    let script = self::script(&rig);
    let mp = rig.scratch.path().join("mp");
    let wild = mp.join("*").join("x.plist");
    assert_eq!(attempt(&rig, &script, &wild), Ok(()));
    fs::create_dir_all(mp.join("u1")).expect("u1");
    assert_eq!(attempt(&rig, &script, &wild), Ok(()));
    fs::write(mp.join("f"), "x").expect("f");
    assert_eq!(attempt(&rig, &script, &wild), Ok(()));
    fs::create_dir(mp.join("*")).expect("a folder named star");
    assert_eq!(attempt(&rig, &script, &wild), Ok(()));
    fs::write(mp.join("*").join("x.plist"), "x").expect("a file in it");
    assert_eq!(
        attempt(&rig, &script, &wild).unwrap_err(),
        t6(&mp.join("*").join("x.plist"))
    );
    fs::remove_file(mp.join("*").join("x.plist")).expect("remove");
    let started = calls(&rig).len();
    fs::create_dir_all(mp.join("u2")).expect("u2");
    fs::write(mp.join("u2").join("x.plist"), "x").expect("u2 file");
    fs::write(mp.join("u1").join("x.plist"), "x").expect("u1 file");
    assert_eq!(
        attempt(&rig, &script, &wild).unwrap_err(),
        t6(&mp.join("u1").join("x.plist"))
    );
    fs::remove_file(mp.join("u1").join("x.plist")).expect("remove");
    fs::remove_file(mp.join("u2").join("x.plist")).expect("remove");

    let modes = Modes(vec![(mp.clone(), 0o755), (mp.join("u1"), 0o755)]);
    fs::set_permissions(mp.join("u1"), fs::Permissions::from_mode(0o000)).expect("mode");
    let first = attempt(&rig, &script, &wild);
    fs::set_permissions(mp.join("u1"), fs::Permissions::from_mode(0o755)).expect("mode");
    fs::set_permissions(&mp, fs::Permissions::from_mode(0o000)).expect("mode");
    let second = attempt(&rig, &script, &wild);
    drop(modes);
    assert_eq!(first.unwrap_err(), t6b(&mp.join("u1").join("x.plist")));
    assert_eq!(second.unwrap_err(), t6b(&mp));
    assert_eq!(calls(&rig).len(), started);
}

#[test]
fn another_release_starts_no_call() {
    for version in [
        "echo \"codex-cli 0.170.0\"",
        "echo \"codex-cli 0.169.0-alpha.1\"",
        "echo \"codex-cli 0.169.0 extra\"",
        "true",
        "echo \"codex-cli 0.169.0\"; exit 1",
    ] {
        let rig = rig("openai-release");
        let script = script_of(&rig, "tool.sh", version, &models(&rig), &standard(&rig, 0));
        assert_eq!(
            refused(&config(&script, ""), &rig),
            t8b(&script),
            "{version}"
        );
        assert_eq!(calls(&rig), ["--version"], "{version}");
    }
    let rig = rig("openai-release-missing");
    let missing = rig.scratch.path().join("missing");
    let error = refused(&config(&missing, ""), &rig);
    assert!(
        error.starts_with(&format!(
            "derive.openai.tool.program: {}: ",
            missing.display()
        )),
        "{error}"
    );
    no_call(&rig);
}

#[test]
fn a_program_with_no_catalog_or_without_the_model_starts_no_call() {
    let catalog = |rig: &Rig| format!("{}; exit 3", models(rig));
    for models in [Some("echo oops"), None] {
        let rig = rig("openai-catalog");
        let models = models.map_or_else(|| catalog(&rig), str::to_owned);
        let script = script_of(&rig, "tool.sh", VERSION, &models, &standard(&rig, 0));
        assert_eq!(
            refused(&config(&script, ""), &rig),
            t9a(&script),
            "{models}"
        );
        assert_eq!(calls(&rig), ["--version", "debug"]);
    }
    let rig = self::rig("openai-no-model");
    let script = self::script(&rig);
    let text = config_in(Some("model-z"), &script.display().to_string(), "[]", "");
    assert_eq!(
        refused(&text, &rig),
        "derive.openai.model: codex 0.169.0 has no such model; codex debug models --bundled prints the ones it has"
    );
    assert_eq!(calls(&rig), ["--version", "debug"]);
    let rig = self::rig("openai-no-effort");
    let script = self::script(&rig);
    let text = config_in(
        Some("model-b"),
        &script.display().to_string(),
        "[]",
        "effort = \"high\"\n",
    );
    assert_eq!(
        refused(&text, &rig),
        "derive.openai.tool.effort: codex 0.169.0 lists no such effort for the model"
    );
    assert_eq!(calls(&rig), ["--version", "debug"]);
}

#[test]
fn arguments_that_could_give_the_model_a_tool_start_no_child() {
    let rig = rig("openai-arguments");
    let script = script(&rig);
    let mut items: Vec<&str> = DEFAULT.to_vec();
    let at = items
        .windows(2)
        .position(|pair| pair == ["--disable", "goals"])
        .expect("goals");
    items.drain(at..at + 2);
    let tail = format!("arguments = {}\n", quoted(&items));
    assert_eq!(
        refused(&config(&script, &tail), &rig),
        "derive.openai.tool.arguments must keep the model without tools: it needs \"--disable\" followed by \"goals\""
    );
    no_call(&rig);
}

#[test]
fn an_empty_model_is_refused_before_any_child() {
    let rig = rig("openai-no-model-set");
    let script = script(&rig);
    let text = config_in(None, &script.display().to_string(), "[]", "");
    assert_eq!(refused(&text, &rig), "derive.openai.model is not set");
    no_call(&rig);
}

#[test]
fn the_checks_run_again_before_each_call() {
    let rig = rig("openai-again");
    let script = script(&rig);
    let etc = rig.scratch.path().join("etc");
    fs::create_dir_all(&etc).expect("etc");
    let listed = etc.join("x");
    let text = config_in(
        Some("model-a"),
        &script.display().to_string(),
        &quoted(&[listed.display().to_string().as_str()]),
        "",
    );
    let mut writer = start(&text, &rig);
    fs::write(rig.home.join("AGENTS.md"), "x").expect("file");
    assert_eq!(
        writer.write(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::NotStarted))
    );
    assert_eq!(writer.why(), Some(t5e(&rig.home, "AGENTS.md").as_str()));
    assert_eq!(calls(&rig), ["--version", "debug"]);
    fs::remove_file(rig.home.join("AGENTS.md")).expect("remove");
    fs::write(&listed, "x").expect("file");
    assert_eq!(
        writer.write(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::NotStarted))
    );
    assert_eq!(writer.why(), Some(t6(&listed).as_str()));
    assert_eq!(calls(&rig), ["--version", "debug"]);
}

#[test]
fn the_work_folder_is_emptied_and_holds_the_two_files_at_each_call() {
    let rig = rig("openai-work");
    let script = script(&rig);
    let mut writer = start(&config(&script, ""), &rig);
    assert_eq!(
        fs::metadata(&rig.work).expect("work").permissions().mode() & 0o777,
        0o700
    );
    fs::write(rig.work.join("stale"), "x").expect("stale");
    writer.write(&prompt(), &limits());
    assert_eq!(read(&rig, "ls"), "catalog.json\ninstruction.txt\n");
    assert_eq!(read(&rig, "instruction.txt"), "Write paragraphs.");
    writer.write(
        &Prompt {
            instruction: "Write more.".to_owned(),
            material: "#r2 hi".to_owned(),
        },
        &limits(),
    );
    assert_eq!(read(&rig, "instruction.txt"), "Write more.");
    assert_eq!(read(&rig, "stdin"), "#r2 hi");
}

fn run_with(reply: &str, exit: i32) -> (Outcome, Option<String>, Rig) {
    let rig = rig("openai-reply");
    fs::write(rig.scratch.path().join("reply.txt"), reply).expect("reply");
    let script = script_of(
        &rig,
        "tool.sh",
        VERSION,
        &models(&rig),
        &standard(&rig, exit),
    );
    let mut writer = start(&config(&script, ""), &rig);
    let outcome = writer.write(&prompt(), &limits());
    let why = writer.why().map(str::to_owned);
    (outcome, why, rig)
}

#[test]
fn a_cut_a_refusal_and_a_spent_plan_come_back_as_the_engines() {
    let (outcome, why, _rig) = run_with(FAILED_CUT, 1);
    assert_eq!(outcome, Outcome::Cut(Usage::default()));
    assert_eq!(why, None);
    let (outcome, why, _rig) = run_with(FAILED_FILTER, 1);
    assert_eq!(
        outcome,
        Outcome::Refused {
            category: Some("content_filter".to_owned()),
            usage: Usage::default()
        }
    );
    assert_eq!(why, None);
    let (outcome, why, _rig) = run_with(FAILED_LIMIT, 1);
    assert_eq!(
        outcome,
        Outcome::Stopped(cause(What::Billing, Some(1), None))
    );
    assert_eq!(why, None);
}

#[test]
fn a_lost_login_stops_the_run_and_says_how_to_log_in() {
    let (outcome, why, rig) = run_with(FAILED_401, 1);
    assert_eq!(
        outcome,
        Outcome::Stopped(cause(What::Unauthorized, Some(1), Some("401")))
    );
    assert_eq!(why, Some(t12(&rig.home)));
}

#[test]
fn a_tool_that_does_not_end_in_time_is_busy_and_is_stopped() {
    let rig = rig("openai-timeout");
    let script = script_of(
        &rig,
        "tool.sh",
        VERSION,
        &models(&rig),
        &format!("echo $$ > {}/pid\nsleep 30", rig.out.display()),
    );
    let mut writer = start(&config(&script, ""), &rig);
    let outcome = writer.write(
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
    let pid = read(&rig, "pid");
    assert!(
        crate::gone(pid.trim().parse().expect("a pid")),
        "the tool still lives"
    );
}

#[test]
fn output_over_the_limit_is_a_failed_call() {
    let rig = rig("openai-limit");
    let script = script_of(
        &rig,
        "tool.sh",
        VERSION,
        &models(&rig),
        "printf '%02000d' 0",
    );
    let mut writer = start(&config(&script, ""), &rig);
    assert_eq!(
        writer.write(
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
fn why_a_call_was_not_made_is_kept_for_that_call_alone() {
    let rig = rig("openai-why");
    let script = script(&rig);
    let mut writer = start(&config(&script, ""), &rig);
    fs::create_dir(rig.home.join("rules")).expect("rules");
    assert_eq!(
        writer.write(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::NotStarted))
    );
    assert_eq!(writer.why(), Some(t5e(&rig.home, "rules").as_str()));
    fs::remove_dir(rig.home.join("rules")).expect("remove");
    assert_eq!(
        writer.write(
            &prompt(),
            &Limits {
                response: 10,
                ..limits()
            }
        ),
        Outcome::Failed {
            cause: Cause::new(What::TooLarge),
            usage: Usage::default()
        }
    );
    assert_eq!(writer.why(), None);
    let outcome = writer.write(&prompt(), &limits());
    assert_eq!(answered(&outcome), ANSWER_TEXT);
    assert_eq!(writer.why(), None);
}

#[test]
fn a_path_that_cannot_stand_in_an_argument_starts_no_child() {
    let rig = rig("openai-path");
    let script = script(&rig);
    let state = rig.scratch.path().join("a\nb").join("state");
    let work = state.join("derive").join("work");
    let home = state.join("derive").join("codex");
    let mut writer = adapter_at(&config(&script, ""), &work, variables(&rig));
    assert_eq!(
        writer.prepare().unwrap_err(),
        format!(
            "derive.openai.tool: {} cannot stand in an argument: it is not UTF-8 or holds a control character",
            home.display()
        )
    );
    no_call(&rig);
}

#[test]
fn the_program_is_resolved_once_and_that_file_is_started() {
    let rig = rig("openai-resolve");
    let script = script(&rig);
    let path = format!("{}:{}", rig.scratch.path().display(), test_path());
    let user = rig.scratch.path().join("user").display().to_string();
    let text = config_in(Some("model-a"), "tool.sh", "[]", "");
    let mut writer = adapter_with(&text, &rig, vars(Some(&path), Some(&user), None));
    writer.prepare().expect("prepare");
    writer.write(&prompt(), &limits());
    let selves: Vec<String> = read(&rig, "self").lines().map(str::to_owned).collect();
    assert_eq!(selves.len(), 4);
    for line in &selves {
        assert_eq!(line, &script.display().to_string());
    }

    let text = config_in(Some("model-a"), "no-such-tool", "[]", "");
    assert_eq!(
        adapter_with(&text, &rig, vars(Some(&path), Some(&user), None))
            .prepare()
            .unwrap_err(),
        t8c("no-such-tool")
    );
    let text = config_in(Some("model-a"), "tool.sh", "[]", "");
    assert_eq!(
        adapter_with(&text, &rig, vars(None, Some(&user), None))
            .prepare()
            .unwrap_err(),
        t8c("tool.sh")
    );

    let rig = self::rig("openai-resolve-link");
    fs::create_dir(rig.scratch.path().join("bin")).expect("bin");
    script_of(
        &rig,
        "bin/tool.sh",
        VERSION,
        &models(&rig),
        &standard(&rig, 0),
    );
    let link = rig.scratch.path().join("lnk");
    symlink(rig.scratch.path().join("bin"), &link).expect("link");
    let through = link.join("tool.sh");
    let mut writer = start(&config(&through, ""), &rig);
    let outcome = writer.write(&prompt(), &limits());
    assert_eq!(answered(&outcome), ANSWER_TEXT);
    assert_eq!(calls(&rig), ["--version", "debug", "--version", "exec"]);
    for line in read(&rig, "self").lines() {
        assert_eq!(line, through.display().to_string());
    }
}

#[test]
fn a_release_that_changed_behind_the_program_starts_no_call() {
    let rig = rig("openai-release-switched");
    let switched = rig.out.join("switched");
    let version = format!(
        "if [ -e {} ]; then echo \"codex-cli 0.170.0\"; else echo \"codex-cli 0.169.0\"; fi",
        switched.display()
    );
    let script = script_of(&rig, "tool.sh", &version, &models(&rig), &standard(&rig, 0));
    let mut writer = start(&config(&script, ""), &rig);
    fs::write(&switched, "").expect("a file");
    assert_eq!(
        writer.write(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::NotStarted))
    );
    assert_eq!(writer.why(), Some(t8b(&script).as_str()));
    assert_eq!(calls(&rig), ["--version", "debug", "--version"]);
    fs::remove_file(&switched).expect("remove");
    let outcome = writer.write(&prompt(), &limits());
    assert_eq!(answered(&outcome), ANSWER_TEXT);
    assert_eq!(writer.why(), None);
    assert_eq!(
        calls(&rig),
        ["--version", "debug", "--version", "--version", "exec"]
    );
}

#[test]
fn a_program_that_changed_after_prepare_starts_no_call() {
    let rig = rig("openai-changed");
    let script = script(&rig);
    let mut writer = start(&config(&script, ""), &rig);
    let again = script_of(
        &rig,
        "tool.sh",
        VERSION,
        &models(&rig),
        &format!("{}\necho more", standard(&rig, 0)),
    );
    assert_eq!(again, script);
    assert_eq!(
        writer.write(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::NotStarted))
    );
    assert_eq!(writer.why(), Some(t8d(&script).as_str()));
    assert_eq!(calls(&rig), ["--version", "debug"]);
    fs::remove_file(&script).expect("remove");
    assert_eq!(
        writer.write(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::NotStarted))
    );
    assert_eq!(writer.why(), Some(t8d(&script).as_str()));
    assert_eq!(calls(&rig), ["--version", "debug"]);

    let rig = self::rig("openai-replaced");
    let script = self::script(&rig);
    let mut writer = start(&config(&script, ""), &rig);
    let body = fs::read_to_string(&script).expect("script");
    let other = rig.scratch.path().join("tool.sh.new");
    fs::write(&other, body.replacen("esac", "esaX", 1)).expect("a file");
    fs::set_permissions(&other, fs::Permissions::from_mode(0o755)).expect("mode");
    fs::rename(&other, &script).expect("rename");
    assert_eq!(
        fs::metadata(&script).expect("script").len(),
        u64::try_from(body.len()).expect("size")
    );
    assert_eq!(
        writer.write(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::NotStarted))
    );
    assert_eq!(writer.why(), Some(t8d(&script).as_str()));
    assert_eq!(calls(&rig), ["--version", "debug"]);

    let rig = self::rig("openai-repointed");
    fs::create_dir(rig.scratch.path().join("bin")).expect("bin");
    let first = script_of(
        &rig,
        "bin/one.sh",
        VERSION,
        &models(&rig),
        &standard(&rig, 0),
    );
    let second = script_of(
        &rig,
        "bin/two.sh",
        VERSION,
        &models(&rig),
        &standard(&rig, 0),
    );
    let link = rig.scratch.path().join("prog");
    symlink(&first, &link).expect("link");
    let mut writer = start(&config(&link, ""), &rig);
    fs::remove_file(&link).expect("remove");
    symlink(&second, &link).expect("link");
    assert_eq!(
        writer.write(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::NotStarted))
    );
    assert_eq!(writer.why(), Some(t8d(&link).as_str()));
    assert_eq!(calls(&rig), ["--version", "debug"]);
}

#[test]
fn a_home_that_clashes_with_the_users_own_or_with_the_work_folder_starts_no_child() {
    let root = |rig: &Rig, name: &str| rig.scratch.path().join(name);
    let attempt = |rig: &Rig, script: &Path, home: &Path, variables: Variables| {
        let tail = format!("home = {:?}\n", home.display().to_string());
        let mut writer = adapter_with(&config(script, &tail), rig, variables);
        writer.prepare().unwrap_err()
    };
    let with_own = |rig: &Rig, own: &Path| {
        let user = rig.scratch.path().join("user").display().to_string();
        vars(
            Some(&test_path()),
            Some(&user),
            Some(&own.display().to_string()),
        )
    };

    let rig = rig("openai-clash");
    let script = script(&rig);
    let own = root(&rig, "own");
    login(&own);
    assert_eq!(
        attempt(&rig, &script, &own, with_own(&rig, &own)),
        t5h(&own, &own)
    );
    no_call(&rig);
    let inside = own.join("sub");
    login(&inside);
    assert_eq!(
        attempt(&rig, &script, &inside, with_own(&rig, &own)),
        t5h(&inside, &own)
    );
    let outer = root(&rig, "outer");
    login(&outer);
    let below = outer.join("sub");
    fs::create_dir_all(&below).expect("below");
    assert_eq!(
        attempt(&rig, &script, &outer, with_own(&rig, &below)),
        t5h(&outer, &below)
    );
    let lnk = root(&rig, "lnk");
    symlink(&own, &lnk).expect("link");
    let through = lnk.join("sub");
    assert_eq!(
        attempt(&rig, &script, &through, with_own(&rig, &own)),
        t5h(&through, &own)
    );
    no_call(&rig);

    let h = root(&rig, "h");
    let under = h.join(".codex");
    login(&under);
    let user = h.display().to_string();
    assert_eq!(
        attempt(
            &rig,
            &script,
            &under,
            vars(Some(&test_path()), Some(&user), None)
        ),
        t5h(&under, &under)
    );
    no_call(&rig);

    let outside = rig.work.clone();
    login(&outside);
    assert_eq!(
        attempt(&rig, &script, &outside, variables(&rig)),
        t5i(&outside, &rig.work)
    );
    assert!(outside.join("auth.json").exists());
    let derive = rig.state.join("derive");
    fs::set_permissions(&derive, fs::Permissions::from_mode(0o700)).expect("a mode");
    assert_eq!(
        attempt(&rig, &script, &derive, variables(&rig)),
        t5i(&derive, &rig.work)
    );
    assert!(outside.join("auth.json").exists());
    no_call(&rig);

    let own = root(&rig, "own-through");
    let held = own.join("codex");
    login(&held);
    let linked = root(&rig, "own-link");
    symlink(&own, &linked).expect("link");
    assert_eq!(
        attempt(&rig, &script, &held, with_own(&rig, &linked)),
        t5h(&held, &linked)
    );
    no_call(&rig);

    let holder = root(&rig, "holder");
    let inner = holder.join("codex");
    login(&inner);
    let door = root(&rig, "door");
    symlink(&holder, &door).expect("link");
    let tail = format!("home = {:?}\n", inner.display().to_string());
    let mut writer = adapter_at(&config(&script, &tail), &door, variables(&rig));
    assert_eq!(writer.prepare().unwrap_err(), t5i(&inner, &door));
    no_call(&rig);

    let locked = root(&rig, "locked");
    fs::create_dir_all(locked.join("sub")).expect("locked");
    let unseen = locked.join("sub").join("home");
    let modes = Modes(vec![(locked.clone(), 0o755)]);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("mode");
    let error = attempt(&rig, &script, &held, with_own(&rig, &unseen));
    drop(modes);
    assert!(
        error.starts_with(&format!("derive.openai.tool.home: {}: ", unseen.display())),
        "{error}"
    );
    no_call(&rig);

    let neither = "derive.openai.tool.home: neither CODEX_HOME nor HOME is set, so bilbo cannot tell your own codex home from this one: set HOME";
    for (home, codex_home) in [(None, None), (Some(""), Some(""))] {
        let mut writer = adapter_with(
            &config(&script, ""),
            &rig,
            vars(Some(&test_path()), home, codex_home),
        );
        assert_eq!(writer.prepare().unwrap_err(), neither);
        no_call(&rig);
    }

    let a_file = root(&rig, "a-file");
    fs::write(&a_file, "x").expect("a file");
    let mut writer = adapter_with(
        &config(&script, ""),
        &rig,
        with_own(&rig, &a_file.join("x")),
    );
    assert_eq!(writer.prepare(), Ok(()));
}

#[test]
fn a_login_that_is_a_link_or_has_a_second_name_starts_no_child() {
    for kind in ["link", "hard", "folder"] {
        let rig = rig("openai-login");
        let script = script(&rig);
        let auth = rig.home.join("auth.json");
        fs::remove_file(&auth).expect("remove");
        let other = rig.scratch.path().join("other-auth");
        fs::write(&other, "").expect("other");
        let expected = match kind {
            "link" => {
                symlink(&other, &auth).expect("link");
                format!(
                    "derive.openai.tool.home: {}/auth.json is a link: the login is a file of this folder alone",
                    rig.home.display()
                )
            }
            "hard" => {
                fs::hard_link(&other, &auth).expect("hard link");
                format!(
                    "derive.openai.tool.home: {}/auth.json has another name too: the login is a file of this folder alone",
                    rig.home.display()
                )
            }
            _ => {
                fs::create_dir(&auth).expect("folder");
                format!(
                    "derive.openai.tool.home: {}/auth.json is not a file",
                    rig.home.display()
                )
            }
        };
        assert_eq!(refused(&config(&script, ""), &rig), expected, "{kind}");
        no_call(&rig);
    }
}

#[test]
fn a_reached_tool_stops_the_run_and_says_so() {
    let rig = rig("openai-reached");
    fs::write(rig.scratch.path().join("reply.txt"), TOOL_COMMAND).expect("reply");
    let script = script(&rig);
    let mut writer = start(&config(&script, ""), &rig);
    assert_eq!(
        writer.write(&prompt(), &limits()),
        Outcome::Stopped(cause(What::ServerError, Some(0), None))
    );
    assert_eq!(writer.why(), Some(T11C));
    fs::write(rig.scratch.path().join("reply.txt"), ANSWER).expect("reply");
    let outcome = writer.write(&prompt(), &limits());
    assert_eq!(answered(&outcome), ANSWER_TEXT);
    assert_eq!(writer.why(), None);
}

#[test]
fn a_tool_reached_before_a_timeout_or_an_overflow_stops_the_run() {
    let rig = rig("openai-reached-late");
    let line = |kind: &str| TOOL_LINE.replace("KIND", kind);
    let script = script_of(
        &rig,
        "tool.sh",
        VERSION,
        &models(&rig),
        &format!(
            "echo $$ > {}/pid\nprintf '%s\\n' '{}'\nsleep 30",
            rig.out.display(),
            line("command_execution")
        ),
    );
    let mut writer = start(&config(&script, ""), &rig);
    let outcome = writer.write(
        &prompt(),
        &Limits {
            timeout: Duration::from_secs(1),
            ..limits()
        },
    );
    assert_eq!(
        outcome,
        Outcome::Stopped(cause(What::ServerError, None, None))
    );
    assert_eq!(writer.why(), Some(T11C));
    let pid = read(&rig, "pid");
    assert!(
        crate::gone(pid.trim().parse().expect("a pid")),
        "the tool still lives"
    );

    let rig = self::rig("openai-reached-over");
    let script = script_of(
        &rig,
        "tool.sh",
        VERSION,
        &models(&rig),
        &format!(
            "printf '%s\\n' '{}'\nprintf '%02000d' 0",
            line("mcp_tool_call")
        ),
    );
    let mut writer = start(&config(&script, ""), &rig);
    let outcome = writer.write(
        &prompt(),
        &Limits {
            response: 1000,
            ..limits()
        },
    );
    assert_eq!(
        outcome,
        Outcome::Stopped(cause(What::ServerError, None, None))
    );
    assert_eq!(writer.why(), Some(T11C));
}

#[test]
fn a_home_others_can_write_starts_no_child() {
    let rig = rig("openai-home-mode");
    let script = script(&rig);
    for mode in [0o770, 0o707] {
        fs::set_permissions(&rig.home, fs::Permissions::from_mode(mode)).expect("mode");
        assert_eq!(
            refused(&config(&script, ""), &rig),
            t5o(&rig.home),
            "{mode:o}"
        );
        no_call(&rig);
    }
    for mode in [0o755, 0o700] {
        fs::set_permissions(&rig.home, fs::Permissions::from_mode(mode)).expect("mode");
        assert_eq!(
            adapter(&config(&script, ""), &rig).prepare(),
            Ok(()),
            "{mode:o}"
        );
    }
}

#[test]
fn a_home_of_another_user_starts_no_child() {
    let rig = rig("openai-home-owner");
    let script = script(&rig);
    let usr = fs::metadata("/usr").expect("usr").uid();
    if usr == fs::metadata(rig.scratch.path()).expect("scratch").uid() {
        return;
    }
    let tail = "home = \"/usr\"\n";
    assert_eq!(
        refused(&config(&script, tail), &rig),
        "derive.openai.tool.home: /usr belongs to another user"
    );
    no_call(&rig);
}

#[test]
fn a_program_that_is_slow_or_prints_too_much_when_asked_says_which_key() {
    let slow = "sleep 30";
    let big = "printf '%02000d' 0";
    let short = Duration::from_millis(1000);
    for (version, models_text, expected) in [
        (slow, None, 0),
        (VERSION, Some(slow), 1),
        (VERSION, Some(big), 2),
        (big, None, 3),
    ] {
        let rig = rig("openai-asked");
        let models_text = models_text.map_or_else(|| models(&rig), str::to_owned);
        let script = script_of(&rig, "tool.sh", version, &models_text, &standard(&rig, 0));
        let error = constructed(&config(&script, ""), &rig.work, variables(&rig), short, 100)
            .prepare()
            .unwrap_err();
        let want = [
            t8e(&script, "its release"),
            t8e(&script, "its models"),
            t8f(&script, "its models"),
            t8f(&script, "its release"),
        ];
        assert_eq!(error, want[expected], "{expected}");
        assert!(
            !calls(&rig).iter().any(|call| call == "exec"),
            "{:?}",
            calls(&rig)
        );
    }
}
