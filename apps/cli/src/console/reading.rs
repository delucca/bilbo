use std::path::Path;

use bilbo_kinds::reader::Reader;
use bilbo_search::analyze::Detect;
use bilbo_search::chunk::Size;
use bilbo_search::lookup::Lookup;
use bilbo_search::tokenizer::Tokenizer;

use crate::console::indexing::Indexing;
use crate::console::outcome::Failure;

pub struct Reading<'a> {
    pub readers: &'a [Box<dyn Reader>],
    pub chunk: Size,
    pub render: &'a str,
    pub tokenizer: &'a dyn Fn() -> Result<Tokenizer, Failure>,
    pub lookup: &'a dyn Fn(&Path) -> Box<dyn Lookup>,
    pub detect: &'a dyn Detect,
}

pub fn of<'a>(
    indexing: &'a Indexing<'a>,
    lookup: &'a dyn Fn(&Path) -> Box<dyn Lookup>,
    detect: &'a dyn Detect,
) -> Reading<'a> {
    Reading {
        readers: indexing.readers,
        chunk: indexing.request.chunk,
        render: &indexing.request.render,
        tokenizer: indexing.tokenizer,
        lookup,
        detect,
    }
}
