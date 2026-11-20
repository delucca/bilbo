use std::env;
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use bilbo_settings::error::Error;
use bilbo_settings::folder;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source, quote};
use serde_json::Value as Json;

use super::{MODEL, NAME};
use crate::child::{self, Bounds, End, Environment};
use crate::table::{show_list, show_texts};
use crate::writer::{
    Cause, Limits, Outcome, Prompt, Usage, Variables, Way, What, Writer, answered_model, identifier,
};

const RELEASE: &str = "0.169.0";
const WITHOUT_TOOLS: &str = "derive.openai.tool.arguments must keep the model without tools: ";
const KEPT_OUT_REFUSED: &str = "must not name HOME or a variable that starts with CODEX_ or OPENAI_: bilbo sets the tool's home, and such a variable can change its login or where it sends a request";
const CATALOG: &str = "{catalog}";
const INSTRUCTION: &str = "{instruction}";
const FLAGS: [(&[&str], usize); 10] = [
    (&["--json"], 0),
    (&["--ephemeral"], 0),
    (&["--ignore-user-config"], 0),
    (&["--ignore-rules"], 0),
    (&["--skip-git-repo-check"], 0),
    (&["--strict-config"], 0),
    (&["-s", "--sandbox"], 1),
    (&["-m", "--model"], 1),
    (&["-c", "--config"], 1),
    (&["--disable"], 1),
];
const SETTINGS: [(&str, Option<&str>, bool); 21] = [
    ("model_catalog_json", Some("\"{catalog}\""), true),
    ("model_instructions_file", Some("\"{instruction}\""), true),
    ("model_reasoning_effort", Some("\"{effort}\""), true),
    ("agents.enabled", Some("false"), true),
    (
        "tools.experimental_request_user_input.enabled",
        Some("false"),
        true,
    ),
    ("web_search", Some("\"disabled\""), true),
    ("cloud.skills.enabled", Some("false"), true),
    ("skills.include_instructions", Some("false"), true),
    ("skills.bundled.enabled", Some("false"), true),
    ("include_environment_context", Some("false"), true),
    ("include_permissions_instructions", Some("false"), true),
    ("include_apps_instructions", Some("false"), true),
    (
        "include_collaboration_mode_instructions",
        Some("false"),
        true,
    ),
    ("project_doc_max_bytes", Some("0"), true),
    ("project_root_markers", Some("[]"), true),
    ("history.persistence", Some("\"none\""), false),
    ("analytics.enabled", Some("false"), false),
    ("otel.metrics_exporter", Some("\"none\""), false),
    ("model_verbosity", None, false),
    ("model_reasoning_summary", None, false),
    ("service_tier", None, false),
];
const FEATURES: [(&str, bool); 13] = [
    ("shell_tool", true),
    ("unified_exec", true),
    ("view_image", true),
    ("sleep_tool", true),
    ("apps", true),
    ("plugins", true),
    ("tool_suggest", true),
    ("hooks", true),
    ("image_generation", true),
    ("goals", true),
    ("shell_snapshot", false),
    ("incremental_tools", false),
    ("unbounded_connection_retries", false),
];
const ENTRY: [(&str, &str); 4] = [
    ("tool_mode", "\"direct\""),
    ("apply_patch_tool_type", "null"),
    ("experimental_supported_tools", "[]"),
    ("supports_search_tool", "false"),
];
const TOOLS: [&str; 6] = [
    "command_execution",
    "file_change",
    "mcp_tool_call",
    "collab_tool_call",
    "web_search",
    "todo_list",
];
const SET: [&str; 2] = ["CODEX_HOME", "HOME"];
const KEPT_OUT: [&str; 2] = ["CODEX_", "OPENAI_"];
const SETUP: [&str; 10] = [
    "AGENTS.md",
    "AGENTS.override.md",
    "config.toml",
    ".env",
    "environments.toml",
    "rules",
    "skills",
    "hooks.json",
    "agents",
    "memories",
];
const CATALOG_FILE: &str = "catalog.json";
const INSTRUCTION_FILE: &str = "instruction.txt";
const LOGIN_FILE: &str = "auth.json";
const HOME_FOLDER: &str = "codex";
const WRITABLE_BY_OTHERS: u32 = 0o022;
const RETRY_NOTICE: &str = "Reconnecting...";
const TOOL_REACHED: &str =
    "codex reached a tool of its own: bilbo's arguments did not keep it without tools";

