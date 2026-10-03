//! The claude and codex plugin commands.

use crate::command::{Output, Runner, first_line};
use serde_json::Value;
use std::path::{Path, PathBuf};

const PLUGIN: &str = "bilbo@bilbo";
const MARKETPLACE_FILE: &str = ".claude-plugin/marketplace.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Claude,
    Codex,
}

impl Tool {
    /// The program name and the report step.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Claude => "claude",
            Tool::Codex => "codex",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Claude => "Claude Code",
            Tool::Codex => "Codex",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Folder(PathBuf),
    GitHub { repo: String, git_ref: String },
}

impl Source {
    /// The folder path, or `owner/repo#ref`.
    pub fn display(&self) -> String {
        match self {
            Source::Folder(path) => path.display().to_string(),
            Source::GitHub { repo, git_ref } => format!("{repo}#{git_ref}"),
        }
    }
}

/// The package's `share/bilbo` when it holds the marketplace, else GitHub `delucca/bilbo` at `v<version>`. `exe` is already canonical.
pub fn default_source(exe: &Path, version: &str) -> Source {
    let share = exe
        .parent()
        .and_then(Path::parent)
        .map(|prefix| prefix.join("share/bilbo"));
    match share {
        Some(folder) if folder.join(MARKETPLACE_FILE).is_file() => Source::Folder(folder),
        _ => Source::GitHub {
            repo: "delucca/bilbo".to_string(),
            git_ref: format!("v{version}"),
        },
    }
}

/// `--plugin-source`: `<owner>/<repo>#<ref>`, or a folder (relative to `cwd`, `~/` expanded with `home`) holding the marketplace.
pub fn parse_source(text: &str, cwd: &Path, home: Option<&Path>) -> Result<Source, String> {
    if let Some((repo, git_ref)) = text.split_once('#')
        && is_repo(repo)
        && !git_ref.is_empty()
        && !git_ref.starts_with('-')
        && !git_ref.chars().any(char::is_whitespace)
    {
        return Ok(Source::GitHub {
            repo: repo.to_string(),
            git_ref: git_ref.to_string(),
        });
    }
    let path = match (text.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => cwd.join(text),
    };
    match std::fs::canonicalize(&path) {
        Ok(folder) if folder.join(MARKETPLACE_FILE).is_file() => Ok(Source::Folder(folder)),
        _ => Err(format!(
            "{text} is not a folder with .claude-plugin/marketplace.json, nor <owner>/<repo>#<ref>"
        )),
    }
}

