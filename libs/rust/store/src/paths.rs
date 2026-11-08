use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    Relative {
        name: &'static str,
        value: OsString,
    },
    /// A `.` or `..` component: the folders would not be where the text says.
    Dotted {
        name: &'static str,
        value: OsString,
    },
    Unset(&'static str),
    Empty(&'static str),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Relative { name, value } => {
                write!(
                    f,
                    "{name} is not an absolute path: {}",
                    value.to_string_lossy()
                )
            }
            Self::Dotted { name, value } => write!(
                f,
                "{name} has a . or .. component: {}",
                value.to_string_lossy()
            ),
            Self::Unset(name) => write!(f, "{name} is not set"),
            Self::Empty(name) => write!(f, "{name} is empty"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub root: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
}

/// The three folders; the config file is not resolved here.
pub fn layout(var: impl Fn(&str) -> Option<OsString>) -> Result<Layout, ConfigError> {
    Ok(Layout {
        root: root(&var)?,
        state: state(&var)?,
        cache: cache(&var)?,
    })
}

pub fn root(var: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, ConfigError> {
    locate(
        &var,
        Some("BILBO_HOME"),
        "XDG_DATA_HOME",
        "bilbo",
        ".local/share/bilbo",
    )
}

pub fn config(var: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, ConfigError> {
    locate(
        &var,
        Some("BILBO_CONFIG"),
        "XDG_CONFIG_HOME",
        "bilbo/config",
        ".config/bilbo/config",
    )
}

pub fn cache(var: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, ConfigError> {
    locate(&var, None, "XDG_CACHE_HOME", "bilbo", ".cache/bilbo")
}

pub fn state(var: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, ConfigError> {
    locate(&var, None, "XDG_STATE_HOME", "bilbo", ".local/state/bilbo")
}

fn dotted(value: &OsString) -> bool {
    use std::os::unix::ffi::OsStrExt;
    value
        .as_bytes()
        .split(|&byte| byte == b'/')
        .any(|part| part == b"." || part == b"..")
}

fn locate(
    var: &impl Fn(&str) -> Option<OsString>,
    override_name: Option<&'static str>,
    xdg_name: &'static str,
    under_xdg: &str,
    under_home: &str,
) -> Result<PathBuf, ConfigError> {
    if let Some(name) = override_name
        && let Some(value) = var(name).filter(|value| !value.is_empty())
    {
        let path = PathBuf::from(&value);
        return if !path.is_absolute() {
            Err(ConfigError::Relative { name, value })
        } else if dotted(&value) {
            Err(ConfigError::Dotted { name, value })
        } else {
            Ok(path)
        };
    }
    if let Some(xdg) = var(xdg_name) {
        let path = PathBuf::from(&xdg);
        if path.is_absolute() {
            if dotted(&xdg) {
                return Err(ConfigError::Dotted {
                    name: xdg_name,
                    value: xdg,
                });
            }
            return Ok(path.join(under_xdg));
        }
    }
    match var("HOME") {
        None => Err(ConfigError::Unset("HOME")),
        Some(home) if home.is_empty() => Err(ConfigError::Empty("HOME")),
        Some(home) => {
            let path = PathBuf::from(&home);
            if path.is_absolute() && dotted(&home) {
                Err(ConfigError::Dotted {
                    name: "HOME",
                    value: home,
                })
            } else if path.is_absolute() {
                Ok(path.join(under_home))
            } else {
                Err(ConfigError::Relative {
                    name: "HOME",
                    value: home,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{ConfigError, Layout, cache, config, layout, root, state};

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn root_is_the_first_that_applies() {
        let cases: &[(&[(&str, &str)], &str)] = &[
            (
                &[
                    ("BILBO_HOME", "/a"),
                    ("XDG_DATA_HOME", "/b"),
                    ("HOME", "/c"),
                ],
                "/a",
            ),
            (
                &[("BILBO_HOME", ""), ("XDG_DATA_HOME", "/b"), ("HOME", "/c")],
                "/b/bilbo",
            ),
            (&[("XDG_DATA_HOME", "/b"), ("HOME", "/c")], "/b/bilbo"),
            (
                &[("XDG_DATA_HOME", "data"), ("HOME", "/c")],
                "/c/.local/share/bilbo",
            ),
            (
                &[("XDG_DATA_HOME", ""), ("HOME", "/c")],
                "/c/.local/share/bilbo",
            ),
            (&[("HOME", "/c")], "/c/.local/share/bilbo"),
        ];
        for (env, expected) in cases {
            assert_eq!(root(vars(env)), Ok(PathBuf::from(expected)), "{env:?}");
        }
    }

    #[test]
    fn config_cache_and_state_follow_their_own_variables() {
        let env = &[
            ("BILBO_CONFIG", "/cfg"),
            ("XDG_CONFIG_HOME", "/xc"),
            ("XDG_CACHE_HOME", "/xa"),
            ("XDG_STATE_HOME", "/xs"),
            ("HOME", "/h"),
        ];
        assert_eq!(config(vars(env)), Ok(PathBuf::from("/cfg")));
        assert_eq!(cache(vars(env)), Ok(PathBuf::from("/xa/bilbo")));
        assert_eq!(state(vars(env)), Ok(PathBuf::from("/xs/bilbo")));
        let env = &[("XDG_CONFIG_HOME", "/xc"), ("HOME", "/h")];
        assert_eq!(config(vars(env)), Ok(PathBuf::from("/xc/bilbo/config")));
        let env = &[("HOME", "/h")];
        assert_eq!(
            config(vars(env)),
            Ok(PathBuf::from("/h/.config/bilbo/config"))
        );
        assert_eq!(cache(vars(env)), Ok(PathBuf::from("/h/.cache/bilbo")));
        assert_eq!(state(vars(env)), Ok(PathBuf::from("/h/.local/state/bilbo")));
    }

    #[test]
    fn a_relative_override_or_a_missing_home_is_a_config_error() {
        let relative = ConfigError::Relative {
            name: "BILBO_HOME",
            value: "store".into(),
        };
        assert_eq!(
            root(vars(&[("BILBO_HOME", "store"), ("HOME", "/c")])),
            Err(relative)
        );
        assert_eq!(root(vars(&[])), Err(ConfigError::Unset("HOME")));
        assert_eq!(root(vars(&[("HOME", "")])), Err(ConfigError::Empty("HOME")));
        assert_eq!(
            root(vars(&[("XDG_DATA_HOME", "data"), ("HOME", "home")])),
            Err(ConfigError::Relative {
                name: "HOME",
                value: "home".into()
            })
        );
    }

    #[test]
    fn a_config_error_names_the_variable() {
        let relative = ConfigError::Relative {
            name: "BILBO_HOME",
            value: "store".into(),
        };
        assert_eq!(
            relative.to_string(),
            "BILBO_HOME is not an absolute path: store"
        );
        assert_eq!(ConfigError::Unset("HOME").to_string(), "HOME is not set");
        assert_eq!(ConfigError::Empty("HOME").to_string(), "HOME is empty");
    }

    #[test]
    fn the_layout_is_the_root_state_and_cache_that_apply() {
        let env = &[
            ("BILBO_HOME", "/store"),
            ("XDG_STATE_HOME", "/xs"),
            ("HOME", "/h"),
        ];
        assert_eq!(
            layout(vars(env)),
            Ok(Layout {
                root: PathBuf::from("/store"),
                state: PathBuf::from("/xs/bilbo"),
                cache: PathBuf::from("/h/.cache/bilbo"),
            })
        );
        assert_eq!(layout(vars(&[])), Err(ConfigError::Unset("HOME")));
    }

    #[test]
    fn a_configured_path_with_a_dot_or_dot_dot_component_is_refused() {
        type Locate = fn(&dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, ConfigError>;
        let cases: [(&str, &str, Locate); 5] = [
            ("BILBO_HOME", "/x/new/../store", |v| root(v)),
            ("BILBO_HOME", "/x/./store", |v| root(v)),
            ("BILBO_HOME", "/x/store/..", |v| root(v)),
            ("XDG_STATE_HOME", "/x/../state", |v| state(v)),
            ("HOME", "/x/./home", |v| root(v)),
        ];
        for (name, value, locate) in cases {
            let var = |asked: &str| (asked == name).then(|| OsString::from(value));
            let found = locate(&var).err();
            assert!(
                matches!(&found, Some(ConfigError::Dotted { name: n, .. }) if *n == name),
                "{name}={value}: {found:?}"
            );
        }
        let message = ConfigError::Dotted {
            name: "BILBO_HOME",
            value: OsString::from("/x/../s"),
        }
        .to_string();
        assert_eq!(message, "BILBO_HOME has a . or .. component: /x/../s");
        let fine =
            |asked: &str| (asked == "BILBO_HOME").then(|| OsString::from("/x/.hidden/..a/store"));
        assert_eq!(root(fine), Ok(PathBuf::from("/x/.hidden/..a/store")));
    }
}
