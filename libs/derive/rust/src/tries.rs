use std::io;

use bilbo_store::entries::Generation;
use bilbo_store::id::Id;

pub const MOST: u8 = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub parent: Id,
    pub generation: Generation,
    pub first: u64,
    pub last: u64,
}

pub trait Tries {
    fn charges(&self, extent: &Extent) -> io::Result<u8>;
    fn charge(&mut self, extent: &Extent) -> io::Result<()>;
    fn forget(&mut self, extent: &Extent) -> io::Result<()>;

    fn mark(&self) -> io::Result<Option<Id>> {
        Ok(None)
    }

    fn set_mark(&mut self, _entry: Option<Id>) -> io::Result<()> {
        Ok(())
    }
}

const AFTER: &str = "after";

#[must_use]
pub fn mark_line(entry: Id) -> String {
    format!("{AFTER}\t{entry}")
}

#[must_use]
pub fn parse_mark(line: &str) -> Option<Id> {
    let (word, entry) = line.split_once('\t')?;
    (word == AFTER).then(|| Id::parse(entry)).flatten()
}

#[must_use]
pub fn line(extent: &Extent, charges: u8) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{charges}",
        extent.parent, extent.generation.0, extent.first, extent.last
    )
}

#[must_use]
pub fn parse(line: &str) -> Option<(Extent, u8)> {
    let fields: Vec<&str> = line.split('\t').collect();
    let [parent, generation, first, last, charges] = fields.as_slice() else {
        return None;
    };
    let extent = Extent {
        parent: Id::parse(parent)?,
        generation: Generation(generation.parse().ok()?),
        first: first.parse().ok()?,
        last: last.parse().ok()?,
    };
    let charges: u8 = charges.parse().ok()?;
    ((1..=MOST).contains(&charges) && extent.first <= extent.last).then_some((extent, charges))
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;

    use super::{Extent, line, mark_line, parse, parse_mark};

    const PARENT: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DB";

    #[test]
    fn the_tries_file_is_one_line_a_window_and_reads_back() {
        let extent = Extent {
            parent: Id::parse(PARENT).expect("id"),
            generation: Generation(2),
            first: 11,
            last: 40,
        };
        assert_eq!(line(&extent, 3), format!("{PARENT}\t2\t11\t40\t3"));
        assert_eq!(parse(&line(&extent, 3)), Some((extent, 3)));
        assert_eq!(parse(&line(&extent, 9)), Some((extent, 9)));
    }

    #[test]
    fn a_tries_line_bilbo_cannot_read_is_ignored() {
        for text in [
            format!("{PARENT}\t2\t11\t40\t0"),
            format!("{PARENT}\t2\t11\t40\t10"),
            "NOTANID\t2\t11\t40\t3".to_owned(),
            format!("{PARENT}\t2\t40\t11\t3"),
            format!("{PARENT}\t2\t11\t40\t3\t1"),
            String::new(),
        ] {
            assert_eq!(parse(&text), None, "{text:?}");
        }
    }

    #[test]
    fn the_marks_line_reads_back_and_is_no_charge() {
        let entry = Id::parse(PARENT).expect("id");
        let line = mark_line(entry);
        assert_eq!(line, format!("after\t{PARENT}"));
        assert_eq!(parse_mark(&line), Some(entry));
        assert_eq!(parse(&line), None);
        assert_eq!(parse_mark("after\tNOTANID"), None);
        assert_eq!(parse_mark(&format!("{PARENT}\t2\t11\t40\t3")), None);
    }
}
