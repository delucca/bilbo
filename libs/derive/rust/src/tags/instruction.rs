use std::io;
use std::path::{Path, PathBuf};

use crate::instruction::Origin;

const TEXT: &str = include_str!(concat!(env!("OUT_DIR"), "/tags.dat"));

#[must_use]
pub fn text() -> &'static str {
    TEXT
}

#[must_use]
pub fn id() -> String {
    crate::instruction::id(TEXT)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub text: String,
    pub id: String,
    pub origin: Origin,
}

pub fn in_force(
    configured: Option<&Path>,
    read: &dyn Fn(&Path) -> io::Result<Vec<u8>>,
) -> Result<Instruction, String> {
    let Some(path) = configured else {
        return Ok(Instruction {
            text: text().to_owned(),
            id: id(),
            origin: Origin::Shipped,
        });
    };
    let key = super::keys::INSTRUCTION.about.name;
    let shown = path.display();
    let bytes = read(path).map_err(|error| format!("{key}: cannot read {shown}: {error}"))?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{key}: {shown} is not UTF-8"))?;
    if text.trim().is_empty() {
        return Err(format!("{key}: {shown} is empty"));
    }
    Ok(Instruction {
        id: crate::instruction::id(&text),
        text,
        origin: Origin::File(PathBuf::from(path)),
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io;
    use std::path::{Path, PathBuf};

    use super::{Instruction, id, in_force, text};
    use crate::instruction::Origin;

    #[test]
    fn the_shipped_tags_instruction_is_the_one_that_was_reviewed() {
        assert_eq!(id(), "a5d31572b3105b1b");
        assert!(text().ends_with("line (none) and nothing else.\n"));
    }

    #[test]
    fn the_tags_instruction_is_no_instruction_of_a_kind() {
        let kinds: Vec<&str> = crate::instruction::shipped()
            .iter()
            .map(|(kind, _)| *kind)
            .collect();
        assert_eq!(kinds, ["session"]);
    }

    #[test]
    fn a_file_of_the_settings_replaces_the_shipped_tags_instruction() {
        let calls = Cell::new(0);
        let read = |path: &Path| {
            calls.set(calls.get() + 1);
            assert_eq!(path, Path::new("/c/tags.txt"));
            Ok(b"Pick the tags.\n".to_vec())
        };
        assert_eq!(
            in_force(None, &read),
            Ok(Instruction {
                text: text().to_owned(),
                id: id(),
                origin: Origin::Shipped,
            })
        );
        assert_eq!(calls.get(), 0);
        assert_eq!(
            in_force(Some(Path::new("/c/tags.txt")), &read),
            Ok(Instruction {
                text: "Pick the tags.\n".to_owned(),
                id: crate::instruction::id("Pick the tags.\n"),
                origin: Origin::File(PathBuf::from("/c/tags.txt")),
            })
        );
        assert_eq!(calls.get(), 1);
        assert_ne!(crate::instruction::id("Pick the tags.\n"), id());
    }

    #[test]
    fn a_tags_instruction_file_that_is_missing_empty_or_not_utf8_is_refused_in_its_keys_name() {
        let read = |path: &Path| match path.to_str() {
            Some("/c/empty.txt") => Ok(b" \n\t".to_vec()),
            Some("/c/binary.txt") => Ok(vec![0xff, 0xfe]),
            _ => Err(io::Error::new(io::ErrorKind::NotFound, "no such file")),
        };
        for (path, message) in [
            (
                "/c/gone.txt",
                "derive.tags.instruction: cannot read /c/gone.txt: no such file",
            ),
            (
                "/c/empty.txt",
                "derive.tags.instruction: /c/empty.txt is empty",
            ),
            (
                "/c/binary.txt",
                "derive.tags.instruction: /c/binary.txt is not UTF-8",
            ),
        ] {
            assert_eq!(
                in_force(Some(Path::new(path)), &read),
                Err(message.to_owned())
            );
        }
    }
}
