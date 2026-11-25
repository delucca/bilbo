use std::io;

use bilbo_search::recall::log::QueryLog;
use bilbo_store::segments::State;

use crate::console::private::{self, How};

const NAME: &str = "queries.log";

pub struct File<'a> {
    state: Option<&'a State>,
}

impl<'a> File<'a> {
    pub fn at(state: Option<&'a State>) -> File<'a> {
        File { state }
    }
}

impl QueryLog for File<'_> {
    fn append(&mut self, line: &str) -> io::Result<()> {
        let state = self
            .state
            .ok_or_else(|| io::Error::other("this store has no state folder that bilbo checked"))?;
        let path = state.path().join(NAME);
        let mut file = private::open(state.folder(), NAME, &path, How::Append)?;
        file.lock()?;
        private::checked(&file, &path)?;
        private::append(&mut file, line, "the log")
    }
}
