use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::ops::Bound;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use bilbo_search::analyze;
use bilbo_search::index::{Doc, Extent, Index, IndexError, Stats, millis};
use bilbo_search::position::Position;
use bilbo_store::entries::{Generation, RecordRange};
use bilbo_store::id::Id;
use tantivy::DocSet;
use tantivy::TERMINATED;
use tantivy::collector::{Collector, SegmentCollector};
use tantivy::index::SegmentComponent;
use tantivy::merge_policy::LogMergePolicy;
use tantivy::postings::Postings;
use tantivy::query::{BooleanQuery, EnableScoring, Occur, Query, RangeQuery, TermQuery};
use tantivy::schema::{
    BytesOptions, FAST, Field, INDEXED, IndexRecordOption, STRING, Schema, TextFieldIndexing,
    TextOptions,
};
use tantivy::store::Compressor;
use tantivy::tokenizer::{PreTokenizedString, Token};
use tantivy::{
    DocAddress, DocId, IndexSettings, IndexWriter, Score, SegmentOrdinal, SegmentReader,
    TantivyDocument, Term,
};

const MARKER_NAME: &str = "bilbo-index";
const MARKER: &[u8] = b"bilbo index\n";
const MANAGED: &str = ".managed.json";
const MANAGED_LIMIT: u64 = 64 * 1024 * 1024;

/// The footer's fixed part, `<(u32, u32)>::SIZE_IN_BYTES` in tantivy 0.27's `directory/footer.rs`:
/// a file shorter than this makes `Footer::extract_footer` panic.
const FOOTER_FIXED_BYTES: u64 = 8;

/// What the index folder is, judged without following a link and before tantivy is called.
enum Folder {
    Absent,
    /// No entry, or only a marker whose write was cut short.
    Unmarked {
        cut_marker: bool,
    },
    /// A valid marker, and every other entry a regular file named as tantivy names its own.
    Marked(Vec<String>),
}

/// The writer's memory, in bytes: tantivy takes 15 MB at least.
const HEAP: usize = 64 * 1024 * 1024;

/// A segment whose deleted share is over this is merged, where tantivy's default waits for all of it.
const DELETED_SHARE_BEFORE_MERGE: f32 = 0.2;

struct Fields {
    text: Field,
    entry: Field,
    generation: Field,
    first: Field,
    count: Field,
    part: Field,
    parts: Field,
    date: Field,
    kind: Field,
}

struct Open {
    writer: IndexWriter<TantivyDocument>,
    fields: Fields,
}

/// The index in one folder. Nothing touches the folder before the first call of a port method.
pub struct Tantivy {
    folder: PathBuf,
    open: Option<Open>,
}

fn schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();
    let words = TextFieldIndexing::default()
        .set_tokenizer("raw")
        .set_fieldnorms(true)
        .set_index_option(IndexRecordOption::WithFreqs);
    let fields = Fields {
        text: builder.add_text_field("text", TextOptions::default().set_indexing_options(words)),
        entry: builder.add_bytes_field("entry", BytesOptions::default().set_indexed().set_fast()),
        generation: builder.add_u64_field("generation", FAST),
        first: builder.add_u64_field("first", FAST),
        count: builder.add_u64_field("count", FAST),
        part: builder.add_u64_field("part", FAST),
        parts: builder.add_u64_field("parts", FAST),
        date: builder.add_i64_field("date", INDEXED | FAST),
        kind: builder.add_text_field("kind", STRING | FAST),
    };
    (builder.build(), fields)
}

/// A name tantivy gives a file of an index: its meta files, its locks, a temporary file, or a
/// segment's file, named by 32 hex digits.
fn is_tantivy_name(name: &str) -> bool {
    if matches!(
        name,
        "meta.json" | ".managed.json" | ".tantivy-writer.lock" | ".tantivy-meta.lock"
    ) || name.starts_with(".tmp")
    {
        return true;
    }
    let (stem, extension) = name.split_once('.').unwrap_or((name, ""));
    stem.len() == 32
        && stem.bytes().all(|byte| byte.is_ascii_hexdigit())
        && !extension.is_empty()
        && extension
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.')
}

