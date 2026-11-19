use std::io;
use std::path::{Path, PathBuf};

use xxhash_rust::xxh3::xxh3_64;

const TABLE: &str = include_str!(concat!(env!("OUT_DIR"), "/instructions.dat"));
const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Shipped,
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub kind: String,
    pub text: String,
    pub id: String,
    pub origin: Origin,
}

#[must_use]
fn split(table: &str) -> Vec<(&str, &str)> {
    table
        .split(RECORD)
        .filter_map(|row| row.split_once(FIELD))
        .collect()
}

#[must_use]
pub fn shipped() -> Vec<(&'static str, &'static str)> {
    split(TABLE)
}

#[must_use]
pub fn id(text: &str) -> String {
    format!("{:016x}", xxh3_64(text.as_bytes()))
}

fn instruction(kind: &str, text: &str, origin: Origin) -> Instruction {
    Instruction {
        kind: kind.to_owned(),
        id: id(text),
        text: text.to_owned(),
        origin,
    }
}

pub fn in_force(
    shipped: &[(&str, &str)],
    configured: &[(String, PathBuf)],
    read: &dyn Fn(&Path) -> io::Result<Vec<u8>>,
    kinds: &[&str],
) -> Result<Vec<Instruction>, String> {
    let mut found: Vec<Instruction> = shipped
        .iter()
        .map(|(kind, text)| instruction(kind, text, Origin::Shipped))
        .collect();
    for (kind, path) in configured {
        let key = format!("derive.instructions.{kind}");
        if !kinds.contains(&kind.as_str()) {
            let mut known = kinds.to_vec();
            known.sort_unstable();
            known.dedup();
            return Err(format!(
                "{key}: no reader reads the kind '{kind}'; the kinds are {}",
                known.join(", ")
            ));
        }
        let shown = path.display();
        let bytes = read(path).map_err(|error| format!("{key}: cannot read {shown}: {error}"))?;
        let text = String::from_utf8(bytes).map_err(|_| format!("{key}: {shown} is not UTF-8"))?;
        if text.trim().is_empty() {
            return Err(format!("{key}: {shown} is empty"));
        }
        found.retain(|held| held.kind != *kind);
        found.push(instruction(kind, &text, Origin::File(path.clone())));
    }
    found.sort_by(|a, b| a.kind.cmp(&b.kind));
    Ok(found)
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::{Path, PathBuf};

    use super::{Origin, id, in_force, shipped, split};

    fn files(path: &Path) -> io::Result<Vec<u8>> {
        match path.to_str() {
            Some("/c/plain.txt") => Ok(b"Write plain notes.\n".to_vec()),
            Some("/c/empty.txt") => Ok(b" \n".to_vec()),
            Some("/c/binary.txt") => Ok(vec![0xff, 0xfe]),
            _ => Err(io::Error::new(io::ErrorKind::NotFound, "no such file")),
        }
    }

    fn configured(kind: &str, path: &str) -> Vec<(String, PathBuf)> {
        vec![(kind.to_owned(), PathBuf::from(path))]
    }

    #[test]
    fn the_shipped_instructions_are_one_a_file_named_for_its_kind() {
        let table = shipped();
        assert_eq!(table.len(), 1);
        assert_eq!(table[0].0, "session");
        assert_eq!(
            table[0].1.lines().next(),
            Some("You are writing notes about a working session, so that a later session can")
        );
    }

    #[test]
    fn the_shipped_session_instruction_is_the_one_that_was_reviewed() {
        assert_eq!(id(shipped()[0].1), "9ea9aba2ae5ed554");
    }

    #[test]
    fn the_table_splits_into_kinds_and_texts_whatever_a_text_holds() {
        let table = "one\u{1f}first line\nsecond\tline\n\u{1e}two\u{1f}\ttabbed\u{1e}";
        assert_eq!(
            split(table),
            [("one", "first line\nsecond\tline\n"), ("two", "\ttabbed")]
        );
    }

    #[test]
    fn an_instructions_id_is_16_hex_digits_of_the_xxh3_of_its_text() {
        assert_eq!(id(""), "2d06800538d394c2");
        assert_ne!(id("a"), id("b"));
        assert_eq!(id("a").len(), 16);
    }

    #[test]
    fn a_configured_file_replaces_the_shipped_instruction_of_its_kind() {
        let found = in_force(
            &[("plain", "Shipped text.")],
            &configured("plain", "/c/plain.txt"),
            &files,
            &["plain"],
        )
        .expect("in force");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "Write plain notes.\n");
        assert_eq!(found[0].id, id("Write plain notes.\n"));
        assert_eq!(found[0].origin, Origin::File(PathBuf::from("/c/plain.txt")));
        let kept =
            in_force(&[("plain", "Shipped text.")], &[], &files, &["plain"]).expect("in force");
        assert_eq!(kept[0].origin, Origin::Shipped);
    }

    #[test]
    fn a_configured_file_adds_a_kind_and_the_list_is_sorted_by_kind() {
        let found = in_force(
            &[("zed", "Z."), ("alpha", "A.")],
            &configured("mid", "/c/plain.txt"),
            &files,
            &["alpha", "mid", "zed"],
        )
        .expect("in force");
        let kinds: Vec<&str> = found.iter().map(|held| held.kind.as_str()).collect();
        assert_eq!(kinds, ["alpha", "mid", "zed"]);
    }

    #[test]
    fn a_file_that_is_missing_empty_or_not_utf8_is_refused_in_its_keys_name() {
        for (path, message) in [
            (
                "/c/gone.txt",
                "derive.instructions.plain: cannot read /c/gone.txt: no such file",
            ),
            (
                "/c/empty.txt",
                "derive.instructions.plain: /c/empty.txt is empty",
            ),
            (
                "/c/binary.txt",
                "derive.instructions.plain: /c/binary.txt is not UTF-8",
            ),
        ] {
            assert_eq!(
                in_force(&[], &configured("plain", path), &files, &["plain"]),
                Err(message.to_owned())
            );
        }
    }

    #[test]
    fn a_kind_no_reader_reads_is_refused_with_the_kinds_that_are() {
        assert_eq!(
            in_force(
                &[],
                &configured("other", "/c/plain.txt"),
                &files,
                &["plain", "folded"]
            ),
            Err(
                "derive.instructions.other: no reader reads the kind 'other'; the kinds are folded, plain"
                    .to_owned()
            )
        );
    }
}
