use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;

use bilbo_search::digest::sessions::{Claimed, Sessions, holds, mark, whole};
use bilbo_store::segments::State;

use crate::console::private::{self, How};

pub const NAME: &str = "digest.sessions";

trait Marks {
    fn lock(&mut self) -> io::Result<()>;
    fn look(&mut self) -> io::Result<()>;
    fn read(&mut self) -> io::Result<Vec<u8>>;
    fn cut(&mut self, len: u64) -> io::Result<()>;
    fn append(&mut self, line: &str) -> io::Result<()>;
    fn unlock(&mut self) -> io::Result<()>;
}

pub(super) struct Ledger {
    file: fs::File,
    path: PathBuf,
    what: &'static str,
}

impl Ledger {
    pub(super) fn open(state: &State, name: &str, what: &'static str) -> io::Result<Ledger> {
        let path = state.path().join(name);
        let file = private::open(state.folder(), name, &path, How::Ledger)?;
        Ok(Ledger { file, path, what })
    }

    pub(super) fn lock(&mut self) -> io::Result<()> {
        self.file.lock()
    }

    pub(super) fn look(&mut self) -> io::Result<()> {
        private::checked(&self.file, &self.path)
    }

    fn read(&mut self) -> io::Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        self.file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn cut(&mut self, len: u64) -> io::Result<()> {
        self.file.set_len(len)
    }

    pub(super) fn cut_tail(&mut self) -> io::Result<()> {
        let len = self.file.metadata()?.len();
        if len == 0 {
            return Ok(());
        }
        self.file.seek(SeekFrom::Start(len - 1))?;
        let mut last = [0_u8; 1];
        self.file.read_exact(&mut last)?;
        if last[0] == b'\n' {
            return Ok(());
        }
        let bytes = self.read()?;
        self.cut(whole(&bytes) as u64)
    }

    pub(super) fn append(&mut self, line: &str) -> io::Result<()> {
        private::append(&mut self.file, line, self.what)
    }

    pub(super) fn unlock(&mut self) -> io::Result<()> {
        self.file.unlock()
    }
}

impl Marks for Ledger {
    fn lock(&mut self) -> io::Result<()> {
        Ledger::lock(self)
    }

    fn look(&mut self) -> io::Result<()> {
        Ledger::look(self)
    }

    fn read(&mut self) -> io::Result<Vec<u8>> {
        Ledger::read(self)
    }

    fn cut(&mut self, len: u64) -> io::Result<()> {
        Ledger::cut(self, len)
    }

    fn append(&mut self, line: &str) -> io::Result<()> {
        Ledger::append(self, line)
    }

    fn unlock(&mut self) -> io::Result<()> {
        Ledger::unlock(self)
    }
}

fn claim_over(marks: &mut dyn Marks, thread: &str, seconds: u64) -> io::Result<Claimed> {
    marks.lock()?;
    let claimed = claim_locked(marks, thread, seconds);
    let _ = marks.unlock();
    claimed
}

pub(super) fn again(error: &io::Error) -> io::Error {
    match error.raw_os_error() {
        Some(code) => io::Error::from_raw_os_error(code),
        None => io::Error::new(error.kind(), error.to_string()),
    }
}

fn claim_locked(marks: &mut dyn Marks, thread: &str, seconds: u64) -> io::Result<Claimed> {
    marks.look()?;
    let bytes = marks.read()?;
    if holds(&bytes, thread) {
        return Ok(Claimed::Seen);
    }
    let whole = whole(&bytes);
    if whole < bytes.len() {
        marks.cut(whole as u64)?;
    }
    marks.append(&mark(seconds, thread))?;
    Ok(Claimed::Marked)
}

pub struct Seen<'a> {
    state: &'a io::Result<State>,
    seconds: u64,
}