impl Tantivy {
    pub fn at(folder: PathBuf) -> Tantivy {
        Tantivy { folder, open: None }
    }

    fn failed(&self, action: &'static str, error: &dyn std::fmt::Display) -> IndexError {
        IndexError::Failed {
            action,
            path: self.folder.clone(),
            error: error.to_string(),
        }
    }

    fn inspect(&self) -> Result<Folder, IndexError> {
        match fs::symlink_metadata(&self.folder) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => return Err(IndexError::Foreign(self.folder.clone())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Folder::Absent),
            Err(error) => return Err(self.failed("read", &error)),
        }
        let listed = fs::read_dir(&self.folder).map_err(|error| self.failed("read", &error))?;
        let mut entries = Vec::new();
        for entry in listed {
            let entry = entry.map_err(|error| self.failed("read", &error))?;
            let plain = entry
                .file_type()
                .map_err(|error| self.failed("read", &error))?
                .is_file();
            let Ok(name) = entry.file_name().into_string() else {
                return Err(IndexError::Foreign(self.folder.clone()));
            };
            entries.push((name, plain));
        }
        if entries.is_empty() {
            return Ok(Folder::Unmarked { cut_marker: false });
        }
        let marker = entries
            .iter()
            .find(|(name, _)| name == MARKER_NAME)
            .filter(|(_, plain)| *plain);
        let Some(_) = marker else {
            return Err(IndexError::Foreign(self.folder.clone()));
        };
        let found = self.marker_bytes()?;
        if found == MARKER {
            let mut names = Vec::new();
            for (name, plain) in entries {
                if name == MARKER_NAME {
                    continue;
                }
                if !plain || !is_tantivy_name(&name) {
                    return Err(IndexError::Foreign(self.folder.clone()));
                }
                names.push(name);
            }
            names.sort_unstable();
            Ok(Folder::Marked(names))
        } else if entries.len() == 1 && MARKER.starts_with(&found) {
            Ok(Folder::Unmarked { cut_marker: true })
        } else {
            Err(IndexError::Foreign(self.folder.clone()))
        }
    }

    fn marker_bytes(&self) -> Result<Vec<u8>, IndexError> {
        let mut found = Vec::new();
        fs::File::open(self.folder.join(MARKER_NAME))
            .and_then(|file| file.take(MARKER.len() as u64 + 1).read_to_end(&mut found))
            .map_err(|error| self.failed("read", &error))?;
        Ok(found)
    }

    /// Whether `.managed.json`, when there is one, is a list of tantivy's file names that tantivy
    /// can safely delete from the folder.
    fn managed_is_safe(&self, names: &[String]) -> Result<bool, IndexError> {
        if !names.iter().any(|name| name == MANAGED) {
            return Ok(true);
        }
        let mut bytes = Vec::new();
        fs::File::open(self.folder.join(MANAGED))
            .and_then(|file| file.take(MANAGED_LIMIT + 1).read_to_end(&mut bytes))
            .map_err(|error| self.failed("read", &error))?;
        Ok(bytes.len() as u64 <= MANAGED_LIMIT && is_managed_list(&bytes))
    }

    fn write_marker(&self, cut_marker: bool) -> Result<(), IndexError> {
        let failed = |error: io::Error| self.failed("create", &error);
        let marker = self.folder.join(MARKER_NAME);
        if cut_marker {
            fs::remove_file(&marker).map_err(failed)?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&marker)
            .map_err(failed)?;
        file.write_all(MARKER).map_err(failed)?;
        file.sync_all().map_err(failed)?;
        fs::File::open(&self.folder)
            .and_then(|folder| folder.sync_all())
            .map_err(failed)
    }

    /// The folder's index when it is bilbo's and its metadata opens with this schema; `None` when
    /// the folder was absent or empty and now holds a marker, or was bilbo's and anything else
    /// and now holds the marker alone. No segment is opened and no component read: what a
    /// component holds cannot stop a rebuild. A folder that is not bilbo's is `Foreign` and
    /// untouched. Only `clear` calls this: every other method leaves a folder that is not whole
    /// as it is.
    fn existing(&self, schema: &Schema) -> Result<Option<tantivy::Index>, IndexError> {
        match self.inspect()? {
            Folder::Absent => {
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(&self.folder)
                    .map_err(|error| self.failed("create", &error))?;
                self.write_marker(false)?;
                Ok(None)
            }
            Folder::Unmarked { cut_marker } => {
                self.write_marker(cut_marker)?;
                Ok(None)
            }
            Folder::Marked(names) => {
                if self.managed_is_safe(&names)? && names.iter().any(|name| name == "meta.json") {
                    match tantivy::Index::open_in_dir(&self.folder) {
                        Ok(index) if index.schema() == *schema => return Ok(Some(index)),
                        Ok(_) => {}
                        Err(error) if is_unreadable(&error) => {
                            return Err(self.failed("read", &error));
                        }
                        Err(_) => {}
                    }
                }
                self.remove(&names)?;
                Ok(None)
            }
        }
    }

    /// The marked folder's index when it is whole: a safe managed list, a `meta.json` of this
    /// schema, and every committed segment opening. `None` when it is broken; an error when
    /// tantivy could not look, which says nothing about the content.
    fn usable(
        &self,
        names: &[String],
        schema: &Schema,
    ) -> Result<Option<tantivy::Index>, IndexError> {
        if !self.managed_is_safe(names)? || !names.iter().any(|name| name == "meta.json") {
            return Ok(None);
        }
        let Some(index) = self.looked(tantivy::Index::open_in_dir(&self.folder))? else {
            return Ok(None);
        };
        if index.schema() != *schema {
            return Ok(None);
        }
        let Some(segments) = self.looked(index.searchable_segments())? else {
            return Ok(None);
        };
        for segment in &segments {
            if self.cut_short(segment)? {
                return Ok(None);
            }
        }
        for segment in &segments {
            let Some(opened) = opens(segment) else {
                return Ok(None);
            };
            if self.looked(opened)?.is_none() {
                return Ok(None);
            }
        }
        Ok(Some(index))
    }

    /// Whether a component file of a committed segment is shorter than tantivy's footer, which
    /// `Footer::extract_footer` slices without checking and so panics on. A file that is absent
    /// is left to tantivy, which names it broken; `.pos` is absent from an index without
    /// positions.
    fn cut_short(&self, segment: &tantivy::Segment) -> Result<bool, IndexError> {
        let mut components = vec![
            SegmentComponent::Postings,
            SegmentComponent::Positions,
            SegmentComponent::Terms,
            SegmentComponent::Store,
            SegmentComponent::FastFields,
            SegmentComponent::FieldNorms,
        ];
        if segment.meta().has_deletes() {
            components.push(SegmentComponent::Delete);
        }
        for component in components {
            let file = self.folder.join(segment.relative_path(component));
            match fs::metadata(&file) {
                Ok(meta) if meta.len() < FOOTER_FIXED_BYTES => return Ok(true),
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(self.failed("read", &error)),
            }
        }
        Ok(false)
    }

    /// What tantivy gave back: `None` when it says the index is broken, an error when it says
    /// only that it could not look.
    fn looked<T>(&self, result: tantivy::Result<T>) -> Result<Option<T>, IndexError> {
        match result {
            Ok(found) => Ok(Some(found)),
            Err(error) if is_broken(&error) => Ok(None),
            Err(error) => Err(self.failed("read", &error)),
        }
    }

    /// The folder's index for reading: no file is made, repaired or removed, and no writer taken.
    fn readable(&self) -> Result<Option<tantivy::Index>, IndexError> {
        match self.inspect()? {
            Folder::Absent | Folder::Unmarked { .. } => Ok(None),
            Folder::Marked(names) => self.usable(&names, &schema().0),
        }
    }

    fn remove(&self, names: &[String]) -> Result<(), IndexError> {
        for name in names {
            match fs::remove_file(self.folder.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(self.failed("clear", &error)),
            }
        }
        Ok(())
    }

    fn opened(&mut self) -> Result<&mut Open, IndexError> {
        let open = match self.open.take() {
            Some(open) => open,
            None => self.build(false)?,
        };
        Ok(self.open.insert(open))
    }

    /// A writer on the folder's index. Only with `repair`, which `clear` passes, a folder that is
    /// absent, empty or broken is made into an empty index; otherwise it is an error.
    fn build(&self, repair: bool) -> Result<Open, IndexError> {
        let (schema, fields) = schema();
        let existing = if repair {
            self.existing(&schema)?
        } else {
            Some(self.whole(&schema)?)
        };
        let index = match existing {
            Some(index) => index,
            None => {
                let settings = IndexSettings {
                    docstore_compression: Compressor::None,
                    ..IndexSettings::default()
                };
                tantivy::Index::builder()
                    .schema(schema)
                    .settings(settings)
                    .create_in_dir(&self.folder)
                    .map_err(|error| self.failed("create", &error))?
            }
        };
        let writer: IndexWriter<TantivyDocument> = index
            .writer_with_num_threads(1, HEAP)
            .map_err(|error| self.failed("write", &error))?;
        let mut policy = LogMergePolicy::default();
        policy.set_del_docs_ratio_before_merge(DELETED_SHARE_BEFORE_MERGE);
        writer.set_merge_policy(Box::new(policy));
        Ok(Open { writer, fields })
    }

    fn whole(&self, schema: &Schema) -> Result<tantivy::Index, IndexError> {
        let names = match self.inspect()? {
            Folder::Marked(names) => names,
            Folder::Absent | Folder::Unmarked { .. } => {
                return Err(self.failed("write", &"it holds no index"));
            }
        };
        self.usable(&names, schema)?
            .ok_or_else(|| self.failed("write", &"its index is broken"))
    }

    fn bytes(&self) -> Result<u64, IndexError> {
        let Folder::Marked(mut names) = self.inspect()? else {
            return Err(IndexError::Foreign(self.folder.clone()));
        };
        names.push(MARKER_NAME.to_owned());
        let mut bytes = 0;
        for name in names {
            bytes += fs::metadata(self.folder.join(name))
                .map_err(|error| self.failed("read", &error))?
                .len();
        }
        Ok(bytes)
    }
}

