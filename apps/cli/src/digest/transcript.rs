use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use rustix::fs::{FileType, Mode, OFlags};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unread {
    NotRegular,
    Failed(String),
}

pub fn tail(path: &str, limit: u64) -> Result<Vec<u8>, Unread> {
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
    let mut file = File::from(fd);
    let size = u64::try_from(stat.st_size).unwrap_or(0);
    file.seek(SeekFrom::Start(size.saturating_sub(limit)))
        .map_err(|error| failed(&error))?;
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|error| failed(&error))?;
    Ok(bytes)
}
