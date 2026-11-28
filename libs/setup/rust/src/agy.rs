use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use bilbo_settings::edit::read_regular;
use serde_json::Value;

use crate::keys::waited;
use crate::plugin::{
    Act, Agent, Held, Installed, Marketplace, NOT_JSON, Seen, Stale, Trusting, ended_badly,
    message, unreadable,
};
use crate::program::{self, Bounds, End, Ran};

pub const FILE: &str = "plugins/bilbo/.antigravity-plugin/plugin.json";

pub struct Agy {
    program: PathBuf,
    bounds: Bounds,
}

impl Agy {
    pub fn new(program: PathBuf, bounds: Bounds) -> Agy {
        Agy { program, bounds }
    }

    fn call(&self, args: &[&str]) -> Ran {
        program::run(&self.program, args, self.bounds)
    }

    fn arguments(act: Act, folder: &Path) -> Option<Vec<String>> {
        match act {
            Act::Uninstall => Some(["plugin", "uninstall", "bilbo"].map(str::to_owned).to_vec()),
            Act::Install => Some(vec![
                "plugin".to_owned(),
                "install".to_owned(),
                folder.join("plugins/bilbo").to_str()?.to_owned(),
            ]),
            Act::Add | Act::Remove => None,
        }
    }
}

type Files = BTreeMap<PathBuf, Vec<u8>>;

fn walk(root: &Path) -> Result<Files, (PathBuf, String)> {
    let mut found = BTreeMap::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let folder = root.join(&relative);
        let at = |error: io::Error| (folder.clone(), error.to_string());
        for entry in fs::read_dir(&folder).map_err(at)? {
            let entry = entry.map_err(|error| (folder.clone(), error.to_string()))?;
            let here = relative.join(entry.file_name());
            let path = root.join(&here);
            let kind = entry
                .file_type()
                .map_err(|error| (path.clone(), error.to_string()))?;
            if kind.is_dir() {
                pending.push(here);
            } else if kind.is_file() {
                let bytes =
                    read_regular(&path).map_err(|error| (path.clone(), error.to_string()))?;
                found.insert(here, bytes);
            } else {
                return Err((path, "not a regular file".to_owned()));
            }
        }
    }
    Ok(found)
}

fn package_files(root: &Path) -> Result<Files, String> {
    walk(root).map_err(|(path, error)| format!("cannot read {}: {error}", path.display()))
}

fn copy_matches(copy: &Path, package: &Files) -> bool {
    walk(copy).is_ok_and(|files| files == *package)
}

pub fn same_files(copy: &Path, package: &Path) -> Result<bool, String> {
    let wanted = package_files(package)?;
    Ok(copy_matches(copy, &wanted))
}

#[derive(Debug, PartialEq, Eq)]
pub struct Listed {
    pub on: bool,
    pub path: PathBuf,
}

pub fn read_plugins(bytes: &[u8]) -> Option<Option<Listed>> {
    let Value::Array(items) = serde_json::from_slice::<Value>(bytes).ok()? else {
        return None;
    };
    let mut plugin = None;
    for item in items {
        let name = item.get("name").and_then(Value::as_str)?;
        if name != "bilbo" {
            continue;
        }
        let on = item.get("enabled").and_then(Value::as_bool)?;
        let path = PathBuf::from(item.get("path").and_then(Value::as_str)?);
        plugin = plugin.or(Some(Listed { on, path }));
    }
    Some(plugin)
}

pub fn held_plugin(bytes: &[u8]) -> Option<bool> {
    let Value::Array(items) = serde_json::from_slice::<Value>(bytes).ok()? else {
        return None;
    };
    let mut listed = false;
    for item in items {
        listed |= item.get("name").and_then(Value::as_str)? == "bilbo";
    }
    Some(listed)
}

pub fn read_manifest(bytes: &[u8], folder: &Path) -> Result<(), String> {
    let no = |why: &str| format!("no plugin at {}: {FILE} {why}", folder.display());
    let Ok(Value::Object(file)) = serde_json::from_slice::<Value>(bytes) else {
        return Err(no("is not the manifest bilbo reads"));
    };
    match file.get("name").and_then(Value::as_str) {
        None => Err(no("is not the manifest bilbo reads")),
        Some("bilbo") => Ok(()),
        Some(name) => Err(no(&format!("names the plugin '{name}', not bilbo"))),
    }
}

