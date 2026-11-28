use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use bilbo_settings::edit::read_regular;
use serde_json::{Map, Value, json};

use crate::keys::waited;
use crate::plugin::{
    Act, Agent, Canon, Held, Hook, Hooks, Installed, KEY_PREFIX, Marketplace, NOT_JSON, Other,
    PACKAGE, PLUGIN_ID, Package, Seen, Stale, Standing, Trusting, ended_badly, message, unreadable,
};
use crate::program::{self, Bounds, End, Ran};
use crate::talk::{Calls, Fault, Talk};

pub const FILE: &str = ".agents/plugins/marketplace.json";
pub const HOOKS_FILE: &str = "plugins/bilbo/hooks/hooks.json";

fn system_canon(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok()
}

fn resolve(folder: &Path) -> Result<Package, String> {
    let canonical = |path: PathBuf| {
        let resolved = fs::canonicalize(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if resolved.is_dir() {
            Ok(resolved)
        } else {
            Err(format!("cannot read {}: not a folder", path.display()))
        }
    };
    Ok(Package {
        folder: canonical(folder.to_path_buf())?,
        plugin: canonical(folder.join(PACKAGE))?,
    })
}

pub struct Codex {
    program: PathBuf,
    bounds: Bounds,
    version: String,
    home: Option<String>,
}

impl Codex {
    pub fn new(program: PathBuf, bounds: Bounds, version: String, home: String) -> Codex {
        Codex {
            program,
            bounds,
            version,
            home: Some(home),
        }
    }

    pub fn without_home(program: PathBuf, bounds: Bounds, version: String) -> Codex {
        Codex {
            program,
            bounds,
            version,
            home: None,
        }
    }

    fn call(&self, args: &[&str]) -> Ran {
        program::run(&self.program, args, self.bounds)
    }

    fn listed(&self, args: &[&str]) -> Result<Vec<u8>, String> {
        let words = format!("codex {}", args.join(" "));
        let ran = self.call(args);
        match ended_badly(&ran, &self.program, &waited(self.bounds.wait)) {
            None => Ok(ran.stdout),
            Some(why) => Err(unreadable(&words, &why)),
        }
    }

    fn arguments(act: Act, folder: &Path) -> Option<Vec<String>> {
        let words: Vec<&str> = match act {
            Act::Uninstall => vec!["plugin", "remove", PLUGIN_ID, "--json"],
            Act::Remove => vec!["plugin", "marketplace", "remove", "bilbo", "--json"],
            Act::Install => vec!["plugin", "add", PLUGIN_ID, "--json"],
            Act::Add => {
                return Some(
                    ["plugin", "marketplace", "add", folder.to_str()?, "--json"]
                        .map(str::to_owned)
                        .to_vec(),
                );
            }
        };
        Some(words.into_iter().map(str::to_owned).collect())
    }
}

fn text(item: &Value, field: &str) -> Option<String> {
    item.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn same_folder(listed: &str, wanted: &Path, canon: &Canon) -> bool {
    canon(Path::new(listed)).is_some_and(|folder| folder == wanted)
}

fn items(bytes: &[u8], field: &str) -> Option<Vec<Value>> {
    match serde_json::from_slice::<Value>(bytes).ok()? {
        Value::Object(mut object) => match object.remove(field)? {
            Value::Array(items) => Some(items),
            _ => None,
        },
        _ => None,
    }
}

pub fn read_marketplaces(bytes: &[u8], package: &Path, canon: &Canon) -> Option<Marketplace> {
    let mut found = Marketplace::None;
    for item in items(bytes, "marketplaces")? {
        let name = text(&item, "name")?;
        if name != "bilbo" {
            continue;
        }
        let source = item.get("marketplaceSource")?;
        let kind = text(source, "sourceType")?;
        found = if kind == "local" {
            let path = text(source, "source")?;
            if same_folder(&path, package, canon) {
                Marketplace::Here
            } else {
                Marketplace::Elsewhere(path)
            }
        } else {
            Marketplace::Elsewhere(kind)
        };
    }
    Some(found)
}

pub fn held_marketplace(bytes: &[u8]) -> Option<bool> {
    let mut listed = false;
    for item in items(bytes, "marketplaces")? {
        listed |= text(&item, "name")? == "bilbo";
    }
    Some(listed)
}

pub fn held_plugins(bytes: &[u8]) -> Option<(bool, Option<Other>)> {
    let (mut plugin, mut other) = (false, None);
    for item in items(bytes, "installed")? {
        let id = text(&item, "pluginId")?;
        let market = text(&item, "marketplaceName")?;
        if id == PLUGIN_ID {
            plugin = true;
        } else if market == "bilbo" && other.is_none() {
            other = Some(Other { id, scope: None });
        }
    }
    Some((plugin, other))
}

pub fn read_plugins(
    bytes: &[u8],
    wanted: &Path,
    version: &str,
    canon: &Canon,
) -> Option<(Option<Installed>, Option<Other>)> {
    let mut plugin = None;
    let mut other = None;
    for item in items(bytes, "installed")? {
        let id = text(&item, "pluginId")?;
        let market = text(&item, "marketplaceName")?;
        if id == PLUGIN_ID {
            let on = item.get("enabled").and_then(Value::as_bool)?;
            let listed_version = text(&item, "version")?;
            let path = text(item.get("source")?, "path")?;
            let stale = if listed_version != version {
                Some(Stale::Version(listed_version))
            } else if same_folder(&path, wanted, canon) {
                None
            } else {
                Some(Stale::InstalledFrom(path))
            };
            plugin = plugin.or(Some(Installed { on, stale }));
        } else if market == "bilbo" && other.is_none() {
            other = Some(Other { id, scope: None });
        }
    }
    Some((plugin, other))
}

pub fn read_manifest(bytes: &[u8], folder: &Path) -> Result<(), String> {
    let no = |why: &str| format!("no plugin at {}: {FILE} {why}", folder.display());
    let Ok(Value::Object(file)) = serde_json::from_slice::<Value>(bytes) else {
        return Err(no("is not the manifest bilbo reads"));
    };
    let Some(name) = file.get("name").and_then(Value::as_str) else {
        return Err(no("is not the manifest bilbo reads"));
    };
    let Some(Value::Array(plugins)) = file.get("plugins") else {
        return Err(no("is not the manifest bilbo reads"));
    };
    if name != "bilbo" {
        return Err(no(&format!("names the marketplace '{name}', not bilbo")));
    }
    let Some(item) = plugins
        .iter()
        .find(|item| item.get("name").and_then(Value::as_str) == Some("bilbo"))
    else {
        return Err(no("names no plugin bilbo"));
    };
    let source = item.get("source");
    let local = source
        .and_then(|source| source.get("source"))
        .and_then(Value::as_str)
        == Some("local")
        && source
            .and_then(|source| source.get("path"))
            .and_then(Value::as_str)
            == Some(format!("./{PACKAGE}").as_str());
    if local {
        Ok(())
    } else {
        Err(no(&format!(
            "takes the plugin bilbo from another folder than ./{PACKAGE}"
        )))
    }
}

impl Agent for Codex {
    fn step(&self) -> &'static str {
        "codex"
    }

    fn name(&self) -> &'static str {
        "codex"
    }

    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn manifest(&self, folder: &Path) -> Result<(), String> {
        if fs::metadata(folder).is_ok_and(|meta| !meta.is_dir()) {
            return Err(format!("cannot read {}: not a folder", folder.display()));
        }
        let path = folder.join(FILE);
        match read_regular(&path) {
            Ok(bytes) => read_manifest(&bytes, folder),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(format!(
                "no plugin at {}: it holds no {FILE}",
                folder.display()
            )),
            Err(error) => Err(format!("cannot read {}: {error}", path.display())),
        }
    }

    fn look(&self, folder: &Path) -> Result<Seen, String> {
        const MARKETS: [&str; 4] = ["plugin", "marketplace", "list", "--json"];
        const PLUGINS: [&str; 3] = ["plugin", "list", "--json"];
        let package = resolve(folder)?;
        let words = |args: &[&str]| format!("codex {}", args.join(" "));
        let market = self.listed(&MARKETS)?;
        let marketplace = read_marketplaces(&market, &package.folder, &system_canon)
            .ok_or_else(|| unreadable(&words(&MARKETS), NOT_JSON))?;
        let plugins = self.listed(&PLUGINS)?;
        let (plugin, other) = read_plugins(&plugins, &package.plugin, &self.version, &system_canon)
            .ok_or_else(|| unreadable(&words(&PLUGINS), NOT_JSON))?;
        Ok(Seen {
            marketplace,
            plugin,
            other,
        })
    }

    fn run(&self, act: Act, folder: &Path) -> Result<(), String> {
        let words = self.words(act, folder);
        let Some(args) = Codex::arguments(act, folder) else {
            return Err(format!(
                "codex {words}: the plugin folder is not UTF-8 text"
            ));
        };
        let ran = self.call(&args.iter().map(String::as_str).collect::<Vec<_>>());
        if matches!(ran.end, End::Exited(0)) {
            return Ok(());
        }
        Err(format!(
            "codex {words}: {}",
            message(&ran, None, &self.program, &waited(self.bounds.wait))
        ))
    }

    fn words(&self, act: Act, folder: &Path) -> String {
        Codex::arguments(act, folder).map_or_else(String::new, |args| args.join(" "))
    }

    fn hooks(&self, folder: &Path) -> Option<Trusting> {
        Some(Trusting {
            step: "codex-hooks",
            hooks: Box::new(Launched {
                program: self.program.clone(),
                bounds: self.bounds,
                version: self.version.clone(),
                home: self.home.clone(),
                file: folder.join(HOOKS_FILE),
                over: None,
            }),
        })
    }

    fn held(&self) -> Result<Held, String> {
        const MARKETS: [&str; 4] = ["plugin", "marketplace", "list", "--json"];
        const PLUGINS: [&str; 3] = ["plugin", "list", "--json"];
        let words = |args: &[&str]| format!("codex {}", args.join(" "));
        let market = self.listed(&MARKETS)?;
        let marketplace =
            held_marketplace(&market).ok_or_else(|| unreadable(&words(&MARKETS), NOT_JSON))?;
        let plugins = self.listed(&PLUGINS)?;
        let (plugin, other) =
            held_plugins(&plugins).ok_or_else(|| unreadable(&words(&PLUGINS), NOT_JSON))?;
        Ok(Held {
            plugin,
            marketplace,
            other,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Handler {
    event: String,
    matcher: Option<String>,
    kind: String,
    command: String,
    timeout: u64,
    asynchronous: bool,
}

fn declared_in(bytes: &[u8], path: &Path) -> Result<Vec<Handler>, String> {
    let bad = || {
        format!(
            "cannot read {}: it is not the hooks file bilbo reads",
            path.display()
        )
    };
    let Ok(Value::Object(file)) = serde_json::from_slice::<Value>(bytes) else {
        return Err(bad());
    };
    let Some(Value::Object(events)) = file.get("hooks") else {
        return Err(bad());
    };
    let mut handlers = Vec::new();
    for (event, groups) in events {
        let Value::Array(groups) = groups else {
            return Err(bad());
        };
        for group in groups {
            let matcher = match group.get("matcher") {
                None | Some(Value::Null) => None,
                Some(Value::String(text)) => Some(text.clone()),
                Some(_) => return Err(bad()),
            };
            let Some(Value::Array(list)) = group.get("hooks") else {
                return Err(bad());
            };
            for handler in list {
                let Some(fields) = handler.as_object() else {
                    return Err(bad());
                };
                if fields
                    .keys()
                    .any(|name| !["type", "command", "timeout", "async"].contains(&name.as_str()))
                {
                    return Err(bad());
                }
                let kind = text(handler, "type").ok_or_else(bad)?;
                let command = text(handler, "command").ok_or_else(bad)?;
                let timeout = handler
                    .get("timeout")
                    .and_then(Value::as_u64)
                    .ok_or_else(bad)?;
                let asynchronous = match handler.get("async") {
                    None => false,
                    Some(Value::Bool(flag)) => *flag,
                    Some(_) => return Err(bad()),
                };
                handlers.push(Handler {
                    event: event.clone(),
                    matcher: matcher.clone(),
                    kind,
                    command,
                    timeout,
                    asynchronous,
                });
            }
        }
    }
    Ok(handlers)
}

fn lower_first(event: &str) -> String {
    let mut chars = event.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_lowercase().chain(chars).collect()
    })
}

const NOT_READ: &str = "codex app-server answered hooks/list with what bilbo does not read";
const NOT_READ_CONFIG: &str = "codex app-server answered config/read with what bilbo does not read";

const REQUIRED: [&str; 9] = [
    "key",
    "eventName",
    "handlerType",
    "command",
    "async",
    "additionalContextLimit",
    "enabled",
    "currentHash",
    "trustStatus",
];

fn shown(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => "null".to_owned(),
    }
}

fn hooks_of(reply: &Value) -> Option<Vec<&Map<String, Value>>> {
    let mut found = Vec::new();
    for data in reply.as_object()?.get("data")?.as_array()? {
        for hook in data.as_object()?.get("hooks")?.as_array()? {
            found.push(hook.as_object()?);
        }
    }
    Some(found)
}

fn belongs(hook: &Map<String, Value>) -> bool {
    hook.get("pluginId").and_then(Value::as_str) == Some(PLUGIN_ID)
        || hook
            .get("key")
            .and_then(Value::as_str)
            .is_some_and(|key| key.starts_with(KEY_PREFIX))
}

fn another(handler: &Handler, hook: &Map<String, Value>) -> Option<&'static str> {
    let is = |field: &str, text: &str| hook.get(field).and_then(Value::as_str) == Some(text);
    if !is("handlerType", &handler.kind) {
        Some("handler type")
    } else if hook.get("matcher")
        != Some(
            &handler
                .matcher
                .as_ref()
                .map_or(Value::Null, |text| Value::String(text.clone())),
        )
    {
        Some("matcher")
    } else if !is("command", &handler.command) {
        Some("command")
    } else if hook.get("timeoutSec").and_then(Value::as_u64) != Some(handler.timeout) {
        Some("timeout")
    } else if hook.get("async").and_then(Value::as_bool) != Some(handler.asynchronous) {
        Some("async")
    } else if hook.get("additionalContextLimit") != Some(&Value::Null) {
        Some("context limit")
    } else {
        None
    }
}

