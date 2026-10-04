//! What setup finds before it plans: the config, the timer, the tools and the folders.

use std::path::{Path, PathBuf};

use super::flags::Flags;
use crate::Failure;
use crate::host::{agents, command, model, timer};
use crate::shared::config::{self, Embedder};
use crate::shared::store;

pub enum ConfigState {
    Absent,
    Present,
    /// `target` is the link's target as written, or the config path when it is not a link.
    Managed {
        target: String,
    },
}

/// What the machine holds now.
pub struct Facts {
    pub root: PathBuf,
    pub notes: PathBuf,
    pub config_path: PathBuf,
    pub config: ConfigState,
    /// The embedder of the file read; `None` for an absent file or one without `embedder.url`.
    pub existing: Option<Embedder>,
    /// The digest lines of the file read, as written, which a rewrite keeps.
    pub digest: Vec<(&'static str, String)>,
    /// The file is there and sets no key: only comments and blank lines.
    pub config_empty: bool,
    pub token_path: PathBuf,
    pub exe: PathBuf,
    pub source: agents::Source,
    pub claude: Option<PathBuf>,
    pub codex: Option<PathBuf>,
    pub timer: TimerFacts,
    /// `llama-server` from `--llama-server` or PATH, as given: links are not resolved.
    pub llama_server: Option<PathBuf>,
    /// Where the model file goes, under the cache folder.
    pub model: Option<PathBuf>,
}

/// What the timer step reads from the machine and the environment.
pub struct TimerFacts {
    pub platform: Option<timer::Platform>,
    pub home: Option<PathBuf>,
    pub config_home: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    /// `launchctl` or `systemctl` on PATH.
    pub tool: Option<PathBuf>,
    /// The locations setup saw, in the order the job carries them, or the first one that cannot be.
    pub locations: Result<Vec<(&'static str, String)>, String>,
}

pub fn timer_facts(env: &store::Env, path: Option<&std::ffi::OsStr>) -> TimerFacts {
    let platform = timer::platform();
    let program = match platform {
        Some(timer::Platform::Launchd) => Some("launchctl"),
        Some(timer::Platform::Systemd) => Some("systemctl"),
        None => None,
    };
    TimerFacts {
        platform,
        home: store::absolute(&env.home),
        config_home: store::config_home(env),
        state_dir: store::state_dir(env),
        tool: program.and_then(|name| command::find(name, path)),
        locations: locations(env),
    }
}

fn locations(env: &store::Env) -> Result<Vec<(&'static str, String)>, String> {
    let vars = [
        ("BILBO_HOME", &env.bilbo_home),
        ("BILBO_CONFIG", &env.bilbo_config),
        ("XDG_DATA_HOME", &env.xdg_data_home),
        ("XDG_CONFIG_HOME", &env.xdg_config_home),
        ("XDG_CACHE_HOME", &env.xdg_cache_home),
        ("XDG_STATE_HOME", &env.xdg_state_home),
    ];
    let mut out = Vec::new();
    for (name, value) in vars {
        if let Some(path) = store::absolute(value) {
            let text = path
                .into_os_string()
                .into_string()
                .map_err(|_| format!("{name} is not valid UTF-8"))?;
            out.push((name, text));
        }
    }
    Ok(out)
}

pub fn gather(
    flags: &Flags,
    env: &store::Env,
    path: Option<std::ffi::OsString>,
) -> Result<Facts, Failure> {
    let root = store::root(env).map_err(Failure::Config)?;
    let notes = root.join("notes");
    let (config_path, _) = config::path(env).map_err(Failure::Config)?.ok_or_else(|| {
        Failure::Config(
            "cannot find the config file: set BILBO_CONFIG, XDG_CONFIG_HOME or HOME to an absolute path"
                .into(),
        )
    })?;
    let config = config_state(&config_path);
    let (existing, digest) = match config {
        ConfigState::Absent => (None, Vec::new()),
        _ if std::fs::symlink_metadata(&config_path).is_err() => (None, Vec::new()),
        ConfigState::Managed { .. } if !config_path.exists() => (None, Vec::new()),
        _ => {
            let settings = config::load(env).map_err(Failure::Config)?;
            (settings.embedder, settings.digest_lines)
        }
    };
    let config_empty = matches!(config, ConfigState::Present) && sets_no_key(&config_path);
    let token_path = config_path.parent().unwrap_or(Path::new("/")).join("token");
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|e| Failure::Refused(format!("cannot find the bilbo executable: {e}")))?;
    let source = flags
        .source
        .clone()
        .unwrap_or_else(|| agents::default_source(&exe, env!("CARGO_PKG_VERSION")));
    let tool = |given: &Option<PathBuf>, name: &str| {
        if flags.no_plugin {
            return None;
        }
        given
            .clone()
            .or_else(|| command::find(name, path.as_deref()))
    };
    let claude = tool(&flags.claude, "claude");
    let codex = tool(&flags.codex, "codex");
    let llama_server = flags
        .local
        .as_ref()
        .and_then(|local| local.llama_server.clone())
        .or_else(|| command::find("llama-server", path.as_deref()));
    Ok(Facts {
        root,
        notes,
        config_path,
        config,
        existing,
        digest,
        config_empty,
        token_path,
        exe,
        source,
        claude,
        codex,
        timer: timer_facts(env, path.as_deref()),
        llama_server,
        model: store::cache_dir(env).map(|cache| model::path(&cache)),
    })
}

/// Whether the file holds only comments and blank lines.
fn sets_no_key(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| {
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        text.lines().all(|line| {
            let line = line.trim();
            line.is_empty() || line.starts_with('#')
        })
    })
}

fn config_state(path: &Path) -> ConfigState {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(path)
                .map_or_else(|_| path.display().to_string(), |t| t.display().to_string());
            return ConfigState::Managed { target };
        }
        return if folder_is_readonly(path) {
            ConfigState::Managed {
                target: path.display().to_string(),
            }
        } else {
            ConfigState::Present
        };
    }
    if folder_is_readonly(path) {
        ConfigState::Managed {
            target: path.display().to_string(),
        }
    } else {
        ConfigState::Absent
    }
}

/// The config's folder exists and has no write bit; nothing is written to find out.
fn folder_is_readonly(path: &Path) -> bool {
    path.parent()
        .and_then(|dir| std::fs::metadata(dir).ok())
        .is_some_and(|meta| meta.is_dir() && meta.permissions().readonly())
}

/// Where the timer's files live, when the folders it needs are known.
pub fn timer_place(t: &TimerFacts) -> Option<timer::Place> {
    match t.platform? {
        timer::Platform::Launchd => t.home.clone().map(|home| timer::Place {
            home,
            config_home: t.config_home.clone().unwrap_or_default(),
        }),
        timer::Platform::Systemd => t.config_home.clone().map(|config_home| timer::Place {
            home: t.home.clone().unwrap_or_default(),
            config_home,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::super::fakes::*;
    use super::*;

    #[test]
    fn sets_no_key_reads_like_the_config_parser() {
        let dir = scratch("sets-no-key");
        let cases = [
            ("", true),
            ("# a\n\n  # b\r\n", true),
            ("\u{feff}", true),
            ("\u{feff}# comment\n", true),
            ("embedder.min_similarity = 0.6\n", false),
            ("\u{feff}embedder.url = http://x\n", false),
        ];
        for (i, (text, want)) in cases.iter().enumerate() {
            let path = dir.join(format!("config-{i}"));
            std::fs::write(&path, text).unwrap();
            assert_eq!(sets_no_key(&path), *want, "{text:?}");
        }
        assert!(!sets_no_key(&dir.join("missing")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
