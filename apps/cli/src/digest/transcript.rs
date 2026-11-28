use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use rustix::fs::{FileType, Mode, OFlags};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unread {
    NotRegular,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tail {
    pub bytes: Vec<u8>,
    pub from_start: bool,
}

fn read_tail<S: Read + Seek>(source: &mut S, size: u64, limit: u64) -> std::io::Result<Tail> {
    let start = size.saturating_sub(limit);
    source.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    source.take(limit).read_to_end(&mut bytes)?;
    Ok(Tail {
        bytes,
        from_start: start == 0,
    })
}

pub fn tail(path: &str, limit: u64) -> Result<Tail, Unread> {
    let failed = |error: &dyn ToString| Unread::Failed(error.to_string());
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| failed(&std::io::Error::from(error)))?;
    let stat = rustix::fs::fstat(&fd).map_err(|error| failed(&std::io::Error::from(error)))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(Unread::NotRegular);
    }
    let size = u64::try_from(stat.st_size).unwrap_or(0);
    read_tail(&mut File::from(fd), size, limit).map_err(|error| failed(&error))
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read, Seek, SeekFrom};

    use super::read_tail;

    struct Counted {
        data: Vec<u8>,
        at: usize,
        sought: Vec<u64>,
        handed: usize,
        before_seek: usize,
    }

    impl Counted {
        fn new(len: usize) -> Counted {
            Counted {
                data: (0..len).map(|n| (n % 251) as u8).collect(),
                at: 0,
                sought: Vec::new(),
                handed: 0,
                before_seek: 0,
            }
        }
    }

    impl Read for Counted {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.sought.is_empty() {
                self.before_seek += 1;
            }
            let count = buffer.len().min(self.data.len() - self.at);
            buffer[..count].copy_from_slice(&self.data[self.at..self.at + count]);
            self.at += count;
            self.handed += count;
            Ok(count)
        }
    }

    impl Seek for Counted {
        fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
            let SeekFrom::Start(place) = to else {
                panic!("only a seek from the start");
            };
            self.sought.push(place);
            self.at = usize::try_from(place).expect("a place");
            Ok(place)
        }
    }

    #[test]
    fn the_tail_is_sought_once_and_read_within_the_limit_though_the_file_grew() {
        let mut source = Counted::new(4096);
        let tail = read_tail(&mut source, 3072, 1024).expect("a tail");
        assert_eq!(source.sought, [2048]);
        assert_eq!(source.before_seek, 0);
        assert!(source.handed <= 1024);
        assert_eq!(tail.bytes, source.data[2048..3072]);
        assert!(!tail.from_start);

        for size in [100, 1024] {
            let mut source = Counted::new(4096);
            let tail = read_tail(&mut source, size, 1024).expect("a tail");
            assert_eq!(source.sought, [0], "{size}");
            assert!(source.handed <= 1024, "{size}");
            assert_eq!(tail.bytes, source.data[..1024], "{size}");
            assert!(tail.from_start);
        }
    }
}
