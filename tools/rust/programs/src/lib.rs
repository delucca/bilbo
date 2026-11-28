use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const PROMPT_COMMAND: &str = "command -v bilbo >/dev/null 2>&1 || exit 0; bilbo digest; exit 0";
pub const COMPACT_COMMAND: &str = "command -v bilbo >/dev/null 2>&1 || exit 0; echo 'Context was compacted. If this session settled something later sessions should know, such as a decision, a gotcha or a plan, save it with the bilbo note skill once the current task allows.'";
pub const TIMEOUT: u32 = 10;

const CLAUDE: &str = include_str!("claude.sh");
const CODEX: &str = include_str!("codex.sh");
const AGY: &str = include_str!("agy.sh");
const LAUNCHCTL: &str = include_str!("launchctl.sh");
const SYSTEMCTL: &str = include_str!("systemctl.sh");

const FIRST_HASH: &str = "25e2d1e2cbe24cbc61bd80f28e947112c2db5245aaf103860d7721932066d0ed";
const SECOND_HASH: &str = "a81858bef5fb353c95c7f98c4f5c8db9e6bd95f30dc9513adff2136f9c6c99dc";

pub const NAMES: [&str; 5] = ["claude", "codex", "agy", "launchctl", "systemctl"];

pub const MARKETPLACE_CLAUDE: &str = ".claude-plugin/marketplace.json";
pub const MARKETPLACE_CODEX: &str = ".agents/plugins/marketplace.json";
pub const MANIFEST_AGY: &str = "plugins/bilbo/.antigravity-plugin/plugin.json";
pub const HOOKS_FILE: &str = "plugins/bilbo/hooks/hooks.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fail<'a> {
    Exit(&'a str),
    Signal,
    Hang,
}

#[derive(Debug, Clone)]
pub struct Programs {
    bin: PathBuf,
    state: PathBuf,
}

fn quoted(text: &str) -> String {
    text.replace('\'', "'\\''")
}

fn script(template: &str, state: &Path, tools: &[(&str, &Path)]) -> String {
    let mut text = template.to_owned();
    for (word, path) in tools {
        text = text.replace(word, &quoted(&path.to_string_lossy()));
    }
    text.replace("@PROMPT@", &quoted(PROMPT_COMMAND))
        .replace("@COMPACT@", &quoted(COMPACT_COMMAND))
        .replace("@TIMEOUT@", &TIMEOUT.to_string())
        .replace("@HASH1@", FIRST_HASH)
        .replace("@HASH2@", SECOND_HASH)
        .replace("@STATE@", &quoted(&state.to_string_lossy()))
}

pub fn find_in(name: &str, path: &OsStr) -> Option<PathBuf> {
    env::split_paths(path)
        .filter(|folder| folder.is_absolute())
        .map(|folder| folder.join(name))
        .find(|candidate| {
            fs::metadata(candidate)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

#[expect(
    clippy::disallowed_methods,
    reason = "the stand-ins' crate reads the test's own PATH to find the three programs systemctl's stand-in runs"
)]
fn process_path() -> std::ffi::OsString {
    env::var_os("PATH").unwrap_or_default()
}

fn found(name: &str, path: &OsStr) -> io::Result<PathBuf> {
    find_in(name, path).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "the systemctl stand-in needs `{name}` on the PATH it is handed, and there is none"
            ),
        )
    })
}

impl Programs {
    pub fn write(bin: &Path, state: &Path) -> io::Result<Programs> {
        Programs::write_with(bin, state, &process_path())
    }

    pub fn write_with(bin: &Path, state: &Path, path: &OsStr) -> io::Result<Programs> {
        fs::create_dir_all(bin)?;
        fs::create_dir_all(state)?;
        let tools = [
            ("@MKDIR@", found("mkdir", path)?),
            ("@LN@", found("ln", path)?),
            ("@RM@", found("rm", path)?),
        ];
        let tools: Vec<(&str, &Path)> = tools
            .iter()
            .map(|(word, path)| (*word, path.as_path()))
            .collect();
        for (name, template) in [
            ("claude", CLAUDE),
            ("codex", CODEX),
            ("agy", AGY),
            ("launchctl", LAUNCHCTL),
            ("systemctl", SYSTEMCTL),
        ] {
            let path = bin.join(name);
            fs::write(&path, script(template, state, &tools))?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }
        Ok(Programs {
            bin: bin.to_owned(),
            state: state.to_owned(),
        })
    }

    pub fn units(&self, folder: &Path) -> io::Result<()> {
        self.set("systemctl", "units", &folder.to_string_lossy())
    }

