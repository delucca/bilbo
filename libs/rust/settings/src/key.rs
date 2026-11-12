use std::ffi::{OsStr, OsString};
use std::fmt::{self, Write};
use std::path::PathBuf;

use toml::de::DeValue;

use crate::error::{At, Error};
use crate::file::{self, File, Kind, Node};
use crate::folder::Folder;

/// Where a key's value came from, lowest precedence first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Default,
    File { path: PathBuf, line: usize },
    Variable(&'static str),
    Flag(&'static str),
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => f.write_str("default"),
            Self::File { path, line } => write!(f, "{}:{line}", path.display()),
            Self::Variable(name) | Self::Flag(name) => f.write_str(name),
        }
    }
}

/// A key's value and the source that gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved<T> {
    pub value: T,
    pub origin: Origin,
}

/// How a key's default is written; the key reads it as if a source gave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Default {
    /// A TOML value, read by the key as if the file held it: `"8 MiB"`, `19`, `["~/a"]`.
    Toml(&'static str),
    Folder(Folder),
}

impl Default {
    /// As the reference page writes it.
    pub fn describe(&self) -> String {
        match self {
            Self::Toml(text) => (*text).to_owned(),
            Self::Folder(folder) => folder.describe(),
        }
    }
}

/// What the reference page, the errors and `bilbo config` know of a key, whatever its type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct About {
    /// The table and the key in the file: `store.seal_at`.
    pub name: &'static str,
    /// The variable over the file, or `None` when only the file sets the key.
    pub variable: Option<&'static str>,
    pub default: Default,
    /// What a value must be, as an error ends: `between 1 MiB and 1 GiB`.
    pub allowed: &'static str,
    /// One line for the reference page.
    pub doc: &'static str,
}

/// A setting: what it is called and allowed, and how to read and show a value of type `T`.
pub struct Key<T> {
    pub about: About,
    /// Reads a value; one outside `about.allowed` is `Err(value.refuse())`.
    pub parse: fn(&Value<'_>) -> Result<T, Error>,
    /// The value as TOML text.
    pub show: fn(&T) -> String,
}

impl<T> fmt::Debug for Key<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Key")
            .field("about", &self.about)
            .finish_non_exhaustive()
    }
}

impl<T> Key<T> {
    pub fn shown(&self, resolved: &Resolved<T>) -> Shown {
        Shown {
            name: self.about.name,
            value: (self.show)(&resolved.value),
            origin: resolved.origin.clone(),
        }
    }

    /// Variable, then file, then default; an empty variable counts as unset. A flag goes on the
    /// result through `Resolved::with_flag`. The file's value for the key is read whenever the
    /// file holds one, so a bad value is an error whichever source wins.
    pub fn resolve(&self, source: &Source<'_>) -> Result<Resolved<T>, Error> {
        let about = &self.about;
        let from_file = source
            .file
            .find(about.name)
            .map(|node| {
                let path = source.file.path().to_path_buf();
                let line = source.file.line(node.span.start);
                let at = At::Line {
                    path: path.clone(),
                    line,
                    key: about.name.to_owned(),
                };
                let value = Value::new(about, source, at, Given::Node(node, source.file.text()));
                Ok(Resolved {
                    value: (self.parse)(&value)?,
                    origin: Origin::File { path, line },
                })
            })
            .transpose()?;
        if let Some(name) = about.variable
            && let Some(text) = (source.var)(name).filter(|text| !text.is_empty())
        {
            let value = Value::new(about, source, At::Variable(name), Given::Text(text));
            return Ok(Resolved {
                value: (self.parse)(&value)?,
                origin: Origin::Variable(name),
            });
        }
        if let Some(resolved) = from_file {
            return Ok(resolved);
        }
        let at = At::Default(about.name);
        let value = match about.default {
            Default::Toml(text) => {
                let node = file::literal(text).ok_or_else(|| Error::Invalid {
                    at: at.clone(),
                    allowed: about.allowed,
                    got: text.to_owned(),
                })?;
                (self.parse)(&Value::new(about, source, at, Given::Node(&node, text)))?
            }
            Default::Folder(folder) => {
                let path = folder.locate(source.var)?.into_os_string();
                (self.parse)(&Value::new(about, source, at, Given::Text(path)))?
            }
        };
        Ok(Resolved {
            value,
            origin: Origin::Default,
        })
    }
}

impl<T> Resolved<T> {
    /// A flag that was given replaces the value and the origin; none leaves both.
    pub fn with_flag(self, flag: Option<Flag<T>>) -> Resolved<T> {
        flag.map_or(self, |flag| Resolved {
            value: flag.value,
            origin: Origin::Flag(flag.name),
        })
    }
}

/// A flag's value, read by its key's own grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flag<T> {
    name: &'static str,
    value: T,
}

