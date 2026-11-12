use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use toml::Spanned;
use toml::de::{DeTable, DeValue};

use crate::error::Error;
use crate::key::segment;

/// A config file's keys, each with where it is written; no type of the toml crate leaks out.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct File {
    path: PathBuf,
    text: String,
    fields: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Field {
    pub(crate) name: String,
    pub(crate) key: Range<usize>,
    pub(crate) value: Node,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Node {
    pub(crate) span: Range<usize>,
    pub(crate) kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Kind {
    Text(String),
    /// `None` past the range of `i64`.
    Integer(Option<i64>),
    /// `None` for `inf`, `nan` and a float past the range of `f64`.
    Decimal(Option<f64>),
    List(Vec<Node>),
    Table(Vec<Field>),
    /// A boolean or a date and time.
    Other,
}

/// Which rule picked a config file's path, and so what its absence means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Named {
    /// The default location: no file there is an empty one.
    ByDefault,
    /// The variable named the file: no file there is an error.
    ByVariable(&'static str),
}

impl File {
    /// No file: every key takes its flag, its variable or its default.
    pub(crate) fn empty(path: &Path) -> File {
        File {
            path: path.to_path_buf(),
            ..File::default()
        }
    }

    /// The file at `path` from what reading it gave. A missing file is an empty one, unless a
    /// variable named it.
    pub fn load(path: &Path, named: Named, read: io::Result<Vec<u8>>) -> Result<File, Error> {
        match read {
            Ok(bytes) => File::parse(path, &bytes),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                match named {
                    Named::ByDefault => Ok(File::empty(path)),
                    Named::ByVariable(variable) => Err(Error::Missing {
                        variable,
                        path: path.to_path_buf(),
                    }),
                }
            }
            Err(error) => Err(Error::Read {
                path: path.to_path_buf(),
                reason: error.to_string(),
            }),
        }
    }

    /// The file's keys; a file that is not UTF-8 or not TOML is the error naming its line.
    pub fn parse(path: &Path, bytes: &[u8]) -> Result<File, Error> {
        let text = std::str::from_utf8(bytes).map_err(|error| Error::NotUtf8 {
            path: path.to_path_buf(),
            line: line_of(bytes, error.valid_up_to()),
        })?;
        let root = DeTable::parse(text).map_err(|error| syntax(path, text, &error))?;
        Ok(File {
            path: path.to_path_buf(),
            text: text.to_owned(),
            fields: fields(root.get_ref()),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The first key in the file that `known` does not name. A known key may hold a table,
    /// which is then its own; any other table is walked to its keys.
    pub fn check(&self, known: &[&'static str]) -> Result<(), Error> {
        let mut unknown = Vec::new();
        walk(&self.fields, "", known, &mut unknown);
        unknown
            .into_iter()
            .min_by_key(|(_, at)| *at)
            .map_or(Ok(()), |(key, at)| {
                Err(Error::Unknown {
                    path: self.path.clone(),
                    line: self.line(at),
                    key,
                    keys: known.to_vec(),
                })
            })
    }

    pub(crate) fn find(&self, name: &str) -> Option<&Node> {
        let mut parts = name.split('.');
        let first = parts.next()?;
        let mut node = &self.fields.iter().find(|field| field.name == first)?.value;
        for part in parts {
            let Kind::Table(fields) = &node.kind else {
                return None;
            };
            node = &fields.iter().find(|field| field.name == part)?.value;
        }
        Some(node)
    }

    pub(crate) fn line(&self, at: usize) -> usize {
        line_of(self.text.as_bytes(), at)
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

/// A TOML value written alone, as a default is: `"8 MiB"`, `19`, `["~/a"]`.
pub(crate) fn literal(text: &str) -> Option<Node> {
    DeValue::parse(text).ok().map(|value| node(&value))
}

fn line_of(bytes: &[u8], at: usize) -> usize {
    1 + bytes[..at.min(bytes.len())]
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count()
}

fn syntax(path: &Path, text: &str, error: &toml::de::Error) -> Error {
    let span = error.span().unwrap_or_default();
    let mut message = error
        .message()
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    if message == "duplicate key"
        && let Some(key) = text.get(span.clone())
    {
        message = format!("{message} {key}");
    }
    Error::Syntax {
        path: path.to_path_buf(),
        line: line_of(text.as_bytes(), span.start),
        message,
    }
}

fn fields(table: &DeTable<'_>) -> Vec<Field> {
    let mut fields: Vec<Field> = table
        .iter()
        .map(|(key, value)| Field {
            name: key.get_ref().to_string(),
            key: key.span(),
            value: node(value),
        })
        .collect();
    fields.sort_by_key(|field| field.key.start);
    fields
}

fn node(value: &Spanned<DeValue<'_>>) -> Node {
    let kind = match value.get_ref() {
        DeValue::String(text) => Kind::Text(text.to_string()),
        DeValue::Integer(integer) => {
            Kind::Integer(i64::from_str_radix(integer.as_str(), integer.radix()).ok())
        }
        DeValue::Float(float) => {
            Kind::Decimal(float.as_str().parse().ok().filter(|f: &f64| f.is_finite()))
        }
        DeValue::Array(items) => Kind::List(items.iter().map(node).collect()),
        DeValue::Table(table) => Kind::Table(fields(table)),
        _ => Kind::Other,
    };
    Node {
        span: value.span(),
        kind,
    }
}

fn walk(fields: &[Field], prefix: &str, known: &[&str], unknown: &mut Vec<(String, usize)>) {
    for field in fields {
        let name = if prefix.is_empty() {
            segment(&field.name)
        } else {
            format!("{prefix}.{}", segment(&field.name))
        };
        if known.contains(&name.as_str()) {
            continue;
        }
        match &field.value.kind {
            Kind::Table(inner) if !inner.is_empty() => walk(inner, &name, known, unknown),
            Kind::Table(_) if known.iter().any(|key| is_prefix(&name, key)) => {}
            _ => unknown.push((name, field.key.start)),
        }
    }
}

fn is_prefix(table: &str, key: &str) -> bool {
    key.strip_prefix(table)
        .is_some_and(|rest| rest.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::{Path, PathBuf};

    use super::{File, Named};
    use crate::error::Error;

    const KEYS: [&str; 6] = [
        "paths.root",
        "paths.state",
        "paths.cache",
        "store.seal_at",
        "store.compression",
        "store.tear_budget",
    ];
    const PATH: &str = "/c/config.toml";

    fn parse(text: &str) -> Result<File, Error> {
        File::parse(Path::new(PATH), text.as_bytes())
    }

    fn message(result: Result<File, Error>) -> String {
        result
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default()
    }

    #[test]
    fn an_unknown_key_names_the_file_its_line_and_every_key() {
        let file = parse("[store]\nseal_at = \"8 MiB\"\n\n# a typo\nsealat = 1\n").expect("toml");
        assert_eq!(
            file.check(&KEYS).err().map(|e| e.to_string()),
            Some(
                "/c/config.toml:5: unknown key store.sealat; the keys are paths.root, \
                 paths.state, paths.cache, store.seal_at, store.compression, store.tear_budget"
                    .to_owned()
            )
        );
    }

    #[test]
    fn the_first_unknown_key_in_the_text_is_the_one_reported() {
        let file = parse("zz = 1\n[aa]\nb = 2\n").expect("toml");
        match file.check(&KEYS) {
            Err(Error::Unknown { line, key, .. }) => assert_eq!((line, key.as_str()), (1, "zz")),
            other => panic!("{other:?}"),
        }
        let file = parse("[stor]\nseal_at = 1\n").expect("toml");
        match file.check(&KEYS) {
            Err(Error::Unknown { line, key, .. }) => {
                assert_eq!((line, key.as_str()), (2, "stor.seal_at"));
            }
            other => panic!("{other:?}"),
        }
        let file = parse("[\"paths.root\"]\n").expect("toml");
        match file.check(&KEYS) {
            Err(Error::Unknown { key, .. }) => assert_eq!(key, "\"paths.root\""),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_empty_table_passes_when_it_is_the_prefix_of_a_known_key() {
        for text in [
            "[store]\n",
            "[paths]\n[store]\n",
            "[paths]\n\n# nothing yet\n",
        ] {
            let file = parse(text).expect("toml");
            assert_eq!(file.check(&KEYS), Ok(()), "{text:?}");
        }
    }

    #[test]
    fn an_empty_table_that_is_no_prefix_is_unknown_at_its_header() {
        let file = parse("[store]\n\n[stor]\n").expect("toml");
        match file.check(&KEYS) {
            Err(Error::Unknown { line, key, .. }) => assert_eq!((line, key.as_str()), (3, "stor")),
            other => panic!("{other:?}"),
        }
        let file = parse("[store.seal]\n").expect("toml");
        match file.check(&KEYS) {
            Err(Error::Unknown { line, key, .. }) => {
                assert_eq!((line, key.as_str()), (1, "store.seal"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_known_key_may_hold_a_whole_table() {
        let file =
            parse("[scope.work]\npaths = [\"/w\"]\n[scope.home]\nsync = \"off\"\n").expect("toml");
        assert_eq!(file.check(&["scope"]), Ok(()));
        assert!(file.check(&["scope.work"]).is_err());
    }

    #[test]
    fn every_known_key_in_tables_or_dotted_form_passes() {
        let file = parse(
            "store.compression = 3\nstore.seal_at = 1\n[paths]\nroot = \"/r\"\nstate = \"/s\"\n",
        );
        assert_eq!(message(file.clone()), "");
        assert_eq!(file.expect("toml").check(&KEYS), Ok(()));
        assert_eq!(parse("").expect("empty").check(&KEYS), Ok(()));
    }

    #[test]
    fn a_table_set_by_dotted_keys_cannot_get_a_header_later() {
        assert_eq!(
            message(parse("paths.root = \"/r\"\n[paths]\nstate = \"/s\"\n")),
            "/c/config.toml:2: duplicate key paths"
        );
    }

    #[test]
    fn a_key_set_twice_is_refused_at_its_second_line() {
        assert_eq!(
            message(parse("[store]\nseal_at = 1\n\nseal_at = 2\n")),
            "/c/config.toml:4: duplicate key seal_at"
        );
        assert_eq!(
            message(parse("[store]\nseal_at = 1\n[store]\ncompression = 2\n")),
            "/c/config.toml:3: duplicate key store"
        );
    }

    #[test]
    fn a_file_that_is_not_toml_names_the_line_and_the_fault() {
        for (text, expected) in [
            ("[store]\nseal_at\n", "2: key with no value, expected `=`"),
            (
                "[store]\nseal_at = \"8 MiB\n",
                "2: invalid basic string, expected `\"`",
            ),
            ("x = 1\ny = = 2\n", "2: extra `=`, expected nothing"),
            ("[store\n", "1: unclosed table, expected `]`"),
            (
                "a = 1 b = 2\n",
                "1: unexpected key or value, expected newline, `#`",
            ),
        ] {
            assert_eq!(
                message(parse(text)),
                format!("{PATH}:{expected}"),
                "{text:?}"
            );
        }
        assert_eq!(message(parse("\u{feff}store.seal_at = 1\n")), "");
    }

    #[test]
    fn a_file_that_is_not_utf8_names_the_line_of_the_first_bad_byte() {
        let bytes = b"[store]\nseal_at = \"\xff\"\n";
        assert_eq!(
            File::parse(Path::new(PATH), bytes).err(),
            Some(Error::NotUtf8 {
                path: PathBuf::from(PATH),
                line: 2
            })
        );
    }

    #[test]
    fn a_missing_file_is_empty_unless_a_variable_named_it() {
        let missing = || Err(io::Error::from(io::ErrorKind::NotFound));
        let path = Path::new(PATH);
        assert_eq!(
            File::load(path, Named::ByDefault, missing()),
            Ok(File::empty(path))
        );
        assert_eq!(
            File::load(
                path,
                Named::ByDefault,
                Err(io::Error::from(io::ErrorKind::NotADirectory))
            ),
            Ok(File::empty(path))
        );
        assert_eq!(
            message(File::load(
                path,
                Named::ByVariable("BILBO_CONFIG"),
                missing()
            )),
            "BILBO_CONFIG names a file that does not exist: /c/config.toml"
        );
        assert_eq!(
            message(File::load(
                path,
                Named::ByDefault,
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            )),
            "cannot read /c/config.toml: permission denied"
        );
        assert_eq!(
            File::load(
                path,
                Named::ByVariable("BILBO_CONFIG"),
                Ok(b"x = 1\n".to_vec())
            )
            .map(|f| f.fields.len()),
            Ok(1)
        );
    }

    #[test]
    fn a_path_that_is_not_a_regular_file_is_a_read_error_in_the_readers_words() {
        let refused = Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
        assert_eq!(
            message(File::load(Path::new(PATH), Named::ByDefault, refused)),
            "cannot read /c/config.toml: not a regular file"
        );
    }

    #[test]
    fn integers_keep_their_value_in_every_base_and_with_underscores() {
        let file =
            parse("a = 0x10\nb = 8_388_608\nc = -3\nd = 99999999999999999999\n").expect("toml");
        let integer = |name: &str| match file.find(name).map(|node| &node.kind) {
            Some(super::Kind::Integer(value)) => *value,
            other => panic!("{name}: {other:?}"),
        };
        assert_eq!(integer("a"), Some(16));
        assert_eq!(integer("b"), Some(8_388_608));
        assert_eq!(integer("c"), Some(-3));
        assert_eq!(integer("d"), None);
    }
}