/// Whether tantivy's error says the index's files are broken, as against not having been able to
/// look at them. A missing file, one cut short and one with other bytes than tantivy wrote are
/// broken: tantivy gives them as `FileDoesNotExist`, `DataCorruption`, an incompatible footer, or
/// an `io::Error` of kind `NotFound`, `UnexpectedEof` or `InvalidData` (a footer cut or wrong, a
/// meta file that is not JSON). Any other I/O error says nothing about the content, and neither
/// does an error of a kind that opening an index does not give.
fn is_broken(error: &tantivy::TantivyError) -> bool {
    use tantivy::TantivyError;
    use tantivy::directory::error::OpenReadError;
    match error {
        TantivyError::DataCorruption(_) | TantivyError::IncompatibleIndex(_) => true,
        TantivyError::IoError(error) => reads_as_broken(error),
        TantivyError::OpenReadError(
            OpenReadError::FileDoesNotExist(_) | OpenReadError::IncompatibleIndex(_),
        ) => true,
        TantivyError::OpenReadError(OpenReadError::IoError { io_error, .. }) => {
            reads_as_broken(io_error)
        }
        _ => false,
    }
}

fn reads_as_broken(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidData
    )
}

/// Whether tantivy's error from opening the metadata is an I/O failure that says nothing about
/// the content (a mode, a full disk). `clear` stops on it; any other error from the metadata
/// means the index is not usable and is made again.
fn is_unreadable(error: &tantivy::TantivyError) -> bool {
    use tantivy::TantivyError;
    use tantivy::directory::error::OpenReadError;
    match error {
        TantivyError::IoError(error) => !reads_as_broken(error),
        TantivyError::OpenReadError(OpenReadError::IoError { io_error, .. }) => {
            !reads_as_broken(io_error)
        }
        TantivyError::OpenDirectoryError(_) => true,
        _ => false,
    }
}

