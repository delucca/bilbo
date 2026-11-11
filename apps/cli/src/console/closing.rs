use bilbo_search::index::Index;
use bilbo_store::segments::Store;

use crate::console::message::Message;

/// What a verb that held the store runs to let it go; the last argument hears what it says as it goes.
pub type Closer<'a> = dyn Fn(Store, &mut dyn Index, &mut dyn FnMut(&Message)) -> Closing + 'a;

/// What a verb that held the store says when it lets it go.
#[derive(Default)]
pub struct Closing {
    pub messages: Vec<Message>,
    /// The store failed: the run exits 1, and `--follow` does not save.
    pub failed: bool,
    /// A rerun refused a file or could not run a follow, or the index was not brought up to
    /// date: the run exits 1, and `--follow` saves.
    pub refused: bool,
}
