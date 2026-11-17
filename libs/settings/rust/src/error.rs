use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

/// What an error names as the source of a refused value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum At {
    Variable(&'static str),
    Line {
        path: PathBuf,
        line: usize,
        key: String,
    },
    Flag(&'static str),
    Default(&'static str),
}

impl fmt::Display for At {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Variable(name) | Self::Flag(name) => f.write_str(name),
            Self::Line { path, line, key } => write!(f, "{}:{line}: {key}", path.display()),
            Self::Default(key) => write!(f, "the default of {key}"),
        }
    }
}

/// Why a setting could not be read, or why the file holding it could not; every variant names
/// where to look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The file is not TOML; a key set twice is one of these.
    Syntax {
        path: PathBuf,
        line: usize,
        message: String,
    },
    NotUtf8 {
        path: PathBuf,
        line: usize,
    },
    Unknown {
        path: PathBuf,
        line: usize,
        key: String,
        keys: Vec<&'static str>,
    },
    Invalid {
        at: At,
        allowed: &'static str,
        got: String,
    },
    Relative {
        at: At,
        value: OsString,
    },
    /// A `.` or `..` component: the folders would not be where the text says.
    Dotted {
        at: At,
        value: OsString,
    },
    /// A `~/` value whose `HOME` is unusable; `reason` ends the sentence `HOME ...`.
    NeedsHome {
        at: At,
        reason: String,
    },
    Unset(&'static str),
    Empty(&'static str),
    Missing {
        variable: &'static str,
        path: PathBuf,
    },
    Read {
        path: PathBuf,
        reason: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax {
                path,
                line,
                message,
            } => write!(f, "{}:{line}: {message}", path.display()),
            Self::NotUtf8 { path, line } => {
                write!(f, "{}:{line}: not valid UTF-8", path.display())
            }
            Self::Unknown {
                path,
                line,
                key,
                keys,
            } => write!(
                f,
                "{}:{line}: unknown key {key}; the keys are {}",
                path.display(),
                keys.join(", ")
            ),
            Self::Invalid { at, allowed, got } => write!(f, "{at} must be {allowed}, got {got}"),
            Self::Relative { at, value } => {
                write!(
                    f,
                    "{at} is not an absolute path: {}",
                    value.to_string_lossy()
                )
            }
            Self::Dotted { at, value } => write!(
                f,
                "{at} has a . or .. component: {}",
                value.to_string_lossy()
            ),
            Self::NeedsHome { at, reason } => write!(f, "{at} needs HOME, which {reason}"),
            Self::Unset(name) => write!(f, "{name} is not set"),
            Self::Empty(name) => write!(f, "{name} is empty"),
            Self::Missing { variable, path } => write!(
                f,
                "{variable} names a file that does not exist: {}",
                path.display()
            ),
            Self::Read { path, reason } => write!(f, "cannot read {}: {reason}", path.display()),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{At, Error};

    fn line(key: &str) -> At {
        At::Line {
            path: PathBuf::from("/c/config.toml"),
            line: 7,
            key: key.to_owned(),
        }
    }

    #[test]
    fn every_error_is_one_line_naming_its_source() {
        let cases = [
            (
                Error::Invalid {
                    at: line("store.seal_at"),
                    allowed: "between 1 MiB and 1 GiB",
                    got: "\"0\"".to_owned(),
                },
                "/c/config.toml:7: store.seal_at must be between 1 MiB and 1 GiB, got \"0\"",
            ),
            (
                Error::Invalid {
                    at: At::Variable("BILBO_STORE_SEAL_AT"),
                    allowed: "between 1 MiB and 1 GiB",
                    got: "\"0\"".to_owned(),
                },
                "BILBO_STORE_SEAL_AT must be between 1 MiB and 1 GiB, got \"0\"",
            ),
            (
                Error::Invalid {
                    at: At::Flag("--budget"),
                    allowed: "a whole number from 1 to 9",
                    got: "0".to_owned(),
                },
                "--budget must be a whole number from 1 to 9, got 0",
            ),
            (
                Error::Invalid {
                    at: At::Default("store.seal_at"),
                    allowed: "between 1 MiB and 1 GiB",
                    got: "\"0\"".to_owned(),
                },
                "the default of store.seal_at must be between 1 MiB and 1 GiB, got \"0\"",
            ),
            (
                Error::Unknown {
                    path: PathBuf::from("/c/config.toml"),
                    line: 9,
                    key: "store.sealat".to_owned(),
                    keys: vec!["paths.root", "store.seal_at"],
                },
                "/c/config.toml:9: unknown key store.sealat; the keys are paths.root, store.seal_at",
            ),
            (
                Error::Syntax {
                    path: PathBuf::from("/c/config.toml"),
                    line: 3,
                    message: "duplicate key seal_at".to_owned(),
                },
                "/c/config.toml:3: duplicate key seal_at",
            ),
            (
                Error::NotUtf8 {
                    path: PathBuf::from("/c/config.toml"),
                    line: 2,
                },
                "/c/config.toml:2: not valid UTF-8",
            ),
            (
                Error::Relative {
                    at: At::Variable("BILBO_HOME"),
                    value: "store".into(),
                },
                "BILBO_HOME is not an absolute path: store",
            ),
            (
                Error::Relative {
                    at: line("paths.root"),
                    value: "store".into(),
                },
                "/c/config.toml:7: paths.root is not an absolute path: store",
            ),
            (
                Error::Dotted {
                    at: At::Variable("BILBO_HOME"),
                    value: "/x/../s".into(),
                },
                "BILBO_HOME has a . or .. component: /x/../s",
            ),
            (Error::Unset("HOME"), "HOME is not set"),
            (Error::Empty("HOME"), "HOME is empty"),
            (
                Error::Missing {
                    variable: "BILBO_CONFIG",
                    path: PathBuf::from("/c/config.toml"),
                },
                "BILBO_CONFIG names a file that does not exist: /c/config.toml",
            ),
            (
                Error::Read {
                    path: PathBuf::from("/c/config.toml"),
                    reason: "Permission denied (os error 13)".to_owned(),
                },
                "cannot read /c/config.toml: Permission denied (os error 13)",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
        }
    }
}