/// A JSON array of names a tantivy index can hold, none of which reaches outside its folder.
fn is_managed_list(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let space = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r');
    let Some(inner) = text
        .trim_matches(space)
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return false;
    };
    let inner = inner.trim_matches(space);
    inner.is_empty()
        || inner.split(',').all(|item| {
            item.trim_matches(space)
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .is_some_and(|name| {
                    is_tantivy_name(name)
                        && name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
                })
        })
}

/// Opens a committed segment; `None` when tantivy panics on a component whose body is damaged
/// under a good footer, which is a broken index like a component that does not open. The
/// closure only reads: tantivy holds no lock across the open, and the caller's `Index` is
/// dropped with this call, so nothing half-held outlives it. The panic message still prints.
fn opens(segment: &tantivy::Segment) -> Option<tantivy::Result<SegmentReader>> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        SegmentReader::open(segment)
    }))
    .ok()
}

impl Index for Tantivy {
    fn position(&mut self) -> Result<Option<Position>, IndexError> {
        let Some(index) = self.readable()? else {
            return Ok(None);
        };
        let payload = index.load_metas().map(|metas| metas.payload);
        let payload = payload.map_err(|error| self.failed("read", &error))?;
        Ok(payload.as_deref().and_then(Position::decode))
    }

    fn clear(&mut self) -> Result<(), IndexError> {
        let open = match self.open.take() {
            Some(open) => open,
            None => self.build(true)?,
        };
        let cleared = self.open.insert(open).writer.delete_all_documents();
        cleared
            .map(|_| ())
            .map_err(|error| self.failed("clear", &error))
    }