fn is_repo(text: &str) -> bool {
    let word = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    text.split_once('/')
        .is_some_and(|(owner, repo)| word(owner) && word(repo))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Listed {
    Folder(PathBuf),
    GitHub {
        repo: String,
        git_ref: Option<String>,
    },
    Other(String),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Installed {
    pub enabled: bool,
    pub version: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct State {
    pub marketplace: Option<Listed>,
    pub plugin: Option<Installed>,
}

fn list_args(what: &str) -> Vec<String> {
    let mut args = vec!["plugin".to_string()];
    if what == "marketplace" {
        args.push("marketplace".to_string());
    }
    args.extend(["list".to_string(), "--json".to_string()]);
    args
}

fn list(tool: Tool, runner: &dyn Runner, program: &Path, what: &str) -> Result<String, String> {
    let args = list_args(what);
    let shown = format!("{} {}", tool.name(), args.join(" "));
    let fail = |reason: String| format!("cannot read `{shown}`: {reason}");
    let output = runner.run(program, &args).map_err(&fail)?;
    if !judge(tool, &output, &args) {
        return Err(fail(message(tool, &output)));
    }
    Ok(output.stdout)
}

/// Runs the two list commands.
pub fn read(tool: Tool, runner: &dyn Runner, program: &Path) -> Result<State, String> {
    let marketplaces = list(tool, runner, program, "marketplace")?;
    let marketplace = parse_marketplaces(tool, &marketplaces).map_err(|e| {
        format!(
            "cannot read `{} plugin marketplace list --json`: {e}",
            tool.name()
        )
    })?;
    let plugins = list(tool, runner, program, "plugin")?;
    let plugin = parse_plugins(tool, &plugins)
        .map_err(|e| format!("cannot read `{} plugin list --json`: {e}", tool.name()))?;
    Ok(State {
        marketplace,
        plugin,
    })
}

fn parse(json: &str) -> Result<Value, String> {
    serde_json::from_str(json).map_err(|_| "unexpected JSON".to_string())
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

pub fn parse_marketplaces(tool: Tool, json: &str) -> Result<Option<Listed>, String> {
    let value = parse(json)?;
    match tool {
        Tool::Claude => {
            let entries = value.as_array().ok_or("unexpected JSON")?;
            let Some(entry) = entries.iter().find(|e| e["name"] == "bilbo") else {
                return Ok(None);
            };
            let source = text(entry, "source").ok_or("unexpected JSON")?;
            Ok(Some(match source.as_str() {
                "directory" => {
                    Listed::Folder(PathBuf::from(text(entry, "path").ok_or("unexpected JSON")?))
                }
                "github" => Listed::GitHub {
                    repo: text(entry, "repo").ok_or("unexpected JSON")?,
                    git_ref: text(entry, "ref"),
                },
                _ => Listed::Other(source),
            }))
        }
        Tool::Codex => {
            let entries = value
                .get("marketplaces")
                .and_then(Value::as_array)
                .ok_or("unexpected JSON")?;
            let Some(entry) = entries.iter().find(|e| e["name"] == "bilbo") else {
                return Ok(None);
            };
            let origin = entry.get("marketplaceSource").ok_or("unexpected JSON")?;
            let kind = text(origin, "sourceType").ok_or("unexpected JSON")?;
            let source = text(origin, "source").ok_or("unexpected JSON")?;
            Ok(Some(match kind.as_str() {
                "local" => Listed::Folder(PathBuf::from(source)),
                "git" => match github_repo(&source) {
                    Some(repo) => Listed::GitHub {
                        repo,
                        git_ref: None,
                    },
                    None => Listed::Other(source),
                },
                _ => Listed::Other(source),
            }))
        }
    }
}

/// `owner/repo` of `https://github.com/<owner>/<repo>.git`.
fn github_repo(url: &str) -> Option<String> {
    let repo = url
        .strip_prefix("https://github.com/")?
        .strip_suffix(".git")?;
    is_repo(repo).then(|| repo.to_string())
}

pub fn parse_plugins(tool: Tool, json: &str) -> Result<Option<Installed>, String> {
    let value = parse(json)?;
    let (entries, key) = match tool {
        Tool::Claude => (value.as_array(), "id"),
        Tool::Codex => (value.get("installed").and_then(Value::as_array), "pluginId"),
    };
    let entries = entries.ok_or("unexpected JSON")?;
    let mut found = entries.iter().filter(|e| e[key] == PLUGIN);
    let first = found.next();
    let entry = match tool {
        Tool::Claude => first
            .into_iter()
            .chain(found)
            .find(|e| e["scope"] == "user")
            .or(first),
        Tool::Codex => first,
    };
    Ok(entry.map(|e| Installed {
        enabled: e["enabled"].as_bool().unwrap_or(false),
        version: text(e, "version").unwrap_or_default(),
    }))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Keep,
    Install(Vec<Vec<String>>),
    Update(Vec<Vec<String>>),
}

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

fn add(tool: Tool, source: &Source) -> Vec<String> {
    let mut command = args(&["plugin", "marketplace", "add"]);
    match (tool, source) {
        (Tool::Codex, Source::GitHub { repo, git_ref }) => {
            command.extend([repo.clone(), "--ref".to_string(), git_ref.clone()]);
        }
        _ => command.push(source.display()),
    }
    command.push("--json".to_string());
    command
}

fn install(tool: Tool) -> Vec<String> {
    match tool {
        Tool::Claude => args(&["plugin", "install", PLUGIN, "--json"]),
        Tool::Codex => args(&["plugin", "add", PLUGIN, "--json"]),
    }
}

fn drop_marketplace() -> Vec<String> {
    args(&["plugin", "marketplace", "remove", "bilbo", "--json"])
}

fn uninstall(tool: Tool) -> Vec<String> {
    match tool {
        Tool::Claude => args(&["plugin", "uninstall", PLUGIN, "--json"]),
        Tool::Codex => args(&["plugin", "remove", PLUGIN, "--json"]),
    }
}

fn same_folder(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn same_source(tool: Tool, listed: &Listed, source: &Source) -> bool {
    match (listed, source) {
        (Listed::Folder(a), Source::Folder(b)) => same_folder(a, b),
        (
            Listed::GitHub { repo, git_ref },
            Source::GitHub {
                repo: planned,
                git_ref: planned_ref,
            },
        ) => {
            repo.eq_ignore_ascii_case(planned)
                && (tool == Tool::Codex || git_ref.as_deref() == Some(planned_ref))
        }
        _ => false,
    }
}

pub fn plan(tool: Tool, state: &State, source: &Source, version: &str) -> Change {
    let fresh = || vec![add(tool, source), install(tool)];
    let Some(listed) = &state.marketplace else {
        return Change::Install(fresh());
    };
    let replace = || Change::Update(vec![drop_marketplace(), add(tool, source), install(tool)]);
    if !same_source(tool, listed, source) {
        return replace();
    }
    let reinstall = || Change::Update(vec![install(tool)]);
    let Some(plugin) = &state.plugin else {
        return if tool == Tool::Codex && matches!(listed, Listed::GitHub { .. }) {
            replace()
        } else {
            reinstall()
        };
    };
    let behind = tool == Tool::Codex && plugin.version != version;
    if behind && matches!(listed, Listed::GitHub { .. }) {
        replace()
    } else if !plugin.enabled || behind {
        reinstall()
    } else {
        Change::Keep
    }
}

/// The commands that remove bilbo from the tool; empty when nothing is listed.
pub fn removal(tool: Tool, state: &State) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    if state.plugin.is_some() {
        commands.push(uninstall(tool));
    }
    if state.marketplace.is_some() {
        commands.push(drop_marketplace());
    }
    commands
}

pub struct Failed {
    pub args: Vec<String>,
    pub message: String,
}

/// Runs `commands` in order and stops at the first failure.
pub fn run_all(
    tool: Tool,
    runner: &dyn Runner,
    program: &Path,
    commands: &[Vec<String>],
) -> Result<(), Failed> {
    for command in commands {
        let failed = |message: String| Failed {
            args: command.clone(),
            message,
        };
        let output = runner.run(program, command).map_err(&failed)?;
        if !judge(tool, &output, command) {
            return Err(failed(message(tool, &output)));
        }
    }
    Ok(())
}

/// Claude by the JSON `outcome` when stdout has one, else by the exit code; Codex by the exit code.
fn judge(tool: Tool, output: &Output, command: &[String]) -> bool {
    if tool == Tool::Codex {
        return output.success();
    }
    let json = serde_json::from_str::<Value>(&output.stdout).ok();
    let outcome = json.as_ref().and_then(|v| v["outcome"].as_str());
    let removing = command.iter().any(|a| a == "uninstall" || a == "remove");
    let gone = json
        .as_ref()
        .and_then(|v| v["failureCode"].as_str())
        .is_some_and(|code| matches!(code, "not_installed" | "not_configured"));
    match outcome {
        Some(outcome) => outcome == "ok" || (removing && gone),
        None => output.success(),
    }
}

/// Claude's JSON `message`, else stderr's first line without `✘ ` or `Error: `, else the exit code.
pub fn message(tool: Tool, output: &Output) -> String {
    if tool == Tool::Claude
        && let Ok(json) = serde_json::from_str::<Value>(&output.stdout)
        && let Some(message) = json["message"].as_str()
    {
        let line = first_line(message);
        if !line.is_empty() {
            return line.to_string();
        }
    }
    let line = first_line(&output.stderr);
    let line = line
        .strip_prefix("✘ ")
        .or_else(|| line.strip_prefix("Error: "))
        .unwrap_or(line);
    if !line.is_empty() {
        return line.to_string();
    }
    match output.code {
        Some(code) => format!("exit {code}"),
        None => "ended by a signal".to_string(),
    }
}

/// What `trust_hooks` did in Codex.
#[derive(Debug, PartialEq, Eq)]
pub enum Trust {
    /// Every bilbo hook was already trusted.
    Kept,
    /// It wrote the trust; `changed` when a hook had been trusted before and changed since.
    Wrote { changed: bool },
    /// Codex lists no bilbo hook.
    NoHook,
}

/// Opens Codex's app-server and says hello.
pub fn app_server(program: &Path) -> Result<crate::command::Rpc, String> {
    use crate::command::Calls;
    let mut rpc =
        crate::command::Rpc::start(program, &["app-server"], std::time::Duration::from_secs(30))?;
    rpc.call(
        "initialize",
        serde_json::json!({"clientInfo": {"name": "bilbo", "version": env!("CARGO_PKG_VERSION")}}),
    )?;
    rpc.notify("initialized")?;
    Ok(rpc)
}

/// Lists Codex's hooks for `cwd` and trusts every bilbo one that is untrusted or changed.
pub fn trust_hooks(rpc: &mut dyn crate::command::Calls, cwd: &Path) -> Result<Trust, String> {
    let listed = rpc.call("hooks/list", serde_json::json!({ "cwds": [cwd] }))?;
    let hooks: Vec<&Value> = listed["data"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|entry| entry["hooks"].as_array().into_iter().flatten())
        .filter(|hook| hook["pluginId"] == PLUGIN)
        .collect();
    if hooks.is_empty() {
        return Ok(Trust::NoHook);
    }
    let mut value = serde_json::Map::new();
    let mut changed = false;
    for hook in hooks {
        let (Some(key), Some(hash), Some(status)) = (
            hook["key"].as_str(),
            hook["currentHash"].as_str(),
            hook["trustStatus"].as_str(),
        ) else {
            return Err("codex listed a bilbo hook without key, currentHash or trustStatus".into());
        };
        match status {
            "trusted" | "managed" => {}
            "untrusted" | "modified" => {
                changed |= status == "modified";
                value.insert(key.to_string(), serde_json::json!({ "trusted_hash": hash }));
            }
            _ => {
                return Err(format!(
                    "codex reports trust status '{status}' for a bilbo hook"
                ));
            }
        }
    }
    if value.is_empty() {
        return Ok(Trust::Kept);
    }
    rpc.call(
        "config/batchWrite",
        serde_json::json!({"edits": [{"keyPath": "hooks.state", "value": value, "mergeStrategy": "upsert"}], "reloadUserConfig": false}),
    )?;
    Ok(Trust::Wrote { changed })
}

/// Deletes every trust entry of a bilbo hook from Codex's config; how many there were.
pub fn forget_hooks(rpc: &mut dyn crate::command::Calls) -> Result<usize, String> {
    let read = rpc.call("config/read", serde_json::json!({ "includeLayers": false }))?;
    let keys: Vec<String> = read["config"]["hooks"]["state"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, _)| key.clone())
        .filter(|key| key.starts_with(&format!("{PLUGIN}:")))
        .collect();
    if keys.is_empty() {
        return Ok(0);
    }
    let edits: Vec<Value> = keys
        .iter()
        .map(|key| serde_json::json!({"keyPath": format!("hooks.state.\"{key}\""), "value": null, "mergeStrategy": "replace"}))
        .collect();
    rpc.call(
        "config/batchWrite",
        serde_json::json!({ "edits": edits, "reloadUserConfig": false }),
    )?;
    Ok(keys.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-agents-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn marketplace_in(folder: &Path) {
        std::fs::create_dir_all(folder.join(".claude-plugin")).unwrap();
        std::fs::write(folder.join(MARKETPLACE_FILE), "{}").unwrap();
    }

    struct Script {
        outputs: RefCell<VecDeque<Output>>,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl Script {
        fn new(outputs: Vec<Output>) -> Script {
            Script {
                outputs: RefCell::new(outputs.into()),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl Runner for Script {
        fn run(&self, _program: &Path, args: &[String]) -> Result<Output, String> {
            self.calls.borrow_mut().push(args.to_vec());
            self.outputs
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| "script ran out".to_string())
        }
    }

    fn done(code: i32, stdout: &str, stderr: &str) -> Output {
        Output {
            code: Some(code),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    /// Splits a recorded fixture: `$ command`, `exit <n>`, `--- stdout`, `--- stderr`.
    fn recorded(text: &str) -> Output {
        let code = text
            .lines()
            .nth(1)
            .and_then(|l| l.strip_prefix("exit "))
            .and_then(|n| n.parse().ok())
            .unwrap();
        let (_, rest) = text.split_once("--- stdout\n").unwrap();
        let (stdout, stderr) = rest.split_once("--- stderr\n").unwrap();
        done(code, stdout, stderr)
    }

    macro_rules! fixture {
        ($name:literal) => {
            recorded(include_str!(concat!(
                "../tests/fixtures/agents/",
                $name,
                ".txt"
            )))
        };
    }

    fn github(repo: &str, git_ref: &str) -> Source {
        Source::GitHub {
            repo: repo.into(),
            git_ref: git_ref.into(),
        }
    }

    fn listed_github(repo: &str, git_ref: Option<&str>) -> Listed {
        Listed::GitHub {
            repo: repo.into(),
            git_ref: git_ref.map(str::to_string),
        }
    }

    fn installed(enabled: bool, version: &str) -> Installed {
        Installed {
            enabled,
            version: version.into(),
        }
    }

    fn state(marketplace: Option<Listed>, plugin: Option<Installed>) -> State {
        State {
            marketplace,
            plugin,
        }
    }

    fn cmds(change: Change) -> Vec<String> {
        let (Change::Install(c) | Change::Update(c)) = change else {
            return Vec::new();
        };
        c.iter().map(|a| a.join(" ")).collect()
    }

    #[test]
    fn parses_every_recorded_list() {
        let cm = |o: Output| parse_marketplaces(Tool::Claude, &o.stdout).unwrap();
        let xm = |o: Output| parse_marketplaces(Tool::Codex, &o.stdout).unwrap();
        let cp = |o: Output| parse_plugins(Tool::Claude, &o.stdout).unwrap();
        let xp = |o: Output| parse_plugins(Tool::Codex, &o.stdout).unwrap();
        assert_eq!(
            cm(fixture!("claude-marketplace-list-github")),
            Some(listed_github("delucca/bilbo", Some("v0.1.0")))
        );
        assert_eq!(
            cm(fixture!("claude-marketplace-list-folder")),
            Some(Listed::Folder("/fixture/bilbo".into()))
        );
        assert_eq!(cm(fixture!("claude-marketplace-list-empty")), None);
        assert_eq!(
            cp(fixture!("claude-plugin-list-disabled")),
            Some(installed(false, "92076707d856"))
        );
        assert!(cp(fixture!("claude-plugin-list-enabled")).unwrap().enabled);
        assert!(cp(fixture!("claude-plugin-list-folder")).unwrap().enabled);
        assert_eq!(cp(fixture!("claude-plugin-list-empty")), None);
        assert_eq!(
            xm(fixture!("codex-marketplace-list-github")),
            Some(listed_github("delucca/bilbo", None))
        );
        assert_eq!(
            xm(fixture!("codex-marketplace-list-folder")),
            Some(Listed::Folder("/fixture/bilbo".into()))
        );
        assert_eq!(xm(fixture!("codex-marketplace-list-empty")), None);
        assert_eq!(
            xp(fixture!("codex-plugin-list-enabled")),
            Some(installed(true, "0.1.0"))
        );
        assert_eq!(
            xp(fixture!("codex-plugin-list-folder")),
            Some(installed(true, "0.1.0"))
        );
        assert_eq!(xp(fixture!("codex-plugin-list-empty")), None);
    }

    #[test]
    fn unexpected_json_is_an_error() {
        assert_eq!(
            parse_marketplaces(Tool::Claude, "{}"),
            Err("unexpected JSON".into())
        );
        assert_eq!(
            parse_plugins(Tool::Codex, "[]"),
            Err("unexpected JSON".into())
        );
        assert_eq!(
            parse_plugins(Tool::Claude, "not json"),
            Err("unexpected JSON".into())
        );
    }

    #[test]
    fn claude_prefers_the_user_scope() {
        let json = r#"[
            {"id":"bilbo@bilbo","version":"a","scope":"project","enabled":false},
            {"id":"bilbo@bilbo","version":"b","scope":"user","enabled":true}
        ]"#;
        assert_eq!(
            parse_plugins(Tool::Claude, json),
            Ok(Some(installed(true, "b")))
        );
    }

    #[test]
    fn read_runs_both_lists() {
        let runner = Script::new(vec![
            fixture!("claude-marketplace-list-github"),
            fixture!("claude-plugin-list-enabled"),
        ]);
        let got = read(Tool::Claude, &runner, Path::new("/x/claude")).unwrap();
        assert_eq!(
            got.marketplace,
            Some(listed_github("delucca/bilbo", Some("v0.1.0")))
        );
        assert!(got.plugin.is_some());
        assert_eq!(
            *runner.calls.borrow(),
            vec![
                args(&["plugin", "marketplace", "list", "--json"]),
                args(&["plugin", "list", "--json"])
            ]
        );
    }

    #[test]
    fn read_names_the_failed_command() {
        let runner = Script::new(vec![done(2, "", "Error: nope\n")]);
        let err = read(Tool::Codex, &runner, Path::new("/x/codex")).unwrap_err();
        assert_eq!(
            err,
            "cannot read `codex plugin marketplace list --json`: nope"
        );
        let runner = Script::new(vec![done(0, "[]", ""), done(0, "{}", "")]);
        let err = read(Tool::Claude, &runner, Path::new("/x/claude")).unwrap_err();
        assert_eq!(
            err,
            "cannot read `claude plugin list --json`: unexpected JSON"
        );
    }

    #[test]
    fn message_takes_the_first_line() {
        assert!(
            message(
                Tool::Claude,
                &fixture!("claude-marketplace-add-missing-tag")
            )
            .starts_with("Failed to clone marketplace repository")
        );
        assert_eq!(
            message(Tool::Codex, &fixture!("codex-marketplace-add-missing-tag")),
            "git checkout v9.9.9 failed with status exit status: 1"
        );
        assert_eq!(
            message(
                Tool::Codex,
                &fixture!("codex-marketplace-add-github-other-ref")
            ),
            "marketplace 'bilbo' is already added from a different source; remove it before adding this source"
        );
        assert_eq!(
            message(Tool::Claude, &done(1, "", "✘ plain failure\n")),
            "plain failure"
        );
        assert_eq!(message(Tool::Codex, &done(7, "", "")), "exit 7");
        let signalled = Output {
            code: None,
            stdout: String::new(),
            stderr: String::new(),
        };
        assert_eq!(message(Tool::Codex, &signalled), "ended by a signal");
    }

    #[test]
    fn plan_claude_without_a_marketplace_installs() {
        let source = github("delucca/bilbo", "v0.1.0");
        let got = plan(Tool::Claude, &state(None, None), &source, "0.1.0");
        assert!(matches!(got, Change::Install(_)));
        assert_eq!(
            cmds(got),
            [
                "plugin marketplace add delucca/bilbo#v0.1.0 --json",
                "plugin install bilbo@bilbo --json"
            ]
        );
    }

    #[test]
    fn plan_claude_from_another_source_replaces() {
        let source = github("delucca/bilbo", "v0.1.0");
        for listed in [
            Listed::Folder("/fixture/bilbo".into()),
            listed_github("delucca/bilbo", Some("v0.0.9")),
        ] {
            let got = plan(
                Tool::Claude,
                &state(Some(listed), Some(installed(true, "abc"))),
                &source,
                "0.1.0",
            );
            assert!(matches!(got, Change::Update(_)));
            assert_eq!(
                cmds(got),
                [
                    "plugin marketplace remove bilbo --json",
                    "plugin marketplace add delucca/bilbo#v0.1.0 --json",
                    "plugin install bilbo@bilbo --json"
                ]
            );
        }
        let folder = Source::Folder("/fixture/bilbo".into());
        let got = plan(
            Tool::Claude,
            &state(Some(listed_github("delucca/bilbo", Some("v0.1.0"))), None),
            &folder,
            "0.1.0",
        );
        assert_eq!(cmds(got)[1], "plugin marketplace add /fixture/bilbo --json");
    }

    #[test]
    fn plan_claude_same_source() {
        let source = github("Delucca/Bilbo", "v0.1.0");
        let market = || Some(listed_github("delucca/bilbo", Some("v0.1.0")));
        let reinstall = ["plugin install bilbo@bilbo --json"];
        let missing = plan(Tool::Claude, &state(market(), None), &source, "0.1.0");
        assert!(matches!(missing, Change::Update(_)));
        assert_eq!(cmds(missing), reinstall);
        let disabled = plan(
            Tool::Claude,
            &state(market(), Some(installed(false, "abc"))),
            &source,
            "0.1.0",
        );
        assert_eq!(cmds(disabled), reinstall);
        let kept = plan(
            Tool::Claude,
            &state(market(), Some(installed(true, "abc"))),
            &source,
            "0.1.0",
        );
        assert_eq!(kept, Change::Keep);
    }

    #[test]
    fn plan_claude_same_folder_keeps() {
        let s = scratch("samefolder");
        let folder = std::fs::canonicalize(&s.0).unwrap();
        let link = s.0.join("link");
        std::os::unix::fs::symlink(&folder, &link).unwrap();
        let got = plan(
            Tool::Claude,
            &state(Some(Listed::Folder(link)), Some(installed(true, "abc"))),
            &Source::Folder(folder),
            "0.1.0",
        );
        assert_eq!(got, Change::Keep);
    }

    #[test]
    fn plan_codex_without_a_marketplace_installs() {
        let got = plan(
            Tool::Codex,
            &state(None, None),
            &github("delucca/bilbo", "v0.1.0"),
            "0.1.0",
        );
        assert_eq!(
            cmds(got),
            [
                "plugin marketplace add delucca/bilbo --ref v0.1.0 --json",
                "plugin add bilbo@bilbo --json"
            ]
        );
        let got = plan(
            Tool::Codex,
            &state(None, None),
            &Source::Folder("/p/share/bilbo".into()),
            "0.1.0",
        );
        assert_eq!(cmds(got)[0], "plugin marketplace add /p/share/bilbo --json");
    }

    #[test]
    fn plan_codex_from_another_source_replaces() {
        let got = plan(
            Tool::Codex,
            &state(
                Some(Listed::Folder("/fixture/bilbo".into())),
                Some(installed(true, "0.1.0")),
            ),
            &github("delucca/bilbo", "v0.1.0"),
            "0.1.0",
        );
        assert!(matches!(got, Change::Update(_)));
        assert_eq!(cmds(got).len(), 3);
    }

    #[test]
    fn plan_codex_same_repo_with_another_version_replaces() {
        let source = github("delucca/bilbo", "v0.2.0");
        let market = || Some(listed_github("delucca/bilbo", None));
        let want = [
            "plugin marketplace remove bilbo --json",
            "plugin marketplace add delucca/bilbo --ref v0.2.0 --json",
            "plugin add bilbo@bilbo --json",
        ];
        let older = plan(
            Tool::Codex,
            &state(market(), Some(installed(true, "0.1.0"))),
            &source,
            "0.2.0",
        );
        assert_eq!(cmds(older), want);
        let older_disabled = plan(
            Tool::Codex,
            &state(market(), Some(installed(false, "0.1.0"))),
            &source,
            "0.2.0",
        );
        assert_eq!(cmds(older_disabled), want);
        let current = plan(
            Tool::Codex,
            &state(market(), Some(installed(true, "0.2.0"))),
            &source,
            "0.2.0",
        );
        assert_eq!(current, Change::Keep);
    }

    #[test]
    fn plan_codex_missing_plugin_replaces_and_disabled_installs_only() {
        let source = github("delucca/bilbo", "v0.1.0");
        let market = || Some(listed_github("delucca/bilbo", None));
        let missing = plan(Tool::Codex, &state(market(), None), &source, "0.1.0");
        assert_eq!(
            cmds(missing),
            [
                "plugin marketplace remove bilbo --json",
                "plugin marketplace add delucca/bilbo --ref v0.1.0 --json",
                "plugin add bilbo@bilbo --json",
            ]
        );
        let disabled = plan(
            Tool::Codex,
            &state(market(), Some(installed(false, "0.1.0"))),
            &source,
            "0.1.0",
        );
        assert_eq!(cmds(disabled), ["plugin add bilbo@bilbo --json"]);
    }

    #[test]
    fn plan_codex_folder_with_another_version_reinstalls() {
        let folder = Source::Folder("/fixture/bilbo".into());
        let market = || Some(Listed::Folder("/fixture/bilbo".into()));
        let older = plan(
            Tool::Codex,
            &state(market(), Some(installed(true, "0.0.9"))),
            &folder,
            "0.1.0",
        );
        assert!(matches!(older, Change::Update(_)));
        assert_eq!(cmds(older), ["plugin add bilbo@bilbo --json"]);
        let current = plan(
            Tool::Codex,
            &state(market(), Some(installed(true, "0.1.0"))),
            &folder,
            "0.1.0",
        );
        assert_eq!(current, Change::Keep);
    }

    #[test]
    fn plan_other_source_is_never_the_same() {
        let got = plan(
            Tool::Claude,
            &state(Some(Listed::Other("git".into())), None),
            &github("delucca/bilbo", "v0.1.0"),
            "0.1.0",
        );
        assert_eq!(cmds(got).len(), 3);
    }

    #[test]
    fn removal_nothing() {
        for tool in [Tool::Claude, Tool::Codex] {
            assert!(removal(tool, &state(None, None)).is_empty());
        }
    }

    #[test]
    fn removal_plugin_only() {
        let s = state(None, Some(installed(true, "0.1.0")));
        assert_eq!(
            removal(Tool::Claude, &s),
            [args(&["plugin", "uninstall", "bilbo@bilbo", "--json"])]
        );
        assert_eq!(
            removal(Tool::Codex, &s),
            [args(&["plugin", "remove", "bilbo@bilbo", "--json"])]
        );
    }

    #[test]
    fn removal_both() {
        let s = state(
            Some(listed_github("delucca/bilbo", None)),
            Some(installed(true, "0.1.0")),
        );
        let got: Vec<String> = removal(Tool::Codex, &s)
            .iter()
            .map(|a| a.join(" "))
            .collect();
        assert_eq!(
            got,
            [
                "plugin remove bilbo@bilbo --json",
                "plugin marketplace remove bilbo --json"
            ]
        );
        let only_market = state(Some(listed_github("delucca/bilbo", None)), None);
        assert_eq!(removal(Tool::Claude, &only_market).len(), 1);
    }

    #[test]
    fn run_all_stops_at_the_first_failure() {
        let runner = Script::new(vec![
            fixture!("claude-marketplace-remove"),
            fixture!("claude-marketplace-add-missing-tag"),
            fixture!("claude-install"),
        ]);
        let commands = vec![
            drop_marketplace(),
            add(Tool::Claude, &github("delucca", "v9.9.9")),
            install(Tool::Claude),
        ];
        let failed = run_all(Tool::Claude, &runner, Path::new("/x/claude"), &commands)
            .err()
            .unwrap();
        assert_eq!(failed.args, commands[1]);
        assert!(failed.message.starts_with("Failed to clone"));
        assert_eq!(runner.calls.borrow().len(), 2);
    }

    #[test]
    fn run_all_codex_judges_by_exit_code() {
        let runner = Script::new(vec![fixture!("codex-marketplace-add-github-other-ref")]);
        let commands = vec![add(Tool::Codex, &github("delucca/bilbo", "main"))];
        let failed = run_all(Tool::Codex, &runner, Path::new("/x/codex"), &commands)
            .err()
            .unwrap();
        assert!(failed.message.contains("already added"));
        let runner = Script::new(vec![fixture!("codex-add")]);
        assert!(run_all(Tool::Codex, &runner, Path::new("/x/codex"), &commands).is_ok());
    }

    #[test]
    fn claude_removal_tolerates_not_installed() {
        let runner = Script::new(vec![
            fixture!("claude-uninstall-missing"),
            fixture!("claude-marketplace-remove-missing"),
        ]);
        let commands = vec![uninstall(Tool::Claude), drop_marketplace()];
        assert!(run_all(Tool::Claude, &runner, Path::new("/x/claude"), &commands).is_ok());
        let runner = Script::new(vec![fixture!("claude-uninstall-missing")]);
        let failed = run_all(
            Tool::Claude,
            &runner,
            Path::new("/x/claude"),
            &[install(Tool::Claude)],
        );
        assert!(failed.is_err(), "only removal tolerates it");
    }

    #[test]
    fn claude_outcome_beats_the_exit_code() {
        let failed_json = done(0, r#"{"outcome":"failed","message":"boom"}"#, "");
        let runner = Script::new(vec![failed_json]);
        let failed = run_all(
            Tool::Claude,
            &runner,
            Path::new("/x/claude"),
            &[install(Tool::Claude)],
        )
        .err()
        .unwrap();
        assert_eq!(failed.message, "boom");
        let runner = Script::new(vec![done(0, "not json", "")]);
        assert!(
            run_all(
                Tool::Claude,
                &runner,
                Path::new("/x/claude"),
                &[install(Tool::Claude)]
            )
            .is_ok()
        );
    }

    #[test]
    fn a_program_that_cannot_start_is_reported() {
        struct Dead;
        impl Runner for Dead {
            fn run(&self, program: &Path, _: &[String]) -> Result<Output, String> {
                Err(format!("cannot run {}: gone", program.display()))
            }
        }
        let failed = run_all(
            Tool::Codex,
            &Dead,
            Path::new("/x/codex"),
            &[install(Tool::Codex)],
        )
        .err()
        .unwrap();
        assert_eq!(failed.message, "cannot run /x/codex: gone");
    }

    #[test]
    fn default_source_uses_the_package_folder() {
        let s = scratch("package");
        let share = s.0.join("share/bilbo");
        marketplace_in(&share);
        let exe = s.0.join("bin/bilbo");
        assert_eq!(default_source(&exe, "0.1.0"), Source::Folder(share));
    }

    #[test]
    fn default_source_falls_back_to_the_tag() {
        let s = scratch("notag");
        let got = default_source(&s.0.join("bin/bilbo"), "0.1.0");
        assert_eq!(got, github("delucca/bilbo", "v0.1.0"));
        assert_eq!(got.display(), "delucca/bilbo#v0.1.0");
    }

    #[test]
    fn parse_source_cases() {
        let s = scratch("source");
        let cwd = std::fs::canonicalize(&s.0).unwrap();
        marketplace_in(&cwd.join("rel"));
        marketplace_in(&cwd.join("home/x"));
        std::fs::create_dir_all(cwd.join("bare")).unwrap();
        let home = cwd.join("home");
        let parse = |t: &str| parse_source(t, &cwd, Some(&home));
        assert_eq!(
            parse("delucca/bilbo#v0.2.0"),
            Ok(github("delucca/bilbo", "v0.2.0"))
        );
        assert_eq!(parse("rel"), Ok(Source::Folder(cwd.join("rel"))));
        assert_eq!(parse("~/x"), Ok(Source::Folder(home.join("x"))));
        let nor = |t: &str| {
            format!(
                "{t} is not a folder with .claude-plugin/marketplace.json, nor <owner>/<repo>#<ref>"
            )
        };
        for bad in ["bare", "a/b", "a/b#", "a/b#-x", "a/b#x y", "a b/c#d"] {
            assert_eq!(parse(bad), Err(nor(bad)), "{bad}");
        }
    }

    /// A `Calls` that answers each method from a recorded reply and keeps every request.
    struct Scripted {
        replies: Vec<(&'static str, Result<Value, String>)>,
        seen: Vec<(String, Value)>,
    }

    impl crate::command::Calls for Scripted {
        fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
            self.seen.push((method.to_string(), params));
            self.replies
                .iter()
                .find(|(name, _)| *name == method)
                .map(|(_, reply)| reply.clone())
                .unwrap_or_else(|| Err(format!("unscripted {method}")))
        }
    }

    /// The `result` of the reply a fixture recorded.
    fn result(output: Output) -> Value {
        let reply: Value = serde_json::from_str(output.stdout.trim()).unwrap();
        reply["result"].clone()
    }

    const KEY: &str = "bilbo@bilbo:hooks/hooks.json:user_prompt_submit:0:0";
    const HASH: &str = "sha256:25e2d1e2cbe24cbc61bd80f28e947112c2db5245aaf103860d7721932066d0ed";

    fn script(listed: Value) -> Scripted {
        Scripted {
            replies: vec![
                ("hooks/list", Ok(listed)),
                (
                    "config/batchWrite",
                    Ok(result(fixture!("codex-app-server-batch-write"))),
                ),
            ],
            seen: Vec::new(),
        }
    }

    fn writes(rpc: &Scripted) -> Vec<&Value> {
        rpc.seen
            .iter()
            .filter(|(method, _)| method == "config/batchWrite")
            .map(|(_, params)| params)
            .collect()
    }

    #[test]
    fn trust_hooks_writes_untrusted_hooks() {
        let mut rpc = script(result(fixture!("codex-app-server-list-untrusted")));
        let trust = trust_hooks(&mut rpc, Path::new("/fixture/home"));
        assert_eq!(trust, Ok(Trust::Wrote { changed: false }));
        assert_eq!(rpc.seen[0].0, "hooks/list");
        assert_eq!(
            rpc.seen[0].1,
            serde_json::json!({"cwds": ["/fixture/home"]})
        );
        assert_eq!(
            writes(&rpc),
            [&serde_json::json!({
                "edits": [{
                    "keyPath": "hooks.state",
                    "value": {KEY: {"trusted_hash": HASH}},
                    "mergeStrategy": "upsert"
                }],
                "reloadUserConfig": false
            })]
        );
    }

    #[test]
    fn trust_hooks_keeps_trusted_hooks() {
        let mut rpc = script(result(fixture!("codex-app-server-list-trusted")));
        assert_eq!(
            trust_hooks(&mut rpc, Path::new("/fixture/home")),
            Ok(Trust::Kept)
        );
        assert!(writes(&rpc).is_empty());
    }

    #[test]
    fn trust_hooks_marks_modified_as_changed() {
        let mut rpc = script(result(fixture!("codex-app-server-list-modified")));
        assert_eq!(
            trust_hooks(&mut rpc, Path::new("/fixture/home")),
            Ok(Trust::Wrote { changed: true })
        );
        assert_eq!(writes(&rpc).len(), 1);
    }

    #[test]
    fn trust_hooks_without_a_bilbo_hook() {
        let mut listed = result(fixture!("codex-app-server-list-untrusted"));
        listed["data"][0]["hooks"][0]["pluginId"] = "other@plugin".into();
        let mut other = script(listed);
        assert_eq!(
            trust_hooks(&mut other, Path::new("/fixture/home")),
            Ok(Trust::NoHook)
        );
        let mut none = script(serde_json::json!({"data": [{"cwd": "/", "hooks": []}]}));
        assert_eq!(
            trust_hooks(&mut none, Path::new("/fixture/home")),
            Ok(Trust::NoHook)
        );
        assert!(writes(&other).is_empty() && writes(&none).is_empty());
    }

    #[test]
    fn trust_hooks_leaves_managed_hooks_and_reports_what_codex_got_wrong() {
        let mut listed = result(fixture!("codex-app-server-list-untrusted"));
        listed["data"][0]["hooks"][0]["trustStatus"] = "managed".into();
        let mut rpc = script(listed);
        assert_eq!(trust_hooks(&mut rpc, Path::new("/")), Ok(Trust::Kept));
        let mut listed = result(fixture!("codex-app-server-list-untrusted"));
        listed["data"][0]["hooks"][0]["currentHash"] = Value::Null;
        let mut rpc = script(listed);
        assert_eq!(
            trust_hooks(&mut rpc, Path::new("/")),
            Err("codex listed a bilbo hook without key, currentHash or trustStatus".into())
        );
        let mut failing = script(Value::Null);
        failing.replies[1].1 = Err("codex app-server answered config/batchWrite: boom".into());
        failing.replies[0].1 = Ok(result(fixture!("codex-app-server-list-untrusted")));
        assert_eq!(
            trust_hooks(&mut failing, Path::new("/")),
            Err("codex app-server answered config/batchWrite: boom".into())
        );
    }

    #[test]
    fn trust_hooks_refuses_an_unknown_trust_status() {
        let mut listed = result(fixture!("codex-app-server-list-untrusted"));
        listed["data"][0]["hooks"][0]["trustStatus"] = "blocked".into();
        let mut rpc = script(listed);
        assert_eq!(
            trust_hooks(&mut rpc, Path::new("/")),
            Err("codex reports trust status 'blocked' for a bilbo hook".into())
        );
        assert!(writes(&rpc).is_empty());
    }

    #[test]
    fn forget_hooks_deletes_bilbo_keys_only() {
        let mut read = result(fixture!("codex-app-server-config-read"));
        read["config"]["hooks"]["state"]["other@plugin:hooks/hooks.json:stop:0:0"] =
            serde_json::json!({"trusted_hash": "sha256:other"});
        let mut rpc = Scripted {
            replies: vec![
                ("config/read", Ok(read)),
                (
                    "config/batchWrite",
                    Ok(result(fixture!("codex-app-server-batch-write"))),
                ),
            ],
            seen: Vec::new(),
        };
        assert_eq!(forget_hooks(&mut rpc), Ok(1));
        assert_eq!(rpc.seen[0].1, serde_json::json!({"includeLayers": false}));
        assert_eq!(
            writes(&rpc),
            [&serde_json::json!({
                "edits": [{
                    "keyPath": format!("hooks.state.\"{KEY}\""),
                    "value": null,
                    "mergeStrategy": "replace"
                }],
                "reloadUserConfig": false
            })]
        );
        let mut none = Scripted {
            replies: vec![(
                "config/read",
                Ok(serde_json::json!({"config": {"hooks": {"state": {"other@plugin:x": {}}}}})),
            )],
            seen: Vec::new(),
        };
        assert_eq!(forget_hooks(&mut none), Ok(0));
        assert_eq!(none.seen.len(), 1);
    }

    #[test]
    fn an_unknown_method_reply_is_an_error_object() {
        let reply: Value =
            serde_json::from_str(fixture!("codex-app-server-unknown-method").stdout.trim())
                .unwrap();
        assert_eq!(reply["error"]["code"], -32600);
    }
}
