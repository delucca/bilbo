use std::io;

use bilbo_store::entries::Generation;
use bilbo_store::id::Id;

const MAGIC: &str = "s1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub parent: Id,
    pub generation: Generation,
    pub generation_before: Option<Generation>,
    pub records_before: u64,
    pub opens: bool,
    pub records: Vec<Vec<u8>>,
}

pub trait Spool {
    fn keep(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn list(&self) -> io::Result<Vec<(String, Vec<u8>)>>;
    fn remove(&mut self, name: &str) -> io::Result<()>;
}

#[must_use]
pub fn bare(record: &[u8]) -> &[u8] {
    record.strip_suffix(b"\n").unwrap_or(record)
}

#[must_use]
pub fn write(block: &Block) -> Vec<u8> {
    let before = block
        .generation_before
        .map_or_else(|| "-".to_owned(), |generation| generation.0.to_string());
    let mut bytes = format!(
        "{MAGIC}\t{}\t{}\t{before}\t{}\t{}\t{}\n",
        block.parent,
        block.generation.0,
        block.records_before,
        u8::from(block.opens),
        block.records.len()
    )
    .into_bytes();
    for record in &block.records {
        bytes.extend_from_slice(record);
        bytes.push(b'\n');
    }
    bytes
}

pub fn read(bytes: &[u8]) -> Result<Block, String> {
    let body = bytes
        .strip_suffix(b"\n")
        .ok_or("it does not end in a line feed")?;
    let mut lines = body.split(|&byte| byte == b'\n');
    let header = lines.next().ok_or("it has no header")?;
    let header = std::str::from_utf8(header).map_err(|_| "its header is not UTF-8")?;
    let fields: Vec<&str> = header.split('\t').collect();
    let [
        magic,
        parent,
        generation,
        before,
        records_before,
        opens,
        count,
    ] = fields.as_slice()
    else {
        return Err("its header does not have seven fields".to_owned());
    };
    if *magic != MAGIC {
        return Err(format!("its first field is {magic}, not {MAGIC}"));
    }
    let records: Vec<Vec<u8>> = lines.map(<[u8]>::to_vec).collect();
    let count: usize = count.parse().map_err(|_| "its record count is no number")?;
    if records.len() != count {
        return Err(format!(
            "its header counts {count} records and it holds {}",
            records.len()
        ));
    }
    Ok(Block {
        parent: Id::parse(parent).ok_or("its parent is no id")?,
        generation: Generation(
            generation
                .parse()
                .map_err(|_| "its generation is no number")?,
        ),
        generation_before: match *before {
            "-" => None,
            number => Some(Generation(
                number
                    .parse()
                    .map_err(|_| "its generation before is no number")?,
            )),
        },
        records_before: records_before
            .parse()
            .map_err(|_| "its records before are no number")?,
        opens: match *opens {
            "0" => false,
            "1" => true,
            _ => return Err("its opens flag is neither 0 nor 1".to_owned()),
        },
        records,
    })
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;

    use super::{Block, read, write};

    const PARENT: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DB";

    fn block(before: Option<u32>, records: &[&str]) -> Block {
        Block {
            parent: Id::parse(PARENT).expect("id"),
            generation: Generation(2),
            generation_before: before.map(Generation),
            records_before: 5,
            opens: before.is_none(),
            records: records
                .iter()
                .map(|record| record.as_bytes().to_vec())
                .collect(),
        }
    }

    #[test]
    fn a_block_is_a_header_line_and_its_records() {
        let bytes = write(&block(Some(1), &[r#"{"a":1}"#, r#"{"b":2}"#]));
        assert_eq!(
            bytes,
            format!("s1\t{PARENT}\t2\t1\t5\t0\t2\n{{\"a\":1}}\n{{\"b\":2}}\n").into_bytes()
        );
        let opening = write(&block(None, &["x"]));
        assert_eq!(
            opening,
            format!("s1\t{PARENT}\t2\t-\t5\t1\t1\nx\n").into_bytes()
        );
    }

    #[test]
    fn a_written_block_reads_back_the_same() {
        for before in [None, Some(3)] {
            let held = block(before, &[r#"{"a":1}"#, r#"{"run":{}}"#]);
            assert_eq!(read(&write(&held)), Ok(held));
        }
    }

    #[test]
    fn a_block_cut_short_or_with_another_header_is_an_error() {
        let whole = String::from_utf8(write(&block(Some(1), &["a", "b"]))).expect("text");
        assert!(read(whole.as_bytes()).is_ok());
        assert!(read(whole.trim_end().as_bytes()).is_err());
        let fewer = whole.replacen("a\n", "", 1);
        assert!(read(fewer.as_bytes()).is_err());
        let more = format!("{whole}c\n");
        assert!(read(more.as_bytes()).is_err());
        assert!(read(whole.replacen("s1", "s2", 1).as_bytes()).is_err());
        assert!(read(b"").is_err());
    }
}
