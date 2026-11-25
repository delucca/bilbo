use std::io;

use bilbo_store::segments::State;

use crate::digest::seen::{Ledger, again};

pub const NAME: &str = "digest.log";

pub fn append(state: &io::Result<State>, line: &str) -> io::Result<()> {
    let state = state.as_ref().map_err(again)?;
    let mut ledger = Ledger::open(state, NAME, "the log")?;
    ledger.lock()?;
    let written = write_locked(&mut ledger, line);
    let _ = ledger.unlock();
    written
}

fn write_locked(ledger: &mut Ledger, line: &str) -> io::Result<()> {
    ledger.look()?;
    ledger.cut_tail()?;
    ledger.append(line)
}
