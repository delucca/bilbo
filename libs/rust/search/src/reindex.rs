use std::collections::BTreeSet;

use bilbo_kinds::reader::Reader;
use bilbo_store::entries::{Change, Cursor, Entries};
use bilbo_store::id::Id;

use crate::fill::Fill;
use crate::index::Index;
use crate::position::{MAX_FAILED, Manifest, Position};
use crate::tokenizer::Tokenizer;
use crate::work::{Error, Output, Request};

/// Empties the index and fills it with the newest generation of every entry, in entry id order
/// and chunk order whatever `workers` is, then commits once with the store's cursor. An entry the
/// store cannot read back is reported and left out; nothing else stops at one entry.
pub fn run(
    request: &Request,
    entries: &dyn Entries,
    readers: &[Box<dyn Reader>],
    tokenizer: &Tokenizer,
    index: &mut dyn Index,
) -> Result<Output, Error> {
    let manifest = Manifest::new(request.chunk, tokenizer.id, &request.render);
    let output = rebuild(
        manifest, request, entries, readers, tokenizer, index, MAX_FAILED,
    );
    if output.is_err() {
        index.rollback();
    }
    output
}

/// `run` with the most failed entries a commit may record.
pub(crate) fn rebuild(
    manifest: Manifest,
    request: &Request,
    entries: &dyn Entries,
    readers: &[Box<dyn Reader>],
    tokenizer: &Tokenizer,
    index: &mut dyn Index,
    max_failed: usize,
) -> Result<Output, Error> {
    let (changes, cursor) = entries.changes(&Cursor::start())?;
    let mut position = Position {
        manifest,
        cursor,
        failed: Vec::new(),
    };
    if position.encode().is_none() {
        return Err(Error::Manifest);
    }
    let ids: BTreeSet<Id> = changes.iter().map(Change::id).collect();
    index.clear()?;
    let fill = Fill {
        size: request.chunk,
        workers: request.workers,
        entries,
        readers,
        tokens: tokenizer.tokens.as_ref(),
        grown: None,
    };
    let mut output = fill.run(&ids, index)?;
    if output.failed.len() > max_failed {
        return Err(Error::Failed(output.failed.len()));
    }
    position.failed = output.failed.iter().map(|(id, _)| (*id, 1)).collect();
    output.index_bytes = index.commit(&position)?.bytes;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use bilbo_kinds::reader::Reader;

    use super::rebuild;
    use crate::chunk::Size;
    use crate::position::Manifest;
    use crate::testing::{Bytes, Commits, Four, OnePassage, id};
    use crate::tokenizer::Tokenizer;
    use crate::work::{Error, Output, Request};

    fn within(max_failed: usize, index: &mut Commits) -> Result<Output, Error> {
        let request = Request {
            chunk: Size {
                tokens: 12,
                overlap: 2,
            },
            render: "lines/1".to_owned(),
            workers: 1,
        };
        let readers: Vec<Box<dyn Reader>> = vec![Box::new(OnePassage)];
        let tokenizer = Tokenizer {
            tokens: Box::new(Bytes),
            id: "bytes/1",
        };
        let manifest = Manifest::new(request.chunk, tokenizer.id, &request.render);
        rebuild(
            manifest, &request, &Four, &readers, &tokenizer, index, max_failed,
        )
    }

    #[test]
    fn more_failed_entries_than_a_position_holds_stops_the_run_and_commits_nothing() {
        let mut index = Commits::default();
        let error = within(2, &mut index).expect_err("a failure");
        assert!(matches!(error, Error::Failed(3)), "{error:?}");
        assert_eq!(
            error.to_string(),
            "3 entries failed: more than a commit can record"
        );
        assert!(index.commits.is_empty());
        let output = within(3, &mut index).expect("a run at the bound");
        assert_eq!(output.failed.len(), 3);
        assert_eq!(
            index.commits[0].failed,
            [(id(1), 1), (id(3), 1), (id(4), 1)]
        );
    }
}
