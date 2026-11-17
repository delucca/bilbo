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

/// `HOME`, absolute and with no `.` or `..` component.
pub(crate) fn home(var: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
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

/// A path key's value: absolute, or `~/` and the rest under `HOME`; never with a `.` or `..`
/// component.
pub fn path(value: &Value<'_>) -> Result<PathBuf, Error> {
    let Some(text) = value
        .os_text()
        .filter(|text| !text.is_empty() && !text.as_bytes().contains(&0))
    else {
        return Err(value.refuse());
    };
    let path = match text.as_bytes().strip_prefix(b"~/") {
        Some(rest) => {
            let start = rest.iter().take_while(|&&byte| byte == b'/').count();
            let home = home(&|name| value.var(name)).map_err(|error| needs_home(value, error))?;
            home.join(OsStr::from_bytes(&rest[start..]))
        }
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

/// `HOME`'s fault as the value that needed it reports it.
fn needs_home(value: &Value<'_>, error: Error) -> Error {
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
    Error::NeedsHome {
        at: value.at().clone(),
        reason,
    }
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

    use super::{Folder, home};
    use crate::error::{At, Error};

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
}