    fn add(&mut self, doc: &Doc<'_>) -> Result<(), IndexError> {
        let open = self.opened()?;
        let fields = &open.fields;
        let tokens = analyze::terms(&doc.chunk.text)
            .into_iter()
            .enumerate()
            .map(|(position, text)| Token {
                offset_from: 0,
                offset_to: 0,
                position,
                text,
                position_length: 1,
            })
            .collect();
        let (part, parts) = doc.chunk.part.unwrap_or((0, 0));
        let mut document = TantivyDocument::default();
        document.add_pre_tokenized_text(
            fields.text,
            PreTokenizedString {
                text: String::new(),
                tokens,
            },
        );
        document.add_bytes(fields.entry, &doc.entry.to_bytes());
        document.add_u64(fields.generation, u64::from(doc.generation.0));
        document.add_u64(fields.first, doc.chunk.records.first);
        document.add_u64(fields.count, doc.chunk.records.count);
        document.add_u64(fields.part, u64::from(part));
        document.add_u64(fields.parts, u64::from(parts));
        document.add_i64(fields.date, millis(doc.date));
        document.add_text(fields.kind, doc.kind);
        let added = open.writer.add_document(document).map(|_| ());
        added.map_err(|error| self.failed("write", &error))
    }

    fn commit(&mut self, position: &Position) -> Result<Stats, IndexError> {
        let Some(payload) = position.encode() else {
            self.rollback();
            return Err(self.failed("commit", &"its position cannot be written down"));
        };
        let mut open = match self.open.take() {
            Some(open) => open,
            None => self.build(false)?,
        };
        let committed = (|| {
            let mut prepared = open.writer.prepare_commit()?;
            prepared.set_payload(&payload);
            prepared.commit().map(|_| ())
        })();
        if let Err(error) = committed {
            let _ = open.writer.wait_merging_threads();
            return Err(self.failed("commit", &error));
        }
        open.writer
            .wait_merging_threads()
            .map_err(|error| self.failed("commit", &error))?;
        Ok(Stats {
            bytes: self.bytes()?,
        })
    }

