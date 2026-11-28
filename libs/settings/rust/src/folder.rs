use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::error::{At, Error};
use crate::key::{Value, quote};

/// Where a folder or file is when nothing names it: under an XDG variable, else under `HOME`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Folder {
    pub xdg: &'static str,
    pub under_xdg: &'static str,
    pub under_home: &'static str,
}

impl Folder {
    /// `$<xdg>/<under_xdg>` when that variable is an absolute path, else `$HOME/<under_home>`.
    pub fn locate(&self, var: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
        if let Some(xdg) = var(self.xdg) {
            let path = PathBuf::from(&xdg);
            if path.is_absolute() {
                if dotted(&xdg) {
                    return Err(Error::Dotted {
                        at: At::Variable(self.xdg),
                        value: xdg,
                    });
                }
                return Ok(path.join(self.under_xdg));
            }
        }
        Ok(home(var)?.join(self.under_home))
    }

    /// As the reference page writes it.
    pub fn describe(&self) -> String {
        format!(
            "${}/{}, else ~/{}",
            self.xdg, self.under_xdg, self.under_home
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Absolute {
    pub variable: &'static str,
    pub fallback: &'static str,
}

impl Absolute {
    pub fn locate(&self, var: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
        match var(self.variable) {
            Some(value) if Path::new(&value).is_absolute() => {
                if dotted(&value) {
                    return Err(Error::Dotted {
                        at: At::Variable(self.variable),
                        value,
                    });
                }
                Ok(PathBuf::from(value))
            }
            _ => Ok(PathBuf::from(self.fallback)),
        }
    }

    pub fn describe(&self) -> String {
        format!("${}, else {}", self.variable, self.fallback)
    }
}

/// `HOME`, absolute and with no `.` or `..` component.
pub fn home(var: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
    match var("HOME") {
        None => Err(Error::Unset("HOME")),
        Some(home) if home.is_empty() => Err(Error::Empty("HOME")),
        Some(home) if !Path::new(&home).is_absolute() => Err(Error::Relative {
            at: At::Variable("HOME"),
            value: home,
        }),
        Some(home) if dotted(&home) => Err(Error::Dotted {
            at: At::Variable("HOME"),
            value: home,
        }),
        Some(home) => Ok(PathBuf::from(home)),
    }
}

pub fn path(value: &Value<'_>) -> Result<PathBuf, Error> {
    let Some(text) = value
        .os_text()
        .filter(|text| !text.is_empty() && !text.as_bytes().contains(&0))
    else {
        return Err(value.refuse());
    };
    let path = match under_home(text, &|name| value.var(name)) {
        Some(under) => under.map_err(|error| needs_home(value.at().clone(), error))?,
        None => PathBuf::from(text),
    };
    if !path.is_absolute() {
        return Err(Error::Relative {
            at: value.at().clone(),
            value: text.to_owned(),
        });
    }
    if dotted(path.as_os_str()) {
        return Err(Error::Dotted {
            at: value.at().clone(),
            value: text.to_owned(),
        });
    }
    Ok(path)
}

fn under_home(
    text: &OsStr,
    var: &dyn Fn(&str) -> Option<OsString>,
) -> Option<Result<PathBuf, Error>> {
    let rest = text.as_bytes().strip_prefix(b"~/")?;
    let start = rest.iter().take_while(|&&byte| byte == b'/').count();
    Some(home(var).map(|home| home.join(OsStr::from_bytes(&rest[start..]))))
}

pub fn typed(text: &str, var: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
    match under_home(OsStr::new(text), var) {
        Some(under) => under.map_err(|error| needs_home(At::Typed(text.to_owned()), error)),
        None => Ok(PathBuf::from(text)),
    }
}

fn needs_home(at: At, error: Error) -> Error {
    let reason = match &error {
        Error::Unset(_) => "is not set".to_owned(),
        Error::Empty(_) => "is empty".to_owned(),
        Error::Relative { value, .. } => {
            format!("is not an absolute path: {}", value.to_string_lossy())
        }
        Error::Dotted { value, .. } => {
            format!("has a . or .. component: {}", value.to_string_lossy())
        }
        _ => return error,
    };
    Error::NeedsHome { at, reason }
}

/// A path as TOML text; bytes that are not UTF-8 show as U+FFFD.
pub fn show(path: &Path) -> String {
    quote(&path.to_string_lossy())
}

fn dotted(value: &OsStr) -> bool {
    value
        .as_bytes()
        .split(|&byte| byte == b'/')
        .any(|part| part == b"." || part == b"..")
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{Absolute, Folder, home, path, show, typed};
    use crate::error::{At, Error};
    use crate::file::File;
    use crate::key::{About, Default, Key, Source};

    const DATA: Folder = Folder {
        xdg: "XDG_DATA_HOME",
        under_xdg: "bilbo",
        under_home: ".local/share/bilbo",
    };

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn a_folder_is_under_its_xdg_variable_when_absolute_else_under_home() {
        let cases: &[(&[(&str, &str)], &str)] = &[
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
            assert_eq!(
                DATA.locate(&vars(env)),
                Ok(PathBuf::from(expected)),
                "{env:?}"
            );
        }
        assert_eq!(Default::Folder(DATA).reads(), ["XDG_DATA_HOME", "HOME"]);
    }

    #[test]
    fn home_must_be_set_absolute_and_without_dots() {
        assert_eq!(home(&vars(&[])), Err(Error::Unset("HOME")));
        assert_eq!(home(&vars(&[("HOME", "")])), Err(Error::Empty("HOME")));
        assert_eq!(
            home(&vars(&[("HOME", "home")])),
            Err(Error::Relative {
                at: At::Variable("HOME"),
                value: "home".into()
            })
        );
        assert_eq!(
            DATA.locate(&vars(&[("HOME", "/x/./home")])),
            Err(Error::Dotted {
                at: At::Variable("HOME"),
                value: "/x/./home".into()
            })
        );
        assert_eq!(
            DATA.locate(&vars(&[("XDG_DATA_HOME", "/x/../d"), ("HOME", "/h")])),
            Err(Error::Dotted {
                at: At::Variable("XDG_DATA_HOME"),
                value: "/x/../d".into()
            })
        );
    }

    #[test]
    fn a_folder_describes_its_rule() {
        assert_eq!(
            DATA.describe(),
            "$XDG_DATA_HOME/bilbo, else ~/.local/share/bilbo"
        );
    }

    const DRAFTS: Absolute = Absolute {
        variable: "BILBO_TEST_TEMP",
        fallback: "/fixed",
    };

    #[test]
    fn a_temp_folder_is_its_variable_when_absolute_else_a_fixed_path() {
        let cases: &[(&[(&str, &str)], &str)] = &[
            (&[("BILBO_TEST_TEMP", "/scratch")], "/scratch"),
            (&[("BILBO_TEST_TEMP", "")], "/fixed"),
            (&[("BILBO_TEST_TEMP", "scratch")], "/fixed"),
            (&[], "/fixed"),
        ];
        for (env, expected) in cases {
            assert_eq!(
                DRAFTS.locate(&vars(env)),
                Ok(PathBuf::from(expected)),
                "{env:?}"
            );
        }
        assert_eq!(
            DRAFTS.locate(&vars(&[("BILBO_TEST_TEMP", "/a/../b")])),
            Err(Error::Dotted {
                at: At::Variable("BILBO_TEST_TEMP"),
                value: "/a/../b".into()
            })
        );
        assert_eq!(DRAFTS.describe(), "$BILBO_TEST_TEMP, else /fixed");
        assert_eq!(Default::Absolute(DRAFTS).reads(), ["BILBO_TEST_TEMP"]);
    }

    #[test]
    fn a_typed_path_under_home_takes_the_rule_of_the_path_keys() {
        let var = vars(&[("HOME", "/home/me")]);
        for (text, expected) in [
            ("~/notes", "/home/me/notes"),
            ("~//notes", "/home/me/notes"),
            ("~/", "/home/me/"),
            ("~", "~"),
            ("~notes", "~notes"),
            ("/etc/x y", "/etc/x y"),
            ("notes", "notes"),
            ("notes/../x", "notes/../x"),
        ] {
            let found = typed(text, &var).expect("a path");
            assert_eq!(found.to_str(), Some(expected), "{text}");
        }
        let key: Key<PathBuf> = Key {
            about: About {
                name: "test.folder",
                variable: Some("BILBO_TEST_FOLDER"),
                default: Default::Toml(r#""/d""#),
                allowed: "an absolute path or one starting with ~/",
                doc: "a path key",
            },
            parse: path,
            show: |path| show(path),
        };
        let file = File::default();
        let read = |text: &str| {
            let env = |name: &str| match name {
                "HOME" => Some(OsString::from("/home/me")),
                "BILBO_TEST_FOLDER" => Some(OsString::from(text)),
                _ => None,
            };
            key.resolve(&Source {
                var: &env,
                file: &file,
            })
            .map(|resolved| resolved.value)
            .map_err(|error| error.to_string())
        };
        let found = read("~//notes").expect("a path");
        assert_eq!(found.to_str(), Some("/home/me/notes"));
        for text in ["notes", "notes/../x"] {
            assert_eq!(
                read(text),
                Err(format!("BILBO_TEST_FOLDER is not an absolute path: {text}"))
            );
        }
        for (env, said) in [
            (&[][..], "'~/notes' needs HOME, which is not set"),
            (&[("HOME", "")][..], "'~/notes' needs HOME, which is empty"),
            (
                &[("HOME", "home")][..],
                "'~/notes' needs HOME, which is not an absolute path: home",
            ),
            (
                &[("HOME", "/x/../me")][..],
                "'~/notes' needs HOME, which has a . or .. component: /x/../me",
            ),
        ] {
            let fault = typed("~/notes", &vars(env)).expect_err("a fault");
            assert_eq!(fault.to_string(), said, "{env:?}");
        }
    }
}