pub const PROGRAM: Key<String> = Key {
    about: About {
        name: "derive.openai.tool.program",
        variable: Some("BILBO_DERIVE_OPENAI_TOOL_PROGRAM"),
        default: Default::Toml(r#""codex""#),
        allowed: crate::tool::PROGRAM_ALLOWED,
        doc: "The program bilbo derive starts for the tool codex. It must be codex 0.169.0. A timer's run needs its full path.",
    },
    parse: crate::tool::program,
    show: |program| quote(program),
};

pub const EFFORT: Key<String> = Key {
    about: About {
        name: "derive.openai.tool.effort",
        variable: Some("BILBO_DERIVE_OPENAI_TOOL_EFFORT"),
        default: Default::Toml(r#""medium""#),
        allowed: crate::tool::EFFORT_ALLOWED,
        doc: "The effort bilbo derive asks the tool codex for.",
    },
    parse: crate::tool::effort,
    show: |word| quote(word),
};

pub const ARGUMENTS: Key<Vec<String>> = Key {
    about: About {
        name: "derive.openai.tool.arguments",
        variable: None,
        default: Default::Toml(
            r##"["exec", "--json", "--ephemeral", "--ignore-user-config", "--ignore-rules", "--skip-git-repo-check", "--strict-config", "-s", "read-only", "-m", "{model}", "-c", 'model_catalog_json="{catalog}"', "-c", 'model_instructions_file="{instruction}"', "-c", 'model_reasoning_effort="{effort}"', "--disable", "shell_tool", "--disable", "unified_exec", "--disable", "view_image", "--disable", "sleep_tool", "--disable", "apps", "--disable", "plugins", "--disable", "tool_suggest", "--disable", "hooks", "--disable", "image_generation", "--disable", "goals", "--disable", "shell_snapshot", "--disable", "incremental_tools", "--disable", "unbounded_connection_retries", "-c", "agents.enabled=false", "-c", "tools.experimental_request_user_input.enabled=false", "-c", 'web_search="disabled"', "-c", "cloud.skills.enabled=false", "-c", "skills.include_instructions=false", "-c", "skills.bundled.enabled=false", "-c", "include_environment_context=false", "-c", "include_permissions_instructions=false", "-c", "include_apps_instructions=false", "-c", "include_collaboration_mode_instructions=false", "-c", "project_doc_max_bytes=0", "-c", "project_root_markers=[]", "-c", 'history.persistence="none"', "-c", "analytics.enabled=false", "-c", 'otel.metrics_exporter="none"', "-"]"##,
        ),
        allowed: crate::tool::ARGUMENTS_ALLOWED,
        doc: "Every argument the tool codex is started with for a call. {model} and {effort} are replaced once, wherever a value stands; {catalog} and {instruction} stand for the files bilbo writes for the call. The list must keep the model without tools.",
    },
    parse: crate::tool::arguments,
    show: |items| show_list(items),
};

pub const INHERIT: Key<Vec<String>> = Key {
    about: About {
        name: "derive.openai.tool.inherit",
        variable: None,
        default: Default::Toml(r#"["PATH", "TMPDIR"]"#),
        allowed: "a list of variables' names: letters, digits and _; not HOME, and none that starts with CODEX_ or OPENAI_",
        doc: "The variables of bilbo's own environment the tool codex gets. It gets no other but the ones environment gives, and CODEX_HOME and HOME, which bilbo sets.",
    },
    parse: |value| {
        let names = crate::tool::inherit(value)?;
        if names.iter().any(|name| kept_out(name)) {
            return Err(value.refused(KEPT_OUT_REFUSED));
        }
        Ok(names)
    },
    show: |items| show_list(items),
};

pub const ENVIRONMENT: Key<Vec<(String, String)>> = Key {
    about: About {
        name: "derive.openai.tool.environment",
        variable: None,
        default: Default::Toml("{}"),
        allowed: "a table of texts whose names are letters, digits and _; not HOME, and none that starts with CODEX_ or OPENAI_",
        doc: "Variables the tool codex is given beside the inherited ones.",
    },
    parse: |value| {
        let items = crate::tool::environment(value)?;
        if items.iter().any(|(name, _)| kept_out(name)) {
            return Err(value.refused(KEPT_OUT_REFUSED));
        }
        Ok(items)
    },
    show: |items| show_texts(items),
};

pub const HOME: Key<Option<PathBuf>> = Key {
    about: About {
        name: "derive.openai.tool.home",
        variable: Some("BILBO_DERIVE_OPENAI_TOOL_HOME"),
        default: Default::Toml(r#""""#),
        allowed: "an absolute path or one starting with ~/, or the empty text for bilbo's own",
        doc: "The folder the tool codex keeps its login and its state in. Empty: the folder codex beside the one the tool runs in. bilbo derive never makes it and never opens its login.",
    },
    parse: |value| match value.os_text() {
        Some(text) if text.is_empty() => Ok(None),
        _ => folder::path(value).map(Some),
    },
    show: |home| home.as_deref().map_or_else(|| quote(""), folder::show),
};

pub const SYSTEM_FILES: Key<Vec<PathBuf>> = Key {
    about: About {
        name: "derive.openai.tool.system_files",
        variable: None,
        default: Default::Toml(
            r#"["/etc/codex/config.toml", "/etc/codex/managed_config.toml", "/etc/codex/requirements.toml", "/etc/codex/skills", "/Library/Managed Preferences/com.openai.codex.plist", "/Library/Managed Preferences/*/com.openai.codex.plist"]"#,
        ),
        allowed: "a list of absolute paths; a part that is * stands for every entry there",
        doc: "The files of the machine that configure the tool codex whatever its arguments are. bilbo derive does not start the tool while one exists: read it, then take it out of this list.",
    },
    parse: |value| {
        let items = value.items().ok_or_else(|| value.refuse())?;
        let paths = items
            .iter()
            .map(folder::path)
            .collect::<Result<Vec<_>, _>>()?;
        let starred = paths.iter().any(|path| {
            path.components().any(|part| {
                let part = part.as_os_str().to_string_lossy();
                part.contains('*') && part != "*"
            })
        });
        if starred {
            return Err(value.refused("must hold * only as a whole part of a path"));
        }
        Ok(paths)
    },
    show: |paths| {
        let texts: Vec<String> = paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        show_list(&texts)
    },
};

pub const ABOUT: [&About; 7] = [
    &PROGRAM.about,
    &EFFORT.about,
    &ARGUMENTS.about,
    &INHERIT.about,
    &ENVIRONMENT.about,
    &HOME.about,
    &SYSTEM_FILES.about,
];

fn kept_out(name: &str) -> bool {
    SET.contains(&name) || KEPT_OUT.iter().any(|start| name.starts_with(start))
}

#[derive(Debug, Clone)]
pub struct Section {
    pub program: Resolved<String>,
    pub model: Resolved<String>,
    pub effort: Resolved<String>,
    pub arguments: Resolved<Vec<String>>,
    pub inherit: Resolved<Vec<String>>,
    pub environment: Resolved<Vec<(String, String)>>,
    pub home: Resolved<Option<PathBuf>>,
    pub system_files: Resolved<Vec<PathBuf>>,
}

impl Section {
    #[must_use]
    pub fn reads(&self) -> Vec<String> {
        let mut names = self.inherit.value.clone();
        for name in ["PATH", "CODEX_HOME", "HOME"] {
            if !names.iter().any(|held| held == name) {
                names.push(name.to_owned());
            }
        }
        names
    }
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        program: PROGRAM.resolve(source)?,
        model: MODEL.resolve(source)?,
        effort: EFFORT.resolve(source)?,
        arguments: ARGUMENTS.resolve(source)?,
        inherit: INHERIT.resolve(source)?,
        environment: ENVIRONMENT.resolve(source)?,
        home: HOME.resolve(source)?,
        system_files: SYSTEM_FILES.resolve(source)?,
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
        HOME.shown(&section.home),
        SYSTEM_FILES.shown(&section.system_files),
    ])
}

pub fn without_tools(arguments: &[String]) -> Result<(), String> {
    let refuse = |rest: &str| Err(format!("{WITHOUT_TOOLS}{rest}"));
    let sandbox = "it needs \"-s\" followed by \"read-only\"";
    let model = "it needs \"-m\" followed by \"{model}\"";
    let [first, body @ .., last] = arguments else {
        return refuse("it must start with \"exec\" and end with \"-\"");
    };
    if first != "exec" || last != "-" {
        return refuse("it must start with \"exec\" and end with \"-\"");
    }
    let mut flags: Vec<&str> = Vec::new();
    let mut settings: Vec<&str> = Vec::new();
    let mut features: Vec<&str> = Vec::new();
    let mut at = 0;
    while let Some(item) = body.get(at) {
        let Some((names, values)) = FLAGS
            .iter()
            .find(|(names, _)| names.contains(&item.as_str()))
        else {
            return refuse(&format!("it must not hold \"{item}\""));
        };
        let Some(given) = body.get(at + 1..=at + values) else {
            return refuse(&format!("it needs a value after \"{item}\""));
        };
        for value in given {
            match names[0] {
                "-s" if value != "read-only" => return refuse(sandbox),
                "-m" if value != "{model}" => return refuse(model),
                "-c" => {
                    let Some((name, text)) = value.split_once('=') else {
                        return refuse(&format!("\"-c\" must not set \"{value}\""));
                    };
                    let Some((_, fixed, _)) =
                        SETTINGS.iter().find(|(setting, ..)| *setting == name)
                    else {
                        return refuse(&format!("\"-c\" must not set \"{name}\""));
                    };
                    if let Some(fixed) = fixed
                        && text != *fixed
                    {
                        return refuse(&format!("it needs \"-c\" followed by '{name}={fixed}'"));
                    }
                    if misplaced(value, name) {
                        return refuse(PLACEHOLDERS);
                    }
                    settings.push(name);
                }
                "--disable" => {
                    if value.contains(CATALOG) || value.contains(INSTRUCTION) {
                        return refuse(PLACEHOLDERS);
                    }
                    if !FEATURES.iter().any(|(feature, _)| feature == value) {
                        return refuse(&format!("\"--disable\" must not name \"{value}\""));
                    }
                    features.push(value);
                }
                _ => {}
            }
        }
        flags.push(names[0]);
        at += 1 + values;
    }
    for flag in ["--ignore-user-config", "--ignore-rules", "--strict-config"] {
        if !flags.contains(&flag) {
            return refuse(&format!("it needs \"{flag}\""));
        }
    }
    if !flags.contains(&"-s") {
        return refuse(sandbox);
    }
    if !flags.contains(&"-m") {
        return refuse(model);
    }
    for (setting, fixed, required) in SETTINGS {
        if let Some(fixed) = fixed
            && required
            && !settings.contains(&setting)
        {
            return refuse(&format!("it needs \"-c\" followed by '{setting}={fixed}'"));
        }
    }
    for (feature, required) in FEATURES {
        if required && !features.contains(&feature) {
            return refuse(&format!("it needs \"--disable\" followed by \"{feature}\""));
        }
    }
    if !flags.contains(&"--json") {
        return Err(
            "derive.openai.tool.arguments needs \"--json\": bilbo reads the events it prints"
                .to_owned(),
        );
    }
    if !flags.contains(&"--ephemeral") {
        return Err("derive.openai.tool.arguments needs \"--ephemeral\": without it the tool keeps every call as a session in its home".to_owned());
    }
    Ok(())
}

const PLACEHOLDERS: &str = "\"{catalog}\" may stand only in model_catalog_json, \"{instruction}\" only in model_instructions_file";

fn misplaced(value: &str, setting: &str) -> bool {
    (value.contains(CATALOG) && setting != "model_catalog_json")
        || (value.contains(INSTRUCTION) && setting != "model_instructions_file")
}

fn unreadable(path: &Path, error: &io::Error) -> String {
    format!("derive.openai.tool.home: {}: {error}", path.display())
}

fn real(path: &Path) -> Result<PathBuf, String> {
    let mut rest = Vec::new();
    let mut front = path;
    loop {
        match fs::canonicalize(front) {
            Ok(found) => {
                return Ok(rest
                    .iter()
                    .rev()
                    .fold(found, |whole, part| whole.join(part)));
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) => {}
            Err(error) => return Err(unreadable(front, &error)),
        }
        match (front.parent(), front.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                front = parent;
            }
            _ => return Ok(path.to_path_buf()),
        }
    }
}

fn clash(one: &Path, other: &Path) -> bool {
    one.starts_with(other) || other.starts_with(one)
}

pub fn own_home(variables: &Variables) -> Result<PathBuf, String> {
    let set = |name: &str| variables(name).filter(|value| !value.is_empty());
    if let Some(home) = set("CODEX_HOME") {
        return Ok(PathBuf::from(home));
    }
    match set("HOME") {
        Some(home) => Ok(Path::new(&home).join(".codex")),
        None => Err("derive.openai.tool.home: neither CODEX_HOME nor HOME is set, so bilbo cannot tell your own codex home from this one: set HOME".to_owned()),
    }
}

pub fn home(folder: &Path, own: &Path, work: &Path) -> Result<(), String> {
    let found = |path: &Path| fs::symlink_metadata(path);
    let meta = match found(folder) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(format!(
                "derive.openai.tool.home: {} does not exist: make it, then log in with CODEX_HOME={} codex login",
                folder.display(),
                folder.display()
            ));
        }
        Err(error) => return Err(unreadable(folder, &error)),
    };
    if meta.file_type().is_symlink() {
        return Err(format!(
            "derive.openai.tool.home: {} is a link",
            folder.display()
        ));
    }
    if !meta.is_dir() {
        return Err(format!(
            "derive.openai.tool.home: {} is not a folder",
            folder.display()
        ));
    }
    child::own(folder, &meta).map_err(|sentence| format!("derive.openai.tool.home: {sentence}"))?;
    if meta.mode() & WRITABLE_BY_OTHERS != 0 {
        return Err(format!(
            "derive.openai.tool.home: {} can be written by others: make it yours alone with chmod 700 {}",
            folder.display(),
            folder.display()
        ));
    }
    let real_home = real(folder)?;
    if clash(&real_home, &real(work)?) {
        return Err(format!(
            "derive.openai.tool.home: {} and the folder the tool runs in, {}, are one folder or one is inside the other",
            folder.display(),
            work.display()
        ));
    }
    if clash(&real_home, &real(own)?) {
        return Err(format!(
            "derive.openai.tool.home: {} and your own codex home, {}, are one folder or one is inside the other: bilbo's is a folder apart",
            folder.display(),
            own.display()
        ));
    }
    for name in SETUP {
        let held = folder.join(name);
        match found(&held) {
            Ok(_) => {
                return Err(format!(
                    "derive.openai.tool.home: {} is the user's setup for codex, and bilbo's codex home holds none",
                    held.display()
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(unreadable(&held, &error)),
        }
    }
    let login = folder.join(LOGIN_FILE);
    let meta = match found(&login) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(format!(
                "derive.openai.tool.home: {} holds no login: run CODEX_HOME={} codex login",
                folder.display(),
                folder.display()
            ));
        }
        Err(error) => return Err(unreadable(&login, &error)),
    };
    if meta.file_type().is_symlink() {
        return Err(format!(
            "derive.openai.tool.home: {} is a link: the login is a file of this folder alone",
            login.display()
        ));
    }
    if !meta.is_file() {
        return Err(format!(
            "derive.openai.tool.home: {} is not a file",
            login.display()
        ));
    }
    if meta.nlink() > 1 {
        return Err(format!(
            "derive.openai.tool.home: {} has another name too: the login is a file of this folder alone",
            login.display()
        ));
    }
    child::own(&login, &meta).map_err(|sentence| format!("derive.openai.tool.home: {sentence}"))
}