impl<T> Flag<T> {
    /// A flag's text as its key reads it, so a refusal names the flag: `--budget must be ...,
    /// got "x"`. `source` serves a `~/` against `HOME`; the file is never read.
    pub fn parse(
        key: &Key<T>,
        source: &Source<'_>,
        name: &'static str,
        text: &str,
    ) -> Result<Flag<T>, Error> {
        let value = Value::new(
            &key.about,
            source,
            At::Flag(name),
            Given::Text(OsString::from(text)),
        );
        Ok(Flag {
            name,
            value: (key.parse)(&value)?,
        })
    }
}

/// Everything a key is read from but a flag.
pub struct Source<'a> {
    pub var: &'a dyn Fn(&str) -> Option<OsString>,
    pub file: &'a File,
}

impl fmt::Debug for Source<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Source")
            .field("file", &self.file)
            .finish_non_exhaustive()
    }
}

/// A key as `bilbo config` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub name: &'static str,
    /// TOML text.
    pub value: String,
    pub origin: Origin,
}

impl Shown {
    /// A string's text without quotes or escapes; any other value as its TOML text.
    pub fn to_bare(&self) -> String {
        match DeValue::parse(&self.value).map(toml::Spanned::into_inner) {
            Ok(DeValue::String(text)) => text.into_owned(),
            _ => self.value.clone(),
        }
    }
}

#[derive(Debug)]
enum Given<'a> {
    /// A variable's value or a located folder.
    Text(OsString),
    /// A value of the file or of a default, and the text its span indexes.
    Node(&'a Node, &'a str),
}

/// A value as one source gives it, before its key reads it.
#[derive(Debug)]
pub struct Value<'a> {
    about: &'a About,
    source: &'a Source<'a>,
    at: At,
    given: Given<'a>,
}

impl<'a> Value<'a> {
    fn new(about: &'a About, source: &'a Source<'a>, at: At, given: Given<'a>) -> Value<'a> {
        Value {
            about,
            source,
            at,
            given,
        }
    }

    /// A variable's text or a TOML string.
    pub fn text(&self) -> Option<&str> {
        match &self.given {
            Given::Text(text) => text.to_str(),
            Given::Node(
                Node {
                    kind: Kind::Text(text),
                    ..
                },
                _,
            ) => Some(text),
            Given::Node(..) => None,
        }
    }

    /// The same, with a variable's text that is not UTF-8.
    pub fn os_text(&self) -> Option<&OsStr> {
        match &self.given {
            Given::Text(text) => Some(text),
            _ => self.text().map(OsStr::new),
        }
    }

    /// A TOML integer, or a variable's text that is a decimal integer.
    pub fn integer(&self) -> Option<i64> {
        match &self.given {
            Given::Text(text) => text.to_str()?.parse().ok(),
            Given::Node(
                Node {
                    kind: Kind::Integer(number),
                    ..
                },
                _,
            ) => *number,
            Given::Node(..) => None,
        }
    }

    /// A TOML float or integer, or a variable's or flag's text of digits with at most one point
    /// between digits.
    pub fn decimal(&self) -> Option<f64> {
        let number = match &self.given {
            Given::Text(text) => {
                let text = text.to_str()?;
                let (whole, fraction) = text.split_once('.').unwrap_or((text, "0"));
                let digits =
                    |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
                (digits(whole) && digits(fraction)).then(|| text.parse().ok())?
            }
            Given::Node(
                Node {
                    kind: Kind::Decimal(number),
                    ..
                },
                _,
            ) => *number,
            Given::Node(
                Node {
                    kind: Kind::Integer(number),
                    ..
                },
                _,
            ) => number.map(|integer| integer as f64),
            Given::Node(..) => None,
        };
        number.filter(|number| number.is_finite())
    }

    /// A TOML array's items, each with its own line.
    pub fn items(&self) -> Option<Vec<Value<'a>>> {
        let Given::Node(
            Node {
                kind: Kind::List(items),
                ..
            },
            text,
        ) = self.given
        else {
            return None;
        };
        Some(
            items
                .iter()
                .map(|item| self.inner(self.key().to_owned(), item, text))
                .collect(),
        )
    }

    /// A TOML table's keys in the order they are written, each value with its own line.
    pub fn fields(&self) -> Option<Vec<(&'a str, Value<'a>)>> {
        let Given::Node(
            Node {
                kind: Kind::Table(fields),
                ..
            },
            text,
        ) = self.given
        else {
            return None;
        };
        Some(
            fields
                .iter()
                .map(|field| {
                    let key = format!("{}.{}", self.key(), segment(&field.name));
                    (field.name.as_str(), self.inner(key, &field.value, text))
                })
                .collect(),
        )
    }

    /// Another variable, for a value read against one: `~/` against `HOME`.
    pub fn var(&self, name: &str) -> Option<OsString> {
        (self.source.var)(name)
    }

    pub fn at(&self) -> &At {
        &self.at
    }

    /// The error for a value outside what its key allows.
    pub fn refuse(&self) -> Error {
        Error::Invalid {
            at: self.at.clone(),
            allowed: self.about.allowed,
            got: self.got(),
        }
    }

    fn key(&self) -> &str {
        match &self.at {
            At::Line { key, .. } => key,
            _ => self.about.name,
        }
    }

    fn inner(&self, key: String, node: &'a Node, text: &'a str) -> Value<'a> {
        let at = match &self.at {
            At::Default(name) => At::Default(name),
            _ => At::Line {
                path: self.source.file.path().to_path_buf(),
                line: self.source.file.line(node.span.start),
                key,
            },
        };
        Value::new(self.about, self.source, at, Given::Node(node, text))
    }

    fn got(&self) -> String {
        match &self.given {
            Given::Text(text) => format!("{:?}", text.to_string_lossy()),
            Given::Node(
                Node {
                    kind: Kind::Table(_),
                    ..
                },
                _,
            ) => "a table".to_owned(),
            Given::Node(node, text) => text.get(node.span.clone()).unwrap_or_default().to_owned(),
        }
    }
}

