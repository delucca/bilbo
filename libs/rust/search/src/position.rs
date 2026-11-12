use bilbo_store::entries::Cursor;
use bilbo_store::id::Id;

use crate::analyze;
use crate::chunk::Size;

const FORMAT: &str = "bilbo index 1";

/// The most entries a position lists as left out: about 120 KB of payload.
pub const MAX_FAILED: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub chunk: Size,
    pub analyzer: String,
    pub tokenizer: String,
    /// How the readers render passages: their versions and the settings they take.
    pub render: String,
}

impl Manifest {
    pub fn new(chunk: Size, tokenizer: &str, render: &str) -> Manifest {
        Manifest {
            chunk,
            analyzer: analyze::ID.to_owned(),
            tokenizer: tokenizer.to_owned(),
            render: render.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub manifest: Manifest,
    pub cursor: Cursor,
    /// By id, each count 1 to 9.
    pub failed: Vec<(Id, u8)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Its documents hold every change up to this sequence number of this catalog.
    At(u64),
    /// Never committed, or its payload is not a position.
    Unmade,
    OtherManifest,
    OtherCatalog,
}

fn hex(bytes: [u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<[u8; 16]> {
    let lower = |byte: u8| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte);
    if text.len() != 32 || !text.bytes().all(lower) {
        return None;
    }
    let mut bytes = [0; 16];
    for (at, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[at * 2..at * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

fn listable(failed: &[(Id, u8)]) -> bool {
    failed.len() <= MAX_FAILED
        && failed.iter().all(|(_, count)| (1..=9).contains(count))
        && failed.windows(2).all(|pair| pair[0].0 < pair[1].0)
}

fn failed_line(failed: &[(Id, u8)]) -> String {
    if failed.is_empty() {
        return "-".to_owned();
    }
    let items: Vec<String> = failed
        .iter()
        .map(|(id, count)| format!("{id}.{count}"))
        .collect();
    items.join(",")
}

fn failed_items(text: &str) -> Option<Vec<(Id, u8)>> {
    if text == "-" {
        return Some(Vec::new());
    }
    let mut failed: Vec<(Id, u8)> = Vec::new();
    for item in text.split(',') {
        let (id, count) = item.split_once('.')?;
        let digit = match count.as_bytes() {
            [digit @ b'1'..=b'9'] => digit - b'0',
            _ => return None,
        };
        failed.push((Id::parse(id)?, digit));
    }
    listable(&failed).then_some(failed)
}

fn plain(value: &str) -> bool {
    !value.is_empty() && !value.contains(['\n', '\r'])
}

impl Position {
    /// One `name value` line each, in this order; a name or a line more or less is no position.
    pub fn encode(&self) -> Option<String> {
        if !plain(&self.manifest.analyzer)
            || !plain(&self.manifest.tokenizer)
            || !plain(&self.manifest.render)
            || !listable(&self.failed)
        {
            return None;
        }
        Some(format!(
            "{FORMAT}\nchunk_tokens {}\nchunk_overlap {}\nanalyzer {}\ntokenizer {}\nrender {}\ncatalog {}\nseq {}\nfailed {}\n",
            self.manifest.chunk.tokens,
            self.manifest.chunk.overlap,
            self.manifest.analyzer,
            self.manifest.tokenizer,
            self.manifest.render,
            hex(self.cursor.catalog()),
            self.cursor.seq(),
            failed_line(&self.failed),
        ))
    }

    pub fn decode(payload: &str) -> Option<Position> {
        let mut lines = payload.strip_suffix('\n')?.split('\n');
        if lines.next()? != FORMAT {
            return None;
        }
        let mut value = |name: &str| lines.next()?.strip_prefix(name)?.strip_prefix(' ');
        let whole = |text: &str| {
            let canonical = text == "0" || !text.starts_with('0');
            (canonical && text.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| text.parse::<u64>().ok())
                .flatten()
        };
        let tokens = usize::try_from(whole(value("chunk_tokens")?)?).ok()?;
        let overlap = usize::try_from(whole(value("chunk_overlap")?)?).ok()?;
        let analyzer = value("analyzer")?.to_owned();
        let tokenizer = value("tokenizer")?.to_owned();
        let render = value("render")?.to_owned();
        let catalog = unhex(value("catalog")?)?;
        let seq = whole(value("seq")?)?;
        let failed = failed_items(value("failed")?)?;
        if lines.next().is_some() || !plain(&analyzer) || !plain(&tokenizer) || !plain(&render) {
            return None;
        }
        Some(Position {
            manifest: Manifest {
                chunk: Size { tokens, overlap },
                analyzer,
                tokenizer,
                render,
            },
            cursor: Cursor::at(catalog, seq),
            failed,
        })
    }
}

/// `now` is the cursor the store gives for its catalog now: its catalog's id and newest sequence
/// number. A found cursor ahead of that number is of another history.
pub fn standing(found: Option<&Position>, manifest: &Manifest, now: &Cursor) -> Standing {
    match found {
        None => Standing::Unmade,
        Some(position) if position.manifest != *manifest => Standing::OtherManifest,
        Some(position)
            if position.cursor.catalog() != now.catalog() || position.cursor.seq() > now.seq() =>
        {
            Standing::OtherCatalog
        }
        Some(position) => Standing::At(position.cursor.seq()),
    }
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::Cursor;

    use bilbo_store::id::Id;

    use super::{MAX_FAILED, Manifest, Position, Standing, standing};
    use crate::chunk::Size;

    fn manifest() -> Manifest {
        Manifest {
            chunk: Size {
                tokens: 512,
                overlap: 64,
            },
            analyzer: "bm25s-en/1".to_owned(),
            tokenizer: "qwen3 97b0c614 def76fb0".to_owned(),
            render: "markdown/1/heading_width=100 session/1".to_owned(),
        }
    }

    fn position() -> Position {
        Position {
            manifest: manifest(),
            cursor: Cursor::at([0xAB; 16], 42),
            failed: Vec::new(),
        }
    }

    const FIRST: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DB";
    const SECOND: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DC";
    const THIRD: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DD";

    fn failing(items: &[(&str, u8)]) -> Vec<(Id, u8)> {
        items
            .iter()
            .map(|(id, count)| (Id::parse(id).expect("an id"), *count))
            .collect()
    }

    const ENCODED: &str = "bilbo index 1\nchunk_tokens 512\nchunk_overlap 64\nanalyzer bm25s-en/1\n\
        tokenizer qwen3 97b0c614 def76fb0\nrender markdown/1/heading_width=100 session/1\ncatalog abababababababababababababababab\nseq 42\nfailed -\n";

    #[test]
    fn a_position_is_nine_lines_and_reads_back_the_same() {
        assert_eq!(position().encode().as_deref(), Some(ENCODED));
        assert_eq!(Position::decode(ENCODED), Some(position()));
        let far = Position {
            cursor: Cursor::at([0; 16], u64::MAX),
            ..position()
        };
        let encoded = far.encode().expect("a position");
        assert_eq!(Position::decode(&encoded), Some(far));
    }

    #[test]
    fn a_position_lists_the_entries_it_left_out_and_reads_them_back() {
        for items in [
            vec![],
            vec![(FIRST, 1)],
            vec![(FIRST, 1), (SECOND, 9), (THIRD, 2)],
        ] {
            let listed = Position {
                failed: failing(&items),
                ..position()
            };
            let encoded = listed.encode().expect("a position");
            assert_eq!(Position::decode(&encoded), Some(listed));
        }
        let three = Position {
            failed: failing(&[(FIRST, 1), (SECOND, 9), (THIRD, 2)]),
            ..position()
        };
        assert!(
            three
                .encode()
                .expect("a position")
                .ends_with(&format!("seq 42\nfailed {FIRST}.1,{SECOND}.9,{THIRD}.2\n"))
        );
    }

    #[test]
    fn a_list_a_position_cannot_hold_is_not_encoded() {
        let unordered = Position {
            failed: failing(&[(SECOND, 1), (FIRST, 1)]),
            ..position()
        };
        let twice = Position {
            failed: failing(&[(FIRST, 1), (FIRST, 2)]),
            ..position()
        };
        let none = Position {
            failed: failing(&[(FIRST, 0)]),
            ..position()
        };
        let ten = Position {
            failed: failing(&[(FIRST, 10)]),
            ..position()
        };
        for bad in [unordered, twice, none, ten] {
            assert_eq!(bad.encode(), None);
        }
        let ids: Vec<Id> = (0..=MAX_FAILED)
            .map(|at| {
                Id::from_bytes([
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    (at >> 8) as u8,
                    at as u8,
                ])
            })
            .collect();
        let held = |count: usize| Position {
            failed: ids[..count].iter().map(|id| (*id, 1)).collect(),
            ..position()
        };
        let full = held(MAX_FAILED).encode().expect("a full list");
        assert!(Position::decode(&full).is_some());
        assert_eq!(held(MAX_FAILED + 1).encode(), None);
    }

    #[test]
    fn a_name_with_a_line_break_or_no_text_is_not_encoded() {
        let changes: [fn(&mut Manifest, String); 3] = [
            |manifest, name| manifest.analyzer = name,
            |manifest, name| manifest.tokenizer = name,
            |manifest, name| manifest.render = name,
        ];
        for name in ["", "a\nseq 9", "a\rb"] {
            for change in changes {
                let mut position = position();
                change(&mut position.manifest, name.to_owned());
                assert_eq!(position.encode(), None, "{name:?}");
            }
        }
    }

    #[test]
    fn a_payload_that_is_not_exactly_a_position_is_none() {
        let cases = [
            String::new(),
            "anything".to_owned(),
            ENCODED.trim_end().to_owned(),
            format!("{ENCODED}\n"),
            format!("{ENCODED}extra 1\n"),
            ENCODED.replace("bilbo index 1", "bilbo index 2"),
            ENCODED.replace("chunk_tokens 512", "chunk_tokens 0512"),
            ENCODED.replace("chunk_tokens 512", "chunk_tokens +512"),
            ENCODED.replace("chunk_tokens 512", "chunk_tokens"),
            ENCODED.replace("chunk_overlap 64\n", ""),
            ENCODED.replace("analyzer bm25s-en/1", "analyzer "),
            ENCODED.replace("render markdown/1/heading_width=100 session/1\n", ""),
            ENCODED.replace("render markdown/1/heading_width=100 session/1", "render "),
            ENCODED.replace("analyzer", "analyser"),
            ENCODED.replace("abababab", "ABABABAB"),
            ENCODED.replace("catalog abab", "catalog ab"),
            ENCODED.replace("catalog ab", "catalog zz"),
            ENCODED.replace("seq 42", "seq 99999999999999999999"),
            ENCODED.replace("seq 42", "seq -1"),
            ENCODED.replace("seq 42\n", "seq 42\r\n"),
            ENCODED.replace("failed -\n", ""),
            ENCODED.replace("failed -", "failed "),
            ENCODED.replace("failed -", "failed"),
            ENCODED.replace("failed -", &format!("failed {SECOND}.1,{FIRST}.1")),
            ENCODED.replace("failed -", &format!("failed {FIRST}.1,{FIRST}.2")),
            ENCODED.replace("failed -", &format!("failed {FIRST}.0")),
            ENCODED.replace("failed -", &format!("failed {FIRST}.10")),
            ENCODED.replace("failed -", &format!("failed {FIRST}")),
            ENCODED.replace("failed -", &format!("failed {FIRST}.1,")),
            ENCODED.replace("failed -", "failed 01JC5Q3V7XKM4P8R2T6W9YB0D.1"),
            ENCODED.replace("failed -", "failed 01JC5Q3V7XKM4P8R2T6W9YB0DU.1"),
            ENCODED.replace("failed -", "failed 91JC5Q3V7XKM4P8R2T6W9YB0DB.1"),
            ENCODED.replace("failed -", &format!("failed {FIRST}.1\nfailed -")),
        ];
        for payload in cases {
            assert_eq!(Position::decode(&payload), None, "{payload:?}");
        }
        assert!(Position::decode(&ENCODED.replace("abababab", "0189cdef")).is_some());
    }

    #[test]
    fn an_index_stands_by_its_manifest_first_and_then_its_catalog() {
        let found = position();
        let catalog = Cursor::at([0xAB; 16], 42);
        let other = Cursor::at([0xAC; 16], 42);
        assert_eq!(standing(None, &manifest(), &catalog), Standing::Unmade);
        assert_eq!(
            standing(Some(&found), &manifest(), &catalog),
            Standing::At(42)
        );
        assert_eq!(
            standing(Some(&found), &manifest(), &other),
            Standing::OtherCatalog
        );
        let changes: [fn(&mut Manifest); 5] = [
            |manifest| manifest.chunk.tokens = 256,
            |manifest| manifest.chunk.overlap = 32,
            |manifest| manifest.analyzer.push('2'),
            |manifest| manifest.tokenizer.push('2'),
            |manifest| manifest.render.push('2'),
        ];
        for change in changes {
            let mut other = manifest();
            change(&mut other);
            assert_eq!(
                standing(Some(&found), &other, &catalog),
                Standing::OtherManifest
            );
            assert_eq!(
                standing(Some(&found), &other, &Cursor::at([0xAC; 16], 42)),
                Standing::OtherManifest
            );
        }
    }

    #[test]
    fn a_cursor_ahead_of_the_stores_newest_change_is_another_catalogs() {
        let found = position();
        let behind = Cursor::at([0xAB; 16], 43);
        let level = Cursor::at([0xAB; 16], 42);
        let ahead = Cursor::at([0xAB; 16], 41);
        assert_eq!(
            standing(Some(&found), &manifest(), &behind),
            Standing::At(42)
        );
        assert_eq!(
            standing(Some(&found), &manifest(), &level),
            Standing::At(42)
        );
        assert_eq!(
            standing(Some(&found), &manifest(), &ahead),
            Standing::OtherCatalog
        );
    }
}