type Group<'a> = (Vec<&'a Handler>, Vec<&'a Map<String, Value>>, &'a str);

pub fn read_hooks(declared: &[u8], file: &Path, reply: &Value) -> Result<Vec<Hook>, String> {
    let handlers = declared_in(declared, file)?;
    let listed = hooks_of(reply).ok_or_else(|| NOT_READ.to_owned())?;
    let mine: Vec<&Map<String, Value>> = listed.into_iter().filter(|hook| belongs(hook)).collect();
    for hook in &mine {
        if let Some(field) = REQUIRED.iter().find(|field| !hook.contains_key(**field)) {
            return Err(format!(
                "Codex listed a hook of bilbo@bilbo without {field}"
            ));
        }
    }
    for hook in &mine {
        let kinds = hook.get("enabled").is_some_and(Value::is_boolean)
            && hook.get("currentHash").is_some_and(Value::is_string)
            && hook.get("trustStatus").is_some_and(Value::is_string);
        if !kinds {
            return Err(NOT_READ.to_owned());
        }
    }
    for hook in &mine {
        let key = shown(hook.get("key"));
        if !hook
            .get("key")
            .and_then(Value::as_str)
            .is_some_and(|key| key.starts_with(KEY_PREFIX))
        {
            return Err(format!(
                "Codex lists the hook {key} for bilbo@bilbo, and its key names another plugin"
            ));
        }
    }
    for hook in &mine {
        if hook.get("pluginId").and_then(Value::as_str) != Some(PLUGIN_ID) {
            return Err(format!(
                "Codex lists the hook {} under the plugin {}",
                shown(hook.get("key")),
                shown(hook.get("pluginId"))
            ));
        }
    }
    for (at, hook) in mine.iter().enumerate() {
        if mine[..at]
            .iter()
            .any(|earlier| earlier.get("key") == hook.get("key"))
        {
            return Err(format!(
                "Codex lists two hooks with the key {}",
                shown(hook.get("key"))
            ));
        }
    }
    let mut events: Vec<&str> = Vec::new();
    for handler in &handlers {
        if !events.contains(&handler.event.as_str()) {
            events.push(&handler.event);
        }
    }
    let named = |hook: &Map<String, Value>, event: &str| {
        hook.get("eventName").and_then(Value::as_str) == Some(lower_first(event).as_str())
    };
    if let Some(hook) = mine
        .iter()
        .find(|hook| !events.iter().any(|event| named(hook, event)))
    {
        return Err(format!(
            "Codex lists a hook of bilbo@bilbo for {}, which the plugin does not declare",
            shown(hook.get("eventName"))
        ));
    }
    let mut grouped: Vec<Group> = Vec::new();
    for event in &events {
        let wanted: Vec<&Handler> = handlers
            .iter()
            .filter(|handler| handler.event == *event)
            .collect();
        let found: Vec<&Map<String, Value>> = mine
            .iter()
            .copied()
            .filter(|hook| named(hook, event))
            .collect();
        if found.is_empty() {
            return Err(format!(
                "Codex lists no hook of bilbo@bilbo for {}",
                lower_first(event)
            ));
        }
        grouped.push((wanted, found, event));
    }
    let mut pairs: Vec<(&Handler, &Map<String, Value>)> = Vec::new();
    for (wanted, found, event) in grouped {
        if found.len() != wanted.len() {
            return Err(format!(
                "Codex lists {} hooks of bilbo@bilbo for {}, where the plugin has {}",
                found.len(),
                lower_first(event),
                wanted.len()
            ));
        }
        pairs.extend(wanted.into_iter().zip(found));
    }
    for (handler, hook) in &pairs {
        if let Some(what) = another(handler, hook) {
            return Err(format!(
                "Codex lists the hook {} with another {what} than the plugin declares",
                shown(hook.get("key"))
            ));
        }
    }
    if let Some(hook) = mine
        .iter()
        .find(|hook| hook.get("enabled") == Some(&Value::Bool(false)))
    {
        return Err(format!(
            "Codex lists the hook {} turned off",
            shown(hook.get("key"))
        ));
    }
    Ok(mine
        .iter()
        .map(|hook| Hook {
            key: shown(hook.get("key")),
            hash: shown(hook.get("currentHash")),
            standing: match shown(hook.get("trustStatus")).as_str() {
                "trusted" | "managed" => Standing::Trusted,
                "untrusted" => Standing::ToTrust,
                "modified" => Standing::Changed,
                other => Standing::Unknown(other.to_owned()),
            },
        })
        .collect())
}

