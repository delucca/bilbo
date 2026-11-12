use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use bilbo_search::recall::log::QueryLog;
use bilbo_store::segments::State;
use rustix::fs::{Mode, OFlags, openat};
use rustix::process::geteuid;

const NAME: &str = "queries.log";

pub struct File<'a> {
    state: Option<&'a State>,
}

impl<'a> File<'a> {
    pub fn at(state: Option<&'a State>) -> File<'a> {
        File { state }
    }
}

trait Sink {
    fn len(&self) -> io::Result<u64>;
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize>;
    fn cut(&mut self, len: u64) -> io::Result<()>;
}

impl Sink for fs::File {
    fn len(&self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Write::write(self, bytes)
    }

    fn cut(&mut self, len: u64) -> io::Result<()> {
        self.set_len(len)
    }
}

/// One `write`, so a line is never interleaved; a line that did not go whole is cut off again.
fn append(sink: &mut dyn Sink, line: &str) -> io::Result<()> {
    let before = sink.len()?;
    let failure = match sink.write(line.as_bytes()) {
        Ok(written) if written == line.len() => return Ok(()),
        Ok(written) => format!("wrote {written} of {} bytes", line.len()),
        Err(error) => error.to_string(),
    };
    match sink.cut(before) {
        Ok(()) => Err(io::Error::other(format!(
            "{failure}; the log was cut back to its length of {before} bytes"
        ))),
        Err(error) => Err(io::Error::other(format!(
            "{failure}; the log could not be cut back: {error}"
        ))),
    }
}

impl QueryLog for File<'_> {
    fn append(&mut self, line: &str) -> io::Result<()> {
        let state = self
            .state
            .ok_or_else(|| io::Error::other("this store has no state folder that bilbo checked"))?;
        let refuse = || refused(&state.path().join(NAME));
        let descriptor = openat(
            state.folder(),
            NAME,
            OFlags::WRONLY
                | OFlags::APPEND
                | OFlags::CREATE
                | OFlags::NOFOLLOW
                | OFlags::NONBLOCK
                | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )?;
        let mut file = fs::File::from(descriptor);
        if !private(&file.metadata()?) {
            return Err(refuse());
        }
        file.lock()?;
        if !private(&file.metadata()?) {
            return Err(refuse());
        }
        append(&mut file, line)
    }
}

fn private(meta: &fs::Metadata) -> bool {
    meta.is_file()
        && meta.mode() & 0o077 == 0
        && meta.nlink() == 1
        && meta.uid() == geteuid().as_raw()
}

fn refused(path: &Path) -> io::Error {
    io::Error::other(format!(
        "{} is not a private file of this user in a folder of this user",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{Sink, append};

    struct Short {
        bytes: Vec<u8>,
        writes: usize,
        fail_cut: bool,
    }

    impl Sink for Short {
        fn len(&self) -> io::Result<u64> {
            Ok(self.bytes.len() as u64)
        }

        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            let part = &bytes[..bytes.len() / 2];
            self.bytes.extend_from_slice(part);
            Ok(part.len())
        }

        fn cut(&mut self, len: u64) -> io::Result<()> {
            if self.fail_cut {
                return Err(io::Error::other("no"));
            }
            self.bytes.truncate(usize::try_from(len).expect("a length"));
            Ok(())
        }
    }

    #[test]
    fn a_short_write_cuts_the_file_back_and_is_a_failed_append() {
        let mut sink = Short {
            bytes: b"before\n".to_vec(),
            writes: 0,
            fail_cut: false,
        };
        let error = append(&mut sink, "a whole line\n").expect_err("a failed append");
        assert_eq!(
            error.to_string(),
            "wrote 6 of 13 bytes; the log was cut back to its length of 7 bytes"
        );
        assert_eq!(sink.bytes, b"before\n");
        assert_eq!(sink.writes, 1, "one write");
        sink.fail_cut = true;
        let error = append(&mut sink, "a whole line\n").expect_err("a failed append");
        assert!(
            error.to_string().contains("could not be cut back: no"),
            "{error}"
        );
    }
}
