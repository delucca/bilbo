use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use bilbo_settings::error::Error;
use bilbo_settings::key::{
    About, Default, Key, Resolved, Shown, Source, quote, show_list, show_texts,
};
use serde_json::Value as Json;

use crate::child::{self, Bounds, End, Environment};
use crate::model::{
    Cause, Limits, Model, Outcome, Prompt, Usage, Variables, Way, What, answered_model, identifier,
};

const INSTRUCTION: &str = "{instruction}";
const MODEL_PLACEHOLDER: &str = "{model}";
const FALLBACK_MODEL: &str = "--fallback-model";
const UNBOUND_MODEL: &str = "llm.anthropic.tool.arguments does not bind a call to its model: it needs \"--model\" followed by \"{model}\", and must not hold \"--fallback-model\"";
const UNBOUND_INSTRUCTION: &str = "llm.anthropic.tool.arguments does not bind a call to its instruction: the value of \"--system-prompt\" and of \"--append-system-prompt\" must be \"{instruction}\"";
const WITHOUT_TOOLS: &str = "llm.anthropic.tool.arguments must keep the model without tools: ";
const SYSTEM_PROMPTS: [&str; 2] = ["--system-prompt", "--append-system-prompt"];
const FLAGS: [(&[&str], usize); 15] = [
    (&["-p", "--print"], 0),
    (&["--model"], 1),
    (&["--effort"], 1),
    (&["--output-format"], 1),
    (&["--tools"], 1),
    (&["--disable-slash-commands"], 0),
    (&["--strict-mcp-config"], 0),
    (&["--setting-sources"], 1),
    (&["--settings"], 1),
    (&["--no-session-persistence"], 0),
    (&["--safe-mode"], 0),
    (&["--system-prompt"], 1),
    (&["--append-system-prompt"], 1),
    (&["--fallback-model"], 1),
    (&["--max-budget-usd"], 1),
];
const REPLY: &str = "bilbo reads the reply as claude 2.1.327 prints it";
const SETUP: &str = ".claude/";

