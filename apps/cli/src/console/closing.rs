use bilbo_search::index::Index;
use bilbo_store::segments::Store;

use crate::console::message::Message;

/// The last argument hears each message as it is said.
pub type Closer<'a> = dyn Fn(Store, &mut dyn Index, &mut dyn FnMut(&Message)) -> Closing + 'a;

#[derive(Default)]
pub struct Closing {
    pub messages: Vec<Message>,
    /// The store failed: the run exits 1, and `--follow` does not save.
    pub failed: bool,
    /// A rerun refused a file or could not run a follow, or the index was not brought up to
    /// date: the run exits 1, and `--follow` saves.
    pub refused: bool,
}
