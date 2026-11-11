use bilbo_search::index::{Index, IndexError};
use bilbo_search::position::Manifest;
use bilbo_search::update::{self, Plan, Reason};
use bilbo_search::work::Error;
use bilbo_store::segments::Store;

use super::messages::{info, warning};
use crate::console::indexing::Indexing;
use crate::console::message::{Message, sanitize};

#[derive(Default)]
pub struct Updated {
    pub messages: Vec<Message>,
    /// The index is not at the store's cursor, or an entry was left out: the run exits 1.
    pub refused: bool,
    /// The update stopped for an error, so the index is as it was: no later update is tried.
    pub fatal: bool,
}

/// Brings the index to the store's cursor, inside the lock the store holds. It leaves no writer
/// open on any path. `kept` is what this invocation has kept so far; a build of no more entries
/// than that says nothing, since nothing was lost to it.
pub fn update(
    store: &Store,
    index: &mut dyn Index,
    retry: bool,
    kept: u64,
    indexing: &Indexing<'_>,
    notice: &mut dyn FnMut(&Message),
) -> Updated {
    let manifest = Manifest::new(
        indexing.request.chunk,
        indexing.tokenizer_id,
        &indexing.request.render,
    );
    let work = match update::plan(&manifest, retry, store, index) {
        Ok(Plan::Current) => return Updated::default(),
        Ok(Plan::Work(work)) => work,
        Err(error) => return failed(&describe(&error)),
    };
    let mut updated = Updated::default();
    match work.rebuilds() {
        Some(Reason::Unmade) if work.entries() as u64 > kept => notice(&info(&format!(
            "building the search index: {} entries",
            work.entries()
        ))),
        Some(Reason::Manifest) => notice(&info(
            "rebuilding the search index: its settings or this bilbo's rules changed",
        )),
        Some(Reason::Catalog) => notice(&info(
            "rebuilding the search index: the store's catalog is not the one it was made from",
        )),
        _ => {}
    }
    let tokenizer = match (indexing.tokenizer)() {
        Ok(tokenizer) => tokenizer,
        Err(failure) => {
            let mut said = failed(&failure.message().text);
            updated.messages.append(&mut said.messages);
            updated.refused = true;
            updated.fatal = true;
            return updated;
        }
    };
    match update::apply(
        work,
        &indexing.request,
        store,
        indexing.readers,
        &tokenizer,
        index,
    ) {
        Ok(output) => {
            updated.refused = !output.failed.is_empty();
            updated.messages.extend(
                output
                    .failed
                    .iter()
                    .map(|(id, why)| warning(&sanitize(&format!("left out {id}: {why}")))),
            );
        }
        Err(error) => {
            let mut said = failed(&describe(&error));
            updated.messages.append(&mut said.messages);
            updated.refused = true;
            updated.fatal = true;
        }
    }
    updated
}

/// Only the index's own failure is something `bilbo reindex` repairs: a folder that is not
/// bilbo's is left alone by everything.
fn describe(error: &Error) -> String {
    match error {
        Error::Index(IndexError::Failed { .. }) => {
            format!("{error}; bilbo reindex makes the index again")
        }
        _ => error.to_string(),
    }
}

fn failed(error: &str) -> Updated {
    Updated {
        messages: vec![Message::error(sanitize(&format!(
            "the search index was not updated: {error}"
        )))],
        refused: true,
        fatal: true,
    }
}