pub const PROGRAM: Key<String> = Key {
    about: About {
        name: "llm.anthropic.tool.program",
        variable: Some("BILBO_LLM_ANTHROPIC_TOOL_PROGRAM"),
        default: Default::Toml(r#""claude""#),
        allowed: crate::tool::PROGRAM_ALLOWED,
        doc: "The program bilbo starts for the tool claude. A timer's run needs its full path.",
    },
    parse: crate::tool::program,
    show: |program| quote(program),
};

pub const EFFORT: Key<String> = Key {
    about: About {
        name: "llm.anthropic.tool.effort",
        variable: Some("BILBO_LLM_ANTHROPIC_TOOL_EFFORT"),
        default: Default::Toml(r#""medium""#),
        allowed: crate::tool::EFFORT_ALLOWED,
        doc: "The effort bilbo asks the tool claude for.",
    },
    parse: crate::tool::effort,
    show: |word| quote(word),
};

pub const ARGUMENTS: Key<Vec<String>> = Key {
    about: About {
        name: "llm.anthropic.tool.arguments",
        variable: None,
        default: Default::Toml(
            r##"["-p", "--model", "{model}", "--effort", "{effort}", "--output-format", "json", "--tools", "", "--disable-slash-commands", "--strict-mcp-config", "--setting-sources", "project", "--settings", '{"disableAllHooks": true}', "--no-session-persistence", "--safe-mode", "--system-prompt", "{instruction}"]"##,
        ),
        allowed: crate::tool::ARGUMENTS_ALLOWED,
        doc: "Every argument the tool claude is started with. {model} and {effort} are replaced once, wherever a value stands. {instruction} stands as the value of --system-prompt or --append-system-prompt, or in none: the instruction then goes first on standard input. The list must keep the model without tools.",
    },
    parse: crate::tool::arguments,
    show: |items| show_list(items),
};

pub const INHERIT: Key<Vec<String>> = Key {
    about: About {
        name: "llm.anthropic.tool.inherit",
        variable: None,
        default: Default::Toml(r#"["HOME", "USER", "PATH", "TMPDIR"]"#),
        allowed: crate::tool::INHERIT_ALLOWED,
        doc: "The variables of bilbo's own environment the tool claude gets. It gets no other.",
    },
    parse: crate::tool::inherit,
    show: |items| show_list(items),
};

pub const ENVIRONMENT: Key<Vec<(String, String)>> = Key {
    about: About {
        name: "llm.anthropic.tool.environment",
        variable: None,
        default: Default::Toml(r#"{ DISABLE_PROMPT_CACHING = "1", LANG = "en_US.UTF-8" }"#),
        allowed: crate::tool::ENVIRONMENT_ALLOWED,
        doc: "Variables the tool claude is given beside the inherited ones.",
    },
    parse: crate::tool::environment,
    show: |items| show_texts(items),
};

pub const PROJECT_FILES: Key<Vec<String>> = Key {
    about: About {
        name: "llm.anthropic.tool.project_files",
        variable: None,
        default: Default::Toml(
            r#"["CLAUDE.md", "CLAUDE.local.md", "AGENTS.md", ".mcp.json", ".claude/CLAUDE.md", ".claude/rules", ".claude/skills", ".claude/commands", ".claude/settings.json", ".claude/settings.local.json"]"#,
        ),
        allowed: "a list of relative paths with no .. part",
        doc: "The files and folders the tool claude reads as a project's. bilbo does not start the tool when a folder above the one it runs in holds one.",
    },
    parse: |value| {
        let items = value.items().ok_or_else(|| value.refuse())?;
        items
            .iter()
            .map(|item| {
                item.text()
                    .filter(|path| {
                        !path.is_empty()
                            && !path.starts_with('/')
                            && path.split('/').all(|part| part != "..")
                    })
                    .map(str::to_owned)
                    .ok_or_else(|| item.refuse())
            })
            .collect()
    },
    show: |items| show_list(items),
};

pub const ABOUT: [&About; 6] = [
    &PROGRAM.about,
    &EFFORT.about,
    &ARGUMENTS.about,
    &INHERIT.about,
    &ENVIRONMENT.about,
    &PROJECT_FILES.about,
];

#[derive(Debug, Clone)]
pub struct Section {
    pub program: Resolved<String>,
    pub model: Resolved<String>,
    pub model_key: &'static str,
    pub effort: Resolved<String>,
    pub arguments: Resolved<Vec<String>>,
    pub inherit: Resolved<Vec<String>>,
    pub environment: Resolved<Vec<(String, String)>>,
    pub project_files: Resolved<Vec<String>>,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        program: PROGRAM.resolve(source)?,
        model: super::MODEL.resolve(source)?,
        model_key: super::MODEL.about.name,
        effort: EFFORT.resolve(source)?,
        arguments: ARGUMENTS.resolve(source)?,
        inherit: INHERIT.resolve(source)?,
        environment: ENVIRONMENT.resolve(source)?,
        project_files: PROJECT_FILES.resolve(source)?,
    })
}

pub fn shown(source: &Source<'_>) -> Result<Vec<Shown>, Error> {
    let section = section(source)?;
    Ok(vec![
        PROGRAM.shown(&section.program),
        EFFORT.shown(&section.effort),
        ARGUMENTS.shown(&section.arguments),
        INHERIT.shown(&section.inherit),
        ENVIRONMENT.shown(&section.environment),
        PROJECT_FILES.shown(&section.project_files),
    ])
}

pub fn without_tools(arguments: &[String]) -> Result<(), String> {
    let refuse = |rest: &str| Err(format!("{WITHOUT_TOOLS}{rest}"));
    let tools = "it needs \"--tools\" followed by \"\"";
    let mut stood: Vec<&str> = Vec::new();
    let mut at = 0;
    while let Some(item) = arguments.get(at) {
        let Some((names, values)) = FLAGS
            .iter()
            .find(|(names, _)| names.contains(&item.as_str()))
        else {
            if item.starts_with("--tools=") {
                return refuse("write \"--tools\" and \"\" as two items");
            }
            return refuse(&format!("it must not hold \"{item}\""));
        };
        if at + values >= arguments.len() && *values > 0 {
            return refuse(&format!("it needs a value after \"{item}\""));
        }
        for value in &arguments[at + 1..=at + values] {
            if value.contains(INSTRUCTION) && !SYSTEM_PROMPTS.contains(&item.as_str()) {
                return refuse(
                    "\"{instruction}\" may stand only as the value of \"--system-prompt\" or \"--append-system-prompt\"",
                );
            }
            if item == "--tools" && !value.is_empty() {
                return refuse(tools);
            }
        }
        stood.push(names[0]);
        at += 1 + values;
    }
    for (flag, text) in [
        ("--tools", tools),
        ("--strict-mcp-config", "it needs \"--strict-mcp-config\""),
        ("--safe-mode", "it needs \"--safe-mode\""),
    ] {
        if !stood.contains(&flag) {
            return refuse(text);
        }
    }
    if !stood.contains(&"--no-session-persistence") {
        return Err("llm.anthropic.tool.arguments needs \"--no-session-persistence\": without it the tool saves every call as a session, and bilbo remember would keep the records a second time".to_owned());
    }
    Ok(())
}

pub fn binds(arguments: &[String]) -> Result<(), String> {
    let mut model = false;
    let mut model_free = true;
    let mut instruction_free = true;
    let mut at = 0;
    while let Some(item) = arguments.get(at) {
        let values = FLAGS
            .iter()
            .find(|(names, _)| names.contains(&item.as_str()))
            .map_or(0, |(_, values)| *values);
        let given =
            &arguments[(at + 1).min(arguments.len())..(at + 1 + values).min(arguments.len())];
        match item.as_str() {
            "--model" => {
                model = true;
                model_free &= given == [MODEL_PLACEHOLDER];
            }
            FALLBACK_MODEL => model_free = false,
            flag if SYSTEM_PROMPTS.contains(&flag) => instruction_free &= given == [INSTRUCTION],
            _ => {}
        }
        at += 1 + values;
    }
    if !model || !model_free {
        return Err(UNBOUND_MODEL.to_owned());
    }
    if !instruction_free {
        return Err(UNBOUND_INSTRUCTION.to_owned());
    }
    Ok(())
}

pub fn above(work: &Path, files: &[String], home: Option<&str>) -> Result<(), String> {
    let real = fs::canonicalize(work).map_err(|error| {
        format!(
            "llm.anthropic.tool: {} cannot be resolved: {error}",
            work.display()
        )
    })?;
    let home = home.and_then(|home| fs::canonicalize(home).ok());
    for folder in real.ancestors().skip(1) {
        for file in files {
            // ~/.claude is the tool's own folder for the user's setup, not a project's.
            if file.starts_with(SETUP) && home.as_deref() == Some(folder) {
                continue;
            }
            let found = folder.join(file);
            if fs::symlink_metadata(&found).is_ok() {
                return Err(format!(
                    "llm.anthropic.tool: {} is above the folder the tool runs in, and claude reads it as a project's; set paths.state outside {}",
                    found.display(),
                    folder.display()
                ));
            }
        }
    }
    Ok(())
}

fn replaced(text: &str, model: &str, effort: &str, instruction: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('{') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let (name, value) = [
            ("{model}", model),
            ("{effort}", effort),
            (INSTRUCTION, instruction),
        ]
        .into_iter()
        .find(|(name, _)| rest.starts_with(name))
        .unzip();
        match name.zip(value) {
            Some((name, value)) => {
                out.push_str(value);
                rest = &rest[name.len()..];
            }
            None => {
                out.push('{');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn invocation(section: &Section, prompt: &Prompt) -> (Vec<String>, String) {
    let model = &section.model.value;
    let effort = &section.effort.value;
    let arguments = section
        .arguments
        .value
        .iter()
        .map(|argument| replaced(argument, model, effort, &prompt.instruction))
        .collect();
    let stdin = if section
        .arguments
        .value
        .iter()
        .any(|argument| argument.contains(INSTRUCTION))
    {
        prompt.material.clone()
    } else {
        format!("{}\n\n{}", prompt.instruction, prompt.material)
    };
    (arguments, stdin)
}

pub struct Claude {
    section: Section,
    folder: PathBuf,
    variables: Variables,
    environment: Option<Vec<(String, String)>>,
    why: Option<String>,
}

impl fmt::Debug for Claude {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Claude")
            .field("program", &self.section.program.value)
            .finish_non_exhaustive()
    }
}

impl Claude {
    pub fn new(section: Section, variables: Variables, folder: PathBuf) -> Claude {
        Claude {
            section,
            folder,
            variables,
            environment: None,
            why: None,
        }
    }

    fn checks(&self, environment: &[(String, String)]) -> Result<(), String> {
        without_tools(&self.section.arguments.value)?;
        child::folder(&self.folder)?;
        let home = environment
            .iter()
            .find(|(name, _)| name == "HOME")
            .map(|(_, value)| value.as_str());
        above(&self.folder, &self.section.project_files.value, home)
    }
}

impl Model for Claude {
    fn prepare(&mut self) -> Result<(), String> {
        if self.section.model.value.is_empty() {
            return Err(format!("{} is not set", self.section.model_key));
        }
        let environment = crate::tool::inherited(
            &self.section.inherit.value,
            &self.section.environment.value,
            &self.variables,
        );
        self.environment = Some(environment.clone());
        self.checks(&environment)
    }

    fn call(&mut self, prompt: &Prompt, limits: &Limits) -> Outcome {
        self.why = None;
        let Some(environment) = self.environment.clone() else {
            return Outcome::Stopped(Cause::new(What::NotStarted));
        };
        if let Err(sentence) = self.checks(&environment) {
            self.why = Some(sentence);
            return Outcome::Stopped(Cause::new(What::NotStarted));
        }
        let (arguments, stdin) = invocation(&self.section, prompt);
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let variables: Vec<(&str, &str)> = environment
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let limit = usize::try_from(limits.response).unwrap_or(usize::MAX);
        let Ok(run) = child::run(
            Path::new(&self.section.program.value),
            &arguments,
            Environment::Only(&variables),
            &self.folder,
            stdin.as_bytes(),
            Bounds {
                limit,
                timeout: limits.timeout,
                poll: limits.poll,
            },
        ) else {
            return failed(What::Unreadable, None, Usage::default());
        };
        match run.end {
            End::NotStarted(error) => {
                self.why = Some(format!("{}: {error}", self.section.program.value));
                Outcome::Stopped(Cause::new(What::NotStarted))
            }
            End::TimedOut => Outcome::Busy {
                wait: None,
                cause: Cause::new(What::Timeout),
            },
            End::OverLimit => failed(What::TooLarge, None, Usage::default()),
            End::Exited(code) => self.reading(&run.stdout, Some(code)),
            End::NoExitCode => self.reading(&run.stdout, None),
        }
    }

    fn id(&self) -> &str {
        &self.section.model.value
    }

    fn describe(&self) -> Vec<String> {
        let section = &self.section;
        vec![
            format!(
                "provider: {}, via {}, model {}, effort {}, program {}",
                super::NAME,
                Way::Tool.word(),
                section.model.value,
                section.effort.value,
                section.program.value
            ),
            format!("arguments: {}", show_list(&section.arguments.value)),
            format!("inherit: {}", show_list(&section.inherit.value)),
            format!("environment: {}", show_texts(&section.environment.value)),
            format!("project_files: {}", show_list(&section.project_files.value)),
        ]
    }

    fn why(&self) -> Option<&str> {
        self.why.as_deref()
    }

    fn bound(&self) -> Result<(), String> {
        binds(&self.section.arguments.value)
    }
}

impl Claude {
    fn reading(&mut self, output: &[u8], code: Option<i32>) -> Outcome {
        let (outcome, sentence) = read(output, code);
        self.why = sentence;
        outcome
    }
}

fn failed(what: What, status: Option<i32>, usage: Usage) -> Outcome {
    Outcome::Failed {
        cause: Cause {
            status,
            ..Cause::new(what)
        },
        usage,
    }
}

fn usage(reply: &Json) -> Usage {
    let Some(counts) = reply.get("usage").filter(|counts| counts.is_object()) else {
        return Usage::default();
    };
    let count = |name: &str| counts.get(name).and_then(Json::as_u64);
    Usage {
        read: count("input_tokens").map(|input| {
            input
                .saturating_add(count("cache_creation_input_tokens").unwrap_or(0))
                .saturating_add(count("cache_read_input_tokens").unwrap_or(0))
        }),
        written: count("output_tokens"),
    }
}

fn missing(path: &str, found: Option<&Json>, wanted: &str) -> String {
    match found {
        None => format!("the reply holds no \"{path}\": {REPLY}"),
        Some(_) => format!("the reply's \"{path}\" is not {wanted}: {REPLY}"),
    }
}

fn whole(path: &str, value: Option<&Json>) -> Result<u64, String> {
    value
        .and_then(Json::as_u64)
        .ok_or_else(|| missing(path, value, "a whole number"))
}

fn object<'a>(
    path: &str,
    value: Option<&'a Json>,
) -> Result<&'a serde_json::Map<String, Json>, String> {
    value
        .and_then(Json::as_object)
        .ok_or_else(|| missing(path, value, "an object"))
}

fn counted_in(path: &str, value: &Json) -> Result<bool, String> {
    match value {
        Json::Object(fields) => {
            let mut used = false;
            for item in fields.values() {
                used |= counted_in(path, item)?;
            }
            Ok(used)
        }
        other => whole(path, Some(other)).map(|number| number > 0),
    }
}

fn guard(reply: &Json) -> Result<bool, String> {
    let mut tool = whole("num_turns", reply.get("num_turns"))? > 1;
    let denials = reply.get("permission_denials");
    tool |= !denials
        .and_then(Json::as_array)
        .ok_or_else(|| missing("permission_denials", denials, "a list"))?
        .is_empty();
    let servers = reply
        .get("usage")
        .and_then(|usage| usage.get("server_tool_use"));
    for item in object("usage.server_tool_use", servers)?.values() {
        tool |= whole("usage.server_tool_use.*", Some(item))? > 0;
    }
    for item in object("modelUsage", reply.get("modelUsage"))?.values() {
        let fields = object("modelUsage.*", Some(item))?;
        tool |= whole(
            "modelUsage.*.webSearchRequests",
            fields.get("webSearchRequests"),
        )? > 0;
    }
    let stats = object("subagent_stats", reply.get("subagent_stats"))?;
    for item in stats.values() {
        tool |= counted_in("subagent_stats.*", item)?;
    }
    Ok(tool)
}

fn read(output: &[u8], code: Option<i32>) -> (Outcome, Option<String>) {
    let Some(reply) = serde_json::from_slice::<Json>(output)
        .ok()
        .filter(Json::is_object)
    else {
        return (failed(What::Unreadable, code, Usage::default()), None);
    };
    let counted = usage(&reply);
    let unreadable = |sentence: String| (failed(What::Unreadable, code, counted), Some(sentence));
    match guard(&reply) {
        Ok(true) => return (failed(What::ToolUsed, code, counted), None),
        Ok(false) => {}
        Err(sentence) => return unreadable(sentence),
    }
    let reason = reply.get("stop_reason").and_then(Json::as_str);
    // The tool reports a refusal as an error, so the stop reason is read first.
    if reason == Some("refusal") {
        let refused = Outcome::Refused {
            category: None,
            usage: counted,
        };
        return (refused, None);
    }
    if reply.get("is_error").and_then(Json::as_bool) == Some(true) || code != Some(0) {
        let error = Outcome::Failed {
            cause: Cause {
                status: code,
                kind: reply.get("subtype").and_then(Json::as_str).map(identifier),
                ..Cause::new(What::ToolError)
            },
            usage: counted,
        };
        return (error, None);
    }
    let Some(reason) = reason else {
        return unreadable(missing("stop_reason", reply.get("stop_reason"), "a text"));
    };
    let outcome = match reason {
        "max_tokens" => Outcome::Cut(counted),
        "end_turn" => match reply.get("result").and_then(Json::as_str) {
            Some(text) => Outcome::Answer {
                text: text.to_owned(),
                usage: counted,
                model: single_model(&reply),
            },
            None => failed(What::Unreadable, code, counted),
        },
        other => Outcome::Failed {
            cause: Cause {
                status: code,
                code: Some(identifier(other)),
                ..Cause::new(What::StopReason)
            },
            usage: counted,
        },
    };
    (outcome, None)
}

fn single_model(reply: &Json) -> Option<String> {
    let used = reply.get("modelUsage")?.as_object()?;
    match used.keys().collect::<Vec<_>>().as_slice() {
        [only] => Some(answered_model(only)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use bilbo_settings::error::{At, Error};
    use bilbo_settings::file::File;
    use bilbo_settings::key::Source;

    use super::{ABOUT, Claude, Section, invocation, read, section, shown, without_tools};
    use crate::model::{Model, Outcome, Prompt};

    const ANSWER: &str = include_str!("../../fixtures/anthropic/tool/answer.json");

    fn resolve(text: &str) -> Result<Section, Error> {
        let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
        section(&Source {
            var: &|_| None,
            file: &file,
        })
    }

    fn refused_at_line_2(text: &str) {
        match resolve(text) {
            Err(Error::Invalid {
                at: At::Line { line, .. },
                ..
            }) => assert_eq!(line, 2, "{text}"),
            other => panic!("{text}: {other:?}"),
        }
    }

    fn with(key: &str, value: &str) -> String {
        format!("[llm.anthropic.tool]\n{key} = {value}\n")
    }

    fn items(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| (*item).to_owned()).collect()
    }

    fn default_arguments() -> Vec<String> {
        resolve("").expect("defaults").arguments.value
    }

    #[test]
    fn each_default_is_its_documented_value() {
        let section = resolve("").expect("defaults");
        assert_eq!(section.program.value, "claude");
        assert_eq!(section.model.value, "");
        assert_eq!(section.effort.value, "medium");
        assert_eq!(
            section.arguments.value,
            items(&[
                "-p",
                "--model",
                "{model}",
                "--effort",
                "{effort}",
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
                "{instruction}",
            ])
        );
        assert_eq!(
            section.inherit.value,
            items(&["HOME", "USER", "PATH", "TMPDIR"])
        );
        assert_eq!(
            section.environment.value,
            [
                ("DISABLE_PROMPT_CACHING".to_owned(), "1".to_owned()),
                ("LANG".to_owned(), "en_US.UTF-8".to_owned()),
            ]
        );
        assert_eq!(
            section.project_files.value,
            items(&[
                "CLAUDE.md",
                "CLAUDE.local.md",
                "AGENTS.md",
                ".mcp.json",
                ".claude/CLAUDE.md",
                ".claude/rules",
                ".claude/skills",
                ".claude/commands",
                ".claude/settings.json",
                ".claude/settings.local.json",
            ])
        );
        let variables: Vec<_> = ABOUT.iter().map(|about| about.variable).collect();
        assert_eq!(
            variables,
            [
                Some("BILBO_LLM_ANTHROPIC_TOOL_PROGRAM"),
                Some("BILBO_LLM_ANTHROPIC_TOOL_EFFORT"),
                None,
                None,
                None,
                None,
            ]
        );
    }

    #[test]
    fn the_program_is_a_text_that_is_not_empty() {
        assert_eq!(
            resolve(&with("program", "\"/usr/bin/claude\""))
                .expect("taken")
                .program
                .value,
            "/usr/bin/claude"
        );
        refused_at_line_2(&with("program", "\"\""));
        refused_at_line_2(&with("program", "[\"claude\"]"));
    }

    #[test]
    fn the_effort_is_a_lower_case_word() {
        for good in ["low", "medium", "xhigh"] {
            assert!(
                resolve(&with("effort", &format!("\"{good}\""))).is_ok(),
                "{good}"
            );
        }
        for bad in ["High", "a b", ""] {
            refused_at_line_2(&with("effort", &format!("\"{bad}\"")));
        }
    }

    #[test]
    fn the_arguments_are_a_list_that_may_hold_an_empty_item() {
        let taken = resolve(&with("arguments", "[\"--tools\", \"\"]")).expect("taken");
        assert_eq!(taken.arguments.value, items(&["--tools", ""]));
        refused_at_line_2(&with("arguments", "\"-p\""));
        refused_at_line_2(&with("arguments", "[1]"));
    }

    #[test]
    fn the_inherited_names_and_the_environments_names_are_variables_names() {
        assert!(resolve(&with("inherit", "[\"A_B1\"]")).is_ok());
        assert!(resolve("[llm.anthropic.tool.environment]\nA_B1 = \"x\"\n").is_ok());
        let said = |text: &str| resolve(text).expect_err("refused").to_string();
        let cases = [
            (
                "llm.anthropic.tool.environment = \"sk-test-PLUM-7391\"\n".to_owned(),
                "/c/config.toml:1: llm.anthropic.tool.environment must be a table of texts",
            ),
            (
                "[llm.anthropic.tool.environment]\nA = 1\n".to_owned(),
                "/c/config.toml:1: llm.anthropic.tool.environment must hold texts alone",
            ),
            (
                "[llm.anthropic.tool.environment]\nMY-TOKEN = \"sk-test-PLUM-7391\"\n".to_owned(),
                "/c/config.toml:1: llm.anthropic.tool.environment must name its variables with letters, digits and _",
            ),
            (
                "llm.anthropic.tool.inherit = \"ANTHROPIC_API_KEY=sk-test-PLUM-7391\"\n".to_owned(),
                "/c/config.toml:1: llm.anthropic.tool.inherit must be a list of variables' names",
            ),
        ];
        for (text, line) in cases {
            assert_eq!(said(&text), line);
            assert!(!said(&text).contains("PLUM"));
        }
        for bad in [
            "[\"A-B\"]",
            "[\"1A=\"]",
            "[\"\"]",
            "[\"KEY=sk-test-PLUM-7391\"]",
            "[1]",
        ] {
            let line = said(&format!("llm.anthropic.tool.inherit = {bad}\n"));
            assert_eq!(
                line,
                "/c/config.toml:1: llm.anthropic.tool.inherit must hold variables' names alone: letters, digits and _"
            );
            assert!(!line.contains("PLUM"), "{line}");
        }
    }

    #[test]
    fn the_project_files_are_relative_paths_with_no_parent_part() {
        let taken =
            resolve(&with("project_files", "[\"CLAUDE.md\", \".claude/rules\"]")).expect("taken");
        assert_eq!(
            taken.project_files.value,
            items(&["CLAUDE.md", ".claude/rules"])
        );
        for bad in ["[\"/etc/x\"]", "[\"../x\"]", "[\"a/../b\"]", "[\"\"]"] {
            refused_at_line_2(&with("project_files", bad));
        }
    }

    #[test]
    fn each_placeholder_is_replaced_once_wherever_it_stands() {
        let text = format!(
            "[llm.anthropic]\nmodel = \"m1\"\n\n[llm.anthropic.tool]\neffort = \"low\"\narguments = {}\n",
            "[\"x={model}\", \"--effort={effort}\", \"{instruction}\", \"{model}{model}\", \"{other}\"]"
        );
        let section = resolve(&text).expect("section");
        let prompt = Prompt {
            instruction: "keep {model} and {effort}".to_owned(),
            material: "records".to_owned(),
        };
        let (arguments, stdin) = invocation(&section, &prompt);
        assert_eq!(
            arguments,
            items(&[
                "x=m1",
                "--effort=low",
                "keep {model} and {effort}",
                "m1m1",
                "{other}"
            ])
        );
        assert_eq!(stdin, "records");
    }

    #[test]
    fn with_no_instruction_argument_the_instruction_goes_first_on_standard_input() {
        let section = resolve(&with("arguments", "[\"-p\"]")).expect("section");
        let prompt = Prompt {
            instruction: "the instruction".to_owned(),
            material: "the material".to_owned(),
        };
        let (arguments, stdin) = invocation(&section, &prompt);
        assert_eq!(arguments, items(&["-p"]));
        assert_eq!(stdin, "the instruction\n\nthe material");
    }

    fn text(rest: &str) -> String {
        format!("llm.anthropic.tool.arguments must keep the model without tools: {rest}")
    }

    fn edit(change: &dyn Fn(&mut Vec<String>)) -> Result<(), String> {
        let mut list = default_arguments();
        change(&mut list);
        without_tools(&list)
    }

    fn at(list: &[String], item: &str) -> usize {
        list.iter().position(|found| found == item).expect("item")
    }

    #[test]
    fn the_arguments_must_keep_the_model_without_tools() {
        let tools = text("it needs \"--tools\" followed by \"\"");
        assert_eq!(without_tools(&default_arguments()), Ok(()));
        for (flag, values) in [
            ("--print", 0),
            ("--append-system-prompt", 1),
            ("--fallback-model", 1),
            ("--max-budget-usd", 1),
        ] {
            let mut more = vec![flag.to_owned()];
            more.extend((0..values).map(|_| "x".to_owned()));
            assert_eq!(edit(&|list| list.extend(more.clone())), Ok(()), "{flag}");
        }
        assert_eq!(
            edit(&|list| {
                let i = at(list, "--tools");
                list.drain(i..=i + 1);
            }),
            Err(tools.clone())
        );
        assert_eq!(
            edit(&|list| {
                let i = at(list, "--tools");
                list[i + 1] = "default".to_owned();
            }),
            Err(tools.clone())
        );
        assert_eq!(
            edit(&|list| list.extend(items(&["--tools", "Bash"]))),
            Err(tools.clone())
        );
        assert_eq!(edit(&|list| list.extend(items(&["--tools", ""]))), Ok(()));
        assert_eq!(
            edit(&|list| list.push("--tools".to_owned())),
            Err(text("it needs a value after \"--tools\""))
        );
        assert_eq!(
            edit(&|list| list.push("--tools=".to_owned())),
            Err(text("write \"--tools\" and \"\" as two items"))
        );
        assert_eq!(
            edit(&|list| list.retain(|item| item != "--strict-mcp-config")),
            Err(text("it needs \"--strict-mcp-config\""))
        );
        assert_eq!(
            edit(&|list| list.retain(|item| item != "--safe-mode")),
            Err(text("it needs \"--safe-mode\""))
        );
        assert_eq!(
            edit(&|list| list.retain(|item| item != "--no-session-persistence")),
            Err("llm.anthropic.tool.arguments needs \"--no-session-persistence\": without it the tool saves every call as a session, and bilbo remember would keep the records a second time".to_owned())
        );
        let hidden = items(&[
            "--model",
            "--strict-mcp-config",
            "--tools",
            "",
            "--safe-mode",
            "--no-session-persistence",
        ]);
        assert_eq!(
            without_tools(&hidden),
            Err(text("it needs \"--strict-mcp-config\""))
        );
    }

    #[test]
    fn an_item_that_is_no_known_flag_and_no_value_is_refused_by_name() {
        for item in [
            "Read",
            "--chrome",
            "--add-dir",
            "--permission-mode",
            "--dangerously-skip-permissions",
            "--mcp-config",
            "--allowedTools",
            "--agent",
            "--model=x",
            "--",
            "--{model}",
        ] {
            assert_eq!(
                edit(&|list| list.push(item.to_owned())),
                Err(text(&format!("it must not hold \"{item}\""))),
                "{item}"
            );
        }
        assert_eq!(
            edit(&|list| {
                let i = at(list, "--tools");
                list.insert(i + 2, "Read".to_owned());
            }),
            Err(text("it must not hold \"Read\""))
        );
        assert_eq!(
            edit(&|list| list.push("--model".to_owned())),
            Err(text("it needs a value after \"--model\""))
        );
        assert_eq!(
            edit(&|list| {
                let i = at(list, "--system-prompt");
                list.insert(i + 1, "--tools".to_owned());
            }),
            Err(text("it must not hold \"{instruction}\""))
        );
    }

    #[test]
    fn the_instruction_stands_only_as_a_system_prompts_value() {
        let only = text(
            "\"{instruction}\" may stand only as the value of \"--system-prompt\" or \"--append-system-prompt\"",
        );
        for flag in ["--model", "--settings"] {
            assert_eq!(
                edit(&|list| {
                    let i = at(list, flag);
                    list[i + 1] = "{instruction}".to_owned();
                }),
                Err(only.clone()),
                "{flag}"
            );
        }
        assert_eq!(
            edit(&|list| {
                let i = at(list, "--system-prompt");
                list[i] = "--append-system-prompt".to_owned();
            }),
            Ok(())
        );
        assert_eq!(
            edit(&|list| {
                let i = at(list, "--system-prompt");
                list.drain(i..=i + 1);
            }),
            Ok(())
        );
    }

    #[test]
    fn what_a_value_holds_changes_no_reading() {
        let text = "[llm.anthropic]\nmodel = \"--chrome\"\n";
        let section = resolve(text).expect("section");
        assert_eq!(without_tools(&section.arguments.value), Ok(()));
        let prompt = Prompt {
            instruction: "--chrome".to_owned(),
            material: "records".to_owned(),
        };
        let (arguments, _) = invocation(&section, &prompt);
        let after = |flag: &str| {
            let i = arguments.iter().position(|item| item == flag).expect(flag);
            arguments[i + 1].clone()
        };
        assert_eq!(after("--model"), "--chrome");
        assert_eq!(after("--system-prompt"), "--chrome");
    }

    #[test]
    fn the_replys_counts_do_not_overflow() {
        let reply = ANSWER.replace(
            "\"input_tokens\":100,\"cache_creation_input_tokens\":5,\"cache_read_input_tokens\":3",
            "\"input_tokens\":18446744073709551615,\"cache_creation_input_tokens\":5,\"cache_read_input_tokens\":3",
        );
        assert_ne!(reply, ANSWER);
        let (Outcome::Answer { usage, .. }, _) = read(reply.as_bytes(), Some(0)) else {
            panic!("an answer");
        };
        assert_eq!(usage.read, Some(u64::MAX));
    }

    #[test]
    fn the_model_that_answered_goes_through_the_models_rule() {
        let long = "m".repeat(201);
        for name in ["", long.as_str(), "a\\u0007b"] {
            let reply = ANSWER.replace("\"test-model-1\":", &format!("\"{name}\":"));
            let (Outcome::Answer { model, .. }, _) = read(reply.as_bytes(), Some(0)) else {
                panic!("an answer");
            };
            assert_eq!(model.as_deref(), Some("unknown"), "{name:?}");
        }
    }

    #[test]
    fn the_keys_show_as_toml_that_reads_back_and_the_way_lists_six() {
        let text = "[llm.anthropic.tool]\nprogram = \"/bin/c\"\neffort = \"low\"\narguments = [\"--tools\", \"\", \"--strict-mcp-config\", \"{\\\"a\\\": 1}\"]\ninherit = [\"HOME\"]\nproject_files = [\"X.md\"]\n\n[llm.anthropic.tool.environment]\nA = \"1\"\n";
        let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
        let first = shown(&Source {
            var: &|_| None,
            file: &file,
        })
        .expect("shown");
        assert_eq!(
            first.iter().map(|item| item.name).collect::<Vec<_>>(),
            ABOUT.map(|about| about.name)
        );
        let again: String = first
            .iter()
            .map(|item| format!("{} = {}\n", item.name, item.value))
            .collect();
        let reread = File::parse(Path::new("/c/again.toml"), again.as_bytes()).expect("reads back");
        let second = shown(&Source {
            var: &|_| None,
            file: &reread,
        })
        .expect("shown again");
        let values = |list: &[bilbo_settings::key::Shown]| -> Vec<String> {
            list.iter().map(|item| item.value.clone()).collect()
        };
        assert_eq!(values(&first), values(&second));
        assert_eq!(first[4].value, "{ A = \"1\" }");
    }

    const UNBOUND_MODEL: &str = "llm.anthropic.tool.arguments does not bind a call to its model: it needs \"--model\" followed by \"{model}\", and must not hold \"--fallback-model\"";
    const UNBOUND_INSTRUCTION: &str = "llm.anthropic.tool.arguments does not bind a call to its instruction: the value of \"--system-prompt\" and of \"--append-system-prompt\" must be \"{instruction}\"";

    fn bound_by(arguments: Vec<String>) -> Result<(), String> {
        let mut section = resolve("").expect("defaults");
        section.arguments.value = arguments;
        Claude::new(
            section,
            Box::new(|_| None),
            std::path::PathBuf::from("/s/derive/work"),
        )
        .bound()
    }

    fn without(list: &mut Vec<String>, flag: &str) {
        let at = list.iter().position(|item| item == flag).expect(flag);
        list.drain(at..=at + 1);
    }

    #[test]
    fn the_default_arguments_bind_a_call() {
        assert_eq!(bound_by(default_arguments()), Ok(()));
    }

    #[test]
    fn a_list_that_names_a_model_itself_or_a_fallback_does_not_bind() {
        let at =
            |list: &[String], flag: &str| list.iter().position(|item| item == flag).expect(flag);
        let mut other_model = default_arguments();
        let model = at(&other_model, "--model");
        other_model[model + 1] = "x".to_owned();
        let mut no_model = default_arguments();
        without(&mut no_model, "--model");
        let mut fallback = default_arguments();
        fallback.extend(items(&["--fallback-model", "y"]));
        let mut both = default_arguments();
        let model = at(&both, "--model");
        both[model + 1] = "y".to_owned();
        both.extend(items(&["--fallback-model", "{model}"]));
        let mut twice = default_arguments();
        twice.extend(items(&["--model", "x"]));
        for (list, case) in [
            (other_model, "another model"),
            (no_model, "no model"),
            (fallback, "a fallback"),
            (both, "a model and a fallback"),
            (twice, "a second model"),
        ] {
            assert_eq!(bound_by(list), Err(UNBOUND_MODEL.to_owned()), "{case}");
        }
    }

    #[test]
    fn a_system_prompt_with_a_text_of_its_own_does_not_bind() {
        let prompt = |list: &[String]| {
            list.iter()
                .position(|item| item == "--system-prompt")
                .expect("--system-prompt")
        };
        let mut text = default_arguments();
        let at = prompt(&text);
        text[at + 1] = "Write notes.".to_owned();
        let mut mixed = default_arguments();
        let at = prompt(&mixed);
        mixed[at + 1] = "Write notes. {instruction}".to_owned();
        let mut appended = default_arguments();
        let at = prompt(&appended);
        appended.splice(at..=at + 1, items(&["--append-system-prompt", "x"]));
        let mut beside = default_arguments();
        beside.extend(items(&["--append-system-prompt", "x"]));
        for (list, case) in [
            (text, "a text"),
            (mixed, "a text around the instruction"),
            (appended, "the other flag"),
            (beside, "the other flag beside"),
        ] {
            assert_eq!(
                bound_by(list),
                Err(UNBOUND_INSTRUCTION.to_owned()),
                "{case}"
            );
        }
        let mut none = default_arguments();
        without(&mut none, "--system-prompt");
        assert_eq!(bound_by(none), Ok(()));
    }

    #[test]
    fn the_debug_of_a_call_and_of_the_tools_adapter_prints_no_secret() {
        let section = resolve(
            "[llm.anthropic]\nmodel = \"m\"\n[llm.anthropic.tool.environment]\nTOKEN = \"sk-test-PLUM-7391\"\n",
        )
        .expect("section");
        let tool = super::Claude::new(
            section,
            Box::new(|_| Some("sk-test-PLUM-7391".to_owned())),
            std::path::PathBuf::from("/s/derive/work"),
        );
        let shown = format!("{tool:?}");
        assert!(
            shown.contains("claude") && !shown.contains("PLUM"),
            "{shown}"
        );
    }
}