    fn extents(&mut self, entry: Id) -> Result<Vec<Extent>, IndexError> {
        let Some(index) = self.readable()? else {
            return Ok(Vec::new());
        };
        let read = |error: tantivy::TantivyError| self.failed("read", &error);
        let (_, fields) = schema();
        let term = Term::from_field_bytes(fields.entry, &entry.to_bytes());
        let query = TermQuery::new(term, IndexRecordOption::Basic);
        let weight = query
            .weight(EnableScoring::disabled_from_schema(&index.schema()))
            .map_err(read)?;
        let mut found = Vec::new();
        for segment in index.searchable_segments().map_err(read)? {
            let segment = SegmentReader::open(&segment).map_err(read)?;
            let mut scorer = weight.scorer(&segment, 1.0).map_err(read)?;
            let mut doc = scorer.doc();
            while doc != TERMINATED {
                if !segment.is_deleted(doc) {
                    found.push(self.row(&segment, doc)?);
                }
                doc = scorer.advance();
            }
        }
        found.sort_by_key(|row| (row.first, row.part));
        Ok(found
            .into_iter()
            .map(|row| Extent {
                generation: Generation(u32::try_from(row.generation).unwrap_or(u32::MAX)),
                records: RecordRange {
                    first: row.first,
                    count: row.count,
                },
                part: (row.parts > 0).then(|| {
                    (
                        u32::try_from(row.part).unwrap_or(u32::MAX),
                        u32::try_from(row.parts).unwrap_or(u32::MAX),
                    )
                }),
                date: row.date,
            })
            .collect())
    }

    fn delete(&mut self, entry: Id, from: u64) -> Result<(), IndexError> {
        let open = self.opened()?;
        let entries = Term::from_field_bytes(open.fields.entry, &entry.to_bytes());
        let deleted = if from <= 1 {
            open.writer.delete_term(entries);
            Ok(())
        } else {
            let entries = TermQuery::new(entries, IndexRecordOption::Basic);
            let tail = RangeQuery::new(
                Bound::Included(Term::from_field_u64(open.fields.first, from)),
                Bound::Unbounded,
            );
            let both = BooleanQuery::new(vec![
                (Occur::Must, Box::new(entries) as Box<dyn Query>),
                (Occur::Must, Box::new(tail)),
            ]);
            // delete_query is hidden from tantivy's docs: `a_delete_from_a_record_takes_the_documents_from_it_on_and_no_other_entrys` guards it across an upgrade.
            open.writer.delete_query(Box::new(both)).map(|_| ())
        };
        deleted.map_err(|error| self.failed("write", &error))
    }

    fn rollback(&mut self) {
        if let Some(open) = self.open.take() {
            let _ = open.writer.wait_merging_threads();
        }
    }
}

/// A document's terms with the number of times each occurs in it, sorted.
pub type Words = Vec<(String, u32)>;

/// A document as its columns give it back.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Row {
    pub entry: [u8; 16],
    pub generation: u64,
    pub first: u64,
    pub count: u64,
    pub part: u64,
    pub parts: u64,
    pub date: i64,
    pub kind: String,
}

impl Tantivy {
    fn committed(&self) -> Result<(tantivy::Searcher, Fields), IndexError> {
        let (schema, fields) = schema();
        let Folder::Marked(names) = self.inspect()? else {
            return Err(self.failed("read", &"it holds no index"));
        };
        if !self.managed_is_safe(&names)? {
            return Err(IndexError::Foreign(self.folder.clone()));
        }
        let index = tantivy::Index::open_in_dir(&self.folder)
            .map_err(|error| self.failed("read", &error))?;
        if index.schema() != schema {
            return Err(IndexError::Foreign(self.folder.clone()));
        }
        let reader = index
            .reader()
            .map_err(|error| self.failed("read", &error))?;
        Ok((reader.searcher(), fields))
    }

