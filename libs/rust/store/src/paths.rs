//! Each lookup takes the environment as an argument and reads nothing itself.

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    Relative { name: &'static str, value: OsString },
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
            Self::Unset(name) => write!(f, "{name} is not set"),
            Self::Empty(name) => write!(f, "{name} is empty"),
        }
    }
}

impl std::error::Error for ConfigError {}

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

/// An override set to a relative path is an error; empty, it is unset. A relative or empty
/// XDG variable falls through to `HOME`, which must be set, non-empty and absolute.
fn locate(
    var: &impl Fn(&str) -> Option<OsString>,
    override_name: Option<&'static str>,
    xdg_name: &str,
    under_xdg: &str,
    under_home: &str,
) -> Result<PathBuf, ConfigError> {
    if let Some(name) = override_name
        && let Some(value) = var(name).filter(|value| !value.is_empty())
    {
        let path = PathBuf::from(&value);
        return if path.is_absolute() {
            Ok(path)
        } else {
            Err(ConfigError::Relative { name, value })
        };
    }
    if let Some(xdg) = var(xdg_name) {
        let path = PathBuf::from(&xdg);
        if path.is_absolute() {
            return Ok(path.join(under_xdg));
        }
    }
    match var("HOME") {
        None => Err(ConfigError::Unset("HOME")),
        Some(home) if home.is_empty() => Err(ConfigError::Empty("HOME")),
        Some(home) => {
            let path = PathBuf::from(&home);
            if path.is_absolute() {
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

    use super::{ConfigError, cache, config, root, state};

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
}
