use std::collections::{BTreeSet, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};

use crate::entries::{Change, Generation, Head, Mark, Meta, RecordRange, Stamp, StoreError};
use crate::files;
use crate::frame;
use crate::id::{Id, Short};
use crate::labels::{self, CaptureLabel, Filed, StoredLabels};
use crate::meta;

const SCHEMA: &str = "
CREATE TABLE catalog (key TEXT PRIMARY KEY, value BLOB NOT NULL);
CREATE TABLE segments (name TEXT PRIMARY KEY, length INTEGER NOT NULL);
CREATE TABLE entries (
  id BLOB PRIMARY KEY, kind TEXT NOT NULL, origin TEXT UNIQUE, captured INTEGER NOT NULL, parent BLOB
) WITHOUT ROWID;
CREATE TABLE stamps (
  entry BLOB PRIMARY KEY, size INTEGER NOT NULL, modified INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE generations (
  entry BLOB NOT NULL, generation INTEGER NOT NULL, records INTEGER NOT NULL,
  origin_end INTEGER NOT NULL, origin_hash BLOB NOT NULL, PRIMARY KEY (entry, generation)
) WITHOUT ROWID;
CREATE TABLE frames (
  entry BLOB NOT NULL, generation INTEGER NOT NULL, first INTEGER NOT NULL, count INTEGER NOT NULL,
  segment TEXT NOT NULL, offset INTEGER NOT NULL, length INTEGER NOT NULL, PRIMARY KEY (segment, offset)
);
CREATE INDEX frames_by_entry ON frames (entry, generation, first);
CREATE TABLE changes (
  seq INTEGER PRIMARY KEY, entry BLOB NOT NULL, generation INTEGER, first INTEGER, count INTEGER
);
CREATE TABLE labels (id INTEGER PRIMARY KEY, text TEXT NOT NULL UNIQUE);
CREATE TABLE label_words (
  word TEXT NOT NULL, label INTEGER NOT NULL, at INTEGER NOT NULL, PRIMARY KEY (word, label, at)
) WITHOUT ROWID;
CREATE TABLE entry_labels (
  entry BLOB NOT NULL, label INTEGER NOT NULL, PRIMARY KEY (entry, label)
) WITHOUT ROWID;
CREATE INDEX entry_labels_by_label ON entry_labels (label);
CREATE INDEX label_words_by_label ON label_words (label);
";

const VERSION: u32 = 5;

pub const OPEN: &str = "open";

type EntryRow = (String, Option<String>, i64, Option<Vec<u8>>);

fn int(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn uint(value: i64, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
}

fn small(value: i64, column: usize) -> rusqlite::Result<u32> {
    u32::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
}

fn blob<const N: usize>(raw: &[u8], column: usize) -> rusqlite::Result<[u8; N]> {
    <[u8; N]>::try_from(raw).map_err(|_| {
        rusqlite::Error::InvalidColumnType(
            column,
            "a blob of the wrong length".into(),
            rusqlite::types::Type::Blob,
        )
    })
}

fn id_of(raw: &[u8], column: usize) -> rusqlite::Result<Id> {
    blob::<16>(raw, column).map(Id::from_bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Follow {
    Append,
    NewGeneration,
}

/// Whether a body frame may follow an entry whose head is `(generation, records)`.
/// A count-0 frame is valid only as the opener of a new generation.
pub fn follows(head: (u32, u64), header: &frame::Header) -> Option<Follow> {
    let (generation, records) = head;
    if header.generation < 1 {
        return None;
    }
    if header.generation == generation
        && records.checked_add(1) == Some(header.first)
        && header.count >= 1
    {
        return Some(Follow::Append);
    }
    if generation.checked_add(1) == Some(header.generation) && header.first == 1 {
        return Some(Follow::NewGeneration);
    }
    None
}

pub struct Place<'a> {
    pub segment: &'a str,
    pub path: &'a Path,
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub first: u64,
    pub count: u64,
    pub segment: String,
    pub offset: u64,
    pub length: u64,
}

/// The store a catalog describes: a catalog never serves another root or state folder.
pub struct Binding {
    pub root: Vec<u8>,
    pub state: Vec<u8>,
}

pub enum Owner {
    Bound,
    Other,
    Unknown,
}

/// SQLite would read a WAL file's sidecars and could create them, so the header and the `-wal`
/// name are looked at with plain reads first: the magic, a rollback journal (bytes 18 and 19 are
/// 1), this catalog version at offset 60, and no `-wal` file. The header is read without following
/// a link, so a catalog that is one is no catalog. The version is read again from the opened
/// database: a writer's open has replayed a hot journal by then, a reader's open fails on one.
fn open_catalog(path: &Path, access: OpenFlags) -> rusqlite::Result<Option<Connection>> {
    let mut header = [0u8; 100];
    let read = files::open_no_follow(path, rustix::fs::OFlags::empty())
        .and_then(|mut file| file.read_exact(&mut header));
    if read.is_err()
        || !header.starts_with(b"SQLite format 3\0")
        || header[18] != 1
        || header[19] != 1
        || header[60..64] != VERSION.to_be_bytes()
    {
        return Ok(None);
    }
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    if !matches!(
        std::fs::symlink_metadata(&wal),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    ) {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(path, access | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok((version == i64::from(VERSION)).then_some(conn))
}

fn binds(conn: &Connection, binding: &Binding) -> rusqlite::Result<Owner> {
    let bound = |key: &str| -> rusqlite::Result<Option<Vec<u8>>> {
        conn.query_row(
            "SELECT value FROM catalog WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
    };
    let (Some(root), Some(state)) = (bound("root")?, bound("state")?) else {
        return Ok(Owner::Unknown);
    };
    Ok(
        if root == binding.root.as_slice() && state == binding.state.as_slice() {
            Owner::Bound
        } else {
            Owner::Other
        },
    )
}

pub struct Catalog {
    conn: Connection,
    path: PathBuf,
    id: [u8; 16],
}

pub enum Fresh {
    Current(Catalog),
    CatchUp(Catalog, u64),
    Rebuild,
}

/// An error that carries no catalog path yet.
pub enum ApplyFault {
    Sql(rusqlite::Error),
    Damaged(String),
}

impl From<rusqlite::Error> for ApplyFault {
    fn from(error: rusqlite::Error) -> Self {
        ApplyFault::Sql(error)
    }
}

impl Catalog {
    /// A new catalog file bound to one store, replacing any earlier one and its journals.
    pub fn create(path: &Path, binding: &Binding) -> Result<Catalog, StoreError> {
        for suffix in ["", "-journal", "-wal", "-shm"] {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            files::remove_if_present(Path::new(&name))?;
        }
        let conn = Connection::open(path).map_err(|error| sql_error(path, &error))?;
        Catalog::initialize(conn, path, Some(binding))
    }

    pub fn in_memory() -> Result<Catalog, StoreError> {
        let path = PathBuf::from(":memory:");
        let conn = Connection::open_in_memory().map_err(|error| sql_error(&path, &error))?;
        Catalog::initialize(conn, &path, None)
    }

    fn initialize(
        conn: Connection,
        path: &Path,
        binding: Option<&Binding>,
    ) -> Result<Catalog, StoreError> {
        let id = Id::mint().map_err(StoreError::Id)?.to_bytes();
        let run = || -> rusqlite::Result<()> {
            conn.execute_batch(SCHEMA)?;
            conn.pragma_update(None, "user_version", VERSION)?;
            let mut insert = conn.prepare("INSERT INTO catalog (key, value) VALUES (?1, ?2)")?;
            insert.execute(params!["id", id.as_slice()])?;
            if let Some(binding) = binding {
                insert.execute(params!["root", binding.root.as_slice()])?;
                insert.execute(params!["state", binding.state.as_slice()])?;
            }
            Ok(())
        };
        run().map_err(|error| sql_error(path, &error))?;
        Ok(Catalog {
            conn,
            path: path.to_path_buf(),
            id,
        })
    }

    /// The catalog at `path` when it is usable as it is, behind `open.seg`, or in need of a rebuild.
    pub fn open_current(
        path: &Path,
        binding: &Binding,
        sealed: &[(String, u64)],
        open_len: u64,
    ) -> Fresh {
        match Catalog::check(
            path,
            binding,
            sealed,
            open_len,
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        ) {
            Ok(fresh) => fresh,
            Err(_) => Fresh::Rebuild,
        }
    }

    /// Opened read-only when it already holds every frame of the store, so a reader walks no segment.
    pub fn open_unchanged(
        path: &Path,
        binding: &Binding,
        sealed: &[(String, u64)],
        open_len: u64,
    ) -> Option<Catalog> {
        match Catalog::check(
            path,
            binding,
            sealed,
            open_len,
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(Fresh::Current(catalog)) => Some(catalog),
            _ => None,
        }
    }

    pub fn owner(path: &Path, binding: &Binding) -> Owner {
        let read = || -> rusqlite::Result<Owner> {
            let Some(conn) = open_catalog(path, OpenFlags::SQLITE_OPEN_READ_ONLY)? else {
                return Ok(Owner::Unknown);
            };
            binds(&conn, binding)
        };
        read().unwrap_or(Owner::Unknown)
    }

    fn check(
        path: &Path,
        binding: &Binding,
        sealed: &[(String, u64)],
        open_len: u64,
        access: OpenFlags,
    ) -> rusqlite::Result<Fresh> {
        let Some(conn) = open_catalog(path, access)? else {
            return Ok(Fresh::Rebuild);
        };
        let raw: Vec<u8> =
            conn.query_row("SELECT value FROM catalog WHERE key = 'id'", [], |row| {
                row.get(0)
            })?;
        let Ok(id) = <[u8; 16]>::try_from(raw) else {
            return Ok(Fresh::Rebuild);
        };
        if !matches!(binds(&conn, binding)?, Owner::Bound) {
            return Ok(Fresh::Rebuild);
        }
        let listed: Vec<(String, u64)> = {
            let mut statement =
                conn.prepare("SELECT name, length FROM segments WHERE name != ?1 ORDER BY name")?;
            let rows = statement.query_map(params![OPEN], |row| {
                Ok((row.get::<_, String>(0)?, uint(row.get(1)?, 1)?))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        if listed != sealed {
            return Ok(Fresh::Rebuild);
        }
        let applied: Option<i64> = conn
            .query_row(
                "SELECT length FROM segments WHERE name = ?1",
                params![OPEN],
                |row| row.get(0),
            )
            .optional()?;
        let Some(applied) = applied.map(|applied| uint(applied, 0)).transpose()? else {
            return Ok(Fresh::Rebuild);
        };
        let catalog = Catalog {
            conn,
            path: path.to_path_buf(),
            id,
        };
        Ok(match applied.cmp(&open_len) {
            std::cmp::Ordering::Equal => Fresh::Current(catalog),
            std::cmp::Ordering::Less => Fresh::CatchUp(catalog, applied),
            std::cmp::Ordering::Greater => Fresh::Rebuild,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn id(&self) -> [u8; 16] {
        self.id
    }

    fn err(&self, error: &rusqlite::Error) -> StoreError {
        sql_error(&self.path, error)
    }

    pub fn batch(&mut self) -> Result<Batch<'_>, StoreError> {
        let tx = self
            .conn
            .transaction()
            .map_err(|error| sql_error(&self.path, &error))?;
        Ok(Batch {
            tx,
            catalog: &self.path,
        })
    }

    pub fn head_of(&self, id: Id) -> Result<Head, StoreError> {
        head_in(&self.conn, id).map_err(|error| self.err(&error))
    }

    pub fn head_by_capture(&self, label: &CaptureLabel) -> Result<Option<Head>, StoreError> {
        self.holder_of_capture(label)?
            .map(|id| self.head_of(id))
            .transpose()
    }

    /// `Some(label)` when an entry holds `id`; the label is `None` when that entry has no capture label.
    pub fn holder_of_id(&self, id: Id) -> Result<Option<Option<CaptureLabel>>, StoreError> {
        let found: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT origin FROM entries WHERE id = ?1",
                params![id.to_bytes().as_slice()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| self.err(&error))?;
        Ok(found.map(|label| label.and_then(CaptureLabel::new)))
    }

    pub fn holder_of_capture(&self, label: &CaptureLabel) -> Result<Option<Id>, StoreError> {
        let found: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT id FROM entries WHERE origin = ?1",
                params![label.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| self.err(&error))?;
        found
            .map(|raw| id_of(&raw, 0))
            .transpose()
            .map_err(|error| self.err(&error))
    }

    /// `instr`, not `LIKE`: no character of `part` is a wildcard.
    pub fn captures_with(&self, part: &str) -> Result<Vec<(Id, CaptureLabel)>, StoreError> {
        let run = || -> rusqlite::Result<Vec<(Id, CaptureLabel)>> {
            let mut statement = self.conn.prepare(
                "SELECT id, origin FROM entries
                 WHERE origin IS NOT NULL AND instr(lower(origin), lower(?1)) > 0 ORDER BY id",
            )?;
            let rows = statement.query_map(params![part], |row| {
                Ok((
                    id_of(&row.get::<_, Vec<u8>>(0)?, 0)?,
                    row.get::<_, String>(1)?,
                ))
            })?;
            let mut found = Vec::new();
            for row in rows {
                let (id, label) = row?;
                if let Some(label) = CaptureLabel::new(label) {
                    found.push((id, label));
                }
            }
            Ok(found)
        };
        run().map_err(|error| self.err(&error))
    }

    pub fn ids_ending(&self, shorts: &[Short]) -> Result<Vec<Id>, StoreError> {
        let mut tails: Vec<[u8; 6]> = shorts.iter().map(|short| short.tail()).collect();
        tails.sort_unstable();
        tails.dedup();
        if tails.is_empty() {
            return Ok(Vec::new());
        }
        let run = || -> rusqlite::Result<Vec<Id>> {
            let marks = vec!["?"; tails.len()].join(", ");
            let mut statement = self.conn.prepare(&format!(
                "SELECT id FROM entries WHERE substr(id, 11) IN ({marks}) ORDER BY id"
            ))?;
            let rows = statement.query_map(
                rusqlite::params_from_iter(tails.iter().map(<[u8; 6]>::as_slice)),
                |row| id_of(&row.get::<_, Vec<u8>>(0)?, 0),
            )?;
            let mut found = Vec::new();
            for row in rows {
                let id = row?;
                if shorts.iter().any(|short| short.ends(id)) {
                    found.push(id);
                }
            }
            Ok(found)
        };
        run().map_err(|error| self.err(&error))
    }

    pub fn meta(&self, id: Id) -> Result<Option<Meta>, StoreError> {
        let run = || -> rusqlite::Result<Option<Meta>> {
            let row: Option<EntryRow> = self
                .conn
                .query_row(
                    "SELECT kind, origin, captured, parent FROM entries WHERE id = ?1",
                    params![id.to_bytes().as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            let Some((kind, capture, captured, id_label)) = row else {
                return Ok(None);
            };
            Ok(Some(Meta {
                id,
                kind,
                labels: StoredLabels {
                    capture: capture.and_then(CaptureLabel::new),
                    id: id_label.map(|raw| id_of(&raw, 3)).transpose()?,
                },
                captured: meta::time_of(captured),
            }))
        };
        run().map_err(|error| self.err(&error))
    }

    /// The catalog is a cache: nothing else is written.
    pub fn set_stamp(&mut self, id: Id, stamp: Option<&Stamp>) -> Result<(), StoreError> {
        stamp_in(&self.conn, id, stamp).map_err(|error| self.err(&error))
    }

    /// The head of every entry that has a stamp, with the capture label the entry has now.
    pub fn stamped(&self) -> Result<Vec<(Head, Option<CaptureLabel>)>, StoreError> {
        let run = || -> rusqlite::Result<Vec<(Head, Option<CaptureLabel>)>> {
            let mut statement = self
                .conn
                .prepare("SELECT entry FROM stamps ORDER BY entry")?;
            let ids = statement
                .query_map([], |row| id_of(&row.get::<_, Vec<u8>>(0)?, 0))?
                .collect::<rusqlite::Result<Vec<Id>>>()?;
            ids.into_iter()
                .map(|id| Ok((head_in(&self.conn, id)?, capture_in(&self.conn, id)?)))
                .collect()
        };
        run().map_err(|error| self.err(&error))
    }

    /// Gives each head its stamp back when this catalog holds the same head and the same capture
    /// label, stamp aside. An entry with no capture label gets none.
    pub fn restamp(&mut self, stamped: &[(Head, Option<CaptureLabel>)]) -> Result<(), StoreError> {
        let mut same = Vec::new();
        for (old, label) in stamped {
            let now = self.head_of(old.id)?;
            let holder = capture_in(&self.conn, old.id).map_err(|error| self.err(&error))?;
            if label.is_some()
                && holder == *label
                && (now.generation, now.records, now.mark)
                    == (old.generation, old.records, old.mark)
            {
                same.push(old);
            }
        }
        let mut batch = self.batch()?;
        for head in same {
            batch.stamp(head.id, head.stamp.as_ref())?;
        }
        batch.commit()
    }

    pub fn entry_exists(&self, id: Id) -> Result<bool, StoreError> {
        Ok(self.holder_of_id(id)?.is_some())
    }

    pub fn generation_records(&self, id: Id, generation: u32) -> Result<Option<u64>, StoreError> {
        self.conn
            .query_row(
                "SELECT records FROM generations WHERE entry = ?1 AND generation = ?2",
                params![id.to_bytes().as_slice(), i64::from(generation)],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .and_then(|records| records.map(|records| uint(records, 0)).transpose())
            .map_err(|error| self.err(&error))
    }

    pub fn rows(
        &self,
        id: Id,
        generation: u32,
        first: u64,
        last: u64,
    ) -> Result<Vec<Row>, StoreError> {
        let run = || -> rusqlite::Result<Vec<Row>> {
            let mut statement = self.conn.prepare(
                "SELECT first, count, segment, offset, length FROM frames
                 WHERE entry = ?1 AND generation = ?2 AND first <= ?4 AND first + count - 1 >= ?3
                 ORDER BY first",
            )?;
            let rows = statement.query_map(
                params![
                    id.to_bytes().as_slice(),
                    i64::from(generation),
                    int(first),
                    int(last)
                ],
                |row| {
                    Ok(Row {
                        first: uint(row.get(0)?, 0)?,
                        count: uint(row.get(1)?, 1)?,
                        segment: row.get(2)?,
                        offset: uint(row.get(3)?, 3)?,
                        length: uint(row.get(4)?, 4)?,
                    })
                },
            )?;
            rows.collect()
        };
        run().map_err(|error| self.err(&error))
    }

    pub fn changes_since(&self, since: u64) -> Result<(Vec<Change>, u64), StoreError> {
        let run = || -> rusqlite::Result<(Vec<Change>, u64)> {
            let mut statement = self.conn.prepare(
                "SELECT seq, entry, generation, first, count FROM changes WHERE seq > ?1 ORDER BY seq",
            )?;
            let mut last = None;
            let mut changes = Vec::new();
            let mut rows = statement.query(params![int(since)])?;
            while let Some(row) = rows.next()? {
                last = Some(uint(row.get(0)?, 0)?);
                let id = id_of(&row.get::<_, Vec<u8>>(1)?, 1)?;
                let generation: Option<i64> = row.get(2)?;
                changes.push(match generation {
                    None => Change::Meta(id),
                    Some(generation) => Change::Records {
                        id,
                        generation: Generation(small(generation, 2)?),
                        records: RecordRange {
                            first: uint(row.get(3)?, 3)?,
                            count: uint(row.get(4)?, 4)?,
                        },
                    },
                });
            }
            drop(rows);
            drop(statement);
            let last = match last {
                Some(last) => last,
                None => {
                    let newest: Option<i64> =
                        self.conn
                            .query_row("SELECT max(seq) FROM changes", [], |row| row.get(0))?;
                    newest.map_or(Ok(0), |newest| uint(newest, 0))?
                }
            };
            Ok((changes, last))
        };
        run().map_err(|error| self.err(&error))
    }

    pub fn counts(&self) -> Result<(u64, u64), StoreError> {
        let run = || -> rusqlite::Result<(u64, u64)> {
            let entries: i64 = self
                .conn
                .query_row("SELECT count(*) FROM entries", [], |row| row.get(0))?;
            let segments: i64 = self.conn.query_row(
                "SELECT count(*) FROM segments WHERE name != ?1",
                params![OPEN],
                |row| row.get(0),
            )?;
            Ok((uint(entries, 0)?, uint(segments, 0)?))
        };
        run().map_err(|error| self.err(&error))
    }

    pub fn belonging_to(&self, entries: &[Id]) -> Result<Vec<Id>, StoreError> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        let run = || -> rusqlite::Result<Vec<(Id, Id)>> {
            let mut statement = self
                .conn
                .prepare("SELECT id, parent FROM entries WHERE parent IS NOT NULL")?;
            let rows = statement.query_map([], |row| {
                Ok((
                    id_of(&row.get::<_, Vec<u8>>(0)?, 0)?,
                    id_of(&row.get::<_, Vec<u8>>(1)?, 1)?,
                ))
            })?;
            rows.collect()
        };
        let links = run().map_err(|error| self.err(&error))?;
        Ok(labels::descendants(&links, entries))
    }

    pub fn with_words(&self, words: &[String]) -> Result<Vec<Id>, StoreError> {
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let run = || -> rusqlite::Result<Vec<Id>> {
            let mut places = self
                .conn
                .prepare("SELECT label, at FROM label_words WHERE word = ?1")?;
            let mut starts: Option<HashSet<(i64, i64)>> = None;
            for (position, word) in words.iter().enumerate() {
                let position = i64::try_from(position).unwrap_or(i64::MAX);
                let at: HashSet<(i64, i64)> = places
                    .query_map(params![word], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                starts = Some(match starts {
                    None => at,
                    Some(starts) => starts
                        .into_iter()
                        .filter(|(label, start)| at.contains(&(*label, start + position)))
                        .collect(),
                });
                if starts.as_ref().is_some_and(HashSet::is_empty) {
                    return Ok(Vec::new());
                }
            }
            let matching: BTreeSet<i64> = starts
                .unwrap_or_default()
                .into_iter()
                .map(|(label, _)| label)
                .collect();
            let mut holders = self
                .conn
                .prepare("SELECT entry FROM entry_labels WHERE label = ?1")?;
            let mut found = BTreeSet::new();
            for label in matching {
                for row in
                    holders.query_map(params![label], |row| id_of(&row.get::<_, Vec<u8>>(0)?, 0))?
                {
                    found.insert(row?);
                }
            }
            Ok(found.into_iter().collect())
        };
        run().map_err(|error| self.err(&error))
    }

    pub fn relabel(&self, filed: &[Filed]) -> Result<(), StoreError> {
        let run = || -> rusqlite::Result<()> {
            let tx = self.conn.unchecked_transaction()?;
            relabel_in(&tx, filed)?;
            tx.commit()
        };
        run().map_err(|error| self.err(&error))
    }
}

fn relabel_in(conn: &Connection, filed: &[Filed]) -> rusqlite::Result<()> {
    let mut released = HashSet::new();
    let mut held = conn.prepare("SELECT label FROM entry_labels WHERE entry = ?1")?;
    let mut release = conn.prepare("DELETE FROM entry_labels WHERE entry = ?1")?;
    let mut find = conn.prepare("SELECT id FROM labels WHERE text = ?1")?;
    let mut add = conn.prepare("INSERT INTO labels (text) VALUES (?1)")?;
    let mut known = conn.prepare("SELECT word, at FROM label_words WHERE label = ?1")?;
    let mut clear = conn.prepare("DELETE FROM label_words WHERE label = ?1")?;
    let mut put = conn.prepare("INSERT INTO label_words (word, label, at) VALUES (?1, ?2, ?3)")?;
    let mut file =
        conn.prepare("INSERT OR IGNORE INTO entry_labels (entry, label) VALUES (?1, ?2)")?;
    for one in filed {
        let entry = one.entry.to_bytes();
        for label in held.query_map(params![entry.as_slice()], |row| row.get::<_, i64>(0))? {
            released.insert(label?);
        }
        release.execute(params![entry.as_slice()])?;
        for worded in &one.labels {
            let found: Option<i64> = find
                .query_row(params![worded.text], |row| row.get(0))
                .optional()?;
            let (label, rewrite) = match found {
                None => {
                    add.execute(params![worded.text])?;
                    (conn.last_insert_rowid(), true)
                }
                Some(label) => {
                    let rows = known
                        .query_map(params![label], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                        })?
                        .collect::<rusqlite::Result<HashSet<_>>>()?;
                    let given: HashSet<(String, i64)> =
                        worded.words.iter().cloned().zip(0i64..).collect();
                    let same = rows == given;
                    if !same {
                        clear.execute(params![label])?;
                    }
                    (label, !same)
                }
            };
            if rewrite {
                for (word, at) in worded.words.iter().zip(0i64..) {
                    put.execute(params![word, label, at])?;
                }
            }
            file.execute(params![entry.as_slice(), label])?;
        }
    }
    let mut taken = conn.prepare("SELECT 1 FROM entry_labels WHERE label = ?1 LIMIT 1")?;
    for label in released {
        if !taken.exists(params![label])? {
            clear.execute(params![label])?;
            conn.execute("DELETE FROM labels WHERE id = ?1", params![label])?;
        }
    }
    Ok(())
}

fn sql_error(path: &Path, error: &rusqlite::Error) -> StoreError {
    StoreError::Catalog {
        path: path.to_path_buf(),
        error: error.to_string(),
    }
}

fn stamp_in(conn: &Connection, id: Id, stamp: Option<&Stamp>) -> rusqlite::Result<()> {
    let Some(stamp) = stamp else {
        return conn
            .execute(
                "DELETE FROM stamps WHERE entry = ?1",
                params![id.to_bytes().as_slice()],
            )
            .map(|_| ());
    };
    conn.execute(
        "INSERT OR REPLACE INTO stamps (entry, size, modified) VALUES (?1, ?2, ?3)",
        params![
            id.to_bytes().as_slice(),
            int(stamp.size),
            meta::nanos(stamp.modified)
        ],
    )
    .map(|_| ())
}

fn capture_in(conn: &Connection, id: Id) -> rusqlite::Result<Option<CaptureLabel>> {
    let found: Option<Option<String>> = conn
        .query_row(
            "SELECT origin FROM entries WHERE id = ?1",
            params![id.to_bytes().as_slice()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.flatten().and_then(CaptureLabel::new))
}

fn stamp_of(conn: &Connection, id: Id) -> rusqlite::Result<Option<Stamp>> {
    let row: Option<(i64, i64)> = conn
        .query_row(
            "SELECT size, modified FROM stamps WHERE entry = ?1",
            params![id.to_bytes().as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(size, modified)| {
        Ok(Stamp {
            size: uint(size, 0)?,
            modified: meta::time_of_nanos(modified),
        })
    })
    .transpose()
}

fn head_in(conn: &Connection, id: Id) -> rusqlite::Result<Head> {
    let row: Option<(i64, i64, i64, Vec<u8>)> = conn
        .query_row(
            "SELECT generation, records, origin_end, origin_hash FROM generations
             WHERE entry = ?1 ORDER BY generation DESC LIMIT 1",
            params![id.to_bytes().as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    Ok(match row {
        Some((generation, records, end, hash)) => Head {
            id,
            generation: Generation(small(generation, 0)?),
            records: uint(records, 1)?,
            mark: Mark {
                end: uint(end, 2)?,
                hash: u128::from_le_bytes(blob::<16>(&hash, 3)?),
            },
            stamp: stamp_of(conn, id)?,
        },
        None => Head {
            id,
            generation: Generation(0),
            records: 0,
            mark: Mark {
                end: 0,
                hash: xxhash_rust::xxh3::xxh3_128(b""),
            },
            stamp: None,
        },
    })
}

pub struct Batch<'c> {
    tx: Transaction<'c>,
    catalog: &'c Path,
}

impl Batch<'_> {
    fn settle(&self, place: &Place<'_>, fault: ApplyFault) -> StoreError {
        match fault {
            ApplyFault::Sql(error) => sql_error(self.catalog, &error),
            ApplyFault::Damaged(what) => StoreError::Damaged {
                path: place.path.to_path_buf(),
                offset: place.offset,
                what,
            },
        }
    }

    pub fn meta(&mut self, meta: &Meta, place: &Place<'_>) -> Result<(), StoreError> {
        meta_in(&self.tx, meta).map_err(|fault| self.settle(place, fault))
    }

    pub fn body(&mut self, header: &frame::Header, place: &Place<'_>) -> Result<(), StoreError> {
        body_in(&self.tx, header, place).map_err(|fault| self.settle(place, fault))
    }

    pub fn stamp(&mut self, id: Id, stamp: Option<&Stamp>) -> Result<(), StoreError> {
        stamp_in(&self.tx, id, stamp).map_err(|error| sql_error(self.catalog, &error))
    }

    pub fn set_segment(&mut self, name: &str, length: u64) -> Result<(), StoreError> {
        self.tx
            .execute(
                "INSERT OR REPLACE INTO segments (name, length) VALUES (?1, ?2)",
                params![name, int(length)],
            )
            .map(|_| ())
            .map_err(|error| sql_error(self.catalog, &error))
    }

    /// The frames of `open.seg` now live in the sealed segment `name`.
    pub fn seal(&mut self, name: &str, length: u64, open_length: u64) -> Result<(), StoreError> {
        self.tx
            .execute(
                "UPDATE frames SET segment = ?1 WHERE segment = ?2",
                params![name, OPEN],
            )
            .map_err(|error| sql_error(self.catalog, &error))?;
        self.set_segment(name, length)?;
        self.set_segment(OPEN, open_length)
    }

    pub fn commit(self) -> Result<(), StoreError> {
        let path = self.catalog;
        self.tx.commit().map_err(|error| sql_error(path, &error))
    }
}

fn meta_in(conn: &Connection, meta: &Meta) -> Result<(), ApplyFault> {
    let id = meta.id.to_bytes();
    if let Some(label) = &meta.labels.capture {
        let other: Option<Vec<u8>> = conn
            .query_row(
                "SELECT id FROM entries WHERE origin = ?1 AND id != ?2",
                params![label.as_str(), id.as_slice()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(other) = other {
            return Err(ApplyFault::Damaged(format!(
                "the capture label {label} is held by {}",
                id_of(&other, 0)?
            )));
        }
    }
    let held = capture_in(conn, meta.id)?;
    if held != meta.labels.capture {
        conn.execute(
            "DELETE FROM stamps WHERE entry = ?1",
            params![id.as_slice()],
        )?;
    }
    conn.execute(
        "INSERT OR REPLACE INTO entries (id, kind, origin, captured, parent) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            id.as_slice(),
            meta.kind,
            meta.labels.capture.as_ref().map(CaptureLabel::as_str),
            meta::millis(meta.captured),
            meta.labels
                .id
                .map(Id::to_bytes)
                .as_ref()
                .map(<[u8; 16]>::as_slice)
        ],
    )?;
    conn.execute(
        "INSERT INTO changes (entry) VALUES (?1)",
        params![id.as_slice()],
    )?;
    Ok(())
}

fn body_in(conn: &Connection, header: &frame::Header, place: &Place<'_>) -> Result<(), ApplyFault> {
    let id = header.entry.to_bytes();
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM entries WHERE id = ?1",
            params![id.as_slice()],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(ApplyFault::Damaged(
            "a body frame before its entry's meta".into(),
        ));
    }
    let head = head_in(conn, header.entry)?;
    let kind = follows((head.generation.0, head.records), header).ok_or_else(|| {
        ApplyFault::Damaged(format!(
            "frame of entry {} generation {} records {}+{} does not follow generation {} with {} records",
            header.entry, header.generation, header.first, header.count, head.generation.0, head.records
        ))
    })?;
    let hash = header.hash.to_le_bytes();
    match kind {
        Follow::Append => {
            conn.execute(
                "UPDATE generations SET records = ?3, origin_end = ?4, origin_hash = ?5
                 WHERE entry = ?1 AND generation = ?2",
                params![
                    id.as_slice(),
                    i64::from(header.generation),
                    int(head.records + header.count),
                    int(header.end),
                    hash.as_slice()
                ],
            )?;
        }
        Follow::NewGeneration => {
            conn.execute(
                "INSERT INTO generations (entry, generation, records, origin_end, origin_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id.as_slice(),
                    i64::from(header.generation),
                    int(header.count),
                    int(header.end),
                    hash.as_slice()
                ],
            )?;
        }
    }
    conn.execute(
        "INSERT INTO frames (entry, generation, first, count, segment, offset, length)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id.as_slice(),
            i64::from(header.generation),
            int(header.first),
            int(header.count),
            place.segment,
            int(place.offset),
            int(place.length)
        ],
    )?;
    conn.execute(
        "INSERT INTO changes (entry, generation, first, count) VALUES (?1, ?2, ?3, ?4)",
        params![
            id.as_slice(),
            i64::from(header.generation),
            int(header.first),
            int(header.count)
        ],
    )?;
    conn.execute(
        "DELETE FROM stamps WHERE entry = ?1",
        params![id.as_slice()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(generation: u32, first: u64, count: u64) -> frame::Header {
        frame::Header::body(
            Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("id"),
            generation,
            first,
            count,
            0,
            0,
        )
    }

    #[test]
    fn a_body_frame_must_follow_its_head() {
        type Case = (&'static str, (u32, u64), frame::Header, Option<Follow>);
        let cases: Vec<Case> = vec![
            ("append", (1, 3), header(1, 4, 2), Some(Follow::Append)),
            (
                "append of one",
                (2, 1),
                header(2, 2, 1),
                Some(Follow::Append),
            ),
            (
                "new generation",
                (1, 3),
                header(2, 1, 5),
                Some(Follow::NewGeneration),
            ),
            (
                "new empty generation",
                (1, 3),
                header(2, 1, 0),
                Some(Follow::NewGeneration),
            ),
            (
                "first generation",
                (0, 0),
                header(1, 1, 2),
                Some(Follow::NewGeneration),
            ),
            (
                "first empty generation",
                (0, 0),
                header(1, 1, 0),
                Some(Follow::NewGeneration),
            ),
            (
                "count 0 in the head generation",
                (1, 3),
                header(1, 4, 0),
                None,
            ),
            ("duplicate opener", (1, 0), header(1, 1, 0), None),
            ("gap in the records", (1, 3), header(1, 5, 1), None),
            ("overlap", (1, 3), header(1, 3, 1), None),
            ("skipped generation", (1, 3), header(3, 1, 1), None),
            ("old generation", (2, 3), header(1, 4, 1), None),
            ("new generation not from 1", (1, 3), header(2, 2, 1), None),
            ("generation 0", (0, 0), header(0, 1, 1), None),
        ];
        for (name, head, header, expected) in cases {
            assert_eq!(follows(head, &header), expected, "{name}");
        }
    }

    #[test]
    fn a_bad_value_in_the_catalog_is_an_error_not_zero() {
        let id = Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("id");
        let catalog = Catalog::in_memory().expect("catalog");
        catalog
            .conn
            .execute(
                "INSERT INTO frames VALUES (?1, 1, 1, 1, 'open', -5, 80)",
                params![id.to_bytes().as_slice()],
            )
            .expect("insert");
        assert!(matches!(
            catalog.rows(id, 1, 1, 1),
            Err(StoreError::Catalog { .. })
        ));
        catalog
            .conn
            .execute("INSERT INTO changes (entry) VALUES (x'0102')", [])
            .expect("insert");
        assert!(matches!(
            catalog.changes_since(0),
            Err(StoreError::Catalog { .. })
        ));
        catalog
            .conn
            .execute(
                "INSERT INTO generations VALUES (?1, 1, 1, 1, x'01')",
                params![id.to_bytes().as_slice()],
            )
            .expect("insert");
        assert!(matches!(
            catalog.head_of(id),
            Err(StoreError::Catalog { .. })
        ));
    }
}