impl Agent for Agy {
    fn step(&self) -> &'static str {
        "agy"
    }

    fn name(&self) -> &'static str {
        "agy"
    }

    fn id(&self) -> &'static str {
        "bilbo"
    }

    fn manifest(&self, folder: &Path) -> Result<(), String> {
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
        const ARGS: [&str; 3] = ["plugin", "list", "--json"];
        let package = package_files(&folder.join("plugins/bilbo"))?;
        let words = format!("agy {}", ARGS.join(" "));
        let ran = self.call(&ARGS);
        if let Some(why) = ended_badly(&ran, &self.program, &waited(self.bounds.wait)) {
            return Err(unreadable(&words, &why));
        }
        let listed = read_plugins(&ran.stdout).ok_or_else(|| unreadable(&words, NOT_JSON))?;
        let plugin = listed.map(|listed| Installed {
            on: listed.on,
            stale: (!copy_matches(&listed.path, &package)).then_some(Stale::CopyDiffers),
        });
        Ok(Seen {
            marketplace: Marketplace::NotUsed,
            plugin,
            other: None,
        })
    }

    fn run(&self, act: Act, folder: &Path) -> Result<(), String> {
        let words = self.words(act, folder);
        let Some(args) = Agy::arguments(act, folder) else {
            return Err(format!("agy {words}: agy has no marketplace"));
        };
        let ran = self.call(&args.iter().map(String::as_str).collect::<Vec<_>>());
        if matches!(ran.end, End::Exited(0)) {
            return Ok(());
        }
        Err(format!(
            "agy {words}: {}",
            message(&ran, None, &self.program, &waited(self.bounds.wait))
        ))
    }

    fn words(&self, act: Act, folder: &Path) -> String {
        Agy::arguments(act, folder).map_or_else(String::new, |args| args.join(" "))
    }

    fn hooks(&self, _folder: &Path) -> Option<Trusting> {
        None
    }

    fn held(&self) -> Result<Held, String> {
        const ARGS: [&str; 3] = ["plugin", "list", "--json"];
        let words = format!("agy {}", ARGS.join(" "));
        let ran = self.call(&ARGS);
        if let Some(why) = ended_badly(&ran, &self.program, &waited(self.bounds.wait)) {
            return Err(unreadable(&words, &why));
        }
        let listed = held_plugin(&ran.stdout).ok_or_else(|| unreadable(&words, NOT_JSON))?;
        Ok(Held {
            plugin: listed,
            marketplace: false,
            other: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{Listed, held_plugin, read_manifest, read_plugins};

    fn fixture(name: &str) -> &'static str {
        match name {
            "list-enabled-text.json" => include_str!("../fixtures/agy/list-enabled-text.json"),
            "list-name-only.json" => include_str!("../fixtures/agy/list-name-only.json"),
            "list-then-no-name.json" => include_str!("../fixtures/agy/list-then-no-name.json"),
            "list-no-enabled.json" => include_str!("../fixtures/agy/list-no-enabled.json"),
            "list-no-name.json" => include_str!("../fixtures/agy/list-no-name.json"),
            "list-no-path.json" => include_str!("../fixtures/agy/list-no-path.json"),
            "list-object.json" => include_str!("../fixtures/agy/list-object.json"),
            "list-off.json" => include_str!("../fixtures/agy/list-off.json"),
            "list-on.json" => include_str!("../fixtures/agy/list-on.json"),
            "manifest-personal.json" => include_str!("../fixtures/agy/manifest-personal.json"),
            "manifest.json" => include_str!("../fixtures/agy/manifest.json"),
            other => panic!("no fixture {other}"),
        }
    }

    #[test]
    fn agys_list_is_read_as_it_prints_it() {
        assert_eq!(read_plugins(b"[]"), Some(None));
        assert_eq!(
            read_plugins(fixture("list-on.json").as_bytes()),
            Some(Some(Listed {
                on: true,
                path: PathBuf::from("/world/copy")
            }))
        );
        assert_eq!(
            read_plugins(fixture("list-off.json").as_bytes()),
            Some(Some(Listed {
                on: false,
                path: PathBuf::from("/world/copy")
            }))
        );
    }

    #[test]
    fn a_list_bilbo_cannot_read_is_said_so() {
        assert_eq!(read_plugins(b"agy: broke"), None);
        for text in [
            fixture("list-object.json"),
            fixture("list-no-name.json"),
            fixture("list-no-enabled.json"),
            fixture("list-no-path.json"),
            fixture("list-enabled-text.json"),
        ] {
            assert_eq!(read_plugins(text.as_bytes()), None, "{text}");
        }
    }

    #[test]
    fn the_file_a_tool_installs_from_must_name_bilbo() {
        let folder = Path::new("/world/pkg");
        let file = "plugins/bilbo/.antigravity-plugin/plugin.json";
        assert_eq!(
            read_manifest(fixture("manifest.json").as_bytes(), folder),
            Ok(())
        );
        let no = |why: &str| Err(format!("no plugin at /world/pkg: {file} {why}"));
        assert_eq!(
            read_manifest(b"nope", folder),
            no("is not the manifest bilbo reads")
        );
        assert_eq!(
            read_manifest(fixture("manifest-personal.json").as_bytes(), folder),
            no("names the plugin 'personal', not bilbo")
        );
        assert_eq!(
            read_manifest(b"{}", folder),
            no("is not the manifest bilbo reads")
        );
    }

    #[test]
    fn agys_removal_look_reads_of_each_item_only_what_names_it() {
        for name in [
            "list-name-only.json",
            "list-no-path.json",
            "list-no-enabled.json",
            "list-off.json",
        ] {
            assert_eq!(held_plugin(fixture(name).as_bytes()), Some(true), "{name}");
        }
        for name in ["list-no-name.json", "list-then-no-name.json"] {
            assert_eq!(held_plugin(fixture(name).as_bytes()), None, "{name}");
        }
    }
}