    /// A document's columns. A column whose body is damaged under a good footer opens with its
    /// segment and panics in tantivy when it is read: that is an index whose content cannot be
    /// read, `Failed`, which `bilbo reindex` repairs. The closure only reads; no lock or writer
    /// is held across it.
    fn row(
        &self,
        segment: &tantivy::SegmentReader,
        doc: tantivy::DocId,
    ) -> Result<Row, IndexError> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.decode(segment, doc)))
            .unwrap_or_else(|_| Err(self.failed("read", &"its content could not be read")))
    }

    fn decode(
        &self,
        segment: &tantivy::SegmentReader,
        doc: tantivy::DocId,
    ) -> Result<Row, IndexError> {
        let read = |error: tantivy::TantivyError| self.failed("read", &error);
        let columns = segment.fast_fields();
        let number = |name: &str| -> Result<u64, IndexError> {
            Ok(columns
                .u64(name)
                .map_err(read)?
                .first(doc)
                .unwrap_or_default())
        };
        let mut entry = Vec::new();
        if let Some(column) = columns.bytes("entry").map_err(read)?
            && let Some(ord) = column.term_ords(doc).next()
        {
            column
                .ord_to_bytes(ord, &mut entry)
                .map_err(|error| self.failed("read", &error))?;
        }
        let mut kind = String::new();
        if let Some(column) = columns.str("kind").map_err(read)?
            && let Some(ord) = column.term_ords(doc).next()
        {
            column
                .ord_to_str(ord, &mut kind)
                .map_err(|error| self.failed("read", &error))?;
        }
        Ok(Row {
            entry: <[u8; 16]>::try_from(entry.as_slice()).unwrap_or_default(),
            generation: number("generation")?,
            first: number("first")?,
            count: number("count")?,
            part: number("part")?,
            parts: number("parts")?,
            date: columns
                .i64("date")
                .map_err(read)?
                .first(doc)
                .unwrap_or_default(),
            kind,
        })
    }

    /// Every document of the last commit, sorted: what two indexes of one store have in common
    /// whatever their files are.
    pub fn documents(&self) -> Result<Vec<Row>, IndexError> {
        let (searcher, _) = self.committed()?;
        let mut rows = Vec::new();
        for segment in searcher.segment_readers() {
            for doc in segment.doc_ids_alive() {
                rows.push(self.row(segment, doc)?);
            }
        }
        rows.sort();
        Ok(rows)
    }

    /// Every live document of the last commit with its terms and their frequencies, sorted: two
    /// indexes of one store hold the same documents when these are equal, whatever their files.
    pub fn terms(&self) -> Result<Vec<(Row, Words)>, IndexError> {
        let (searcher, fields) = self.committed()?;
        let read = |error: tantivy::TantivyError| self.failed("read", &error);
        let mut found: BTreeMap<(usize, DocId), Words> = BTreeMap::new();
        for (ordinal, segment) in searcher.segment_readers().iter().enumerate() {
            let inverted = segment.inverted_index(fields.text).map_err(read)?;
            let mut stream = inverted
                .terms()
                .stream()
                .map_err(|e| self.failed("read", &e))?;
            while stream.advance() {
                let term = String::from_utf8_lossy(stream.key()).into_owned();
                let mut postings = inverted
                    .read_postings_from_terminfo(stream.value(), IndexRecordOption::WithFreqs)
                    .map_err(|e| self.failed("read", &e))?;
                let mut doc = postings.doc();
                while doc != TERMINATED {
                    if !segment.is_deleted(doc) {
                        found
                            .entry((ordinal, doc))
                            .or_default()
                            .push((term.clone(), postings.term_freq()));
                    }
                    doc = postings.advance();
                }
            }
        }
        let mut terms = Vec::new();
        for (ordinal, segment) in searcher.segment_readers().iter().enumerate() {
            for doc in segment.doc_ids_alive() {
                let mut words = found.remove(&(ordinal, doc)).unwrap_or_default();
                words.sort();
                terms.push((self.row(segment, doc)?, words));
            }
        }
        terms.sort();
        Ok(terms)
    }

    /// The `limit` best documents for the text's terms, any of them, by BM25: best first, and
    /// documents of one score in the order of `Row`.
    pub fn search(&self, text: &str, limit: usize) -> Result<Vec<(f32, Row)>, IndexError> {
        let (searcher, fields) = self.committed()?;
        let terms: Vec<Box<dyn Query>> = analyze::terms(text)
            .into_iter()
            .map(|term| {
                let term = Term::from_field_text(fields.text, &term);
                Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)) as Box<dyn Query>
            })
            .collect();
        if terms.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let mut found = searcher
            .search(&BooleanQuery::union(terms), &Everything)
            .map_err(|error| self.failed("read", &error))?;
        found.sort_by(|a, b| b.0.total_cmp(&a.0));
        if found.len() > limit {
            let cut = found[limit - 1].0;
            found.truncate(found.partition_point(|(score, _)| score.total_cmp(&cut).is_ge()));
        }
        let mut hits = Vec::new();
        for (score, address) in found {
            let segment = searcher.segment_reader(address.segment_ord);
            hits.push((score, self.row(segment, address.doc_id)?));
        }
        hits.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        hits.truncate(limit);
        Ok(hits)
    }
}