fn fault_sentence(fault: Fault, wait: &str) -> String {
    match fault {
        Fault::NotTaken { method } => {
            format!("codex app-server did not take {method} within {wait}")
        }
        Fault::Silent { method } => {
            format!("codex app-server did not answer {method} within {wait}")
        }
        Fault::Ended { method, line } => match line {
            Some(line) => format!("codex app-server ended before answering {method}: {line}"),
            None => format!("codex app-server ended before answering {method}"),
        },
        Fault::Answered { method, message } => {
            format!("codex app-server answered {method}: {message}")
        }
    }
}

pub struct Over<C: Calls> {
    calls: C,
    declared: Vec<u8>,
    file: PathBuf,
    version: String,
    home: Option<String>,
    waited: String,
    ready: bool,
}

impl<C: Calls> Over<C> {
    pub fn new(
        calls: C,
        declared: Vec<u8>,
        file: PathBuf,
        version: String,
        home: Option<String>,
        waited: String,
    ) -> Over<C> {
        Over {
            calls,
            declared,
            file,
            version,
            home,
            waited,
            ready: false,
        }
    }

    fn ask(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.calls
            .request(method, params)
            .map_err(|fault| fault_sentence(fault, &self.waited))
    }

    fn open(&mut self) -> Result<(), String> {
        if !self.ready {
            let params = json!({"clientInfo": {"name": "bilbo", "version": self.version}});
            self.ask("initialize", params)?;
            self.calls
                .notice("initialized", None)
                .map_err(|fault| fault_sentence(fault, &self.waited))?;
            self.ready = true;
        }
        Ok(())
    }
}

fn read_keys(reply: &Value) -> Result<Vec<String>, String> {
    let Some(config) = reply.get("config").and_then(Value::as_object) else {
        return Err(NOT_READ_CONFIG.to_owned());
    };
    let state = match config.get("hooks") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Object(hooks)) => match hooks.get("state") {
            None => return Ok(Vec::new()),
            Some(Value::Object(state)) => state,
            Some(_) => return Err(NOT_READ_CONFIG.to_owned()),
        },
        Some(_) => return Err(NOT_READ_CONFIG.to_owned()),
    };
    Ok(state
        .keys()
        .filter(|key| key.starts_with(KEY_PREFIX))
        .cloned()
        .collect())
}

fn key_path(key: &str) -> String {
    let escaped = key.replace('\\', "\\\\").replace('"', "\\\"");
    format!("hooks.state.\"{escaped}\"")
}

impl<C: Calls> Hooks for Over<C> {
    fn list(&mut self) -> Result<Vec<Hook>, String> {
        self.open()?;
        let Some(home) = self.home.clone() else {
            return Err("codex app-server needs a home to list hooks".to_owned());
        };
        let reply = self.ask("hooks/list", json!({"cwds": [home]}))?;
        read_hooks(&self.declared, &self.file, &reply)
    }