impl<'a> Seen<'a> {
    pub fn new(state: &'a io::Result<State>, seconds: u64) -> Seen<'a> {
        Seen { state, seconds }
    }
}

impl Sessions for Seen<'_> {
    fn claim(&mut self, thread: &str) -> io::Result<Claimed> {
        let state = self.state.as_ref().map_err(again)?;
        claim_over(
            &mut Ledger::open(state, NAME, "the file")?,
            thread,
            self.seconds,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use bilbo_search::digest::sessions::Claimed;

    use super::{Marks, Seen, claim_over};

    struct Recording {
        held: Vec<u8>,
        events: Vec<String>,
        look_fails: bool,
        append_fails: bool,
        unlock_fails: bool,
        lock_fails: bool,
        cut_fails: bool,
    }

    impl Recording {
        fn of(held: &[u8]) -> Recording {
            Recording {
                held: held.to_vec(),
                events: Vec::new(),
                look_fails: false,
                append_fails: false,
                unlock_fails: false,
                lock_fails: false,
                cut_fails: false,
            }
        }
    }

    impl Marks for Recording {
        fn lock(&mut self) -> io::Result<()> {
            self.events.push("lock".to_owned());
            if self.lock_fails {
                Err(io::Error::other("cannot lock"))
            } else {
                Ok(())
            }
        }

        fn look(&mut self) -> io::Result<()> {
            self.events.push("look".to_owned());
            if self.look_fails {
                Err(io::Error::other("not private"))
            } else {
                Ok(())
            }
        }

        fn read(&mut self) -> io::Result<Vec<u8>> {
            self.events.push("read".to_owned());
            Ok(self.held.clone())
        }

        fn cut(&mut self, len: u64) -> io::Result<()> {
            self.events.push(format!("cut {len}"));
            if self.cut_fails {
                Err(io::Error::other("cannot cut"))
            } else {
                Ok(())
            }
        }

        fn append(&mut self, line: &str) -> io::Result<()> {
            self.events.push(format!("append {}", line.escape_debug()));
            if self.append_fails {
                Err(io::Error::other("disk full"))
            } else {
                Ok(())
            }
        }

        fn unlock(&mut self) -> io::Result<()> {
            self.events.push("unlock".to_owned());
            if self.unlock_fails {
                Err(io::Error::other("cannot unlock"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn a_claim_locks_the_file_before_it_looks_and_lets_go_after_the_append() {
        let held = b"s1\t1\told\ns1\t2\tha";
        let mut marks = Recording::of(held);
        let claimed = claim_over(&mut marks, "new", 9).expect("a claim");
        assert_eq!(claimed, Claimed::Marked);
        assert_eq!(
            marks.events,
            [
                "lock",
                "look",
                "read",
                "cut 9",
                "append s1\\t9\\tnew\\n",
                "unlock"
            ]
        );

        let mut marks = Recording::of(b"s1\t1\told\n");
        assert_eq!(
            claim_over(&mut marks, "new", 9).expect("a claim"),
            Claimed::Marked
        );
        assert_eq!(
            marks.events,
            ["lock", "look", "read", "append s1\\t9\\tnew\\n", "unlock"]
        );

        let mut marks = Recording::of(b"s1\t1\told\ns1\t2\tha");
        assert_eq!(
            claim_over(&mut marks, "old", 9).expect("a claim"),
            Claimed::Seen
        );
        assert_eq!(marks.events, ["lock", "look", "read", "unlock"]);

        let mut marks = Recording::of(b"");
        marks.look_fails = true;
        let error = claim_over(&mut marks, "new", 9).expect_err("a refused file");
        assert_eq!(error.to_string(), "not private");
        assert_eq!(marks.events, ["lock", "look", "unlock"]);

        let mut marks = Recording::of(b"s1\t1\told\ns1\t2\tha");
        marks.append_fails = true;
        let error = claim_over(&mut marks, "new", 9).expect_err("a failed append");
        assert_eq!(error.to_string(), "disk full");
        assert_eq!(marks.events.last().map(String::as_str), Some("unlock"));
        assert_eq!(
            marks
                .events
                .iter()
                .position(|event| event.starts_with("append")),
            Some(marks.events.len() - 2)
        );

        for held in [&b"s1\t1\told\n"[..], b""] {
            let mut marks = Recording::of(held);
            marks.unlock_fails = true;
            let marked = claim_over(&mut marks, "new", 9).expect("a completed claim");
            assert_eq!(marked, Claimed::Marked);
            let mut marks = Recording::of(b"s1\t1\told\n");
            marks.unlock_fails = true;
            let seen = claim_over(&mut marks, "old", 9).expect("a completed claim");
            assert_eq!(seen, Claimed::Seen);
        }

        let held = Err(io::Error::from_raw_os_error(24));
        let mut marks = Recording::of(b"s1\t1\told\n");
        marks.lock_fails = true;
        let error = claim_over(&mut marks, "new", 9).expect_err("a refused lock");
        assert_eq!(error.to_string(), "cannot lock");
        assert_eq!(marks.events, ["lock"]);

        let mut marks = Recording::of(b"s1\t1\told\ns1\t2\tha");
        marks.cut_fails = true;
        let error = claim_over(&mut marks, "new", 9).expect_err("a refused cut");
        assert_eq!(error.to_string(), "cannot cut");
        assert_eq!(marks.events, ["lock", "look", "read", "cut 9", "unlock"]);

        let mut unusable = Seen::new(&held, 9);
        let error = bilbo_search::digest::sessions::Sessions::claim(&mut unusable, "new")
            .expect_err("no state folder handle");
        assert_eq!(error.raw_os_error(), Some(24));
        assert_eq!(
            error.to_string(),
            io::Error::from_raw_os_error(24).to_string()
        );
    }
}