pub fn resolve(program: &str, variables: &Variables) -> Result<PathBuf, String> {
    let file = if program.contains('/') {
        PathBuf::from(program)
    } else {
        let path = variables("PATH").unwrap_or_default();
        env::split_paths(&path)
            .filter(|folder| !folder.as_os_str().is_empty())
            .map(|folder| folder.join(program))
            .find(|file| file.is_file())
            .ok_or_else(|| format!("derive.openai.tool.program: {program} is not on PATH"))?
    };
    std::path::absolute(&file)
        .map_err(|error| format!("derive.openai.tool.program: {program}: {error}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    size: u64,
    seconds: i64,
    nanoseconds: i64,
}

fn identity(file: &Path) -> io::Result<Identity> {
    let meta = fs::metadata(file)?;
    Ok(Identity {
        device: meta.dev(),
        inode: meta.ino(),
        size: meta.size(),
        seconds: meta.mtime(),
        nanoseconds: meta.mtime_nsec(),
    })
}

pub fn system(files: &[PathBuf]) -> Result<(), String> {
    for file in files {
        let parts: Vec<_> = file.components().collect();
        if let Some(found) = existing(PathBuf::new(), &parts)? {
            return Err(format!(
                "derive.openai.tool: {} configures codex whatever its arguments are; read it, then take it out of derive.openai.tool.system_files to run with it",
                found.display()
            ));
        }
    }
    Ok(())
}

fn existing(front: PathBuf, rest: &[Component<'_>]) -> Result<Option<PathBuf>, String> {
    let cannot = |at: &Path, error: &io::Error| {
        format!(
            "derive.openai.tool: {} is there and cannot be read ({error}), so bilbo cannot tell whether a file there configures codex: name the files you can read in derive.openai.tool.system_files instead",
            at.display()
        )
    };
    let Some(star) = rest.iter().position(|part| part.as_os_str() == "*") else {
        let mut path = front;
        for part in rest {
            path.push(part.as_os_str());
        }
        return match fs::symlink_metadata(&path) {
            Ok(_) => Ok(Some(path)),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(cannot(&path, &error)),
        };
    };
    let mut before = front;
    for part in &rest[..star] {
        before.push(part.as_os_str());
    }
    let entries = match fs::read_dir(&before) {
        Ok(entries) => entries,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(cannot(&before, &error)),
    };
    let mut names = Vec::new();
    for entry in entries {
        names.push(entry.map_err(|error| cannot(&before, &error))?.file_name());
    }
    names.sort();
    for name in names {
        if let Some(found) = existing(before.join(name), &rest[star + 1..])? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn no_catalog(program: &str) -> String {
    format!("derive.openai.tool.program: {program} printed no catalog of its models")
}

pub fn catalog(program: &str, output: &[u8], model: &str, effort: &str) -> Result<Vec<u8>, String> {
    let none = || no_catalog(program);
    let listed: Json = serde_json::from_slice(output).map_err(|_| none())?;
    let models = listed
        .get("models")
        .and_then(Json::as_array)
        .ok_or_else(none)?;
    let Some(entry) = models
        .iter()
        .find(|entry| entry.get("slug").and_then(Json::as_str) == Some(model))
    else {
        return Err(format!(
            "derive.openai.model: codex {RELEASE} has no such model; codex debug models --bundled prints the ones it has"
        ));
    };
    let lists = entry
        .get("supported_reasoning_levels")
        .and_then(Json::as_array)
        .is_some_and(|levels| {
            levels
                .iter()
                .any(|level| level.get("effort").and_then(Json::as_str) == Some(effort))
        });
    if !lists {
        return Err(format!(
            "derive.openai.tool.effort: codex {RELEASE} lists no such effort for the model"
        ));
    }
    let mut entry = entry.clone();
    if let Some(fields) = entry.as_object_mut() {
        for (name, value) in ENTRY {
            fields.insert(
                name.to_owned(),
                serde_json::from_str(value).unwrap_or(Json::Null),
            );
        }
    }
    let mut whole = serde_json::Map::new();
    whole.insert("models".to_owned(), Json::Array(vec![entry]));
    serde_json::to_vec(&Json::Object(whole))
        .map_err(|error| format!("derive.openai.tool.program: {program}: {error}"))
}

fn standable(path: &Path) -> Result<&str, String> {
    path.to_str()
        .filter(|text| !text.chars().any(char::is_control))
        .ok_or_else(|| {
            format!(
                "derive.openai.tool: {} cannot stand in an argument: it is not UTF-8 or holds a control character",
                path.display()
            )
        })
}

fn replaced(text: &str, values: [(&str, &str); 4]) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('{') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        match values.iter().find(|(name, _)| rest.starts_with(name)) {
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

fn toml_text(path: &str) -> String {
    path.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn invocation(
    section: &Section,
    catalog: &Path,
    instruction: &Path,
) -> Result<Vec<String>, String> {
    let catalog = toml_text(standable(catalog)?);
    let instruction = toml_text(standable(instruction)?);
    let values = [
        ("{model}", section.model.value.as_str()),
        ("{effort}", section.effort.value.as_str()),
        (CATALOG, catalog.as_str()),
        (INSTRUCTION, instruction.as_str()),
    ];
    Ok(section
        .arguments
        .value
        .iter()
        .map(|argument| replaced(argument, values))
        .collect())
}

#[derive(Clone)]
struct Prepared {
    environment: Vec<(String, String)>,
    found: PathBuf,
    file: PathBuf,
    identity: Identity,
    catalog: Vec<u8>,
}

pub struct Codex {
    section: Section,
    folder: PathBuf,
    home: PathBuf,
    variables: Variables,
    timeout: Duration,
    limit: usize,
    poll: Duration,
    prepared: Option<Prepared>,
    why: Option<String>,
}

impl fmt::Debug for Codex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Codex")
            .field("program", &self.section.program.value)
            .finish_non_exhaustive()
    }
}

impl Codex {
    pub fn new(
        section: Section,
        variables: Variables,
        folder: PathBuf,
        timeout: Duration,
        limit: usize,
        poll: Duration,
    ) -> Codex {
        let home = section
            .home
            .value
            .clone()
            .unwrap_or_else(|| folder.with_file_name(HOME_FOLDER));
        Codex {
            section,
            folder,
            home,
            variables,
            timeout,
            limit,
            poll,
            prepared: None,
            why: None,
        }
    }

    fn checks(&self) -> Result<(), String> {
        without_tools(&self.section.arguments.value)?;
        let own = own_home(&self.variables)?;
        home(&self.home, &own, &self.folder)?;
        system(&self.section.system_files.value)?;
        child::folder(&self.folder)
    }

    fn asked(
        &self,
        file: &Path,
        arguments: &[&str],
        environment: &[(String, String)],
        refused: String,
        what: &str,
    ) -> Result<Vec<u8>, String> {
        let variables: Vec<(&str, &str)> = environment
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let program = &self.section.program.value;
        let run = child::run(
            file,
            arguments,
            Environment::Only(&variables),
            &self.folder,
            b"",
            Bounds {
                limit: self.limit,
                timeout: self.timeout,
                poll: self.poll,
            },
        )
        .map_err(|error| format!("derive.openai.tool.program: {program}: {error}"))?;
        match run.end {
            End::NotStarted(error) => {
                Err(format!("derive.openai.tool.program: {program}: {error}"))
            }
            End::Exited(0) => Ok(run.stdout),
            End::TimedOut => Err(format!(
                "derive.openai.tool.program: {program} did not end within derive.timeout when asked {what}"
            )),
            End::OverLimit => Err(format!(
                "derive.openai.tool.program: {program} printed more than derive.response allows for {what}: raise derive.response"
            )),
            _ => Err(refused),
        }
    }

    fn release(&self, found: &Path, environment: &[(String, String)]) -> Result<(), String> {
        let program = &self.section.program.value;
        let not_codex = format!(
            "derive.openai.tool.program: {program} is not codex {RELEASE}, the release bilbo's arguments are read from"
        );
        let release = self.asked(
            found,
            &["--version"],
            environment,
            not_codex.clone(),
            "its release",
        )?;
        if release != format!("codex-cli {RELEASE}\n").as_bytes() {
            return Err(not_codex);
        }
        Ok(())
    }

    fn stopped(&mut self, sentence: String) -> Outcome {
        self.why = Some(sentence);
        Outcome::Stopped(Cause::new(What::NotStarted))
    }
}

impl Writer for Codex {
    fn prepare(&mut self) -> Result<(), String> {
        if self.section.model.value.is_empty() {
            return Err("derive.openai.model is not set".to_owned());
        }
        let program = self.section.program.value.clone();
        let mut environment = crate::tool::inherited(
            &self.section.inherit.value,
            &self.section.environment.value,
            &self.variables,
        );
        environment.push((SET[0].to_owned(), standable(&self.home)?.to_owned()));
        environment.push((SET[1].to_owned(), standable(&self.folder)?.to_owned()));
        self.checks()?;
        let found = resolve(&program, &self.variables)?;
        let file = fs::canonicalize(&found)
            .map_err(|error| format!("derive.openai.tool.program: {program}: {error}"))?;
        let identity = identity(&file)
            .map_err(|error| format!("derive.openai.tool.program: {program}: {error}"))?;
        self.release(&found, &environment)?;
        let models = self.asked(
            &found,
            &["debug", "models", "--bundled"],
            &environment,
            no_catalog(&program),
            "its models",
        )?;
        let catalog = catalog(
            &program,
            &models,
            &self.section.model.value,
            &self.section.effort.value,
        )?;
        self.prepared = Some(Prepared {
            environment,
            found,
            file,
            identity,
            catalog,
        });
        Ok(())
    }

    fn write(&mut self, prompt: &Prompt, limits: &Limits) -> Outcome {
        self.why = None;
        let Some(prepared) = self.prepared.clone() else {
            return Outcome::Stopped(Cause::new(What::NotStarted));
        };
        if let Err(sentence) = self.checks() {
            return self.stopped(sentence);
        }
        let unchanged = fs::canonicalize(&prepared.found).is_ok_and(|file| {
            file == prepared.file && identity(&file).ok() == Some(prepared.identity)
        });
        if !unchanged {
            return self.stopped(format!(
                "derive.openai.tool.program: {} changed since bilbo asked it its release",
                prepared.found.display()
            ));
        }
        if let Err(sentence) = self.release(&prepared.found, &prepared.environment) {
            return self.stopped(sentence);
        }
        let folder = match fs::canonicalize(&self.folder) {
            Ok(folder) => folder,
            Err(error) => {
                return self.stopped(format!("{}: {error}", self.folder.display()));
            }
        };
        let catalog = folder.join(CATALOG_FILE);
        let instruction = folder.join(INSTRUCTION_FILE);
        for (path, bytes) in [
            (&instruction, prompt.instruction.as_bytes()),
            (&catalog, prepared.catalog.as_slice()),
        ] {
            if let Err(error) = fs::write(path, bytes) {
                return self.stopped(format!("{}: {error}", path.display()));
            }
        }
        let arguments = match invocation(&self.section, &catalog, &instruction) {
            Ok(arguments) => arguments,
            Err(sentence) => return self.stopped(sentence),
        };
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let variables: Vec<(&str, &str)> = prepared
            .environment
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let limit = usize::try_from(limits.response).unwrap_or(usize::MAX);
        let run = match child::run(
            &prepared.found,
            &arguments,
            Environment::Only(&variables),
            &self.folder,
            prompt.material.as_bytes(),
            Bounds {
                limit,
                timeout: limits.timeout,
                poll: limits.poll,
            },
        ) {
            Ok(run) => run,
            Err(error) => {
                let (outcome, why) = not_run(&self.section.program.value, &error);
                self.why = Some(why);
                return outcome;
            }
        };
        match run.end {
            End::NotStarted(error) => {
                self.stopped(format!("{}: {error}", self.section.program.value))
            }
            End::TimedOut | End::OverLimit if reached(&run.stdout) => {
                self.why = Some(TOOL_REACHED.to_owned());
                unknown(None)
            }
            End::TimedOut => Outcome::Busy {
                wait: None,
                cause: Cause::new(What::Timeout),
            },
            End::OverLimit => Outcome::Failed {
                cause: Cause::new(What::TooLarge),
                usage: Usage::default(),
            },
            End::Exited(code) => self.reading(&run.stdout, Some(code)),
            End::NoExitCode => self.reading(&run.stdout, None),
        }
    }

    fn model(&self) -> &str {
        &self.section.model.value
    }

    fn describe(&self) -> Vec<String> {
        let section = &self.section;
        vec![
            format!(
                "provider: {}, via {}, model {}, effort {}, program {}",
                NAME,
                Way::Tool.word(),
                section.model.value,
                section.effort.value,
                section.program.value
            ),
            format!("arguments: {}", show_list(&section.arguments.value)),
            format!("inherit: {}", show_list(&section.inherit.value)),
            format!("environment: {}", show_texts(&section.environment.value)),
            format!("home: {}", self.home.display()),
            format!(
                "system_files: {}",
                (SYSTEM_FILES.show)(&section.system_files.value)
            ),
        ]
    }

    fn why(&self) -> Option<&str> {
        self.why.as_deref()
    }
}

fn not_run(program: &str, error: &io::Error) -> (Outcome, String) {
    (
        unknown(None),
        format!("bilbo could not run {program}: {error}"),
    )
}

fn reached(output: &[u8]) -> bool {
    let mut lines = output.split(|byte| *byte == b'\n');
    lines.next_back();
    lines.any(|line| matches!(event(line), Ok(Event::Tool)))
}

impl Codex {
    fn reading(&mut self, output: &[u8], code: Option<i32>) -> Outcome {
        let (outcome, sentence) = read(output, code);
        self.why = match &outcome {
            Outcome::Stopped(Cause {
                what: What::Unauthorized,
                ..
            }) => Some(format!(
                "log in again with CODEX_HOME={} codex login",
                self.home.display()
            )),
            _ => sentence,
        };
        outcome
    }
}

#[derive(Debug)]
enum Event {
    Nothing,
    Tool,
    Message(String),
    Warning(String),
    Completed(Usage),
    Failed(String),
    Error(String),
}

fn reply(rest: &str) -> String {
    format!("{rest}: bilbo reads the reply as codex {RELEASE} prints it")
}

fn missing(path: &str, found: Option<&Json>, wanted: &str) -> String {
    match found {
        None => reply(&format!("the reply holds no \"{path}\"")),
        Some(_) => reply(&format!("the reply's \"{path}\" is not {wanted}")),
    }
}

fn text<'a>(path: &str, found: Option<&'a Json>) -> Result<&'a str, String> {
    found
        .and_then(Json::as_str)
        .ok_or_else(|| missing(path, found, "a text"))
}

fn whole(path: &str, found: Option<&Json>) -> Result<u64, String> {
    found
        .and_then(Json::as_u64)
        .ok_or_else(|| missing(path, found, "a whole number"))
}

fn event(line: &[u8]) -> Result<Event, String> {
    let line = serde_json::from_slice::<Json>(line)
        .map_err(|_| reply("the reply holds a line that is no event"))?;
    let Some(kind) = line.get("type").and_then(Json::as_str) else {
        return Err(reply("the reply holds a line that is no event"));
    };
    match kind {
        "thread.started" | "turn.started" => Ok(Event::Nothing),
        "item.started" | "item.updated" | "item.completed" => {
            let item = line.get("item");
            let unknown = || reply("the reply holds an item bilbo does not know");
            match item
                .and_then(|item| item.get("type"))
                .and_then(Json::as_str)
            {
                Some("reasoning") => Ok(Event::Nothing),
                Some(kind) if TOOLS.contains(&kind) => Ok(Event::Tool),
                Some("agent_message") if line["type"] == "item.completed" => Ok(Event::Message(
                    text("item.text", item.and_then(|item| item.get("text")))?.to_owned(),
                )),
                Some("agent_message") => Ok(Event::Nothing),
                Some("error") => Ok(Event::Warning(
                    text("item.message", item.and_then(|item| item.get("message")))?.to_owned(),
                )),
                _ => Err(unknown()),
            }
        }
        "turn.completed" => {
            let usage = line.get("usage");
            let count = |name: &str| {
                whole(
                    &format!("usage.{name}"),
                    usage.and_then(|usage| usage.get(name)),
                )
            };
            Ok(Event::Completed(Usage {
                read: Some(count("input_tokens")?),
                written: Some(count("output_tokens")?),
            }))
        }
        "turn.failed" => Ok(Event::Failed(
            text(
                "error.message",
                line.get("error").and_then(|error| error.get("message")),
            )?
            .to_owned(),
        )),
        "error" => Ok(Event::Error(
            text("message", line.get("message"))?.to_owned(),
        )),
        _ => Err(reply("the reply holds an event bilbo does not know")),
    }
}

fn reroute(message: &str) -> Option<String> {
    let rest = message.strip_prefix("model rerouted: ")?;
    let Some((_, after)) = rest.split_once(" -> ") else {
        return Some("unknown".to_owned());
    };
    let name = after.rfind(" (").map_or(after, |end| &after[..end]);
    Some(answered_model(name))
}

fn unknown(code: Option<i32>) -> Outcome {
    Outcome::Stopped(Cause {
        status: code,
        ..Cause::new(What::ServerError)
    })
}

fn sentence(message: &str, code: Option<i32>) -> Outcome {
    let starts = |starts: &[&str]| starts.iter().any(|start| message.starts_with(start));
    let cause = |what| Cause {
        status: code,
        ..Cause::new(what)
    };
    let stopped = |what| Outcome::Stopped(cause(what));
    let busy = |what| Outcome::Busy {
        wait: None,
        cause: cause(what),
    };
    if starts(&[
        "You’ve hit your usage limit",
        "Your workspace is out of credits.",
        "You hit your spend cap set",
        "Quota exceeded. Check your plan and billing details.",
    ]) {
        stopped(What::Billing)
    } else if starts(&["Your access token could not be refreshed"]) {
        stopped(What::Unauthorized)
    } else if starts(&["To use Codex with your ChatGPT plan, upgrade to Plus"]) {
        stopped(What::Forbidden)
    } else if starts(&[
        "Selected model is at capacity.",
        "Flex capacity unavailable.",
        "Policy-restricted capacity is unavailable.",
    ]) {
        busy(What::Overloaded)
    } else if starts(&["We’re currently experiencing high demand"]) {
        busy(What::ServerError)
    } else if starts(&["rate limit exceeded: "]) {
        busy(What::RateLimit)
    } else if starts(&["request timed out"]) {
        busy(What::Timeout)
    } else if starts(&["Connection failed: "]) {
        busy(What::NoConnection)
    } else if message.contains("Incomplete response returned, reason: content_filter") {
        Outcome::Refused {
            category: Some("content_filter".to_owned()),
            usage: Usage::default(),
        }
    } else if starts(&["Incomplete response returned, reason: max_output_tokens"]) {
        Outcome::Cut(Usage::default())
    } else if let Some(reason) = message.strip_prefix("Incomplete response returned, reason: ") {
        Outcome::Failed {
            cause: Cause {
                code: Some(identifier(reason)),
                ..cause(What::StopReason)
            },
            usage: Usage::default(),
        }
    } else if starts(&[
        "stream disconnected before completion: ",
        "Error while reading the server response: ",
    ]) {
        busy(What::ServerError)
    } else if starts(&["Codex ran out of room in the model's context window."]) {
        stopped(What::Rejected)
    } else if let Some(digits) = ["unexpected status ", "exceeded retry limit, last status: "]
        .iter()
        .find_map(|start| message.strip_prefix(start))
        .map(|rest| &rest[..rest.bytes().take_while(u8::is_ascii_digit).count()])
        .filter(|digits| !digits.is_empty())
    {
        let cause = |what| Cause {
            code: Some(identifier(digits)),
            ..cause(what)
        };
        let stopped = |what| Outcome::Stopped(cause(what));
        let busy = |what| Outcome::Busy {
            wait: None,
            cause: cause(what),
        };
        match digits.parse::<u32>() {
            Ok(401) => stopped(What::Unauthorized),
            Ok(403) => stopped(What::Forbidden),
            Ok(404) => stopped(What::NotFound),
            Ok(408) => busy(What::Timeout),
            Ok(429) => busy(What::RateLimit),
            Ok(503) => busy(What::Overloaded),
            Ok(409 | 500..=502 | 504..) => busy(What::ServerError),
            Ok(400..=499) => stopped(What::Rejected),
            _ => unknown(code),
        }
    } else {
        unknown(code)
    }
}

pub fn read(output: &[u8], code: Option<i32>) -> (Outcome, Option<String>) {
    let mut tool = false;
    let mut unreadable: Option<String> = None;
    let mut message: Option<String> = None;
    let mut model: Option<String> = None;
    let mut completed: Option<Usage> = None;
    let mut failed: Option<String> = None;
    let mut error: Option<String> = None;
    for line in output.split(|byte| *byte == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match event(line) {
            Err(sentence) => {
                unreadable.get_or_insert(sentence);
            }
            Ok(Event::Nothing) => {}
            Ok(Event::Tool) => tool = true,
            Ok(Event::Message(text)) => message = Some(text),
            Ok(Event::Warning(text)) => {
                if let Some(answered) = reroute(&text) {
                    model = Some(answered);
                }
            }
            Ok(Event::Completed(usage)) => completed = Some(usage),
            Ok(Event::Failed(text)) => failed = Some(text),
            Ok(Event::Error(text)) if !text.starts_with(RETRY_NOTICE) => error = Some(text),
            Ok(Event::Error(_)) => {}
        }
    }
    let ended = code == Some(0);
    let says = |why: String| (unknown(code), Some(why));
    if tool {
        return says(TOOL_REACHED.to_owned());
    }
    if let Some(why) = unreadable {
        return says(why);
    }
    if let (Some(usage), true) = (completed, ended) {
        return match message {
            Some(text) => (Outcome::Answer { text, usage, model }, None),
            None => says(reply("the reply holds no answer")),
        };
    }
    if let Some(why) = failed.or(error.filter(|_| !ended)) {
        return (sentence(&why, code), None);
    }
    if completed.is_some() {
        return (unknown(code), None);
    }
    if ended {
        return says(reply("the reply holds no end of a turn"));
    }
    let exit = code.map_or_else(
        || "an unknown exit".to_owned(),
        |code| format!("exit {code}"),
    );
    says(format!(
        "codex ended with {exit} and printed no end of a turn"
    ))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use bilbo_settings::error::Error;
    use bilbo_settings::file::File;
    use bilbo_settings::key::Source;

    use super::{
        ABOUT, Codex, FEATURES, KEPT_OUT, SETTINGS, Section, catalog, invocation, not_run,
        own_home, reached, read, section, shown, without_tools,
    };
    use crate::writer::{Cause, Outcome, Usage, Variables, What, Writer};

    const MODELS: &str = include_str!("../../fixtures/openai/tool/models.json");
    const ANSWER: &str = include_str!("../../fixtures/openai/tool/answer.jsonl");
    const ANSWER_REASONING: &str =
        include_str!("../../fixtures/openai/tool/answer-reasoning.jsonl");
    const TWO_MESSAGES: &str = include_str!("../../fixtures/openai/tool/two-messages.jsonl");
    const REROUTED: &str = include_str!("../../fixtures/openai/tool/rerouted.jsonl");
    const WARNING: &str = include_str!("../../fixtures/openai/tool/warning.jsonl");
    const TOOL_COMMAND: &str = include_str!("../../fixtures/openai/tool/tool-command.jsonl");
    const NO_MESSAGE: &str = include_str!("../../fixtures/openai/tool/no-message.jsonl");
    const FAILED_LIMIT: &str = include_str!("../../fixtures/openai/tool/failed-limit.jsonl");
    const FAILED_401: &str = include_str!("../../fixtures/openai/tool/failed-401.jsonl");
    const FAILED_OTHER: &str = include_str!("../../fixtures/openai/tool/failed-other.jsonl");
    const REST: &str = "derive.openai.tool.arguments must keep the model without tools: ";
    const READS: &str = "bilbo reads the reply as codex 0.169.0 prints it";
    const CAPACITY: &str = "Selected model is at capacity. Please try a different model.";

    fn resolve_with(text: &str, variables: &[(&str, &str)]) -> Result<Section, Error> {
        let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
        let given = |name: &str| {
            variables
                .iter()
                .find(|(asked, _)| *asked == name)
                .map(|(_, value)| OsString::from(value))
        };
        section(&Source {
            var: &given,
            file: &file,
        })
    }

    fn resolve(text: &str) -> Result<Section, Error> {
        resolve_with(text, &[])
    }

    fn said(text: &str) -> String {
        resolve(text).expect_err("refused").to_string()
    }

    fn with(key: &str, value: &str) -> String {
        format!("[derive.openai.tool]\n{key} = {value}\n")
    }

    fn items(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| (*item).to_owned()).collect()
    }

    fn default_arguments() -> Vec<String> {
        resolve("").expect("defaults").arguments.value
    }

    fn edit(change: &dyn Fn(&mut Vec<String>)) -> Result<(), String> {
        let mut list = default_arguments();
        change(&mut list);
        without_tools(&list)
    }

    fn refused(rest: &str) -> Result<(), String> {
        Err(format!("{REST}{rest}"))
    }

    fn at(list: &[String], item: &str) -> usize {
        list.iter().position(|found| found == item).expect("item")
    }

    fn without_pair(list: &mut Vec<String>, flag: &str, value: &str) {
        let at = list
            .windows(2)
            .position(|pair| pair[0] == flag && pair[1].starts_with(value))
            .expect("pair");
        list.drain(at..=at + 1);
    }

    fn before_last(list: &mut Vec<String>, more: &[&str]) {
        let last = list.len() - 1;
        for (offset, item) in more.iter().enumerate() {
            list.insert(last + offset, (*item).to_owned());
        }
    }

    fn lines(base: &str) -> Vec<String> {
        base.lines().map(str::to_owned).collect()
    }

    fn put(base: &str, at: usize, line: &str) -> Vec<u8> {
        let mut all = lines(base);
        all.insert(at, line.to_owned());
        all.join("\n").into_bytes()
    }

    fn swap(base: &str, at: usize, line: &str) -> Vec<u8> {
        let mut all = lines(base);
        all[at] = line.to_owned();
        all.join("\n").into_bytes()
    }

    fn unknown(status: Option<i32>) -> Outcome {
        Outcome::Stopped(Cause {
            status,
            ..Cause::new(What::ServerError)
        })
    }

    fn turn(message: &str) -> Vec<u8> {
        let message = serde_json::to_string(message).expect("a text");
        format!(
            "{{\"type\":\"thread.started\",\"thread_id\":\"t\"}}\n{{\"type\":\"turn.started\"}}\n{{\"type\":\"error\",\"message\":{message}}}\n{{\"type\":\"turn.failed\",\"error\":{{\"message\":{message}}}}}\n"
        )
        .into_bytes()
    }

    fn adapter(text: &str) -> Codex {
        Codex::new(
            resolve(text).expect("section"),
            Box::new(|_| None),
            PathBuf::from("/s/derive/work"),
            Duration::from_secs(1),
            1024,
            Duration::from_millis(5),
        )
    }

    #[test]
    fn each_default_is_its_documented_value() {
        let section = resolve("").expect("defaults");
        assert_eq!(section.program.value, "codex");
        assert_eq!(section.model.value, "");
        assert_eq!(section.effort.value, "medium");
        assert_eq!(
            section.arguments.value,
            items(&[
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
                "-"
            ])
        );
        assert_eq!(section.inherit.value, items(&["PATH", "TMPDIR"]));
        assert!(section.environment.value.is_empty());
        assert_eq!(section.home.value, None);
        assert_eq!(
            section.system_files.value,
            [
                "/etc/codex/config.toml",
                "/etc/codex/managed_config.toml",
                "/etc/codex/requirements.toml",
                "/etc/codex/skills",
                "/Library/Managed Preferences/com.openai.codex.plist",
                "/Library/Managed Preferences/*/com.openai.codex.plist",
            ]
            .map(PathBuf::from)
        );
    }

    #[test]
    fn the_program_the_effort_and_the_arguments_are_read_as_claudes_are() {
        for (key, value) in [
            ("program", "\"\""),
            ("program", "5"),
            ("effort", "\"High\""),
            ("effort", "\"\""),
            ("arguments", "\"exec\""),
            ("arguments", "[1]"),
        ] {
            let error = resolve(&with(key, value)).expect_err("refused").to_string();
            assert!(error.starts_with("/c/config.toml:2: "), "{key} {error}");
        }
        assert_eq!(
            said(&with("effort", "\"High\"")),
            "/c/config.toml:2: derive.openai.tool.effort must be a word of lower-case letters, got \"High\""
        );
        let taken = resolve(&with("arguments", "[\"\", \"x\"]")).expect("taken");
        assert_eq!(taken.arguments.value, items(&["", "x"]));
    }

    #[test]
    fn a_variable_that_changes_the_tools_home_login_or_address_is_refused() {
        let inherit = "/c/config.toml:2: derive.openai.tool.inherit must not name HOME or a variable that starts with CODEX_ or OPENAI_: bilbo sets the tool's home, and such a variable can change its login or where it sends a request";
        let environment = inherit.replace("tool.inherit", "tool.environment");
        for list in [
            "[\"HOME\"]",
            "[\"PATH\", \"CODEX_HOME\"]",
            "[\"CODEX_API_KEY\"]",
            "[\"CODEX_REFRESH_TOKEN_URL_OVERRIDE\"]",
            "[\"CODEX_EXEC_SERVER_URL\"]",
            "[\"OPENAI_API_KEY\"]",
        ] {
            assert_eq!(said(&with("inherit", list)), inherit, "{list}");
        }
        for table in [
            "{ HOME = \"/x\" }",
            "{ CODEX_ACCESS_TOKEN = \"x\" }",
            "{ OPENAI_ORGANIZATION = \"x\" }",
        ] {
            assert_eq!(said(&with("environment", table)), environment, "{table}");
        }
        let taken = resolve(
            "[derive.openai.tool]\ninherit = [\"HTTPS_PROXY\", \"NO_PROXY\", \"A_B\", \"XCODEX_A\", \"MY_OPENAI_KEY\", \"codex_home\"]\nenvironment = { SSL_CERT_FILE = \"/x\" }\n",
        )
        .expect("taken");
        assert_eq!(taken.inherit.value.len(), 6);
        assert_eq!(taken.environment.value.len(), 1);
        assert_eq!(
            said(&with("inherit", "[\"A-B\"]")),
            "/c/config.toml:2: derive.openai.tool.inherit must hold variables' names alone: letters, digits and _"
        );
        assert_eq!(
            said(&with("environment", "{ \"A-B\" = \"1\" }")),
            "/c/config.toml:2: derive.openai.tool.environment must name its variables with letters, digits and _"
        );
        assert_eq!(KEPT_OUT, ["CODEX_", "OPENAI_"]);
    }

    #[test]
    fn the_home_is_a_path_or_empty_for_bilbos_own() {
        let none = resolve(&with("home", "\"\"")).expect("empty");
        assert_eq!(none.home.value, None);
        let listed = shown(&Source {
            var: &|_| None,
            file: &File::parse(Path::new("/c/config.toml"), b"").expect("toml"),
        })
        .expect("shown");
        assert_eq!(listed[5].value, "\"\"");
        let given = resolve(&with("home", "\"/a/b\"")).expect("path");
        assert_eq!(given.home.value, Some(PathBuf::from("/a/b")));
        let tilde = resolve_with(&with("home", "\"~/x\""), &[("HOME", "/h")]).expect("tilde");
        assert_eq!(tilde.home.value, Some(PathBuf::from("/h/x")));
        let variable = resolve_with(
            &with("home", "\"/a/b\""),
            &[("BILBO_DERIVE_OPENAI_TOOL_HOME", "/v")],
        )
        .expect("variable");
        assert_eq!(variable.home.value, Some(PathBuf::from("/v")));
        assert_eq!(
            said(&with("home", "\"a/b\"")),
            "/c/config.toml:2: derive.openai.tool.home is not an absolute path: a/b"
        );
        assert_eq!(
            said(&with("home", "5")),
            "/c/config.toml:2: derive.openai.tool.home must be an absolute path or one starting with ~/, or the empty text for bilbo's own, got 5"
        );
    }

    #[test]
    fn the_machines_files_are_absolute_paths_with_whole_stars() {
        assert_eq!(
            said(&with("system_files", "[\"etc/x\"]")),
            "/c/config.toml:2: derive.openai.tool.system_files is not an absolute path: etc/x"
        );
        assert_eq!(
            said(&with("system_files", "[\"/a/x*/b\"]")),
            "/c/config.toml:2: derive.openai.tool.system_files must hold * only as a whole part of a path"
        );
        assert!(resolve(&with("system_files", "[\"/a/*/b\", \"/c\"]")).is_ok());
        assert!(resolve(&with("system_files", "[\"/etc/codex/*\"]")).is_ok());
        let empty = resolve(&with("system_files", "[]")).expect("empty");
        assert!(empty.system_files.value.is_empty());
        let error = said(&with("system_files", "\"/a\""));
        assert!(error.starts_with("/c/config.toml:2: "), "{error}");
    }

    #[test]
    fn the_keys_show_as_toml_that_reads_back_and_the_way_lists_seven() {
        let names: Vec<_> = ABOUT.iter().map(|about| about.name).collect();
        assert_eq!(
            names,
            [
                "derive.openai.tool.program",
                "derive.openai.tool.effort",
                "derive.openai.tool.arguments",
                "derive.openai.tool.inherit",
                "derive.openai.tool.environment",
                "derive.openai.tool.home",
                "derive.openai.tool.system_files",
            ]
        );
        let text = "[derive.openai.tool]\nprogram = \"/bin/c\"\neffort = \"low\"\narguments = [\"exec\", \"-c\", 'a=\"b\"', \"-\"]\ninherit = [\"PATH\"]\nhome = \"/h/c\"\nsystem_files = [\"/a/*/b\"]\n\n[derive.openai.tool.environment]\nA = \"1\"\n";
        let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
        let first = shown(&Source {
            var: &|_| None,
            file: &file,
        })
        .expect("shown");
        assert_eq!(first.len(), 7);
        assert_eq!(
            first.iter().map(|item| item.name).collect::<Vec<_>>(),
            names
        );
        let again: String = first
            .iter()
            .map(|item| format!("{} = {}\n", item.name, item.value))
            .collect();
        assert!(again.contains(
            "derive.openai.tool.arguments = [\"exec\", \"-c\", \"a=\\\"b\\\"\", \"-\"]\n"
        ));
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
        assert_eq!(first[5].value, "\"/h/c\"");
        assert_eq!(first[6].value, "[\"/a/*/b\"]");
    }

    #[test]
    fn the_default_arguments_keep_the_model_without_tools() {
        assert_eq!(without_tools(&default_arguments()), Ok(()));
    }

    #[test]
    fn each_item_that_removes_a_tool_or_the_users_setup_must_stand() {
        for flag in ["--ignore-user-config", "--ignore-rules", "--strict-config"] {
            assert_eq!(
                edit(&|list| list.retain(|item| item != flag)),
                refused(&format!("it needs \"{flag}\"")),
                "{flag}"
            );
        }
        assert_eq!(
            edit(&|list| without_pair(list, "-s", "read-only")),
            refused("it needs \"-s\" followed by \"read-only\"")
        );
        assert_eq!(
            edit(&|list| without_pair(list, "-m", "{model}")),
            refused("it needs \"-m\" followed by \"{model}\"")
        );
        let required = [
            ("model_catalog_json", "\"{catalog}\""),
            ("model_instructions_file", "\"{instruction}\""),
            ("model_reasoning_effort", "\"{effort}\""),
            ("agents.enabled", "false"),
            ("tools.experimental_request_user_input.enabled", "false"),
            ("web_search", "\"disabled\""),
            ("cloud.skills.enabled", "false"),
            ("skills.include_instructions", "false"),
            ("skills.bundled.enabled", "false"),
            ("include_environment_context", "false"),
            ("include_permissions_instructions", "false"),
            ("include_apps_instructions", "false"),
            ("include_collaboration_mode_instructions", "false"),
            ("project_doc_max_bytes", "0"),
            ("project_root_markers", "[]"),
        ];
        for (setting, value) in required {
            assert_eq!(
                edit(&|list| without_pair(list, "-c", &format!("{setting}="))),
                refused(&format!("it needs \"-c\" followed by '{setting}={value}'")),
                "{setting}"
            );
        }
        for feature in [
            "shell_tool",
            "unified_exec",
            "view_image",
            "sleep_tool",
            "apps",
            "plugins",
            "tool_suggest",
            "hooks",
            "image_generation",
            "goals",
        ] {
            assert_eq!(
                edit(&|list| without_pair(list, "--disable", feature)),
                refused(&format!("it needs \"--disable\" followed by \"{feature}\"")),
                "{feature}"
            );
        }
        assert_eq!(
            edit(&|list| without_pair(list, "--disable", "shell_snapshot")),
            Ok(())
        );
        assert_eq!(
            edit(&|list| without_pair(list, "-c", "history.persistence=")),
            Ok(())
        );
        assert_eq!(
            edit(&|list| list.retain(|item| item != "--skip-git-repo-check")),
            Ok(())
        );
        assert_eq!(
            edit(&|list| list.retain(|item| item != "--strict-config" && item != "--ignore-rules")),
            refused("it needs \"--ignore-rules\"")
        );
        assert_eq!(
            edit(&|list| {
                without_pair(list, "--disable", "goals");
                without_pair(list, "-c", "web_search=");
            }),
            refused("it needs \"-c\" followed by 'web_search=\"disabled\"'")
        );
        assert_eq!(
            SETTINGS.iter().filter(|(_, _, required)| *required).count(),
            15
        );
        assert_eq!(
            FEATURES.iter().filter(|(_, required)| *required).count(),
            10
        );
    }

    #[test]
    fn a_value_that_differs_from_the_one_bilbo_needs_is_refused() {
        let replaced = |old: &str, new: &str| {
            edit(&|list| {
                let i = at(list, old);
                list[i] = new.to_owned();
            })
        };
        assert_eq!(
            replaced("web_search=\"disabled\"", "web_search=\"live\""),
            refused("it needs \"-c\" followed by 'web_search=\"disabled\"'")
        );
        assert_eq!(
            replaced("agents.enabled=false", "agents.enabled=true"),
            refused("it needs \"-c\" followed by 'agents.enabled=false'")
        );
        assert_eq!(
            replaced("read-only", "workspace-write"),
            refused("it needs \"-s\" followed by \"read-only\"")
        );
        assert_eq!(
            replaced("{model}", "model-x"),
            refused("it needs \"-m\" followed by \"{model}\"")
        );
        assert_eq!(
            replaced(
                "history.persistence=\"none\"",
                "history.persistence=\"save-all\""
            ),
            refused("it needs \"-c\" followed by 'history.persistence=\"none\"'")
        );
        assert_eq!(
            replaced("project_root_markers=[]", "project_root_markers=[\".git\"]"),
            refused("it needs \"-c\" followed by 'project_root_markers=[]'")
        );
        assert_eq!(
            edit(&|list| before_last(list, &["-c", "web_search=\"live\""])),
            refused("it needs \"-c\" followed by 'web_search=\"disabled\"'")
        );
    }

    #[test]
    fn an_item_bilbo_does_not_know_is_refused_by_name() {
        for item in [
            "--enable",
            "--dangerously-bypass-approvals-and-sandbox",
            "-C",
            "--add-dir",
            "-p",
            "--oss",
            "-i",
            "--output-schema",
            "--config=a=b",
            "--sandbox=read-only",
            "stray",
        ] {
            assert_eq!(
                edit(&|list| before_last(list, &[item])),
                refused(&format!("it must not hold \"{item}\"")),
                "{item}"
            );
        }
        for (value, setting) in [
            ("mcp_servers.x.command=\"y\"", "mcp_servers.x.command"),
            ("developer_instructions=\"x\"", "developer_instructions"),
            (
                "openai_base_url=\"http://127.0.0.1:1/v1\"",
                "openai_base_url",
            ),
            ("features.shell_tool=true", "features.shell_tool"),
            ("model_catalog_json", "model_catalog_json"),
        ] {
            assert_eq!(
                edit(&|list| before_last(list, &["-c", value])),
                refused(&format!("\"-c\" must not set \"{setting}\"")),
                "{value}"
            );
        }
        assert_eq!(
            edit(&|list| before_last(list, &["--disable", "no_such"])),
            refused("\"--disable\" must not name \"no_such\"")
        );
        for (flag, value, rest) in [
            (
                "--config",
                "features.shell_tool=true",
                "\"-c\" must not set \"features.shell_tool\"",
            ),
            (
                "--sandbox",
                "workspace-write",
                "it needs \"-s\" followed by \"read-only\"",
            ),
            ("--model", "x", "it needs \"-m\" followed by \"{model}\""),
        ] {
            assert_eq!(
                edit(&|list| before_last(list, &[flag, value])),
                refused(rest),
                "{flag}"
            );
        }
        assert_eq!(
            without_tools(&items(&["exec", "-c", "-"])),
            refused("it needs a value after \"-c\"")
        );
        assert_eq!(
            without_tools(&items(&["exec", "-m", "-"])),
            refused("it needs a value after \"-m\"")
        );
        let edges = refused("it must start with \"exec\" and end with \"-\"");
        assert_eq!(without_tools(&items(&["exec", "--json"])), edges);
        assert_eq!(without_tools(&items(&["--json", "-"])), edges);
    }

    #[test]
    fn a_free_setting_takes_any_value_and_the_effort_is_the_keys() {
        for value in [
            "model_verbosity=\"high\"",
            "service_tier=\"flex\"",
            "model_reasoning_summary=\"none\"",
        ] {
            assert_eq!(
                edit(&|list| before_last(list, &["-c", value])),
                Ok(()),
                "{value}"
            );
        }
        assert_eq!(
            edit(&|list| {
                let i = at(list, "model_reasoning_effort=\"{effort}\"");
                list[i] = "model_reasoning_effort=\"high\"".to_owned();
            }),
            refused("it needs \"-c\" followed by 'model_reasoning_effort=\"{effort}\"'")
        );
    }

    #[test]
    fn the_events_and_the_session_have_sentences_of_their_own() {
        let json =
            "derive.openai.tool.arguments needs \"--json\": bilbo reads the events it prints";
        let ephemeral = "derive.openai.tool.arguments needs \"--ephemeral\": without it the tool keeps every call as a session in its home";
        assert_eq!(
            edit(&|list| list.retain(|item| item != "--json")),
            Err(json.to_owned())
        );
        assert_eq!(
            edit(&|list| list.retain(|item| item != "--ephemeral")),
            Err(ephemeral.to_owned())
        );
        assert_eq!(
            edit(&|list| list.retain(|item| item != "--json" && item != "--ephemeral")),
            Err(json.to_owned())
        );
    }

    #[test]
    fn a_files_placeholder_stands_only_in_its_own_setting() {
        let only = "\"{catalog}\" may stand only in model_catalog_json, \"{instruction}\" only in model_instructions_file";
        assert_eq!(
            edit(&|list| {
                let i = at(list, "{model}");
                list[i] = "{catalog}".to_owned();
            }),
            refused("it needs \"-m\" followed by \"{model}\"")
        );
        assert_eq!(
            edit(&|list| before_last(list, &["-c", "model_verbosity=\"{instruction}\""])),
            refused(only)
        );
        assert_eq!(
            edit(&|list| before_last(list, &["-c", "service_tier=\"{catalog}\""])),
            refused(only)
        );
        assert_eq!(
            edit(&|list| before_last(list, &["--disable", "{catalog}"])),
            refused(only)
        );
    }

    #[test]
    fn each_placeholder_is_replaced_once_wherever_it_stands() {
        let section =
            resolve("derive.openai.model = \"m-1\"\n\n[derive.openai.tool]\neffort = \"high\"\n")
                .expect("section");
        let made = invocation(
            &section,
            Path::new("/w/catalog.json"),
            Path::new("/w/instruction.txt"),
        )
        .expect("arguments");
        let mut wanted = default_arguments();
        for (old, new) in [
            ("{model}", "m-1"),
            (
                "model_catalog_json=\"{catalog}\"",
                "model_catalog_json=\"/w/catalog.json\"",
            ),
            (
                "model_instructions_file=\"{instruction}\"",
                "model_instructions_file=\"/w/instruction.txt\"",
            ),
            (
                "model_reasoning_effort=\"{effort}\"",
                "model_reasoning_effort=\"high\"",
            ),
        ] {
            let i = at(&wanted, old);
            wanted[i] = new.to_owned();
        }
        assert_eq!(made, wanted);
        let braced = resolve("derive.openai.model = \"{effort}\"\n").expect("section");
        let made = invocation(&braced, Path::new("/w/c"), Path::new("/w/i")).expect("arguments");
        assert_eq!(made[at(&default_arguments(), "{model}")], "{effort}");
    }

    #[test]
    fn a_path_stands_in_an_argument_as_toml_text() {
        let section = resolve("derive.openai.model = \"m\"\n").expect("section");
        let made = |catalog: &str| {
            invocation(
                &section,
                Path::new(catalog),
                Path::new("/w/instruction.txt"),
            )
        };
        let catalog_argument = |made: Vec<String>| made[at(&default_arguments(), "-c") + 1].clone();
        assert_eq!(
            catalog_argument(made("/w/a\"b/catalog.json").expect("arguments")),
            "model_catalog_json=\"/w/a\\\"b/catalog.json\""
        );
        assert_eq!(
            catalog_argument(made("/w/a\\b/c.json").expect("arguments")),
            "model_catalog_json=\"/w/a\\\\b/c.json\""
        );
        assert_eq!(
            made("/w/a\nb/catalog.json").expect_err("refused"),
            "derive.openai.tool: /w/a\nb/catalog.json cannot stand in an argument: it is not UTF-8 or holds a control character"
        );
    }

    #[test]
    fn the_catalog_is_the_models_entry_with_four_fields_set() {
        let made = catalog("codex", MODELS.as_bytes(), "model-a", "medium").expect("catalog");
        let parsed: serde_json::Value = serde_json::from_slice(&made).expect("json");
        let models = parsed["models"].as_array().expect("models");
        assert_eq!(models.len(), 1);
        let entry = models[0].as_object().expect("entry");
        assert_eq!(entry["slug"], "model-a");
        assert_eq!(entry["tool_mode"], "direct");
        assert!(entry["apply_patch_tool_type"].is_null());
        assert_eq!(entry["experimental_supported_tools"], serde_json::json!([]));
        assert_eq!(entry["supports_search_tool"], false);
        let shipped: serde_json::Value = serde_json::from_str(MODELS).expect("fixture");
        let original = shipped["models"][0].as_object().expect("entry");
        assert_eq!(
            entry.keys().collect::<Vec<_>>(),
            original.keys().collect::<Vec<_>>()
        );
        for (name, value) in original {
            if ![
                "tool_mode",
                "apply_patch_tool_type",
                "experimental_supported_tools",
                "supports_search_tool",
            ]
            .contains(&name.as_str())
            {
                assert_eq!(&entry[name], value, "{name}");
            }
        }
        assert_eq!(entry["use_responses_lite"], true);
        assert_eq!(entry["zz_kept"], serde_json::json!({"a": 2, "b": 1}));
        assert_eq!(
            made,
            catalog("codex", MODELS.as_bytes(), "model-a", "medium").expect("again")
        );
        let names: Vec<_> = entry.keys().collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        let text = String::from_utf8(made).expect("text");
        assert!(text.find("\"a\":2").expect("a") < text.find("\"b\":1").expect("b"));
    }

    #[test]
    fn a_model_or_an_effort_the_catalog_lacks_is_refused_in_bilbos_words() {
        let model = "derive.openai.model: codex 0.169.0 has no such model; codex debug models --bundled prints the ones it has";
        let effort = "derive.openai.tool.effort: codex 0.169.0 lists no such effort for the model";
        let none = "derive.openai.tool.program: codex printed no catalog of its models";
        assert_eq!(
            catalog("codex", MODELS.as_bytes(), "model-z", "medium").expect_err("no model"),
            model
        );
        assert_eq!(
            catalog("codex", MODELS.as_bytes(), "model-b", "medium").expect_err("no effort"),
            effort
        );
        assert_eq!(
            catalog("codex", MODELS.as_bytes(), "model-a", "ultra").expect_err("no effort"),
            effort
        );
        for said in [model, effort] {
            for word in ["model-z", "model-a", "ultra"] {
                assert!(!said.contains(word), "{said}");
            }
        }
        for output in ["not json", "[]", "{}", "{\"models\": 5}"] {
            assert_eq!(
                catalog("codex", output.as_bytes(), "model-a", "medium").expect_err("none"),
                none,
                "{output}"
            );
        }
        let bare = "{\"models\":[{\"slug\":\"model-a\"}]}";
        assert_eq!(
            catalog("codex", bare.as_bytes(), "model-a", "medium").expect_err("no levels"),
            effort
        );
    }

    #[test]
    fn an_answer_is_the_last_message_with_the_turns_counts() {
        let document = "#r1-r2 The user asked for the carrot rows to be thinned.";
        for reply in [ANSWER, ANSWER_REASONING] {
            let (outcome, why) = read(reply.as_bytes(), Some(0));
            assert_eq!(
                outcome,
                Outcome::Answer {
                    text: document.to_owned(),
                    usage: Usage {
                        read: Some(1200),
                        written: Some(80)
                    },
                    model: None
                }
            );
            assert_eq!(why, None);
        }
        let (outcome, _) = read(TWO_MESSAGES.as_bytes(), Some(0));
        assert!(
            matches!(&outcome, Outcome::Answer { text, .. } if text == document),
            "{outcome:?}"
        );
    }

    #[test]
    fn a_rerouted_answer_names_the_model_that_answered() {
        let model = |reply: &[u8]| match read(reply, Some(0)).0 {
            Outcome::Answer { model, .. } => model,
            other => panic!("{other:?}"),
        };
        assert_eq!(model(REROUTED.as_bytes()), Some("model-b".to_owned()));
        assert_eq!(
            model(&swap(
                REROUTED,
                2,
                "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_0\",\"type\":\"error\",\"message\":\"model rerouted: gone\"}}"
            )),
            Some("unknown".to_owned())
        );
        assert_eq!(model(WARNING.as_bytes()), None);
        assert_eq!(read(WARNING.as_bytes(), Some(0)).1, None);
    }

    #[test]
    fn an_item_of_a_tool_ends_the_run_and_says_so() {
        let tool =
            "codex reached a tool of its own: bilbo's arguments did not keep it without tools";
        assert_eq!(
            read(TOOL_COMMAND.as_bytes(), Some(0)),
            (unknown(Some(0)), Some(tool.to_owned()))
        );
        for kind in [
            "command_execution",
            "file_change",
            "mcp_tool_call",
            "collab_tool_call",
            "web_search",
            "todo_list",
        ] {
            for stage in ["item.started", "item.updated", "item.completed"] {
                let line = format!(
                    "{{\"type\":\"{stage}\",\"item\":{{\"id\":\"item_0\",\"type\":\"{kind}\"}}}}"
                );
                assert_eq!(
                    read(&put(ANSWER, 2, &line), Some(0)),
                    (unknown(Some(0)), Some(tool.to_owned())),
                    "{stage} {kind}"
                );
            }
        }
        assert_eq!(read(FAILED_OTHER.as_bytes(), Some(1)).0, unknown(Some(1)));
        let mixed = put(
            FAILED_OTHER,
            2,
            "{\"type\":\"item.started\",\"item\":{\"id\":\"item_0\",\"type\":\"web_search\"}}",
        );
        assert_eq!(
            read(&mixed, Some(1)),
            (unknown(Some(1)), Some(tool.to_owned()))
        );
    }

    #[test]
    fn a_line_or_a_field_bilbo_cannot_read_stops_the_run_and_says_which() {
        let reply = |rest: &str| format!("{rest}: {READS}");
        let cases: Vec<(Vec<u8>, String)> = vec![
            (
                put(ANSWER, 2, "oops"),
                reply("the reply holds a line that is no event"),
            ),
            (
                put(ANSWER, 2, "[1]"),
                reply("the reply holds a line that is no event"),
            ),
            (
                put(ANSWER, 2, "{\"no\":\"type\"}"),
                reply("the reply holds a line that is no event"),
            ),
            (
                put(ANSWER, 2, "{\"type\":\"thread.resumed\"}"),
                reply("the reply holds an event bilbo does not know"),
            ),
            (
                put(
                    ANSWER,
                    2,
                    "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_0\",\"type\":\"image_view\"}}",
                ),
                reply("the reply holds an item bilbo does not know"),
            ),
            (
                put(
                    ANSWER,
                    2,
                    "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_0\"}}",
                ),
                reply("the reply holds an item bilbo does not know"),
            ),
            (
                swap(
                    ANSWER,
                    2,
                    "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_0\",\"type\":\"agent_message\"}}",
                ),
                reply("the reply holds no \"item.text\""),
            ),
            (
                swap(
                    ANSWER,
                    2,
                    "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_0\",\"type\":\"agent_message\",\"text\":5}}",
                ),
                reply("the reply's \"item.text\" is not a text"),
            ),
            (
                swap(
                    ANSWER,
                    3,
                    "{\"type\":\"turn.completed\",\"usage\":{\"output_tokens\":80}}",
                ),
                reply("the reply holds no \"usage.input_tokens\""),
            ),
            (
                swap(
                    ANSWER,
                    3,
                    "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1200,\"output_tokens\":\"80\"}}",
                ),
                reply("the reply's \"usage.output_tokens\" is not a whole number"),
            ),
        ];
        for (bytes, why) in cases {
            assert_eq!(
                read(&bytes, Some(0)),
                (unknown(Some(0)), Some(why)),
                "{}",
                String::from_utf8_lossy(&bytes)
            );
        }
        let both = put(
            &String::from_utf8(put(ANSWER, 2, "oops")).expect("text"),
            2,
            "{\"type\":\"item.started\",\"item\":{\"id\":\"item_0\",\"type\":\"command_execution\"}}",
        );
        assert_eq!(
            read(&both, Some(0)),
            (
                unknown(Some(0)),
                Some(
                    "codex reached a tool of its own: bilbo's arguments did not keep it without tools"
                        .to_owned()
                )
            )
        );
    }

    #[test]
    fn a_completed_turn_needs_an_answer_and_exit_0() {
        assert_eq!(
            read(NO_MESSAGE.as_bytes(), Some(0)),
            (
                unknown(Some(0)),
                Some(format!("the reply holds no answer: {READS}"))
            )
        );
        assert_eq!(read(ANSWER.as_bytes(), Some(1)), (unknown(Some(1)), None));
        assert_eq!(read(ANSWER.as_bytes(), None), (unknown(None), None));
        let busy = put(
            ANSWER,
            2,
            &format!("{{\"type\":\"error\",\"message\":\"{CAPACITY}\"}}"),
        );
        assert_eq!(
            read(&busy, Some(1)).0,
            Outcome::Busy {
                wait: None,
                cause: Cause {
                    status: Some(1),
                    ..Cause::new(What::Overloaded)
                }
            }
        );
        assert!(matches!(read(&busy, Some(0)).0, Outcome::Answer { .. }));
    }

    #[test]
    fn each_sentence_of_codex_has_its_cause() {
        let cause = |what, code: Option<&str>| Cause {
            status: Some(1),
            code: code.map(str::to_owned),
            ..Cause::new(what)
        };
        let stopped = |what| Outcome::Stopped(cause(what, None));
        let busy = |what| Outcome::Busy {
            wait: None,
            cause: cause(what, None),
        };
        let mut cases: Vec<(String, Outcome)> = Vec::new();
        for message in [
            "You’ve hit your usage limit. Try again later.",
            "You’ve hit your usage limit",
            "Your workspace is out of credits. Ask an admin.",
            "You hit your spend cap set by an admin",
            "Quota exceeded. Check your plan and billing details. (details)",
        ] {
            cases.push((message.to_owned(), stopped(What::Billing)));
        }
        cases.push((
            "Your access token could not be refreshed because your refresh token has expired. Please log out and sign in again.".to_owned(),
            stopped(What::Unauthorized),
        ));
        cases.push((
            "To use Codex with your ChatGPT plan, upgrade to Plus: https://x".to_owned(),
            stopped(What::Forbidden),
        ));
        for message in [
            CAPACITY,
            "Flex capacity unavailable. Try later.",
            "Policy-restricted capacity is unavailable.",
        ] {
            cases.push((message.to_owned(), busy(What::Overloaded)));
        }
        cases.push((
            "We’re currently experiencing high demand, which may cause temporary errors."
                .to_owned(),
            busy(What::ServerError),
        ));
        cases.push((
            "rate limit exceeded: slow down".to_owned(),
            busy(What::RateLimit),
        ));
        cases.push(("request timed out".to_owned(), busy(What::Timeout)));
        cases.push((
            "Connection failed: refused".to_owned(),
            busy(What::NoConnection),
        ));
        cases.push((
            "stream disconnected before completion: Incomplete response returned, reason: content_filter".to_owned(),
            Outcome::Refused {
                category: Some("content_filter".to_owned()),
                usage: Usage::default(),
            },
        ));
        cases.push((
            "Incomplete response returned, reason: max_output_tokens".to_owned(),
            Outcome::Cut(Usage::default()),
        ));
        for (reason, code) in [("weird", "weird"), ("two words", "unknown")] {
            cases.push((
                format!("Incomplete response returned, reason: {reason}"),
                Outcome::Failed {
                    cause: cause(What::StopReason, Some(code)),
                    usage: Usage::default(),
                },
            ));
        }
        for message in [
            "stream disconnected before completion: stream closed",
            "Error while reading the server response: eof",
        ] {
            cases.push((message.to_owned(), busy(What::ServerError)));
        }
        cases.push((
            "Codex ran out of room in the model's context window. Start a new thread.".to_owned(),
            stopped(What::Rejected),
        ));
        for start in ["unexpected status ", "exceeded retry limit, last status: "] {
            for (number, what, stops) in [
                ("401", What::Unauthorized, true),
                ("403", What::Forbidden, true),
                ("404", What::NotFound, true),
                ("408", What::Timeout, false),
                ("429", What::RateLimit, false),
                ("503", What::Overloaded, false),
                ("409", What::ServerError, false),
                ("500", What::ServerError, false),
                ("501", What::ServerError, false),
                ("502", What::ServerError, false),
                ("504", What::ServerError, false),
                ("599", What::ServerError, false),
                ("400", What::Rejected, true),
                ("418", What::Rejected, true),
            ] {
                let made = cause(what, Some(number));
                let outcome = if stops {
                    Outcome::Stopped(made)
                } else {
                    Outcome::Busy {
                        wait: None,
                        cause: made,
                    }
                };
                cases.push((
                    format!("{start}{number} Reason: a body, url: https://x"),
                    outcome,
                ));
            }
            cases.push((format!("{start}302 Found"), unknown(Some(1))));
        }
        cases.push(("something else".to_owned(), unknown(Some(1))));
        for (message, outcome) in cases {
            assert_eq!(read(&turn(&message), Some(1)).0, outcome, "{message}");
        }
    }

    #[test]
    fn an_error_bilbo_does_not_know_is_the_providers_and_is_kept_nowhere() {
        let (outcome, why) = read(FAILED_OTHER.as_bytes(), Some(1));
        assert_eq!(outcome, unknown(Some(1)));
        assert_eq!(why, None);
        assert!(!format!("{outcome:?}").contains("SENTINEL-7f3a"));
        let (outcome, _) = read(FAILED_401.as_bytes(), Some(1));
        assert_eq!(
            outcome,
            Outcome::Stopped(Cause {
                status: Some(1),
                code: Some("401".to_owned()),
                ..Cause::new(What::Unauthorized)
            })
        );
        let shown = format!("{outcome:?}");
        assert!(!shown.contains("a body bilbo never keeps") && !shown.contains("req-1"));
        let refreshed = "Your access token could not be refreshed because your refresh token has expired. Please log out and sign in again.";
        assert_eq!(
            read(&turn(refreshed), Some(1)).0,
            Outcome::Stopped(Cause {
                status: Some(1),
                ..Cause::new(What::Unauthorized)
            })
        );
    }

    #[test]
    fn an_error_with_no_failed_turn_is_read_as_the_turns() {
        let limit = |reply: &[u8]| read(reply, Some(1)).0;
        let billing = Outcome::Stopped(Cause {
            status: Some(1),
            ..Cause::new(What::Billing)
        });
        let capacity_line = format!("{{\"type\":\"error\",\"message\":\"{CAPACITY}\"}}");
        let mut only_error = lines(FAILED_LIMIT);
        only_error.pop();
        let only_error = only_error.join("\n").into_bytes();
        assert_eq!(limit(&only_error), billing);
        let mut later = lines(FAILED_LIMIT);
        later.push(capacity_line.clone());
        let mut later_error = later.clone();
        later_error.remove(3);
        assert_eq!(
            limit(later_error.join("\n").as_bytes()),
            Outcome::Busy {
                wait: None,
                cause: Cause {
                    status: Some(1),
                    ..Cause::new(What::Overloaded)
                }
            }
        );
        assert_eq!(limit(&put(FAILED_LIMIT, 2, &capacity_line)), billing);
        assert_eq!(limit(later.join("\n").as_bytes()), billing);
        let start = lines(FAILED_LIMIT)[..2].join("\n");
        let usage_error = lines(FAILED_LIMIT)[2].clone();
        let only = format!("{start}\n{usage_error}\n");
        let no_end = format!("the reply holds no end of a turn: {READS}");
        assert_eq!(
            read(only.as_bytes(), Some(0)),
            (unknown(Some(0)), Some(no_end))
        );
        let retry = "{\"type\":\"error\",\"message\":\"Reconnecting... 2/5 (Incomplete response returned, reason: content_filter)\"}";
        assert_eq!(
            read(format!("{start}\n{retry}\n").as_bytes(), Some(1)),
            (
                unknown(Some(1)),
                Some("codex ended with exit 1 and printed no end of a turn".to_owned())
            )
        );
        assert_eq!(
            limit(format!("{start}\n{retry}\n{usage_error}\n").as_bytes()),
            billing
        );
    }

    #[test]
    fn a_reply_with_no_end_is_the_providers_and_says_what_is_missing() {
        let by_hand = |exit: &str| format!("codex ended with {exit} and printed no end of a turn");
        assert_eq!(
            read(b"", Some(1)),
            (unknown(Some(1)), Some(by_hand("exit 1")))
        );
        assert_eq!(
            read(b"", None),
            (unknown(None), Some(by_hand("an unknown exit")))
        );
        let no_end = format!("the reply holds no end of a turn: {READS}");
        assert_eq!(
            read(
                b"{\"type\":\"thread.started\",\"thread_id\":\"t\"}\n{\"type\":\"item.completed\",\"item\":{\"id\":\"item_0\",\"type\":\"reasoning\"}}\n",
                Some(101)
            ),
            (unknown(Some(101)), Some(by_hand("exit 101")))
        );
        assert_eq!(read(b"", Some(0)), (unknown(Some(0)), Some(no_end.clone())));
        let started =
            "{\"type\":\"thread.started\",\"thread_id\":\"t\"}\n{\"type\":\"turn.started\"}\n";
        assert_eq!(
            read(started.as_bytes(), Some(0)),
            (unknown(Some(0)), Some(no_end))
        );
    }

    #[test]
    fn an_error_of_the_plumbing_at_a_call_is_the_providers() {
        assert_eq!(
            not_run("codex", &io::Error::other("a pipe of the child is missing")),
            (
                unknown(None),
                "bilbo could not run codex: a pipe of the child is missing".to_owned()
            )
        );
    }

    #[test]
    fn a_reached_tool_is_read_from_the_complete_lines_alone() {
        let line = "{\"type\":\"item.started\",\"item\":{\"id\":\"item_0\",\"type\":\"command_execution\"}}";
        assert!(reached(format!("{line}\n").as_bytes()));
        assert!(reached(format!("{line}\n{}", &line[..20]).as_bytes()));
        assert!(!reached(line.as_bytes()));
        assert!(!reached(b"no event\n"));
        assert!(!reached(b"{\"type\":\"thread.started\"}\n"));
        assert!(!reached(ANSWER.as_bytes()));
        assert!(!reached(b""));
    }

    #[test]
    fn a_program_given_by_a_relative_path_is_made_absolute() {
        let none: Variables = Box::new(|_| None);
        assert_eq!(
            super::resolve("bin/codex", &none),
            Ok(std::env::current_dir()
                .expect("a working folder")
                .join("bin/codex"))
        );
    }

    #[test]
    fn the_lines_for_the_dry_run_show_the_home_in_force() {
        let lines = adapter("derive.openai.model = \"m\"\n").describe();
        assert_eq!(lines.len(), 6);
        assert_eq!(
            lines[0],
            "provider: openai, via tool, model m, effort medium, program codex"
        );
        assert!(lines[1].starts_with("arguments: ["));
        assert!(lines[2].starts_with("inherit: ["));
        assert!(lines[3].starts_with("environment: "));
        assert_eq!(lines[4], "home: /s/derive/codex");
        assert!(lines[5].starts_with("system_files: ["));
        let set = adapter("derive.openai.model = \"m\"\n\n[derive.openai.tool]\nhome = \"/h/c\"\n")
            .describe();
        assert_eq!(set[4], "home: /h/c");
    }

    #[test]
    fn the_debug_of_the_tools_adapter_prints_its_program_alone() {
        assert_eq!(
            format!("{:?}", adapter("derive.openai.model = \"m\"\n")),
            "Codex { program: \"codex\", .. }"
        );
    }

    #[test]
    fn the_names_read_are_the_inherited_ones_and_the_three_bilbo_needs() {
        assert_eq!(
            resolve("").expect("defaults").reads(),
            ["PATH", "TMPDIR", "CODEX_HOME", "HOME"]
        );
        assert_eq!(
            resolve(&with("inherit", "[\"A_B\"]"))
                .expect("section")
                .reads(),
            ["A_B", "PATH", "CODEX_HOME", "HOME"]
        );
        let given = |pairs: &'static [(&'static str, &'static str)]| -> Variables {
            Box::new(|name| {
                pairs
                    .iter()
                    .find(|(asked, _)| *asked == name)
                    .map(|(_, value)| (*value).to_owned())
            })
        };
        assert_eq!(
            own_home(&given(&[("CODEX_HOME", "/x")])),
            Ok(PathBuf::from("/x"))
        );
        assert_eq!(
            own_home(&given(&[("CODEX_HOME", ""), ("HOME", "/h")])),
            Ok(PathBuf::from("/h/.codex"))
        );
        let neither = "derive.openai.tool.home: neither CODEX_HOME nor HOME is set, so bilbo cannot tell your own codex home from this one: set HOME";
        assert_eq!(own_home(&given(&[])), Err(neither.to_owned()));
        assert_eq!(
            own_home(&given(&[("CODEX_HOME", ""), ("HOME", "")])),
            Err(neither.to_owned())
        );
    }
}