    fn kept(&mut self) -> Result<Vec<String>, String> {
        self.open()?;
        let reply = self.ask("config/read", json!({"includeLayers": false}))?;
        read_keys(&reply)
    }

    fn forget(&mut self, keys: &[String]) -> Result<(), String> {
        let edits: Vec<Value> = keys
            .iter()
            .map(|key| json!({"keyPath": key_path(key), "value": Value::Null, "mergeStrategy": "replace"}))
            .collect();
        let params = json!({"edits": edits, "reloadUserConfig": false});
        self.ask("config/batchWrite", params).map(|_| ())
    }

    fn trust(&mut self, hooks: &[Hook]) -> Result<(), String> {
        let mut state = Map::new();
        for hook in hooks {
            state.insert(hook.key.clone(), json!({"trusted_hash": hook.hash}));
        }
        let params = json!({
            "edits": [{"keyPath": "hooks.state", "value": state, "mergeStrategy": "upsert"}],
            "reloadUserConfig": false
        });
        self.ask("config/batchWrite", params).map(|_| ())
    }

    fn close(&mut self) {}
}

struct Launched {
    program: PathBuf,
    bounds: Bounds,
    version: String,
    home: Option<String>,
    file: PathBuf,
    over: Option<Over<Talk>>,
}

impl Launched {
    fn start(&mut self, declared: Vec<u8>) -> Result<(), String> {
        if self.over.is_none() {
            let talk = Talk::start(&self.program, &["app-server"], self.bounds)
                .map_err(|error| format!("cannot start {}: {error}", self.program.display()))?;
            self.over = Some(Over::new(
                talk,
                declared,
                self.file.clone(),
                self.version.clone(),
                self.home.clone(),
                waited(self.bounds.wait),
            ));
        }
        Ok(())
    }

    fn running(&mut self) -> Result<&mut Over<Talk>, String> {
        self.over
            .as_mut()
            .ok_or_else(|| "codex app-server is not running".to_owned())
    }
}

impl Hooks for Launched {
    fn list(&mut self) -> Result<Vec<Hook>, String> {
        if self.over.is_none() {
            let declared = read_regular(&self.file)
                .map_err(|error| format!("cannot read {}: {error}", self.file.display()))?;
            declared_in(&declared, &self.file)?;
            self.start(declared)?;
        }
        self.running()?.list()
    }

    fn trust(&mut self, hooks: &[Hook]) -> Result<(), String> {
        self.running()?.trust(hooks)
    }

    fn kept(&mut self) -> Result<Vec<String>, String> {
        self.start(Vec::new())?;
        self.running()?.kept()
    }

    fn forget(&mut self, keys: &[String]) -> Result<(), String> {
        self.running()?.forget(keys)
    }

