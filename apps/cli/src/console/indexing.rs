use std::path::Path;

use bilbo_kinds::reader::Reader;
use bilbo_search::index::Index;
use bilbo_search::tokenizer::Tokenizer;
use bilbo_search::work::Request;

use crate::console::outcome::Failure;

/// What a run needs to bring the index to the store.
pub struct Indexing<'a> {
    pub readers: &'a [Box<dyn Reader>],
    pub request: Request,
    pub tokenizer_id: &'static str,
    /// Called only when there is work: loading the tokenizer costs a tenth of a second.
    pub tokenizer: &'a dyn Fn() -> Result<Tokenizer, Failure>,
    pub index: &'a dyn Fn(&Path) -> Box<dyn Index>,
}