    pub fn loaded_labels(&self) -> Vec<String> {
        fs::read_to_string(self.file("launchctl", "loaded"))
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    pub fn bin(&self) -> &Path {
        &self.bin
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.bin.join(name)
    }

    fn file(&self, name: &str, key: &str) -> PathBuf {
        self.state.join(format!("{name}.{key}"))
    }

    pub fn set(&self, name: &str, key: &str, text: &str) -> io::Result<()> {
        fs::write(self.file(name, key), format!("{text}\n"))
    }

    pub fn flag(&self, name: &str, key: &str) -> io::Result<()> {
        fs::write(self.file(name, key), "on\n")
    }

    pub fn get(&self, name: &str, key: &str) -> Option<String> {
        let text = fs::read_to_string(self.file(name, key)).ok()?;
        Some(text.lines().next().unwrap_or_default().to_owned())
    }

    pub fn print(&self, name: &str, list: &str, text: &str) -> io::Result<()> {
        fs::write(
            self.file(name, &format!("print.{list}")),
            format!("{text}\n"),
        )
    }

    pub fn items(&self, name: &str, key: &str, objects: &[&str]) -> io::Result<()> {
        fs::write(self.file(name, key), format!("{}\n", objects.join("\n")))
    }

    pub fn fail(
        &self,
        name: &str,
        command: &str,
        how: Fail<'_>,
        after_work: bool,
    ) -> io::Result<()> {
        self.fail_for(name, command, None, false, how, after_work)
    }

    pub fn fail_for(
        &self,
        name: &str,
        command: &str,
        target: Option<&str>,
        once: bool,
        how: Fail<'_>,
        after_work: bool,
    ) -> io::Result<()> {
        let (word, line) = match how {
            Fail::Exit(line) => ("exit", line),
            Fail::Signal => ("signal", ""),
            Fail::Hang => ("hang", ""),
        };
        let key = match target {
            Some(target) => format!("fail.{command}.for.{target}"),
            None => format!("fail.{command}"),
        };
        let once = if once { "once\n" } else { "" };
        fs::write(self.file(name, &key), format!("{word}\n{line}\n{once}"))?;
        let after = self.file(name, &format!("{key}.after"));
        if after_work {
            fs::write(after, "on\n")
        } else {
            match fs::remove_file(after) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            }
        }
    }

    pub fn code_for(
        &self,
        name: &str,
        command: &str,
        target: Option<&str>,
        once: bool,
        code: i32,
    ) -> io::Result<()> {
        let key = match target {
            Some(target) => format!("code.{command}.for.{target}"),
            None => format!("code.{command}"),
        };
        let once = if once { "once\n" } else { "" };
        fs::write(self.file(name, &key), format!("{code}\n{once}"))
    }

    pub fn leave_before_bootout(&self, service: &str) -> io::Result<()> {
        self.set("launchctl", "leave", service)
    }

    pub fn unfail(&self, name: &str, command: &str) -> io::Result<()> {
        for key in [format!("fail.{command}"), format!("fail.{command}.after")] {
            match fs::remove_file(self.file(name, &key)) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
        }
        Ok(())
    }

    pub fn log(&self, name: &str) -> Vec<String> {
        fs::read_to_string(self.file(name, "log"))
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    pub fn trust(&self) -> Vec<(String, String)> {
        fs::read_to_string(self.file("codex", "trust"))
            .map(|text| {
                text.lines()
                    .filter_map(|line| line.split_once('\t'))
                    .map(|(key, hash)| (key.to_owned(), hash.to_owned()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn seed_trust(&self, key: &str, hash: &str) -> io::Result<()> {
        let mut text = fs::read_to_string(self.file("codex", "trust")).unwrap_or_default();
        text.push_str(&format!("{key}\t{hash}\n"));
        fs::write(self.file("codex", "trust"), text)
    }

    pub fn hash_of(event: &str) -> &'static str {
        if event == "SessionStart" {
            SECOND_HASH
        } else {
            FIRST_HASH
        }
    }
}

const SKILL: &str =
    "---\nname: recall\ndescription: Search what bilbo keeps.\n---\n\nSearch with bilbo recall.\n";

fn hooks_file() -> String {
    format!(
        "{{\n  \"hooks\": {{\n    \"UserPromptSubmit\": [\n      {{\n        \"hooks\": [\n          {{\n            \"type\": \"command\",\n            \"command\": {},\n            \"timeout\": {TIMEOUT}\n          }}\n        ]\n      }}\n    ],\n    \"SessionStart\": [\n      {{\n        \"matcher\": \"compact\",\n        \"hooks\": [\n          {{\n            \"type\": \"command\",\n            \"command\": {},\n            \"timeout\": {TIMEOUT}\n          }}\n        ]\n      }}\n    ]\n  }}\n}}\n",
        json_text(PROMPT_COMMAND),
        json_text(COMPACT_COMMAND),
    )
}

fn json_text(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn plugin_folder(folder: &Path) -> io::Result<()> {
    let files: [(&str, String); 5] = [
        (
            MARKETPLACE_CLAUDE,
            "{\n  \"name\": \"bilbo\",\n  \"plugins\": [\n    {\n      \"name\": \"bilbo\",\n      \"source\": \"./plugins/bilbo\"\n    }\n  ]\n}\n"
                .to_owned(),
        ),
        (
            MARKETPLACE_CODEX,
            "{\n  \"name\": \"bilbo\",\n  \"plugins\": [\n    {\n      \"name\": \"bilbo\",\n      \"source\": {\n        \"source\": \"local\",\n        \"path\": \"./plugins/bilbo\"\n      }\n    }\n  ]\n}\n"
                .to_owned(),
        ),
        (MANIFEST_AGY, "{\n  \"name\": \"bilbo\"\n}\n".to_owned()),
        (HOOKS_FILE, hooks_file()),
        ("plugins/bilbo/skills/recall/SKILL.md", SKILL.to_owned()),
    ];
    for (relative, text) in files {
        let path = folder.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, text)?;
    }
    Ok(())
}
