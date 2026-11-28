use std::fs;
use std::path::{Path, PathBuf};

use bilbo_settings::edit::read_regular;
use serde_json::Value;

use crate::keys::waited;
use crate::plugin::{
    Act, Agent, Canon, Held, Installed, Marketplace, NOT_JSON, Other, PACKAGE, PLUGIN_ID, Package,
    Seen, Stale, Trusting, ended_badly, message, unreadable,
};
use crate::program::{self, Bounds, End, Ran};

pub const FILE: &str = ".claude-plugin/marketplace.json";

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

pub struct Claude {
    program: PathBuf,
    bounds: Bounds,
}

impl Claude {
    pub fn new(program: PathBuf, bounds: Bounds) -> Claude {
        Claude { program, bounds }
    }

    fn call(&self, args: &[&str]) -> Ran {
        program::run(&self.program, args, self.bounds)
    }
}

fn text(item: &Value, field: &str) -> Option<String> {
    item.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn same_folder(listed: &str, wanted: &Path, canon: &Canon) -> bool {
    canon(Path::new(listed)).is_some_and(|folder| folder == wanted)
}

fn array(bytes: &[u8]) -> Option<Vec<Value>> {
    match serde_json::from_slice::<Value>(bytes).ok()? {
        Value::Array(items) => Some(items),
        _ => None,
    }
}

pub fn read_marketplaces(bytes: &[u8], package: &Path, canon: &Canon) -> Option<Marketplace> {
    let mut found = Marketplace::None;
    for item in array(bytes)? {
        let name = text(&item, "name")?;
        if name != "bilbo" {
            continue;
        }
        let source = text(&item, "source")?;
        found = if source == "directory" {
            let path = text(&item, "path")?;
            if same_folder(&path, package, canon) {
                Marketplace::Here
            } else {
                Marketplace::Elsewhere(path)
            }
        } else {
            Marketplace::Elsewhere(source)
        };
    }
    Some(found)
}

pub fn held_marketplace(bytes: &[u8]) -> Option<bool> {
    let mut listed = false;
    for item in array(bytes)? {
        listed |= text(&item, "name")? == "bilbo";
    }
    Some(listed)
}

pub fn held_plugins(bytes: &[u8]) -> Option<(bool, Option<Other>)> {
    let (mut plugin, mut other) = (false, None);
    for item in array(bytes)? {
        let id = text(&item, "id")?;
        let scope = text(&item, "scope")?;
        if id == PLUGIN_ID && scope == "user" {
            plugin = true;
        } else if id.ends_with("@bilbo") && other.is_none() {
            other = Some(Other {
                id,
                scope: Some(scope),
            });
        }
    }
    Some((plugin, other))
}

pub fn read_plugins(
    bytes: &[u8],
    wanted: &Path,
    canon: &Canon,
) -> Option<(Option<Installed>, Option<Other>)> {
    let mut plugin = None;
    let mut other = None;
    for item in array(bytes)? {
        let id = text(&item, "id")?;
        let scope = text(&item, "scope")?;
        if id == PLUGIN_ID && scope == "user" {
            let on = item.get("enabled").and_then(Value::as_bool)?;
            let stale = match item.get("readFromFolder") {
                None => Some(Stale::NoFolder),
                Some(Value::String(read)) if same_folder(read, wanted, canon) => None,
                Some(Value::String(read)) => Some(Stale::ReadFrom(read.clone())),
                Some(_) => return None,
            };
            plugin = plugin.or(Some(Installed { on, stale }));
        } else if id.ends_with("@bilbo") && other.is_none() {
            other = Some(Other {
                id,
                scope: Some(scope),
            });
        }
    }
    Some((plugin, other))
}

fn outcome(ran: &Ran) -> Option<bool> {
    let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(&ran.stdout) else {
        return None;
    };
    object
        .get("outcome")
        .and_then(Value::as_str)
        .map(|said| said == "ok")
}

fn own_message(ran: &Ran) -> Option<String> {
    let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(&ran.stdout) else {
        return None;
    };
    object
        .get("message")
        .and_then(Value::as_str)
        .and_then(|text| text.lines().find(|line| !line.trim().is_empty()))
        .map(|line| line.trim().to_owned())
}

pub fn worked(ran: &Ran) -> bool {
    match (&ran.end, outcome(ran)) {
        (End::Exited(_), Some(ok)) => ok,
        (End::Exited(code), None) => *code == 0,
        _ => false,
    }
}

pub fn said(ran: &Ran, program: &Path, waited: &str) -> String {
    message(ran, own_message(ran), program, waited)
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
    if item.get("source").and_then(Value::as_str) != Some(format!("./{PACKAGE}").as_str()) {
        return Err(no(&format!(
            "takes the plugin bilbo from another folder than ./{PACKAGE}"
        )));
    }
    match item.get("dependencies") {
        None => Ok(()),
        Some(Value::Array(list)) if list.is_empty() => Ok(()),
        Some(Value::Array(_)) => Err(no(
            "makes the plugin bilbo depend on other plugins, which bilbo setup installs none of",
        )),
        Some(_) => Err(no("is not the manifest bilbo reads")),
    }
}

const LIST: [&str; 4] = ["plugin", "marketplace", "list", "--json"];
const PLUGINS: [&str; 3] = ["plugin", "list", "--json"];

impl Claude {
    fn listed(&self, args: &[&str]) -> Result<Vec<u8>, String> {
        let words = format!("claude {}", args.join(" "));
        let ran = self.call(args);
        match ended_badly(&ran, &self.program, &waited(self.bounds.wait)) {
            None => Ok(ran.stdout),
            Some(why) => Err(unreadable(&words, &why)),
        }
    }
}

impl Agent for Claude {
    fn step(&self) -> &'static str {
        "claude"
    }

    fn name(&self) -> &'static str {
        "claude"
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(format!(
                "no plugin at {}: it holds no {FILE}",
                folder.display()
            )),
            Err(error) => Err(format!("cannot read {}: {error}", path.display())),
        }
    }

    fn look(&self, folder: &Path) -> Result<Seen, String> {
        let package = resolve(folder)?;
        let words = |args: &[&str]| format!("claude {}", args.join(" "));
        let market = self.listed(&LIST)?;
        let marketplace = read_marketplaces(&market, &package.folder, &system_canon)
            .ok_or_else(|| unreadable(&words(&LIST), NOT_JSON))?;
        let plugins = self.listed(&PLUGINS)?;
        let (plugin, other) = read_plugins(&plugins, &package.plugin, &system_canon)
            .ok_or_else(|| unreadable(&words(&PLUGINS), NOT_JSON))?;
        Ok(Seen {
            marketplace,
            plugin,
            other,
        })
    }

    fn run(&self, act: Act, folder: &Path) -> Result<(), String> {
        let words = self.words(act, folder);
        let Some(args) = Claude::arguments(act, folder) else {
            return Err(format!(
                "claude {words}: the plugin folder is not UTF-8 text"
            ));
        };
        let ran = self.call(&args.iter().map(String::as_str).collect::<Vec<_>>());
        if worked(&ran) {
            return Ok(());
        }
        Err(format!(
            "claude {words}: {}",
            said(&ran, &self.program, &waited(self.bounds.wait))
        ))
    }

    fn words(&self, act: Act, folder: &Path) -> String {
        match Claude::arguments(act, folder) {
            Some(args) => args.join(" "),
            None => String::new(),
        }
    }

    fn hooks(&self, _folder: &Path) -> Option<Trusting> {
        None
    }

    fn held(&self) -> Result<Held, String> {
        let words = |args: &[&str]| format!("claude {}", args.join(" "));
        let market = self.listed(&LIST)?;
        let marketplace =
            held_marketplace(&market).ok_or_else(|| unreadable(&words(&LIST), NOT_JSON))?;
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

impl Claude {
    fn arguments(act: Act, folder: &Path) -> Option<Vec<String>> {
        let words: Vec<&str> = match act {
            Act::Uninstall => vec!["plugin", "uninstall", PLUGIN_ID, "--json"],
            Act::Remove => vec!["plugin", "marketplace", "remove", "bilbo", "--json"],
            Act::Install => vec!["plugin", "install", PLUGIN_ID, "--json"],
            Act::Add => {
                return Some(vec![
                    "plugin".to_owned(),
                    "marketplace".to_owned(),
                    "add".to_owned(),
                    folder.to_str()?.to_owned(),
                    "--json".to_owned(),
                ]);
            }
        };
        Some(words.into_iter().map(str::to_owned).collect())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        held_marketplace, held_plugins, read_manifest, read_marketplaces, read_plugins, worked,
    };
    use crate::plugin::{Installed, Marketplace, Other, Stale};
    use crate::program::{End, Ran};

    fn fixture(name: &str) -> &'static str {
        match name {
            "manifest-depends.json" => include_str!("../fixtures/claude/manifest-depends.json"),
            "manifest-null-depends.json" => {
                include_str!("../fixtures/claude/manifest-null-depends.json")
            }
            "manifest-text-depends.json" => {
                include_str!("../fixtures/claude/manifest-text-depends.json")
            }
            "manifest-empty-depends.json" => {
                include_str!("../fixtures/claude/manifest-empty-depends.json")
            }
            "manifest-no-bilbo.json" => include_str!("../fixtures/claude/manifest-no-bilbo.json"),
            "manifest-other-first.json" => {
                include_str!("../fixtures/claude/manifest-other-first.json")
            }
            "manifest-other-source.json" => {
                include_str!("../fixtures/claude/manifest-other-source.json")
            }
            "manifest-personal.json" => include_str!("../fixtures/claude/manifest-personal.json"),
            "manifest-plugins-text.json" => {
                include_str!("../fixtures/claude/manifest-plugins-text.json")
            }
            "manifest.json" => include_str!("../fixtures/claude/manifest.json"),
            "market-elsewhere.json" => include_str!("../fixtures/claude/market-elsewhere.json"),
            "market-github.json" => include_str!("../fixtures/claude/market-github.json"),
            "market-here.json" => include_str!("../fixtures/claude/market-here.json"),
            "market-name-only.json" => include_str!("../fixtures/claude/market-name-only.json"),
            "plugins-id-scope-only.json" => {
                include_str!("../fixtures/claude/plugins-id-scope-only.json")
            }
            "plugins-then-no-scope.json" => {
                include_str!("../fixtures/claude/plugins-then-no-scope.json")
            }
            "market-no-name.json" => include_str!("../fixtures/claude/market-no-name.json"),
            "market-no-path.json" => include_str!("../fixtures/claude/market-no-path.json"),
            "market-none.json" => include_str!("../fixtures/claude/market-none.json"),
            "market-other-name.json" => include_str!("../fixtures/claude/market-other-name.json"),
            "not-json.txt" => include_str!("../fixtures/claude/not-json.txt"),
            "object.json" => include_str!("../fixtures/claude/object.json"),
            "plugins-enabled-text.json" => {
                include_str!("../fixtures/claude/plugins-enabled-text.json")
            }
            "plugins-no-enabled.json" => include_str!("../fixtures/claude/plugins-no-enabled.json"),
            "plugins-no-id.json" => include_str!("../fixtures/claude/plugins-no-id.json"),
            "plugins-no-scope.json" => include_str!("../fixtures/claude/plugins-no-scope.json"),
            "plugins-none.json" => include_str!("../fixtures/claude/plugins-none.json"),
            "plugins-off.json" => include_str!("../fixtures/claude/plugins-off.json"),
            "plugins-on-elsewhere.json" => {
                include_str!("../fixtures/claude/plugins-on-elsewhere.json")
            }
            "plugins-on-here.json" => include_str!("../fixtures/claude/plugins-on-here.json"),
            "plugins-on-no-folder.json" => {
                include_str!("../fixtures/claude/plugins-on-no-folder.json")
            }
            "plugins-other-plugin.json" => {
                include_str!("../fixtures/claude/plugins-other-plugin.json")
            }
            "plugins-project-and-user.json" => {
                include_str!("../fixtures/claude/plugins-project-and-user.json")
            }
            "plugins-project-only.json" => {
                include_str!("../fixtures/claude/plugins-project-only.json")
            }
            other => panic!("no fixture {other}"),
        }
    }

    const PKG: &str = "/world/pkg";

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

    fn market(bytes: &[u8]) -> Option<Marketplace> {
        read_marketplaces(bytes, Path::new(PKG), &table)
    }

    fn plugins(bytes: &[u8]) -> Option<(Option<Installed>, Option<Other>)> {
        read_plugins(bytes, Path::new("/world/pkg/plugins/bilbo"), &table)
    }

    #[test]
    fn claudes_own_message_is_read_whatever_its_outcome() {
        let program = Path::new("/p/claude");
        let said = |end: End, stdout: &str, stderr: &str| {
            let ran = Ran {
                end,
                stdout: stdout.as_bytes().to_vec(),
                stderr: stderr.as_bytes().to_vec(),
            };
            (worked(&ran), super::said(&ran, program, "30s"))
        };
        assert_eq!(
            said(End::Exited(1), r#"{"message":"disk full"}"#, ""),
            (false, "disk full".to_owned())
        );
        assert_eq!(
            said(
                End::Exited(0),
                r#"{"outcome":"failed","message":"\n  no room\nmore"}"#,
                ""
            ),
            (false, "no room".to_owned())
        );
        assert_eq!(
            said(End::Exited(1), r#"{"message":42}"#, "boom"),
            (false, "boom".to_owned())
        );
        assert_eq!(
            said(
                End::NoExitCode,
                r#"{"outcome":"failed","message":"half done"}"#,
                ""
            ),
            (false, "half done".to_owned())
        );
    }

    fn current() -> Option<Installed> {
        Some(Installed {
            on: true,
            stale: None,
        })
    }

    #[test]
    fn claudes_lists_are_read_as_it_prints_them() {
        assert_eq!(
            market(fixture("market-none.json").as_bytes()),
            Some(Marketplace::None)
        );
        assert_eq!(
            market(fixture("market-here.json").as_bytes()),
            Some(Marketplace::Here)
        );
        assert_eq!(
            market(fixture("market-elsewhere.json").as_bytes()),
            Some(Marketplace::Elsewhere("/world/old".to_owned()))
        );
        assert_eq!(
            market(fixture("market-github.json").as_bytes()),
            Some(Marketplace::Elsewhere("github".to_owned()))
        );
        assert_eq!(
            market(fixture("market-other-name.json").as_bytes()),
            Some(Marketplace::None)
        );
        assert_eq!(
            plugins(fixture("plugins-none.json").as_bytes()),
            Some((None, None))
        );
        assert_eq!(
            plugins(fixture("plugins-on-here.json").as_bytes()),
            Some((current(), None))
        );
        assert_eq!(
            plugins(fixture("plugins-on-elsewhere.json").as_bytes()),
            Some((
                Some(Installed {
                    on: true,
                    stale: Some(Stale::ReadFrom("/world/old/plugins/bilbo".to_owned())),
                }),
                None
            ))
        );
        assert_eq!(
            plugins(fixture("plugins-on-no-folder.json").as_bytes()),
            Some((
                Some(Installed {
                    on: true,
                    stale: Some(Stale::NoFolder),
                }),
                None
            ))
        );
        assert_eq!(
            plugins(fixture("plugins-off.json").as_bytes()),
            Some((
                Some(Installed {
                    on: false,
                    stale: None,
                }),
                None
            ))
        );
        let here = fixture("market-here.json");
        let by_link = here.replace("/world/pkg", "/world/link");
        assert_eq!(market(by_link.as_bytes()), Some(Marketplace::Here));
        let gone = here.replace("/world/pkg", "/world/gone");
        assert_eq!(
            market(gone.as_bytes()),
            Some(Marketplace::Elsewhere("/world/gone".to_owned()))
        );
        let linked = fixture("plugins-on-here.json").replace("/world/pkg", "/world/link");
        assert_eq!(plugins(linked.as_bytes()), Some((current(), None)));
        assert_eq!(
            read_marketplaces(here.as_bytes(), Path::new(PKG), &nothing),
            Some(Marketplace::Elsewhere("/world/pkg".to_owned()))
        );
        let (installed, _) = read_plugins(
            fixture("plugins-on-here.json").as_bytes(),
            Path::new("/world/pkg/plugins/bilbo"),
            &nothing,
        )
        .expect("read");
        assert_eq!(
            installed,
            Some(Installed {
                on: true,
                stale: Some(Stale::ReadFrom("/world/pkg/plugins/bilbo".to_owned()))
            })
        );
    }

    #[test]
    fn a_list_bilbo_cannot_read_is_said_so() {
        for (name, bytes) in [
            ("not json", fixture("not-json.txt").as_bytes()),
            ("an object", fixture("object.json").as_bytes()),
            ("no name", fixture("market-no-name.json").as_bytes()),
            ("no path", fixture("market-no-path.json").as_bytes()),
        ] {
            assert_eq!(market(bytes), None, "{name}");
        }
        for (name, bytes) in [
            ("not json", fixture("not-json.txt").as_bytes()),
            ("an object", fixture("object.json").as_bytes()),
            ("no id", fixture("plugins-no-id.json").as_bytes()),
            ("no scope", fixture("plugins-no-scope.json").as_bytes()),
            ("no enabled", fixture("plugins-no-enabled.json").as_bytes()),
            (
                "enabled as text",
                fixture("plugins-enabled-text.json").as_bytes(),
            ),
        ] {
            assert_eq!(plugins(bytes), None, "{name}");
        }
    }

    #[test]
    fn another_installation_of_the_marketplace_is_seen() {
        let other = |id: &str, scope: &str| {
            Some(Other {
                id: id.to_owned(),
                scope: Some(scope.to_owned()),
            })
        };
        assert_eq!(
            plugins(fixture("plugins-other-plugin.json").as_bytes()),
            Some((current(), other("other@bilbo", "user")))
        );
        assert_eq!(
            plugins(fixture("plugins-project-only.json").as_bytes()),
            Some((None, other("bilbo@bilbo", "project")))
        );
        assert_eq!(
            plugins(fixture("plugins-project-and-user.json").as_bytes()),
            Some((current(), other("bilbo@bilbo", "project")))
        );
    }

    fn ran(end: End, stdout: &str) -> Ran {
        Ran {
            end,
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn a_claude_command_is_judged_by_its_outcome_then_by_its_exit_code() {
        assert!(worked(&ran(End::Exited(1), r#"{"outcome":"ok"}"#)));
        assert!(!worked(&ran(End::Exited(0), r#"{"outcome":"failed"}"#)));
        assert!(worked(&ran(End::Exited(0), "no json")));
        assert!(!worked(&ran(End::Exited(1), "no json")));
        assert!(worked(&ran(End::Exited(0), r#"{"outcome":3}"#)));
        assert!(!worked(&ran(End::NoExitCode, r#"{"outcome":"ok"}"#)));
        assert!(!worked(&ran(End::StillRunning, "")));
    }

    #[test]
    fn the_file_a_tool_installs_from_must_name_bilbo() {
        let folder = Path::new(PKG);
        let said = |bytes: &[u8]| read_manifest(bytes, folder);
        assert_eq!(said(fixture("manifest.json").as_bytes()), Ok(()));
        assert_eq!(
            said(fixture("manifest-other-first.json").as_bytes()),
            Ok(())
        );
        assert_eq!(
            said(fixture("manifest-empty-depends.json").as_bytes()),
            Ok(())
        );
        let file = ".claude-plugin/marketplace.json";
        let no = |why: &str| Err(format!("no plugin at {PKG}: {file} {why}"));
        assert_eq!(said(b"not json"), no("is not the manifest bilbo reads"));
        assert_eq!(
            said(fixture("manifest-plugins-text.json").as_bytes()),
            no("is not the manifest bilbo reads")
        );
        assert_eq!(
            said(fixture("manifest-personal.json").as_bytes()),
            no("names the marketplace 'personal', not bilbo")
        );
        assert_eq!(
            said(fixture("manifest-no-bilbo.json").as_bytes()),
            no("names no plugin bilbo")
        );
        assert_eq!(
            said(fixture("manifest-other-source.json").as_bytes()),
            no("takes the plugin bilbo from another folder than ./plugins/bilbo")
        );
        assert_eq!(
            said(fixture("manifest-depends.json").as_bytes()),
            no(
                "makes the plugin bilbo depend on other plugins, which bilbo setup installs none of"
            )
        );
        for name in ["manifest-null-depends.json", "manifest-text-depends.json"] {
            assert_eq!(
                said(fixture(name).as_bytes()),
                no("is not the manifest bilbo reads"),
                "{name}"
            );
        }
    }

    #[test]
    fn claudes_removal_look_reads_of_each_item_only_what_names_it() {
        assert_eq!(
            held_marketplace(fixture("market-name-only.json").as_bytes()),
            Some(true)
        );
        assert_eq!(
            held_marketplace(fixture("market-none.json").as_bytes()),
            Some(false)
        );
        assert_eq!(
            held_plugins(fixture("plugins-id-scope-only.json").as_bytes()),
            Some((true, None))
        );
        assert_eq!(
            held_plugins(fixture("plugins-off.json").as_bytes()),
            Some((true, None))
        );
        assert_eq!(
            held_plugins(fixture("plugins-project-only.json").as_bytes()),
            Some((
                false,
                Some(Other {
                    id: "bilbo@bilbo".to_owned(),
                    scope: Some("project".to_owned())
                })
            ))
        );
        assert_eq!(
            held_marketplace(fixture("market-no-name.json").as_bytes()),
            None
        );
        assert_eq!(
            held_plugins(fixture("plugins-then-no-scope.json").as_bytes()),
            None
        );
    }
}
