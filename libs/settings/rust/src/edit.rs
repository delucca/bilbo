//! The config file on disk: the loader's read, and every byte bilbo writes to it.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use rustix::fs::{FileType, Mode, OFlags};

use crate::error::Error;
use crate::file::{self, File, Kind};
use crate::key::{quote, segment};

/// A file's bytes, read only from a regular file: a named pipe or a device would block the read.
pub fn read_regular(path: &Path) -> io::Result<Vec<u8>> {
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let stat = rustix::fs::fstat(&fd)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    fs::File::from(fd).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Whether the file exists. A link (a Nix-managed config), a read-only file or another account's
/// file is refused: replacing it would cut it from what manages it.
pub fn check(config: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(config) {
        Ok(meta) if meta.file_type().is_symlink() => Err("it is a symbolic link".to_owned()),
        Ok(meta) if !meta.is_file() => Err("it is not a regular file".to_owned()),
        Ok(_) => fs::OpenOptions::new()
            .write(true)
            .open(config)
            .map(|_| true)
            .map_err(|_| "it is read-only".to_owned()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

/// The bytes once `check` passes; none when there is no file.
pub fn read(config: &Path) -> Result<Vec<u8>, String> {
    if check(config)? {
        fs::read(config).map_err(|error| error.to_string())
    } else {
        Ok(Vec::new())
    }
}

/// `<config>.lock` beside the file, created private with its folder, never removed. The caller
/// holds the lock from its read through `replace`.
pub fn lock_file(config: &Path) -> io::Result<fs::File> {
    let folder = config
        .parent()
        .ok_or_else(|| io::Error::other("it has no folder"))?;
    fs::create_dir_all(folder)?;
    let mut name = config
        .file_name()
        .ok_or_else(|| io::Error::other("it has no name"))?
        .to_owned();
    name.push(".lock");
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(folder.join(name))
}

/// `tables` alone, or after `before` and a blank line.
pub fn appended(before: &[u8], tables: &str) -> Vec<u8> {
    appended_with(before, tables, "\n")
}

fn appended_with(before: &[u8], tables: &str, eol: &str) -> Vec<u8> {
    let mut text = before.to_vec();
    if !text.is_empty() {
        if !text.ends_with(b"\n") {
            text.extend_from_slice(eol.as_bytes());
        }
        text.extend_from_slice(eol.as_bytes());
    }
    text.extend_from_slice(tables.as_bytes());
    text
}

/// Whether the key declared at `line` holds an inline value: only the `[[name]]` form starts it
/// on a `[[` header.
pub fn inline(before: &[u8], line: usize) -> bool {
    before
        .split(|&byte| byte == b'\n')
        .nth(line.saturating_sub(1))
        .is_some_and(|text| !text.trim_ascii_start().starts_with(b"[["))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    Dotted,
    Inline,
    OwnTable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Misplaced {
    pub how: How,
    pub line: usize,
    pub holds: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Failed {
    Misplaced(Misplaced),
    Changed,
    Unreadable(Error),
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Misplaced(found) => {
                let how = match (found.how, found.holds) {
                    (How::Dotted, true) => "sets it with a dotted key",
                    (How::Dotted, false) => "sets its table with a dotted key",
                    (How::Inline, true) => "sets it inside braces",
                    (How::Inline, false) => "sets its table inside braces",
                    (How::OwnTable, _) => "sets it as a table of its own",
                };
                write!(f, "line {} {how}", found.line)
            }
            Self::Changed => f.write_str("bilbo could not place it"),
            Self::Unreadable(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Failed {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value(String);

impl Value {
    pub fn literal(text: &str) -> Option<Value> {
        file::literal(text).map(|_| Value(text.to_owned()))
    }

    pub fn text(text: &str) -> Value {
        Value(quote(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const BOM: &str = "\u{feff}";

fn bom(text: &str) -> usize {
    if text.starts_with(BOM) { BOM.len() } else { 0 }
}

fn eol(text: &str) -> &'static str {
    match text.find('\n') {
        Some(at) if text[..at].ends_with('\r') => "\r\n",
        _ => "\n",
    }
}

pub fn set(path: &Path, before: &[u8], key: &str, value: &Value) -> Result<Vec<u8>, Failed> {
    let value = value.as_str();
    let parsed = File::parse(path, before).map_err(Failed::Unreadable)?;
    let text = parsed.text();
    let lines = scan(text);
    let parts: Vec<&str> = key.split('.').collect();
    let (name, table) = parts.split_last().expect("a split gives one part");
    let held = |header: &Header| {
        header.path.len() >= parts.len() && header.path[..parts.len()] == parts[..]
    };
    if let Some((index, _)) = lines
        .iter()
        .enumerate()
        .find(|(_, line)| line.header.as_ref().is_some_and(held))
    {
        return Err(misplaced(How::OwnTable, index + 1, true));
    }
    let eol = eol(text);
    let after = if let Some(node) = parsed.find(key) {
        let start = node.span.start;
        let line = parsed.line(start);
        let begins = lines[line - 1].start;
        let prefix = text[begins..start].trim_start_matches(BOM);
        let plain = prefix
            .trim_end()
            .strip_suffix('=')
            .is_some_and(|key_text| same_key(key_text.trim(), name));
        if !plain {
            let how = if prefix.contains('{') {
                How::Inline
            } else {
                How::Dotted
            };
            return Err(misplaced(how, line, true));
        }
        [&before[..start], value.as_bytes(), &before[node.span.end..]].concat()
    } else {
        for end in 1..parts.len() {
            let prefix = parts[..end].join(".");
            let named = lines.iter().any(|line| {
                line.header
                    .as_ref()
                    .is_some_and(|header| !header.array && header.path == parts[..end])
            });
            if let Some(node) = parsed.find(&prefix)
                && !matches!(node.kind, Kind::Table(_))
            {
                return Err(Failed::Changed);
            }
            if let Some(node) = parsed.find(&prefix)
                && !named
                && lines[parsed.line(node.span.start) - 1].header.is_none()
            {
                let start = node.span.start;
                let how = if text[start..].starts_with('{') {
                    How::Inline
                } else {
                    How::Dotted
                };
                let line = parsed.line(start);
                return Err(misplaced(how, line, false));
            }
        }
        let line = format!("{} = {value}", segment(name));
        let header = lines.iter().position(|candidate| {
            candidate
                .header
                .as_ref()
                .is_some_and(|header| !header.array && header.path == *table)
        });
        match (header, table.is_empty()) {
            (Some(index), _) => {
                let last =
                    anchor(text, &lines, index + 1, section_end(&lines, index)).unwrap_or(index);
                insert_after(text, &lines, last, &line)
            }
            (None, false) => {
                let name = table
                    .iter()
                    .map(|part| segment(part))
                    .collect::<Vec<_>>()
                    .join(".");
                let tables = format!("[{name}]\n{line}\n").replace('\n', eol);
                appended_with(before, &tables, eol)
            }
            (None, true) => {
                let first = lines
                    .iter()
                    .position(|candidate| candidate.header.is_some());
                match first {
                    None if text.trim_start_matches(BOM).is_empty() => {
                        format!("{BOM}{line}{eol}", BOM = &text[..bom(text)]).into_bytes()
                    }
                    None => insert_after(text, &lines, lines.len() - 1, &line),
                    Some(index) => match anchor(text, &lines, 0, index) {
                        Some(last) => insert_after(text, &lines, last, &line),
                        None => {
                            let at = lines[index].start + if index == 0 { bom(text) } else { 0 };
                            [
                                &before[..at],
                                format!("{line}{eol}{eol}").as_bytes(),
                                &before[at..],
                            ]
                            .concat()
                        }
                    },
                }
            }
        }
    };
    placed(path, before, after, key, value)
}

fn placed(
    path: &Path,
    before: &[u8],
    after: Vec<u8>,
    key: &str,
    value: &str,
) -> Result<Vec<u8>, Failed> {
    if File::parse(path, &after).is_err() || !holds_only_the_key(before, &after, key, value) {
        return Err(Failed::Changed);
    }
    Ok(after)
}

#[derive(Debug, PartialEq)]
enum Held {
    Text(String),
    Integer(Option<i128>, String),
    Float(Option<u64>, String),
    Boolean(bool),
    Date(String),
    List(Vec<Held>),
    Table(std::collections::BTreeMap<String, Held>),
}

fn held(value: &toml::de::DeValue<'_>) -> Held {
    use toml::de::DeValue;
    match value {
        DeValue::String(text) => Held::Text(text.to_string()),
        DeValue::Integer(integer) => {
            let digits = integer.as_str().replace('_', "");
            match i128::from_str_radix(&digits, integer.radix()) {
                Ok(number) => Held::Integer(Some(number), String::new()),
                Err(_) => Held::Integer(None, digits),
            }
        }
        DeValue::Float(float) => {
            let digits = float.as_str().replace('_', "");
            match digits.parse::<f64>() {
                Ok(number) => Held::Float(Some(number.to_bits()), String::new()),
                Err(_) => Held::Float(None, digits),
            }
        }
        DeValue::Boolean(boolean) => Held::Boolean(*boolean),
        DeValue::Datetime(date) => Held::Date(date.to_string()),
        DeValue::Array(items) => {
            Held::List(items.iter().map(|item| held(item.get_ref())).collect())
        }
        DeValue::Table(table) => Held::Table(
            table
                .iter()
                .map(|(key, item)| (key.get_ref().to_string(), held(item.get_ref())))
                .collect(),
        ),
    }
}

fn document(bytes: &[u8]) -> Option<Held> {
    let text = std::str::from_utf8(bytes).ok()?;
    let text = text.strip_prefix(BOM).unwrap_or(text);
    let table = toml::de::DeTable::parse(text).ok()?;
    Some(Held::Table(
        table
            .get_ref()
            .iter()
            .map(|(key, item)| (key.get_ref().to_string(), held(item.get_ref())))
            .collect(),
    ))
}

fn holds_only_the_key(before: &[u8], after: &[u8], key: &str, value: &str) -> bool {
    let Some(mut expected) = document(before) else {
        return false;
    };
    let Ok(alone) = toml::de::DeValue::parse(value) else {
        return false;
    };
    let wanted = held(alone.get_ref());
    let parts: Vec<&str> = key.split('.').collect();
    let (name, table) = parts.split_last().expect("a split gives one part");
    let mut at = &mut expected;
    for part in table {
        let Held::Table(map) = at else {
            return false;
        };
        at = map
            .entry((*part).to_owned())
            .or_insert_with(|| Held::Table(std::collections::BTreeMap::new()));
    }
    let Held::Table(map) = at else {
        return false;
    };
    map.insert((*name).to_owned(), wanted);
    document(after).is_some_and(|found| found == expected)
}

fn misplaced(how: How, line: usize, holds: bool) -> Failed {
    Failed::Misplaced(Misplaced { how, line, holds })
}

fn same_key(written: &str, name: &str) -> bool {
    written == name
        || written == quote(name)
        || written
            .strip_prefix('\'')
            .and_then(|rest| rest.strip_suffix('\''))
            == Some(name)
}

fn section_end(lines: &[Line], header: usize) -> usize {
    lines[header + 1..]
        .iter()
        .position(|line| line.header.is_some())
        .map_or(lines.len(), |offset| header + 1 + offset)
}

fn anchor(text: &str, lines: &[Line], from: usize, to: usize) -> Option<usize> {
    (from..to).rev().find(|&index| {
        let content = text[lines[index].start..lines[index].end]
            .trim_start_matches(BOM)
            .trim();
        lines[index].inside || (!content.is_empty() && !content.starts_with('#'))
    })
}

fn insert_after(text: &str, lines: &[Line], at: usize, line: &str) -> Vec<u8> {
    let target = &lines[at];
    let crlf = text[target.start..target.end].ends_with('\r');
    let eol = if crlf { "\r\n" } else { "\n" };
    let mut added = String::new();
    if target.next == target.end {
        added.push_str(eol);
    }
    added.push_str(line);
    added.push_str(eol);
    let mut out = text.as_bytes()[..target.next].to_vec();
    out.extend_from_slice(added.as_bytes());
    out.extend_from_slice(&text.as_bytes()[target.next..]);
    out
}

struct Header {
    path: Vec<String>,
    array: bool,
}

struct Line {
    start: usize,
    end: usize,
    next: usize,
    header: Option<Header>,
    inside: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Open {
    No,
    Basic,
    Literal,
}

fn scan(text: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut depth = 0usize;
    let mut open = Open::No;
    let mut start = 0;
    for raw in text.split_inclusive('\n') {
        let next = start + raw.len();
        let end = start + raw.trim_end_matches('\n').len();
        let content = &text[start..end];
        let content = if start == 0 {
            content.strip_prefix(BOM).unwrap_or(content)
        } else {
            content
        };
        let inside = depth > 0 || open != Open::No;
        let header = (!inside).then(|| header(content)).flatten();
        let bytes = content.as_bytes();
        let mut index = 0;
        let mut single: Option<u8> = None;
        while index < bytes.len() {
            let byte = bytes[index];
            if open != Open::No {
                let quote = if open == Open::Basic { b'"' } else { b'\'' };
                if open == Open::Basic && byte == b'\\' {
                    index += 2;
                    continue;
                }
                if byte == quote && bytes[index..].starts_with(&[quote; 3]) {
                    open = Open::No;
                    index += 3;
                    continue;
                }
            } else if let Some(quote) = single {
                if quote == b'"' && byte == b'\\' {
                    index += 2;
                    continue;
                }
                if byte == quote {
                    single = None;
                }
            } else {
                match byte {
                    b'#' => break,
                    b'"' | b'\'' => {
                        if bytes[index..].starts_with(&[byte; 3]) {
                            open = if byte == b'"' {
                                Open::Basic
                            } else {
                                Open::Literal
                            };
                            index += 3;
                            continue;
                        }
                        single = Some(byte);
                    }
                    b'[' | b'{' => depth += 1,
                    b']' | b'}' => depth = depth.saturating_sub(1),
                    _ => {}
                }
            }
            index += 1;
        }
        lines.push(Line {
            start,
            end,
            next,
            header,
            inside,
        });
        start = next;
    }
    lines
}

fn header(content: &str) -> Option<Header> {
    let rest = content.trim_start().strip_prefix('[')?;
    let (array, rest) = rest
        .strip_prefix('[')
        .map_or((false, rest), |inner| (true, inner));
    let bytes = rest.as_bytes();
    let mut path = Vec::new();
    let mut index = 0;
    loop {
        while bytes.get(index).is_some_and(|b| *b == b' ' || *b == b'\t') {
            index += 1;
        }
        let first = *bytes.get(index)?;
        if first == b'"' || first == b'\'' {
            let begin = index + 1;
            index = begin;
            while *bytes.get(index)? != first {
                index += if first == b'"' && bytes[index] == b'\\' {
                    2
                } else {
                    1
                };
            }
            let name = &rest[begin..index];
            path.push(if first == b'"' {
                name.replace("\\\"", "\"").replace("\\\\", "\\")
            } else {
                name.to_owned()
            });
            index += 1;
        } else {
            let begin = index;
            while bytes
                .get(index)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
            {
                index += 1;
            }
            if begin == index {
                return None;
            }
            path.push(rest[begin..index].to_owned());
        }
        while bytes.get(index).is_some_and(|b| *b == b' ' || *b == b'\t') {
            index += 1;
        }
        if bytes.get(index) == Some(&b'.') {
            index += 1;
        } else {
            break;
        }
    }
    let tail = rest[index..].strip_prefix(']')?;
    let tail = if array { tail.strip_prefix(']')? } else { tail };
    let tail = tail.trim();
    (tail.is_empty() || tail.starts_with('#')).then_some(Header { path, array })
}

/// Temp file then rename, keeping the old permissions; a failed folder sync is returned, not
/// refused.
pub fn replace(config: &Path, text: &[u8]) -> io::Result<Option<String>> {
    let folder = config
        .parent()
        .ok_or_else(|| io::Error::other("it has no folder"))?;
    let name = config
        .file_name()
        .ok_or_else(|| io::Error::other("it has no name"))?;
    let mut temporary = OsString::from(".");
    temporary.push(name);
    temporary.push(format!(".{}.tmp", std::process::id()));
    let temporary = folder.join(temporary);
    let mut file = fs::File::create_new(&temporary)?;
    let renamed = (|| {
        if let Ok(meta) = fs::metadata(config) {
            file.set_permissions(meta.permissions())?;
        }
        file.write_all(text)?;
        file.sync_all()?;
        fs::rename(&temporary, config)
    })();
    if renamed.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    finish(renamed, || fs::File::open(folder)?.sync_all())
}

fn finish(
    renamed: io::Result<()>,
    sync: impl FnOnce() -> io::Result<()>,
) -> io::Result<Option<String>> {
    renamed?;
    Ok(sync().err().map(|error| error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;

    use super::{Failed, How, Misplaced, Value, appended, finish, inline, set};
    use crate::file::File;

    const PATH: &str = "/c/config.toml";

    fn put(before: &str, key: &str, value: &str) -> String {
        let value = Value::literal(value).expect("a literal");
        let after = set(Path::new(PATH), before.as_bytes(), key, &value).expect("set");
        String::from_utf8(after).expect("utf8")
    }

    fn misplaced(before: &str, key: &str) -> Misplaced {
        let one = Value::literal("1").expect("a literal");
        match set(Path::new(PATH), before.as_bytes(), key, &one) {
            Err(Failed::Misplaced(found)) => found,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_value_goes_over_the_old_one_and_no_other_byte_changes() {
        let before = "# mine\r\n\n[store]  \n#  spaced   comment\nseal_at   =    \"8 MiB\"   # why\n\n\n[recall]\nbudget=1\n";
        assert_eq!(
            put(before, "store.seal_at", "\"16 MiB\""),
            before.replace("\"8 MiB\"", "\"16 MiB\"")
        );
        assert_eq!(
            put(before, "recall.budget", "2"),
            before.replace("budget=1", "budget=2")
        );
    }

    #[test]
    fn a_key_new_to_its_table_is_a_line_after_the_tables_last() {
        let before = "[store]\nseal_at = \"8 MiB\"\n# note\n\n[recall]\nbudget = 1\n";
        assert_eq!(
            put(before, "store.compression", "3"),
            "[store]\nseal_at = \"8 MiB\"\ncompression = 3\n# note\n\n[recall]\nbudget = 1\n"
        );
        assert_eq!(
            put("[store]\nseal_at = 1", "store.compression", "3"),
            "[store]\nseal_at = 1\ncompression = 3\n"
        );
        assert_eq!(
            put("[store]\n", "store.compression", "3"),
            "[store]\ncompression = 3\n"
        );
        assert_eq!(
            put(
                "[derive]\nlist = [\n  [1],\n]\n[derive.x]\na = 1\n",
                "derive.enabled",
                "true"
            ),
            "[derive]\nlist = [\n  [1],\n]\nenabled = true\n[derive.x]\na = 1\n"
        );
    }

    #[test]
    fn a_table_new_to_the_file_is_added_at_its_end() {
        assert_eq!(
            put("", "derive.enabled", "true"),
            "[derive]\nenabled = true\n"
        );
        assert_eq!(
            put("[store]\ncompression = 3\n", "derive.enabled", "true"),
            "[store]\ncompression = 3\n\n[derive]\nenabled = true\n"
        );
        assert_eq!(
            put(
                "[store]\ncompression = 3\n",
                "derive.anthropic.model",
                "\"m\""
            ),
            "[store]\ncompression = 3\n\n[derive.anthropic]\nmodel = \"m\"\n"
        );
        assert_eq!(
            put(
                "[derive.anthropic]\nmodel = \"m\"\n",
                "derive.enabled",
                "true"
            ),
            "[derive.anthropic]\nmodel = \"m\"\n\n[derive]\nenabled = true\n"
        );
    }

    #[test]
    fn a_key_with_no_table_goes_before_the_first_table() {
        assert_eq!(
            put("[store]\ncompression = 3\n", "follow", "[]"),
            "follow = []\n\n[store]\ncompression = 3\n"
        );
        assert_eq!(
            put(
                "# top\nother = 1\n\n[store]\ncompression = 3\n",
                "follow",
                "[]"
            ),
            "# top\nother = 1\nfollow = []\n\n[store]\ncompression = 3\n"
        );
        assert_eq!(put("", "follow", "[]"), "follow = []\n");
        assert_eq!(put("other = 1", "follow", "[]"), "other = 1\nfollow = []\n");
    }

    #[test]
    fn a_key_set_with_a_dotted_key_or_in_an_inline_table_is_misplaced_at_its_line() {
        assert_eq!(
            misplaced("# top\nderive.enabled = true\n", "derive.enabled"),
            Misplaced {
                how: How::Dotted,
                line: 2,
                holds: true
            }
        );
        assert_eq!(
            misplaced(
                "[derive]\nanthropic.model = \"m\"\n",
                "derive.anthropic.model"
            ),
            Misplaced {
                how: How::Dotted,
                line: 2,
                holds: true
            }
        );
        assert_eq!(
            misplaced("a = 1\nderive = { enabled = true }\n", "derive.enabled"),
            Misplaced {
                how: How::Inline,
                line: 2,
                holds: true
            }
        );
        assert_eq!(
            misplaced("derive = { }\n", "derive.enabled"),
            Misplaced {
                how: How::Inline,
                line: 1,
                holds: false
            }
        );
        assert_eq!(
            misplaced("derive = { anthropic = {} }\n", "derive.anthropic.model"),
            Misplaced {
                how: How::Inline,
                line: 1,
                holds: false
            }
        );
        assert_eq!(
            misplaced("derive.in = [\n  \"sessions\",\n]\n", "derive.in"),
            Misplaced {
                how: How::Dotted,
                line: 1,
                holds: true
            }
        );
        assert_eq!(
            misplaced("derive.quiet = true\n", "derive.enabled"),
            Misplaced {
                how: How::Dotted,
                line: 1,
                holds: false
            }
        );
    }

    #[test]
    fn a_key_declared_as_a_table_of_its_own_is_misplaced() {
        assert_eq!(
            misplaced(
                "[derive]\nenabled = true\n\n[derive.instructions]\nsession = \"x\"\n",
                "derive.instructions"
            ),
            Misplaced {
                how: How::OwnTable,
                line: 4,
                holds: true
            }
        );
        assert_eq!(
            misplaced("[[follow]]\npath = \"/x\"\n", "follow"),
            Misplaced {
                how: How::OwnTable,
                line: 1,
                holds: true
            }
        );
    }

    #[test]
    fn the_new_text_reads_back_with_the_value_set() {
        let cases = [
            (
                "# c\n[store]\nseal_at = \"8 MiB\"\n",
                "store.seal_at",
                "\"16 MiB\"",
            ),
            ("[store]\nseal_at = 1", "store.compression", "3"),
            ("", "derive.anthropic.model", "\"m\""),
            ("[store]\ncompression = 3\n", "derive.enabled", "true"),
            ("[store]\ncompression = 3\n", "follow", "[]"),
            (
                "[derive]\nenabled = false\n",
                "derive.instructions",
                "{ session = \"x\" }",
            ),
        ];
        for (before, key, value) in cases {
            let after = put(before, key, value);
            let file = File::parse(Path::new(PATH), after.as_bytes()).expect("parses");
            let node = file.find(key).expect("found");
            assert_eq!(&after[node.span.clone()], value, "{key} in {after}");
        }
    }

    #[test]
    fn the_tables_go_after_the_file_as_it_was_with_a_blank_line_between() {
        let tables = "[[follow]]\npath = \"/x\"\n";
        assert_eq!(appended(b"", tables), tables.as_bytes());
        assert_eq!(
            appended(b"[store]\ncompression = 3\n", tables),
            b"[store]\ncompression = 3\n\n[[follow]]\npath = \"/x\"\n"
        );
        assert_eq!(
            appended(b"# no newline at the end", tables),
            b"# no newline at the end\n\n[[follow]]\npath = \"/x\"\n"
        );
        let odd = b"[store]  \r\n#  spaced   comment\r\n\n\n";
        assert!(appended(odd, tables).starts_with(odd));
    }

    #[test]
    fn a_failed_folder_sync_after_the_rename_is_returned_not_refused() {
        assert_eq!(
            finish(Ok(()), || Err(io::Error::other("no sync"))).expect("renamed"),
            Some("no sync".to_owned())
        );
        assert!(
            finish(Err(io::Error::other("no rename")), || panic!(
                "never synced"
            ))
            .is_err()
        );
    }

    #[test]
    fn only_a_double_bracket_header_at_the_line_is_not_inline() {
        assert!(!inline(b"# mine\n[[follow]]\npath = \"/x\"\n", 2));
        assert!(!inline(b"  [[follow]]\npath = \"/x\"\n", 1));
        assert!(inline(b"follow = [{ path = \"/x\" }]\n", 1));
        assert!(inline(b"# mine\nfollow = [\n  { path = \"/x\" },\n]\n", 2));
        assert!(!inline(b"follow = []\n", 3), "a line past the end");
    }

    fn lit(text: &str) -> Value {
        Value::literal(text).expect("a literal")
    }

    fn tried(before: &str, key: &str, value: &str) -> Result<String, Failed> {
        set(Path::new(PATH), before.as_bytes(), key, &lit(value))
            .map(|after| String::from_utf8(after).expect("utf8"))
    }

    fn same(before: &str, after: &str, key: &str, value: &str) -> bool {
        super::holds_only_the_key(before.as_bytes(), after.as_bytes(), key, value)
    }

    #[test]
    fn a_line_never_lands_inside_another_keys_value() {
        let file = "[derive]\nprovider = \"\"\"\nline\n#\"\"\"\n";
        assert_eq!(
            tried(file, "derive.enabled", "true").expect("set"),
            "[derive]\nprovider = \"\"\"\nline\n#\"\"\"\nenabled = true\n"
        );
        let literal = "[derive]\nprovider = '''\nline\n#'''\n";
        assert_eq!(
            tried(literal, "derive.enabled", "true").expect("set"),
            "[derive]\nprovider = '''\nline\n#'''\nenabled = true\n"
        );
        let array = "[derive]\nlist = [\n  \"a\",\n  # last\n]\n";
        assert_eq!(
            tried(array, "derive.enabled", "true").expect("set"),
            "[derive]\nlist = [\n  \"a\",\n  # last\n]\nenabled = true\n"
        );
        let landed = "[derive]\nprovider = \"\"\"\nline\nenabled = true\n#\"\"\"\n";
        assert!(!same(file, landed, "derive.enabled", "true"));
    }

    #[test]
    fn new_bytes_in_which_another_key_holds_another_value_are_refused() {
        let before = "[derive]\na = 1\nb = \"x\"\n";
        assert!(!same(before, before, "derive.c", "2"));
        assert!(!same(
            before,
            "[derive]\na = 1\nb = \"x\"\nc = 3\n",
            "derive.c",
            "2"
        ));
        assert!(!same(before, "[derive]\na = 1\nc = 2\n", "derive.c", "2"));
        assert!(!same(
            before,
            "[derive]\na = 9\nb = \"x\"\nc = 2\n",
            "derive.c",
            "2"
        ));
        assert!(!same(
            "[[follow]]\np = 1\n[[follow]]\np = 2\n",
            "[[follow]]\np = 2\n[[follow]]\np = 1\nz = 1\n",
            "z",
            "1"
        ));
        assert!(same(
            before,
            "[derive]\na = 1\nb = \"x\"\nc = 2\n",
            "derive.c",
            "2"
        ));
        assert!(same(
            before,
            "[derive]\nb = 'x'\na = 1\nc = 2 # c\n",
            "derive.c",
            "2"
        ));
        assert!(same(
            before,
            "[derive]\na = 1\nb = \"x\"\n\n[derive.x.y]\nc = 2\n",
            "derive.x.y.c",
            "2"
        ));
    }

    #[test]
    fn a_bom_is_kept_and_the_file_takes_the_key() {
        assert_eq!(
            tried(
                "\u{feff}[derive]\nprovider = \"a\"\n",
                "derive.enabled",
                "true"
            )
            .expect("set"),
            "\u{feff}[derive]\nprovider = \"a\"\nenabled = true\n"
        );
        assert_eq!(
            tried("\u{feff}[store]\nx = 1\n", "follow", "[]").expect("set"),
            "\u{feff}follow = []\n\n[store]\nx = 1\n"
        );
        assert_eq!(
            tried("\u{feff}", "follow", "[]").expect("set"),
            "\u{feff}follow = []\n"
        );
    }

    #[test]
    fn a_new_table_and_a_first_line_take_the_files_own_line_end() {
        let first = tried("[store]\r\nx = 1\r\n", "follow", "[]").expect("set");
        let table = tried("[store]\r\nx = 1\r\n", "derive.enabled", "true").expect("set");
        for text in [first, table] {
            assert!(!text.replace("\r\n", "").contains('\n'), "{text:?}");
        }
    }

    #[test]
    fn a_key_absent_under_a_dotted_table_is_misplaced_at_the_dotted_line() {
        let found = misplaced(
            "derive.enabled = true\n\n[derive.anthropic]\nmodel = \"m\"\n",
            "derive.quiet",
        );
        assert_eq!(
            found,
            Misplaced {
                how: How::Dotted,
                line: 1,
                holds: false
            }
        );
        let inside = misplaced(
            "[derive]\nanthropic.model = \"m\"\n",
            "derive.anthropic.version",
        );
        assert_eq!(
            inside,
            Misplaced {
                how: How::Dotted,
                line: 2,
                holds: false
            }
        );
        let itself = misplaced("derive.quiet = \"1h\"\n", "derive.quiet");
        assert_eq!(
            itself,
            Misplaced {
                how: How::Dotted,
                line: 1,
                holds: true
            }
        );
    }

    #[test]
    fn a_key_under_a_value_that_is_no_table_cannot_be_placed() {
        for before in ["derive = 1\n", "derive = \"x\"\n", "derive = [1]\n"] {
            assert_eq!(
                tried(before, "derive.enabled", "true"),
                Err(Failed::Changed),
                "{before}"
            );
        }
    }

    #[test]
    fn new_bytes_that_parse_and_change_another_key_are_not_placed() {
        let before = "[derive]\nquiet = \"1h\"\n";
        let changed = b"[derive]\nquiet = \"2h\"\nenabled = true\n".to_vec();
        assert_eq!(
            super::placed(
                Path::new(PATH),
                before.as_bytes(),
                changed,
                "derive.enabled",
                "true"
            ),
            Err(Failed::Changed)
        );
        let right = b"[derive]\nquiet = \"1h\"\nenabled = true\n".to_vec();
        assert_eq!(
            super::placed(
                Path::new(PATH),
                before.as_bytes(),
                right.clone(),
                "derive.enabled",
                "true"
            ),
            Ok(right)
        );
    }

    #[test]
    fn new_bytes_that_do_not_parse_are_not_the_files_fault() {
        let before = "[\"der\\u0069ve\"]\nprovider = \"a\"\n";
        assert_eq!(
            tried(before, "derive.enabled", "true"),
            Err(Failed::Changed)
        );
        assert!(matches!(
            tried("[store\n", "derive.enabled", "true"),
            Err(Failed::Unreadable(_))
        ));
    }

    #[test]
    fn a_temporary_this_run_did_not_make_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("edit-temporary-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let config = dir.join("config.toml");
        std::fs::write(&config, "a = 1\n").expect("a config");
        let other = dir.join(format!(".config.toml.{}.tmp", std::process::id()));
        std::fs::write(&other, "someone else's").expect("a file");
        assert!(super::replace(&config, b"a = 2\n").is_err());
        assert_eq!(
            std::fs::read_to_string(&other).expect("kept"),
            "someone else's"
        );
        assert_eq!(std::fs::read_to_string(&config).expect("kept"), "a = 1\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_line_shaped_like_a_header_inside_a_value_is_no_header() {
        for file in [
            "[derive]\nprovider = \"\"\"\n[derive.x]\n\"\"\"\n",
            "[derive]\nprovider = '''\n[derive.x]\n'''\n",
            "[derive]\nprovider = \"a\"\n\n[\"store\"]\nx = 1\n",
        ] {
            let after = tried(file, "derive.enabled", "true").expect("set");
            let parsed = File::parse(Path::new(PATH), after.as_bytes()).expect("parses");
            assert!(parsed.find("derive.enabled").is_some(), "{after}");
        }
    }

    #[test]
    fn a_value_is_built_as_a_literal_or_as_a_text() {
        assert_eq!(Value::literal("true").expect("literal").as_str(), "true");
        assert!(Value::literal("30m").is_none());
        let text = Value::text("2\nother = 3");
        assert_eq!(text.as_str(), "\"2\\nother = 3\"");
        let after = set(Path::new(PATH), b"[derive]\n", "derive.version", &text).expect("set");
        let file = File::parse(Path::new(PATH), &after).expect("parses");
        assert!(file.find("derive.other").is_none());
        assert_eq!(Value::text("30m").as_str(), "\"30m\"");
    }

    #[test]
    fn a_failure_says_itself() {
        let found = |how| {
            Failed::Misplaced(Misplaced {
                how,
                line: 3,
                holds: true,
            })
        };
        assert_eq!(
            found(How::Dotted).to_string(),
            "line 3 sets it with a dotted key"
        );
        assert_eq!(
            found(How::Inline).to_string(),
            "line 3 sets it inside braces"
        );
        assert_eq!(
            found(How::OwnTable).to_string(),
            "line 3 sets it as a table of its own"
        );
        assert_eq!(
            Failed::Misplaced(Misplaced {
                how: How::Dotted,
                line: 3,
                holds: false
            })
            .to_string(),
            "line 3 sets its table with a dotted key"
        );
        assert_eq!(Failed::Changed.to_string(), "bilbo could not place it");
    }

    #[test]
    fn a_text_that_is_no_toml_has_no_literal_and_becomes_a_quoted_string() {
        for (text, value) in [("30m", "\"30m\""), ("a\"b", "\"a\\\"b\"")] {
            assert!(Value::literal(text).is_none(), "{text}");
            assert_eq!(Value::text(text).as_str(), value);
        }
        for text in ["true", "20000", "0.5", "[\"a\"]", "{ a = \"b\" }"] {
            assert_eq!(Value::literal(text).expect("literal").as_str(), text);
        }
    }
}