    fn close(&mut self) {
        if let Some(over) = self.over.take() {
            over.calls.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use serde_json::{Value, json};

    use super::{
        Over, fault_sentence, held_marketplace, held_plugins, read_hooks, read_keys, read_manifest,
        read_marketplaces, read_plugins,
    };
    use crate::plugin::{
        Hook, Hooks, Installed, Marketplace, Other, Stale, Standing, forget, trust,
    };
    use crate::report::Status;
    use crate::talk::{Calls, Fault};

    fn fixture(name: &str) -> &'static str {
        match name {
            "hooks-declared-fifth-field.json" => {
                include_str!("../fixtures/codex/hooks-declared-fifth-field.json")
            }
            "hooks-declared.json" => include_str!("../fixtures/codex/hooks-declared.json"),
            "hooks-other-plugins.json" => {
                include_str!("../fixtures/codex/hooks-other-plugins.json")
            }
            "hooks-untrusted.json" => include_str!("../fixtures/codex/hooks-untrusted.json"),
            "hooks-modified.json" => include_str!("../fixtures/codex/hooks-modified.json"),
            "hooks-trusted.json" => include_str!("../fixtures/codex/hooks-trusted.json"),
            "manifest-other-first.json" => {
                include_str!("../fixtures/codex/manifest-other-first.json")
            }
            "installed-elsewhere-market.json" => {
                include_str!("../fixtures/codex/installed-elsewhere-market.json")
            }
            "installed-enabled-text.json" => {
                include_str!("../fixtures/codex/installed-enabled-text.json")
            }
            "installed-here.json" => include_str!("../fixtures/codex/installed-here.json"),
            "installed-no-enabled.json" => {
                include_str!("../fixtures/codex/installed-no-enabled.json")
            }
            "installed-no-version.json" => {
                include_str!("../fixtures/codex/installed-no-version.json")
            }
            "installed-then-no-market.json" => {
                include_str!("../fixtures/codex/installed-then-no-market.json")
            }
            "market-name-only.json" => include_str!("../fixtures/codex/market-name-only.json"),
            "installed-no-id.json" => include_str!("../fixtures/codex/installed-no-id.json"),
            "installed-none.json" => include_str!("../fixtures/codex/installed-none.json"),
            "installed-object.json" => include_str!("../fixtures/codex/installed-object.json"),
            "installed-off.json" => include_str!("../fixtures/codex/installed-off.json"),
            "installed-old-version.json" => {
                include_str!("../fixtures/codex/installed-old-version.json")
            }
            "installed-other-path.json" => {
                include_str!("../fixtures/codex/installed-other-path.json")
            }
            "installed-other-plugin.json" => {
                include_str!("../fixtures/codex/installed-other-plugin.json")
            }
            "manifest-no-bilbo.json" => include_str!("../fixtures/codex/manifest-no-bilbo.json"),
            "manifest-not-local.json" => include_str!("../fixtures/codex/manifest-not-local.json"),
            "manifest-other-source.json" => {
                include_str!("../fixtures/codex/manifest-other-source.json")
            }
            "manifest-personal.json" => include_str!("../fixtures/codex/manifest-personal.json"),
            "manifest-plugins-object.json" => {
                include_str!("../fixtures/codex/manifest-plugins-object.json")
            }
            "manifest.json" => include_str!("../fixtures/codex/manifest.json"),
            "market-array.json" => include_str!("../fixtures/codex/market-array.json"),
            "market-elsewhere.json" => include_str!("../fixtures/codex/market-elsewhere.json"),
            "market-git.json" => include_str!("../fixtures/codex/market-git.json"),
            "market-here.json" => include_str!("../fixtures/codex/market-here.json"),
            "market-no-name.json" => include_str!("../fixtures/codex/market-no-name.json"),
            "market-none.json" => include_str!("../fixtures/codex/market-none.json"),
            other => panic!("no fixture {other}"),
        }
    }

    const PKG: &str = "/world/pkg";
    const VERSION: &str = "1.2.3";
    const FILE: &str = "/world/pkg/plugins/bilbo/hooks/hooks.json";

    fn table(path: &Path) -> Option<PathBuf> {
        let known = [
            ("/world/pkg", "/world/pkg"),
            ("/world/pkg/plugins/bilbo", "/world/pkg/plugins/bilbo"),
            ("/world/link", "/world/pkg"),
            ("/world/link/plugins/bilbo", "/world/pkg/plugins/bilbo"),
            ("/world/old", "/world/old"),
            ("/world/old/plugins/bilbo", "/world/old/plugins/bilbo"),
        ];
        known
            .iter()
            .find(|(listed, _)| Path::new(listed) == path)
            .map(|(_, folder)| PathBuf::from(folder))
    }

    fn nothing(_: &Path) -> Option<PathBuf> {
        None
    }

    fn market(text: &str) -> Option<Marketplace> {
        read_marketplaces(text.as_bytes(), Path::new(PKG), &table)
    }

    fn installed(text: &str) -> Option<(Option<Installed>, Option<Other>)> {
        read_plugins(
            text.as_bytes(),
            Path::new("/world/pkg/plugins/bilbo"),
            VERSION,
            &table,
        )
    }

    fn current() -> Option<Installed> {
        Some(Installed {
            on: true,
            stale: None,
        })
    }

    #[test]
    fn codexs_lists_are_read_as_it_prints_them() {
        assert_eq!(market(fixture("market-none.json")), Some(Marketplace::None));
        assert_eq!(market(fixture("market-here.json")), Some(Marketplace::Here));
        assert_eq!(
            market(fixture("market-elsewhere.json")),
            Some(Marketplace::Elsewhere("/world/old".to_owned()))
        );
        assert_eq!(
            market(fixture("market-git.json")),
            Some(Marketplace::Elsewhere("git".to_owned()))
        );
        assert_eq!(
            installed(fixture("installed-none.json")),
            Some((None, None))
        );
        assert_eq!(
            installed(fixture("installed-here.json")),
            Some((current(), None))
        );
        assert_eq!(
            installed(fixture("installed-off.json")),
            Some((
                Some(Installed {
                    on: false,
                    stale: None
                }),
                None
            ))
        );
        assert_eq!(
            installed(fixture("installed-old-version.json")),
            Some((
                Some(Installed {
                    on: true,
                    stale: Some(Stale::Version("0.9.0".to_owned()))
                }),
                None
            ))
        );
        assert_eq!(
            installed(fixture("installed-other-path.json")),
            Some((
                Some(Installed {
                    on: true,
                    stale: Some(Stale::InstalledFrom("/world/old/plugins/bilbo".to_owned()))
                }),
                None
            ))
        );
        let here = fixture("market-here.json");
        assert_eq!(
            market(&here.replace("/world/pkg", "/world/link")),
            Some(Marketplace::Here)
        );
        assert_eq!(
            market(&here.replace("/world/pkg", "/world/gone")),
            Some(Marketplace::Elsewhere("/world/gone".to_owned()))
        );
        let on = fixture("installed-here.json");
        assert_eq!(
            installed(&on.replace("/world/pkg", "/world/link")),
            Some((current(), None))
        );
        assert_eq!(
            read_marketplaces(here.as_bytes(), Path::new(PKG), &nothing),
            Some(Marketplace::Elsewhere("/world/pkg".to_owned()))
        );
        let (plugin, _) = read_plugins(
            on.as_bytes(),
            Path::new("/world/pkg/plugins/bilbo"),
            VERSION,
            &nothing,
        )
        .expect("read");
        assert_eq!(
            plugin,
            Some(Installed {
                on: true,
                stale: Some(Stale::InstalledFrom("/world/pkg/plugins/bilbo".to_owned()))
            })
        );
    }

    #[test]
    fn a_list_bilbo_cannot_read_is_said_so() {
        for text in [
            "codex: broke",
            fixture("market-array.json"),
            fixture("market-no-name.json"),
        ] {
            assert_eq!(market(text), None, "{text}");
        }
        for text in [
            "codex: broke",
            fixture("installed-object.json"),
            fixture("installed-no-id.json"),
            fixture("installed-no-enabled.json"),
            fixture("installed-enabled-text.json"),
        ] {
            assert_eq!(installed(text), None, "{text}");
        }
    }

    #[test]
    fn another_installation_of_the_marketplace_is_seen() {
        assert_eq!(
            installed(fixture("installed-other-plugin.json")),
            Some((
                current(),
                Some(Other {
                    id: "other@bilbo".to_owned(),
                    scope: None
                })
            ))
        );
        assert_eq!(
            installed(fixture("installed-elsewhere-market.json")),
            Some((current(), None))
        );
    }

    #[test]
    fn the_file_a_tool_installs_from_must_name_bilbo() {
        let folder = Path::new(PKG);
        let said = |text: &str| read_manifest(text.as_bytes(), folder);
        assert_eq!(said(fixture("manifest.json")), Ok(()));
        assert_eq!(said(fixture("manifest-other-first.json")), Ok(()));
        let file = ".agents/plugins/marketplace.json";
        let no = |why: &str| Err(format!("no plugin at {PKG}: {file} {why}"));
        assert_eq!(said("nope"), no("is not the manifest bilbo reads"));
        assert_eq!(
            said(fixture("manifest-plugins-object.json")),
            no("is not the manifest bilbo reads")
        );
        assert_eq!(
            said(fixture("manifest-personal.json")),
            no("names the marketplace 'personal', not bilbo")
        );
        assert_eq!(
            said(fixture("manifest-no-bilbo.json")),
            no("names no plugin bilbo")
        );
        for text in [
            fixture("manifest-other-source.json"),
            fixture("manifest-not-local.json"),
        ] {
            assert_eq!(
                said(text),
                no("takes the plugin bilbo from another folder than ./plugins/bilbo")
            );
        }
    }

    struct Script {
        replies: Vec<Result<Value, Fault>>,
        sent: Vec<String>,
        params: Vec<Option<Value>>,
    }

    impl Script {
        fn new(replies: Vec<Result<Value, Fault>>) -> Script {
            Script {
                replies,
                sent: Vec::new(),
                params: Vec::new(),
            }
        }
    }

    impl Calls for Script {
        fn request(&mut self, method: &str, params: Value) -> Result<Value, Fault> {
            self.sent.push(method.to_owned());
            self.params.push(Some(params));
            self.replies.remove(0)
        }

        fn notice(&mut self, method: &str, params: Option<Value>) -> Result<(), Fault> {
            self.sent.push(format!("notice {method}"));
            self.params.push(params);
            Ok(())
        }
    }

    fn json_of(text: &str) -> Value {
        serde_json::from_str(text).expect("a fixture is JSON")
    }

    fn over(script: Script, declared: &str) -> Over<Script> {
        Over::new(
            script,
            declared.as_bytes().to_vec(),
            Path::new(FILE).to_owned(),
            VERSION.to_owned(),
            Some("/world/home".to_owned()),
            "30s".to_owned(),
        )
    }

    fn hook(key: &str, hash: &str, standing: Standing) -> Hook {
        Hook {
            key: key.to_owned(),
            hash: hash.to_owned(),
            standing,
        }
    }

    type Change = Box<dyn Fn(&mut Value)>;

    const KP: &str = "bilbo@bilbo:hooks/hooks.json:user_prompt_submit:0:0";
    const KS: &str = "bilbo@bilbo:hooks/hooks.json:session_start:0:0";

    #[test]
    fn codexs_hooks_are_read_as_its_app_server_lists_them() {
        let script = Script::new(vec![
            Ok(json!({})),
            Ok(json_of(fixture("hooks-untrusted.json"))),
        ]);
        let mut hooks = over(script, fixture("hooks-declared.json"));
        let listed = hooks.list().expect("the plugin's hooks");
        assert_eq!(
            listed,
            [
                hook(KP, "sha256:p1", Standing::ToTrust),
                hook(KS, "sha256:s1", Standing::ToTrust)
            ]
        );
        assert_eq!(
            hooks.calls.sent,
            ["initialize", "notice initialized", "hooks/list"]
        );
        assert_eq!(
            hooks.calls.params,
            [
                Some(json!({"clientInfo": {"name": "bilbo", "version": "1.2.3"}})),
                None,
                Some(json!({"cwds": ["/world/home"]}))
            ]
        );
        let script = Script::new(vec![
            Ok(json!({})),
            Ok(json_of(fixture("hooks-modified.json"))),
            Ok(json!({})),
            Ok(json_of(fixture("hooks-trusted.json"))),
        ]);
        let mut hooks = over(script, fixture("hooks-declared.json"));
        let line = trust("codex-hooks", &mut hooks);
        assert_eq!(
            (line.status, line.detail.as_str()),
            (Status::Updated, "Codex trusts 2 hooks of bilbo@bilbo")
        );
        assert_eq!(
            hooks.calls.params[3],
            Some(json!({
                "edits": [{
                    "keyPath": "hooks.state",
                    "value": {KP: {"trusted_hash": "sha256:p1"}},
                    "mergeStrategy": "upsert"
                }],
                "reloadUserConfig": false
            }))
        );
        let script = Script::new(vec![
            Ok(json!({})),
            Ok(json_of(fixture("hooks-trusted.json"))),
        ]);
        let mut hooks = over(script, fixture("hooks-declared.json"));
        let line = trust("codex-hooks", &mut hooks);
        assert_eq!(line.status, Status::Kept, "{}", line.detail);
        assert!(
            !hooks
                .calls
                .sent
                .iter()
                .any(|sent| sent == "config/batchWrite")
        );
    }

    #[test]
    fn trust_is_asked_only_for_hooks_of_bilbos_plugin() {
        let before = json_of(fixture("hooks-other-plugins.json"));
        let after =
            json_of(&fixture("hooks-other-plugins.json").replace("\"untrusted\"", "\"trusted\""));
        let script = Script::new(vec![Ok(json!({})), Ok(before), Ok(json!({})), Ok(after)]);
        let mut hooks = over(script, fixture("hooks-declared.json"));
        let line = trust("codex-hooks", &mut hooks);
        assert_eq!(
            (line.status, line.detail.as_str()),
            (Status::Installed, "Codex trusts 2 hooks of bilbo@bilbo")
        );
        assert_eq!(
            hooks.calls.sent,
            [
                "initialize",
                "notice initialized",
                "hooks/list",
                "config/batchWrite",
                "hooks/list"
            ]
        );
        assert_eq!(
            hooks.calls.params[3],
            Some(json!({
                "edits": [{
                    "keyPath": "hooks.state",
                    "value": {
                        KP: {"trusted_hash": "sha256:p1"},
                        KS: {"trusted_hash": "sha256:s1"}
                    },
                    "mergeStrategy": "upsert"
                }],
                "reloadUserConfig": false
            }))
        );
    }

    fn reply() -> Value {
        json_of(fixture("hooks-untrusted.json"))
    }

    fn listed(reply: &Value) -> Result<Vec<Hook>, String> {
        read_hooks(
            fixture("hooks-declared.json").as_bytes(),
            Path::new(FILE),
            reply,
        )
    }

    fn hooks_of(reply: &mut Value) -> &mut Vec<Value> {
        reply["data"][0]["hooks"].as_array_mut().expect("hooks")
    }

    #[test]
    fn the_hooks_codex_lists_must_be_the_plugins() {
        assert!(listed(&reply()).is_ok());
        let unread = "codex app-server answered hooks/list with what bilbo does not read";
        let broken: Vec<(&str, Value)> = vec![
            ("data is an object", json!({"data": {}})),
            ("an item without hooks", json!({"data": [{"cwd": "/x"}]})),
            ("hooks is a text", json!({"data": [{"hooks": "none"}]})),
            ("no data", json!({})),
        ];
        for (name, reply) in broken {
            assert_eq!(listed(&reply), Err(unread.to_owned()), "{name}");
        }
        let mut yes = reply();
        hooks_of(&mut yes)[0]["enabled"] = json!("yes");
        assert_eq!(listed(&yes), Err(unread.to_owned()));
        for answer in [json!({"data": {}}), json!({"data": [{"hooks": ["none"]}]})] {
            let script = Script::new(vec![Ok(json!({})), Ok(answer)]);
            let mut hooks = over(script, fixture("hooks-declared.json"));
            let line = trust("codex-hooks", &mut hooks);
            assert_eq!(
                (line.status, line.detail.as_str()),
                (Status::Failed, unread)
            );
            assert!(
                !hooks
                    .calls
                    .sent
                    .iter()
                    .any(|sent| sent == "config/batchWrite")
            );
        }
        let cases: Vec<(&str, Change, &str)> = vec![
            (
                "no key",
                Box::new(|r| {
                    hooks_of(r)[0]
                        .as_object_mut()
                        .expect("object")
                        .remove("key");
                }),
                "Codex listed a hook of bilbo@bilbo without key",
            ),
            (
                "a key naming another plugin",
                Box::new(|r| hooks_of(r)[0]["key"] = json!("other@bilbo:hooks/hooks.json:x:0:0")),
                "Codex lists the hook other@bilbo:hooks/hooks.json:x:0:0 for bilbo@bilbo, and its key names another plugin",
            ),
            (
                "a key naming another plugin after a hook under another id",
                Box::new(|r| {
                    hooks_of(r)[0]["pluginId"] = json!("other@market");
                    hooks_of(r)[1]["key"] = json!("other@bilbo:x");
                }),
                "Codex lists the hook other@bilbo:x for bilbo@bilbo, and its key names another plugin",
            ),
            (
                "another plugin id",
                Box::new(|r| hooks_of(r)[1]["pluginId"] = json!("other@market")),
                "Codex lists the hook bilbo@bilbo:hooks/hooks.json:session_start:0:0 under the plugin other@market",
            ),
            (
                "one key twice",
                Box::new(|r| hooks_of(r)[1]["key"] = json!(KP)),
                "Codex lists two hooks with the key bilbo@bilbo:hooks/hooks.json:user_prompt_submit:0:0",
            ),
            (
                "an undeclared event",
                Box::new(|r| hooks_of(r)[1]["eventName"] = json!("stop")),
                "Codex lists a hook of bilbo@bilbo for stop, which the plugin does not declare",
            ),
            (
                "the prompt hook missing",
                Box::new(|r| {
                    hooks_of(r).remove(0);
                }),
                "Codex lists no hook of bilbo@bilbo for userPromptSubmit",
            ),
            (
                "two hooks for one event",
                Box::new(|r| {
                    let again = hooks_of(r)[0].clone();
                    let mut again = again;
                    again["key"] = json!("bilbo@bilbo:hooks/hooks.json:user_prompt_submit:0:1");
                    hooks_of(r).push(again);
                }),
                "Codex lists 2 hooks of bilbo@bilbo for userPromptSubmit, where the plugin has 1",
            ),
        ];
        for (name, change, sentence) in cases {
            let mut reply = reply();
            change(&mut reply);
            assert_eq!(listed(&reply), Err(sentence.to_owned()), "{name}");
        }
        let file = "/world/pkg/plugins/bilbo/hooks/hooks.json";
        let not_read = Err(format!(
            "cannot read {file}: it is not the hooks file bilbo reads"
        ));
        let reply = reply();
        assert_eq!(read_hooks(b"not json", Path::new(FILE), &reply), not_read);
        assert_eq!(
            read_hooks(
                fixture("hooks-declared-fifth-field.json").as_bytes(),
                Path::new(FILE),
                &reply
            ),
            not_read
        );
        let mut two_for_one_event = reply.clone();
        hooks_of(&mut two_for_one_event)[0]["eventName"] = json!("sessionStart");
        hooks_of(&mut two_for_one_event)[0]["key"] =
            json!("bilbo@bilbo:hooks/hooks.json:session_start:0:1");
        assert_eq!(
            listed(&two_for_one_event),
            Err("Codex lists no hook of bilbo@bilbo for userPromptSubmit".to_owned())
        );
        let script = Script::new(vec![Ok(json!({})), Ok(two_for_one_event)]);
        let mut hooks = over(script, fixture("hooks-declared.json"));
        let line = trust("codex-hooks", &mut hooks);
        assert_eq!(
            (line.status, line.detail.as_str()),
            (
                Status::Failed,
                "Codex lists no hook of bilbo@bilbo for userPromptSubmit"
            )
        );
        assert!(
            !hooks
                .calls
                .sent
                .iter()
                .any(|sent| sent == "config/batchWrite")
        );
        let broken = json!({"data": [{"hooks": []}]});
        let script = Script::new(vec![Ok(json!({})), Ok(broken)]);
        let mut hooks = over(script, fixture("hooks-declared.json"));
        let line = trust("codex-hooks", &mut hooks);
        assert_eq!(line.status, Status::Failed);
        assert!(
            !hooks
                .calls
                .sent
                .iter()
                .any(|sent| sent == "config/batchWrite")
        );
    }

    #[test]
    fn a_hook_with_the_right_event_and_another_command_is_not_trusted() {
        let cases: Vec<(&str, Change, String)> = vec![
            (
                "an older command",
                Box::new(|r| hooks_of(r)[0]["command"] = json!("bilbo digest")),
                format!("Codex lists the hook {KP} with another command than the plugin declares"),
            ),
            (
                "no matcher",
                Box::new(|r| hooks_of(r)[1]["matcher"] = Value::Null),
                format!("Codex lists the hook {KS} with another matcher than the plugin declares"),
            ),
            (
                "a timeout of 5",
                Box::new(|r| hooks_of(r)[0]["timeoutSec"] = json!(5)),
                format!("Codex lists the hook {KP} with another timeout than the plugin declares"),
            ),
            (
                "no command type",
                Box::new(|r| hooks_of(r)[0]["handlerType"] = json!("prompt")),
                format!(
                    "Codex lists the hook {KP} with another handler type than the plugin declares"
                ),
            ),
            (
                "async",
                Box::new(|r| hooks_of(r)[0]["async"] = json!(true)),
                format!("Codex lists the hook {KP} with another async than the plugin declares"),
            ),
            (
                "no async",
                Box::new(|r| {
                    hooks_of(r)[0]
                        .as_object_mut()
                        .expect("object")
                        .remove("async");
                }),
                "Codex listed a hook of bilbo@bilbo without async".to_owned(),
            ),
            (
                "a context limit",
                Box::new(|r| hooks_of(r)[0]["additionalContextLimit"] = json!(200)),
                format!(
                    "Codex lists the hook {KP} with another context limit than the plugin declares"
                ),
            ),
            (
                "no context limit field",
                Box::new(|r| {
                    hooks_of(r)[0]
                        .as_object_mut()
                        .expect("object")
                        .remove("additionalContextLimit");
                }),
                "Codex listed a hook of bilbo@bilbo without additionalContextLimit".to_owned(),
            ),
            (
                "a matcher of 42",
                Box::new(|r| hooks_of(r)[0]["matcher"] = json!(42)),
                format!("Codex lists the hook {KP} with another matcher than the plugin declares"),
            ),
            (
                "no matcher field",
                Box::new(|r| {
                    hooks_of(r)[0]
                        .as_object_mut()
                        .expect("object")
                        .remove("matcher");
                }),
                format!("Codex lists the hook {KP} with another matcher than the plugin declares"),
            ),
            (
                "a null command",
                Box::new(|r| hooks_of(r)[0]["command"] = Value::Null),
                format!("Codex lists the hook {KP} with another command than the plugin declares"),
            ),
            (
                "a null async",
                Box::new(|r| hooks_of(r)[0]["async"] = Value::Null),
                format!("Codex lists the hook {KP} with another async than the plugin declares"),
            ),
            (
                "turned off",
                Box::new(|r| hooks_of(r)[1]["enabled"] = json!(false)),
                format!("Codex lists the hook {KS} turned off"),
            ),
        ];
        let mut after =
            json_of(&fixture("hooks-untrusted.json").replace("\"untrusted\"", "\"trusted\""));
        hooks_of(&mut after)[0]["matcher"] = json!(42);
        let script = Script::new(vec![Ok(json!({})), Ok(reply()), Ok(json!({})), Ok(after)]);
        let mut hooks = over(script, fixture("hooks-declared.json"));
        let line = trust("codex-hooks", &mut hooks);
        assert_eq!(
            (line.status, line.detail),
            (
                Status::Failed,
                format!("Codex lists the hook {KP} with another matcher than the plugin declares")
            )
        );
        for (name, change, sentence) in cases {
            let mut reply = reply();
            change(&mut reply);
            let script = Script::new(vec![Ok(json!({})), Ok(reply)]);
            let mut hooks = over(script, fixture("hooks-declared.json"));
            let line = trust("codex-hooks", &mut hooks);
            assert_eq!(
                (line.status, line.detail),
                (Status::Failed, sentence),
                "{name}"
            );
            assert!(
                !hooks
                    .calls
                    .sent
                    .iter()
                    .any(|sent| sent == "config/batchWrite"),
                "{name}"
            );
        }
        for (name, change, sentence) in [
            (
                "async after the write",
                Box::new(|r: &mut Value| hooks_of(r)[0]["async"] = json!(true)) as Change,
                format!("Codex lists the hook {KP} with another async than the plugin declares"),
            ),
            (
                "a limit after the write",
                Box::new(|r: &mut Value| hooks_of(r)[0]["additionalContextLimit"] = json!(200)),
                format!(
                    "Codex lists the hook {KP} with another context limit than the plugin declares"
                ),
            ),
        ] {
            let mut after =
                json_of(&fixture("hooks-untrusted.json").replace("\"untrusted\"", "\"trusted\""));
            change(&mut after);
            let script = Script::new(vec![Ok(json!({})), Ok(reply()), Ok(json!({})), Ok(after)]);
            let mut hooks = over(script, fixture("hooks-declared.json"));
            let line = trust("codex-hooks", &mut hooks);
            assert_eq!(
                (line.status, line.detail),
                (Status::Failed, sentence),
                "{name}"
            );
        }
    }

    #[test]
    fn the_peers_failures_are_said_in_the_words_of_the_plan() {
        let says = |fault| fault_sentence(fault, "30s");
        let method = || "hooks/list".to_owned();
        assert_eq!(
            says(Fault::NotTaken { method: method() }),
            "codex app-server did not take hooks/list within 30s"
        );
        assert_eq!(
            says(Fault::Silent { method: method() }),
            "codex app-server did not answer hooks/list within 30s"
        );
        assert_eq!(
            says(Fault::Ended {
                method: method(),
                line: Some("boom".to_owned())
            }),
            "codex app-server ended before answering hooks/list: boom"
        );
        assert_eq!(
            says(Fault::Ended {
                method: method(),
                line: None
            }),
            "codex app-server ended before answering hooks/list"
        );
        assert_eq!(
            says(Fault::Answered {
                method: method(),
                message: "no".to_owned()
            }),
            "codex app-server answered hooks/list: no"
        );
    }

    fn config(state: &Value) -> Value {
        json!({"config": {"hooks": {"state": state}}})
    }

    #[test]
    fn codexs_trust_entries_are_the_keys_that_begin_with_the_plugins_id() {
        let script = Script::new(vec![
            Ok(json!({})),
            Ok(config(&json!({
                "bilbo@bilbo:hooks/hooks.json:session_start:0:0": {"trusted_hash": "a"},
                "bilbo@bilbo:hooks/hooks.json:user_prompt_submit:0:0": {"trusted_hash": "b"},
                "bilbo@other:hooks/hooks.json:session_start:0:0": {"trusted_hash": "c"},
                "/home/me/.codex/hooks.json:session_start:0:0": {"trusted_hash": "d"}
            }))),
            Ok(json!({})),
            Ok(config(&json!({"bilbo@other:x": {"trusted_hash": "c"}}))),
        ]);
        let mut hooks = over(script, "{}");
        let first = hooks.kept().expect("keys");
        assert_eq!(
            first,
            [
                "bilbo@bilbo:hooks/hooks.json:session_start:0:0",
                "bilbo@bilbo:hooks/hooks.json:user_prompt_submit:0:0"
            ]
        );
        hooks.forget(&first).expect("written");
        assert!(hooks.kept().expect("keys").is_empty());
        let sent = &hooks.calls.sent;
        assert_eq!(
            sent,
            &[
                "initialize",
                "notice initialized",
                "config/read",
                "config/batchWrite",
                "config/read"
            ]
        );
        assert_eq!(hooks.calls.params[2], Some(json!({"includeLayers": false})));
        let write = hooks.calls.params[3].clone().expect("params");
        assert_eq!(write["reloadUserConfig"], json!(false));
        assert_eq!(
            write["edits"],
            json!([
                {"keyPath": "hooks.state.\"bilbo@bilbo:hooks/hooks.json:session_start:0:0\"", "value": null, "mergeStrategy": "replace"},
                {"keyPath": "hooks.state.\"bilbo@bilbo:hooks/hooks.json:user_prompt_submit:0:0\"", "value": null, "mergeStrategy": "replace"}
            ])
        );
    }

    #[test]
    fn a_trust_key_is_one_component_of_the_path_whatever_it_holds() {
        let odd = [
            "bilbo@bilbo:a.b.c",
            "bilbo@bilbo:with \"quote\"",
            "bilbo@bilbo:back\\slash",
        ];
        let script = Script::new(vec![
            Ok(json!({})),
            Ok(config(&json!({
                odd[0]: {"trusted_hash": "a"},
                odd[1]: {"trusted_hash": "b"},
                odd[2]: {"trusted_hash": "c"}
            }))),
            Ok(json!({})),
            Ok(config(&json!({}))),
        ]);
        let mut hooks = over(script, "{}");
        let line = forget("codex-hooks", &mut hooks);
        assert_eq!(line.status, Status::Removed, "{}", line.detail);
        let write = hooks.calls.params[3].clone().expect("params");
        assert_eq!(write["reloadUserConfig"], json!(false));
        assert_eq!(
            write["edits"],
            json!([
                {"keyPath": "hooks.state.\"bilbo@bilbo:a.b.c\"", "value": null, "mergeStrategy": "replace"},
                {"keyPath": "hooks.state.\"bilbo@bilbo:back\\\\slash\"", "value": null, "mergeStrategy": "replace"},
                {"keyPath": "hooks.state.\"bilbo@bilbo:with \\\"quote\\\"\"", "value": null, "mergeStrategy": "replace"}
            ])
        );
    }

    #[test]
    fn a_trust_table_of_null_fails_on_either_read() {
        let said = "codex app-server answered config/read with what bilbo does not read";
        let script = Script::new(vec![
            Ok(json!({})),
            Ok(json!({"config": {"hooks": {"state": null}}})),
        ]);
        let mut hooks = over(script, "{}");
        let line = forget("codex-hooks", &mut hooks);
        assert_eq!((line.status, line.detail.as_str()), (Status::Failed, said));
        assert!(
            !hooks
                .calls
                .sent
                .iter()
                .any(|sent| sent == "config/batchWrite"),
            "nothing is written after a null table"
        );

        let script = Script::new(vec![
            Ok(json!({})),
            Ok(config(&json!({"bilbo@bilbo:a": {"trusted_hash": "a"}}))),
            Ok(json!({})),
            Ok(json!({"config": {"hooks": {"state": null}}})),
        ]);
        let mut hooks = over(script, "{}");
        let line = forget("codex-hooks", &mut hooks);
        assert_eq!((line.status, line.detail.as_str()), (Status::Failed, said));

        for reply in [json!({"config": {}}), json!({"config": {"hooks": {}}})] {
            let script = Script::new(vec![Ok(json!({})), Ok(reply)]);
            let mut hooks = over(script, "{}");
            assert_eq!(forget("codex-hooks", &mut hooks).status, Status::Absent);
        }
    }

    #[test]
    fn a_config_reply_bilbo_does_not_read_is_said() {
        let said = "codex app-server answered config/read with what bilbo does not read";
        for reply in [
            json!({}),
            json!({"config": []}),
            json!({"config": {"hooks": []}}),
            json!({"config": {"hooks": {"state": []}}}),
            json!({"config": {"hooks": {"state": null}}}),
        ] {
            assert_eq!(read_keys(&reply), Err(said.to_owned()), "{reply}");
        }
        assert_eq!(read_keys(&json!({"config": {}})), Ok(Vec::new()));
        assert_eq!(read_keys(&json!({"config": {"hooks": {}}})), Ok(Vec::new()));
    }

    #[test]
    fn codexs_removal_look_reads_of_each_item_only_what_names_it() {
        assert_eq!(
            held_marketplace(fixture("market-name-only.json").as_bytes()),
            Some(true)
        );
        assert_eq!(
            held_plugins(fixture("installed-no-version.json").as_bytes()),
            Some((true, None))
        );
        assert_eq!(
            held_plugins(fixture("installed-off.json").as_bytes()),
            Some((true, None))
        );
        assert_eq!(
            held_plugins(fixture("installed-other-plugin.json").as_bytes()),
            Some((
                true,
                Some(Other {
                    id: "other@bilbo".to_owned(),
                    scope: None
                })
            ))
        );
        assert_eq!(
            held_plugins(fixture("installed-no-id.json").as_bytes()),
            None
        );
        assert_eq!(
            held_plugins(fixture("installed-then-no-market.json").as_bytes()),
            None
        );
    }
}