/// A key name as it is written in a dotted path: bare when it can be, else quoted.
pub(crate) fn segment(name: &str) -> String {
    let bare = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare { name.to_owned() } else { quote(name) }
}

/// `text` as a TOML basic string.
pub fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for c in text.chars() {
        match c {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\t' => quoted.push_str("\\t"),
            '\r' => quoted.push_str("\\r"),
            c if c.is_control() => {
                let _ = write!(quoted, "\\u{:04X}", u32::from(c));
            }
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use super::{About, Default, Flag, Key, Origin, Resolved, Source, quote};
    use crate::error::Error;
    use crate::file::File;
    use crate::folder::{self, Folder};
    use crate::size::{self, GIB, MIB};

    const PATH: &str = "/c/config.toml";

    const SEAL_SIZE: Key<u64> = Key {
        about: About {
            name: "store.seal_at",
            variable: Some("BILBO_STORE_SEAL_AT"),
            default: Default::Toml(r#""8 MiB""#),
            allowed: "between 1 MiB and 1 GiB",
            doc: "a size key",
        },
        parse: |value| {
            size::read(value)
                .filter(|bytes| (MIB..=GIB).contains(bytes))
                .ok_or_else(|| value.refuse())
        },
        show: |bytes| size::show(*bytes),
    };

    const LEVEL: Key<i64> = Key {
        about: About {
            name: "store.compression",
            variable: Some("BILBO_STORE_COMPRESSION"),
            default: Default::Toml("19"),
            allowed: "a zstd level from 1 to 22",
            doc: "an integer key",
        },
        parse: |value| {
            value
                .integer()
                .filter(|level| (1..=22).contains(level))
                .ok_or_else(|| value.refuse())
        },
        show: i64::to_string,
    };

    const ROOT: Key<PathBuf> = Key {
        about: About {
            name: "paths.root",
            variable: Some("BILBO_HOME"),
            default: Default::Folder(Folder {
                xdg: "XDG_DATA_HOME",
                under_xdg: "bilbo",
                under_home: ".local/share/bilbo",
            }),
            allowed: "an absolute path or one starting with ~/",
            doc: "a path key",
        },
        parse: folder::path,
        show: |path| folder::show(path),
    };

    const PAIRS: Key<Vec<(String, i64)>> = Key {
        about: About {
            name: "follow",
            variable: None,
            default: Default::Toml("[]"),
            allowed: "a list of tables, each with a name and a size",
            doc: "a list key",
        },
        parse: |value| {
            let Some(items) = value.items() else {
                return Ok(Vec::new());
            };
            let mut pairs = Vec::new();
            for item in items {
                let fields = item.fields().ok_or_else(|| item.refuse())?;
                let mut name = None;
                let mut size = None;
                for (key, field) in fields {
                    match key {
                        "name" => {
                            name = Some(field.text().ok_or_else(|| field.refuse())?.to_owned());
                        }
                        "size" => size = Some(field.integer().ok_or_else(|| field.refuse())?),
                        _ => return Err(field.refuse()),
                    }
                }
                pairs.push((
                    name.ok_or_else(|| item.refuse())?,
                    size.ok_or_else(|| item.refuse())?,
                ));
            }
            Ok(pairs)
        },
        show: |pairs| format!("{pairs:?}"),
    };

    const RATIO: Key<f64> = Key {
        about: About {
            name: "search.ratio",
            variable: Some("BILBO_SEARCH_RATIO"),
            default: Default::Toml("0.75"),
            allowed: "a number from 0 to 1",
            doc: "a decimal key",
        },
        parse: |value| {
            value
                .decimal()
                .filter(|ratio| (0.0..=1.0).contains(ratio))
                .ok_or_else(|| value.refuse())
        },
        show: f64::to_string,
    };

    const ANY: Key<f64> = Key {
        about: About {
            name: "search.any",
            variable: Some("BILBO_SEARCH_ANY"),
            default: Default::Toml("0.0"),
            allowed: "a number",
            doc: "a decimal key",
        },
        parse: |value| value.decimal().ok_or_else(|| value.refuse()),
        show: f64::to_string,
    };

    fn file(text: &str) -> File {
        File::parse(Path::new(PATH), text.as_bytes()).expect("toml")
    }

    fn get<T>(key: &Key<T>, env: &[(&str, &str)], text: &str) -> Result<Resolved<T>, Error> {
        let var = |name: &str| {
            env.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        };
        let file = file(text);
        key.resolve(&Source {
            var: &var,
            file: &file,
        })
    }

    fn message<T: std::fmt::Debug>(result: Result<Resolved<T>, Error>) -> String {
        result.expect_err("an error").to_string()
    }

    fn in_file(line: usize) -> Origin {
        Origin::File {
            path: PathBuf::from(PATH),
            line,
        }
    }

    #[test]
    fn a_flag_beats_a_variable_beats_the_file_beats_the_default() {
        let text = "[store]\nseal_at = \"16 MiB\"\n";
        let env = [("BILBO_STORE_SEAL_AT", "32MiB")];
        let flag = || Some(flag_of(&SEAL_SIZE, "--seal-at", "64 MiB").expect("valid"));
        let resolved = |value, origin| Ok(Resolved { value, origin });
        assert_eq!(
            get(&SEAL_SIZE, &env, text).map(|r| r.with_flag(flag())),
            resolved(64 * MIB, Origin::Flag("--seal-at"))
        );
        assert_eq!(
            get(&SEAL_SIZE, &env, text),
            resolved(32 * MIB, Origin::Variable("BILBO_STORE_SEAL_AT"))
        );
        assert_eq!(get(&SEAL_SIZE, &[], text), resolved(16 * MIB, in_file(2)));
        assert_eq!(
            get(&SEAL_SIZE, &[("BILBO_STORE_SEAL_AT", "")], text),
            resolved(16 * MIB, in_file(2))
        );
        assert_eq!(get(&SEAL_SIZE, &[], ""), resolved(8 * MIB, Origin::Default));
        assert_eq!(
            get(&LEVEL, &[], ""),
            Ok(Resolved {
                value: 19,
                origin: Origin::Default
            })
        );
        assert_eq!(
            get(
                &LEVEL,
                &[("BILBO_STORE_COMPRESSION", "3")],
                "store.compression = 5"
            ),
            Ok(Resolved {
                value: 3,
                origin: Origin::Variable("BILBO_STORE_COMPRESSION")
            })
        );
    }

    fn flag_of<T>(key: &Key<T>, name: &'static str, text: &str) -> Result<Flag<T>, Error> {
        let var = |_: &str| None;
        let file = file("");
        Flag::parse(
            key,
            &Source {
                var: &var,
                file: &file,
            },
            name,
            text,
        )
    }

    #[test]
    fn a_flag_text_is_read_by_the_keys_own_grammar_and_refused_in_its_name() {
        let read = |text: &str| flag_of(&LEVEL, "--level", text);
        assert_eq!(read("7").map(|flag| flag.value), Ok(7));
        assert_eq!(
            read("x").expect_err("refused").to_string(),
            "--level must be a zstd level from 1 to 22, got \"x\""
        );
        assert_eq!(
            read("23").expect_err("refused").to_string(),
            "--level must be a zstd level from 1 to 22, got \"23\""
        );
        assert_eq!(
            flag_of(&SEAL_SIZE, "--seal-at", "8M")
                .expect_err("refused")
                .to_string(),
            "--seal-at must be between 1 MiB and 1 GiB, got \"8M\""
        );
    }

    #[test]
    fn a_flag_replaces_a_resolved_value_and_its_origin() {
        let resolved = || get(&LEVEL, &[], "[store]\ncompression = 3\n").expect("file");
        let flag = flag_of(&LEVEL, "--level", "9").expect("valid");
        assert_eq!(
            resolved().with_flag(Some(flag)),
            Resolved {
                value: 9,
                origin: Origin::Flag("--level")
            }
        );
        assert_eq!(
            resolved().with_flag(None),
            Resolved {
                value: 3,
                origin: in_file(2)
            }
        );
    }

    #[test]
    fn an_invalid_variable_is_refused_even_when_a_flag_would_win() {
        let flag = flag_of(&LEVEL, "--level", "9").expect("valid");
        let resolved = get(&LEVEL, &[("BILBO_STORE_COMPRESSION", "0")], "")
            .map(|resolved| resolved.with_flag(Some(flag)));
        assert_eq!(
            message(resolved),
            "BILBO_STORE_COMPRESSION must be a zstd level from 1 to 22, got \"0\""
        );
    }

    #[test]
    fn an_origin_reads_as_the_config_verb_prints_it() {
        assert_eq!(Origin::Default.to_string(), "default");
        assert_eq!(in_file(7).to_string(), "/c/config.toml:7");
        assert_eq!(
            Origin::Variable("BILBO_STORE_SEAL_AT").to_string(),
            "BILBO_STORE_SEAL_AT"
        );
        assert_eq!(Origin::Flag("--budget").to_string(), "--budget");
    }

    #[test]
    fn a_value_out_of_range_names_the_file_the_line_and_the_key() {
        let text = "# bilbo\n\n[store]\n\n# size\n\nseal_at = \"0\"\n";
        assert_eq!(
            message(get(&SEAL_SIZE, &[], text)),
            "/c/config.toml:7: store.seal_at must be between 1 MiB and 1 GiB, got \"0\""
        );
        assert_eq!(
            message(get(&SEAL_SIZE, &[("BILBO_STORE_SEAL_AT", "0")], "")),
            "BILBO_STORE_SEAL_AT must be between 1 MiB and 1 GiB, got \"0\""
        );
        assert_eq!(
            message(get(&LEVEL, &[], "[store]\ncompression = 23\n")),
            "/c/config.toml:2: store.compression must be a zstd level from 1 to 22, got 23"
        );
    }

    #[test]
    fn a_value_of_the_wrong_type_is_refused_as_written() {
        for (text, got) in [
            ("store.seal_at = true", "true"),
            ("store.seal_at = [1, 2]", "[1, 2]"),
            ("store.seal_at = 1.5", "1.5"),
            ("store.seal_at = -1", "-1"),
            ("store.seal_at = \"8 MB\"", "\"8 MB\""),
            ("[store.seal_at]\nx = 1", "a table"),
        ] {
            let error = message(get(&SEAL_SIZE, &[], text));
            assert!(
                error.ends_with(&format!(", got {got}")),
                "{text:?}: {error}"
            );
        }
        assert_eq!(
            message(get(&LEVEL, &[], "store.compression = \"19\"")),
            "/c/config.toml:1: store.compression must be a zstd level from 1 to 22, got \"19\""
        );
        assert_eq!(
            message(get(&LEVEL, &[("BILBO_STORE_COMPRESSION", "high")], "")),
            "BILBO_STORE_COMPRESSION must be a zstd level from 1 to 22, got \"high\""
        );
    }

    #[test]
    fn a_decimal_is_digits_with_at_most_one_point() {
        let read = |text: &str| get(&ANY, &[("BILBO_SEARCH_ANY", text)], "").map(|r| r.value);
        for (text, number) in [
            ("1.2", 1.2),
            ("0.75", 0.75),
            ("1", 1.0),
            ("0", 0.0),
            ("10.50", 10.5),
        ] {
            assert_eq!(read(text), Ok(number), "{text:?}");
        }
        for text in [
            "1e3", ".5", "5.", "-1", "+1", "1_0", "inf", "nan", "1,2", "1.2.3", " 1", "1 ", "1.5e2",
        ] {
            assert!(read(text).is_err(), "{text:?}");
        }
        assert_eq!(
            flag_of(&ANY, "--any", "2.5").map(|flag| flag.value),
            Ok(2.5)
        );
        assert!(flag_of(&ANY, "--any", "1e3").is_err());
        assert_eq!(
            get(&ANY, &[("BILBO_SEARCH_ANY", "")], "search.any = 3").map(|r| r.value),
            Ok(3.0)
        );
    }

    #[test]
    fn a_toml_float_or_integer_is_a_decimal_and_inf_and_nan_are_not() {
        let read = |text: &str| get(&RATIO, &[], text).map(|r| r.value);
        for (text, number) in [
            ("search.ratio = 0.5", 0.5),
            ("search.ratio = 1", 1.0),
            ("search.ratio = 0", 0.0),
            ("search.ratio = 5e-1", 0.5),
            ("search.ratio = 0.2_5", 0.25),
        ] {
            assert_eq!(read(text), Ok(number), "{text:?}");
        }
        for text in ["inf", "nan", "-inf", "+inf", "1e999"] {
            let text = format!("search.any = {text}");
            assert!(get(&ANY, &[], &text).is_err(), "{text:?}");
        }
        for text in [
            "search.ratio = 1.5",
            "search.ratio = -0.5",
            "search.ratio = \"0.5\"",
            "search.ratio = true",
            "search.ratio = [0.5]",
            "search.ratio = 99999999999999999999",
        ] {
            assert!(read(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn a_decimal_too_large_to_hold_is_refused_from_a_variable_and_a_flag() {
        let big = "9".repeat(400);
        assert_eq!(
            message(get(&ANY, &[("BILBO_SEARCH_ANY", &big)], "")),
            format!("BILBO_SEARCH_ANY must be a number, got {big:?}")
        );
        assert_eq!(
            flag_of(&ANY, "--any", &big)
                .expect_err("refused")
                .to_string(),
            format!("--any must be a number, got {big:?}")
        );
        assert_eq!(
            message(get(&ANY, &[], "search.any = 1e999")),
            "/c/config.toml:1: search.any must be a number, got 1e999"
        );
    }

    #[test]
    fn a_refused_decimal_names_where_it_was_set_and_what_was_given() {
        assert_eq!(
            message(get(&RATIO, &[], "# c\n[search]\nratio = 1.5\n")),
            "/c/config.toml:3: search.ratio must be a number from 0 to 1, got 1.5"
        );
        assert_eq!(
            message(get(&RATIO, &[("BILBO_SEARCH_RATIO", "1e3")], "")),
            "BILBO_SEARCH_RATIO must be a number from 0 to 1, got \"1e3\""
        );
        let default = Key {
            about: About {
                default: Default::Toml("2.5"),
                ..RATIO.about
            },
            ..RATIO
        };
        assert_eq!(
            message(get(&default, &[], "")),
            "the default of search.ratio must be a number from 0 to 1, got 2.5"
        );
    }

    #[test]
    fn a_size_is_a_string_with_a_binary_unit_or_an_integer_of_bytes() {
        let size = |text: &str| get(&SEAL_SIZE, &[], text).map(|r| r.value);
        assert_eq!(size("store.seal_at = 16777216"), Ok(16 * MIB));
        assert_eq!(size("store.seal_at = \"1 GiB\""), Ok(GIB));
        assert!(size("store.seal_at = 1048575").is_err());
        assert!(size("store.seal_at = \"1025 MiB\"").is_err());
    }

    #[test]
    fn a_path_is_absolute_or_under_home_and_keeps_the_old_messages() {
        let root = |env: &[(&str, &str)], text: &str| get(&ROOT, env, text);
        let home = [("HOME", "/h")];
        assert_eq!(
            root(&home, "paths.root = \"~/store\"").map(|r| r.value),
            Ok(PathBuf::from("/h/store"))
        );
        assert_eq!(
            root(&[("HOME", "/h"), ("BILBO_HOME", "~/s")], "").map(|r| r.value),
            Ok(PathBuf::from("/h/s"))
        );
        assert_eq!(
            root(&[("HOME", "/h"), ("XDG_DATA_HOME", "/d")], ""),
            Ok(Resolved {
                value: PathBuf::from("/d/bilbo"),
                origin: Origin::Default
            })
        );
        assert_eq!(
            root(&[("HOME", "/h"), ("BILBO_HOME", "")], "paths.root = \"/f\"").map(|r| r.origin),
            Ok(in_file(1))
        );
        assert_eq!(
            message(root(&[("BILBO_HOME", "store"), ("HOME", "/h")], "")),
            "BILBO_HOME is not an absolute path: store"
        );
        assert_eq!(
            message(root(&home, "[paths]\nroot = \"store\"\n")),
            "/c/config.toml:2: paths.root is not an absolute path: store"
        );
        assert_eq!(
            message(root(&home, "paths.root = \"~/a/../b\"")),
            "/c/config.toml:1: paths.root has a . or .. component: ~/a/../b"
        );
        assert_eq!(
            message(root(&[("BILBO_HOME", "/x/./s")], "")),
            "BILBO_HOME has a . or .. component: /x/./s"
        );
        assert_eq!(
            message(root(&[], "paths.root = \"~/s\"")),
            "/c/config.toml:1: paths.root needs HOME, which is not set"
        );
        assert_eq!(message(root(&[], "")), "HOME is not set");
        assert_eq!(
            message(root(&home, "paths.root = \"\"")),
            "/c/config.toml:1: paths.root must be an absolute path or one starting with ~/, got \"\""
        );
        assert_eq!(
            message(root(&home, "paths.root = 5")),
            "/c/config.toml:1: paths.root must be an absolute path or one starting with ~/, got 5"
        );
        assert_eq!(
            root(&[("BILBO_HOME", "/x/.hidden/..a/store")], "").map(|r| r.value),
            Ok(PathBuf::from("/x/.hidden/..a/store"))
        );
    }

    #[test]
    fn a_tilde_value_names_its_source_when_home_is_unusable() {
        let root = |env: &[(&str, &str)], text: &str| message(get(&ROOT, env, text));
        let file_value = "[paths]\nroot = \"~/s\"\n";
        for (home, because) in [
            (None, "is not set"),
            (Some(""), "is empty"),
            (Some("h"), "is not an absolute path: h"),
            (Some("/h/../x"), "has a . or .. component: /h/../x"),
        ] {
            let env: Vec<(&str, &str)> = home.map(|home| ("HOME", home)).into_iter().collect();
            assert_eq!(
                root(&env, file_value),
                format!("/c/config.toml:2: paths.root needs HOME, which {because}")
            );
            let mut with_variable = env.clone();
            with_variable.push(("BILBO_HOME", "~/s"));
            assert_eq!(
                root(&with_variable, ""),
                format!("BILBO_HOME needs HOME, which {because}")
            );
        }
        assert_eq!(
            root(&[("HOME", "")], ""),
            "HOME is empty",
            "a folder default has no value to name"
        );
    }

    #[test]
    fn a_path_holding_a_nul_byte_is_refused() {
        assert_eq!(
            message(get(&ROOT, &[], "paths.root = \"/a\\u0000b\"")),
            "/c/config.toml:1: paths.root must be an absolute path or one starting with ~/, \
             got \"/a\\u0000b\""
        );
        assert_eq!(
            message(get(&ROOT, &[("BILBO_HOME", "/a\0b")], "")),
            "BILBO_HOME must be an absolute path or one starting with ~/, got \"/a\\0b\""
        );
    }

    #[test]
    fn extra_leading_slashes_after_the_tilde_stay_under_home() {
        let root = |env: &[(&str, &str)], text: &str| get(&ROOT, env, text).map(|r| r.value);
        for rest in ["~/store", "~//store", "~///store"] {
            assert_eq!(
                root(&[("HOME", "/h")], &format!("paths.root = \"{rest}\"")),
                Ok(PathBuf::from("/h/store")),
                "{rest}"
            );
            assert_eq!(
                root(&[("HOME", "/h"), ("BILBO_HOME", rest)], ""),
                Ok(PathBuf::from("/h/store")),
                "{rest}"
            );
        }
    }

    #[test]
    fn a_shown_value_is_toml_and_its_bare_form_has_no_quotes() {
        let resolved = get(&SEAL_SIZE, &[], "").expect("default");
        let shown = SEAL_SIZE.shown(&resolved);
        assert_eq!(shown.value, "\"8 MiB\"");
        assert_eq!(shown.to_bare(), "8 MiB");
        let level = LEVEL.shown(&get(&LEVEL, &[], "").expect("default"));
        assert_eq!(
            (level.value.as_str(), level.to_bare().as_str()),
            ("19", "19")
        );
        let odd = ROOT.shown(&Resolved {
            value: PathBuf::from("/a \"b\"\\c\nd"),
            origin: Origin::Default,
        });
        assert_eq!(odd.value, "\"/a \\\"b\\\"\\\\c\\nd\"");
        assert_eq!(odd.to_bare(), "/a \"b\"\\c\nd");
        assert_eq!(quote("\u{1b}x\t"), "\"\\u001Bx\\t\"");
    }

    #[test]
    fn a_default_is_described_as_the_reference_page_writes_it() {
        assert_eq!(SEAL_SIZE.about.default.describe(), "\"8 MiB\"");
        assert_eq!(LEVEL.about.default.describe(), "19");
        assert_eq!(
            ROOT.about.default.describe(),
            "$XDG_DATA_HOME/bilbo, else ~/.local/share/bilbo"
        );
    }

    #[test]
    fn a_list_of_tables_refuses_an_item_at_its_own_line() {
        let text = "[[follow]]\nname = \"a\"\nsize = 1\n\n[[follow]]\nname = \"b\"\nsize = \"x\"\n";
        assert_eq!(
            message(get(&PAIRS, &[], text)),
            "/c/config.toml:7: follow.size must be a list of tables, each with a name and a \
             size, got \"x\""
        );
        let fine = "follow = [{ name = \"a\", size = 1 }, { name = \"b\", size = 2 }]";
        assert_eq!(
            get(&PAIRS, &[], fine).map(|r| r.value),
            Ok(vec![("a".to_owned(), 1), ("b".to_owned(), 2)])
        );
    }

    #[test]
    fn a_list_default_is_read_like_a_list_in_the_file() {
        const IGNORE: Key<Vec<PathBuf>> = Key {
            about: About {
                name: "project.ignore",
                variable: None,
                default: Default::Toml(r#"["~/Developer", "/srv"]"#),
                allowed: "a list of absolute paths or ones starting with ~/",
                doc: "a list key with a list default",
            },
            parse: |value| {
                let items = value.items().ok_or_else(|| value.refuse())?;
                items.iter().map(folder::path).collect()
            },
            show: |paths| format!("{paths:?}"),
        };
        let home = [("HOME", "/h")];
        assert_eq!(
            get(&IGNORE, &home, ""),
            Ok(Resolved {
                value: vec![PathBuf::from("/h/Developer"), PathBuf::from("/srv")],
                origin: Origin::Default
            })
        );
        assert_eq!(
            get(&IGNORE, &home, "project.ignore = [\"/a\"]").map(|r| r.value),
            Ok(vec![PathBuf::from("/a")])
        );
        assert_eq!(
            IGNORE.about.default.describe(),
            r#"["~/Developer", "/srv"]"#
        );
    }

    #[test]
    fn a_default_the_key_refuses_is_named_as_the_default() {
        const BAD: Key<u64> = Key {
            about: About {
                default: Default::Toml(r#""0""#),
                ..SEAL_SIZE.about
            },
            ..SEAL_SIZE
        };
        const NOT_TOML: Key<u64> = Key {
            about: About {
                default: Default::Toml("8 MiB"),
                ..SEAL_SIZE.about
            },
            ..SEAL_SIZE
        };
        assert_eq!(
            message(get(&BAD, &[], "")),
            "the default of store.seal_at must be between 1 MiB and 1 GiB, got \"0\""
        );
        assert_eq!(
            message(get(&NOT_TOML, &[], "")),
            "the default of store.seal_at must be between 1 MiB and 1 GiB, got 8 MiB"
        );
    }

    #[test]
    fn a_bad_file_value_is_refused_even_when_a_variable_wins() {
        assert_eq!(
            message(get(
                &SEAL_SIZE,
                &[("BILBO_STORE_SEAL_AT", "16MiB")],
                "[store]\nseal_at = \"0\"\n"
            )),
            "/c/config.toml:2: store.seal_at must be between 1 MiB and 1 GiB, got \"0\""
        );
        assert_eq!(
            get(
                &SEAL_SIZE,
                &[("BILBO_STORE_SEAL_AT", "16MiB")],
                "[store]\nseal_at = \"2 MiB\"\n"
            ),
            Ok(Resolved {
                value: 16 * MIB,
                origin: Origin::Variable("BILBO_STORE_SEAL_AT")
            })
        );
    }

    #[test]
    fn a_scalar_key_written_as_a_table_is_refused_whatever_the_variables_say() {
        let error = message(get(
            &ROOT,
            &[("BILBO_HOME", "/r"), ("HOME", "/h")],
            "[paths.root]\nunknown = 1\n",
        ));
        assert!(error.starts_with("/c/config.toml:"), "{error}");
        assert!(
            error.ends_with(
                "paths.root must be an absolute path or one starting with ~/, got a table"
            ),
            "{error}"
        );
    }

    fn deepest(value: &super::Value<'_>) -> Error {
        match value.fields().and_then(|fields| fields.into_iter().next()) {
            Some((_, inner)) => deepest(&inner),
            None => value.refuse(),
        }
    }

    const DEEP: Key<()> = Key {
        about: About {
            name: "scope",
            variable: None,
            default: Default::Toml("{}"),
            allowed: "nothing",
            doc: "a table key",
        },
        parse: |value| Err(deepest(value)),
        show: |()| String::new(),
    };

    #[test]
    fn an_error_two_tables_deep_names_every_table() {
        assert_eq!(
            message(get(&DEEP, &[], "[scope.work]\npaths = 5\n")),
            "/c/config.toml:2: scope.work.paths must be nothing, got 5"
        );
    }

    #[test]
    fn a_field_name_that_is_not_bare_is_quoted_in_an_error() {
        assert_eq!(
            message(get(&DEEP, &[], "[scope]\n\"a.b\" = 5\n")),
            "/c/config.toml:2: scope.\"a.b\" must be nothing, got 5"
        );
    }

    #[test]
    fn a_refused_item_of_a_list_default_is_named_as_the_default() {
        const IGNORE: Key<Vec<PathBuf>> = Key {
            about: About {
                name: "project.ignore",
                variable: None,
                default: Default::Toml(r#"["/a", "rel"]"#),
                allowed: "a list of absolute paths",
                doc: "a list key",
            },
            parse: |value| {
                let items = value.items().ok_or_else(|| value.refuse())?;
                items.iter().map(folder::path).collect()
            },
            show: |paths| format!("{paths:?}"),
        };
        assert_eq!(
            message(get(&IGNORE, &[("HOME", "/h")], "")),
            "the default of project.ignore is not an absolute path: rel"
        );
    }

    #[test]
    fn a_path_variable_that_is_not_utf8_is_the_path() {
        use std::os::unix::ffi::OsStringExt;
        let var =
            |name: &str| (name == "BILBO_HOME").then(|| OsString::from_vec(b"/x/\xff".to_vec()));
        let file = file("");
        let resolved = ROOT.resolve(&Source {
            var: &var,
            file: &file,
        });
        assert_eq!(
            resolved.map(|r| r.value),
            Ok(PathBuf::from(OsString::from_vec(b"/x/\xff".to_vec())))
        );
    }
}