/// Every matching document with its score: the cut by score and row is made after all of them
/// are known, since tantivy's own cut breaks ties by segment and document number.
struct Everything;

struct EverythingInSegment {
    segment: SegmentOrdinal,
    found: Vec<(Score, DocId)>,
}

impl Collector for Everything {
    type Fruit = Vec<(Score, DocAddress)>;
    type Child = EverythingInSegment;

    fn for_segment(
        &self,
        segment: SegmentOrdinal,
        _reader: &SegmentReader,
    ) -> tantivy::Result<EverythingInSegment> {
        Ok(EverythingInSegment {
            segment,
            found: Vec::new(),
        })
    }

    fn requires_scoring(&self) -> bool {
        true
    }

    fn merge_fruits(
        &self,
        fruits: Vec<(SegmentOrdinal, Vec<(Score, DocId)>)>,
    ) -> tantivy::Result<Self::Fruit> {
        Ok(fruits
            .into_iter()
            .flat_map(|(segment, found)| {
                found
                    .into_iter()
                    .map(move |(score, doc)| (score, DocAddress::new(segment, doc)))
            })
            .collect())
    }
}

impl SegmentCollector for EverythingInSegment {
    type Fruit = (SegmentOrdinal, Vec<(Score, DocId)>);

    fn collect(&mut self, doc: DocId, score: Score) {
        self.found.push((score, doc));
    }

    fn harvest(self) -> Self::Fruit {
        (self.segment, self.found)
    }
}

pub fn folder(cache: &Path) -> PathBuf {
    cache.join("index")
}

#[cfg(test)]
mod tests {
    use super::{is_managed_list, is_tantivy_name};

    #[test]
    fn a_tantivy_name_is_a_meta_file_a_lock_a_temporary_file_or_a_segment_file() {
        for name in [
            "meta.json",
            ".managed.json",
            ".tantivy-writer.lock",
            ".tantivy-meta.lock",
            ".tmpAbC123",
            "7827f006c8a44482bed98f5b97d6420f.idx",
            "7827f006c8a44482bed98f5b97d6420f.fieldnorm",
            "7827f006c8a44482bed98f5b97d6420f.123.del",
        ] {
            assert!(is_tantivy_name(name), "{name}");
        }
        for name in [
            "",
            "notes.txt",
            "catalog.sqlite",
            "meta.json.bak",
            "7827f006c8a44482bed98f5b97d6420f",
            "7827f006c8a44482bed98f5b97d6420f.",
            "7827f006c8a44482bed98f5b97d6420.idx",
            "7827f006c8a44482bed98f5b97d6420g.idx",
            "7827f006c8a44482bed98f5b97d6420f.i dx",
            "4887fa5108fbc055.vectors",
            "bilbo-index",
        ] {
            assert!(!is_tantivy_name(name), "{name}");
        }
    }

    #[test]
    fn the_analyzers_longest_term_is_one_tantivy_keeps() {
        const { assert!(bilbo_search::analyze::MAX_TERM_BYTES <= tantivy::tokenizer::MAX_TOKEN_LEN) };
    }

    #[test]
    fn a_managed_list_is_an_array_of_names_inside_the_folder_and_nothing_else() {
        for good in [
            "[]",
            " [ ] ",
            r#"["meta.json"]"#,
            r#"[".managed.json", "7827f006c8a44482bed98f5b97d6420f.idx",".tmpAb1"]"#,
        ] {
            assert!(is_managed_list(good.as_bytes()), "{good}");
        }
        for bad in [
            "",
            "{}",
            "[1]",
            "not json",
            r#"["../victim"]"#,
            r#"["/abs"]"#,
            r#"["sub/x"]"#,
            r#"[".."]"#,
            r#"["."]"#,
            r#"[""]"#,
            r#"["notes.txt"]"#,
            r#"["meta.json",]"#,
            r#"[".tmp/../x"]"#,
            r#"["meta.json"] x"#,
        ] {
            assert!(!is_managed_list(bad.as_bytes()), "{bad}");
        }
    }
}
