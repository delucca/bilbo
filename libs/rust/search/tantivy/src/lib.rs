use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{self, Read, Write};
use std::ops::Bound;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use bilbo_search::index::{Doc, Extent, Index, IndexError, Stats, millis};
use bilbo_search::lookup::{Filter, Found, Lookup, Posting, Totals};
use bilbo_search::position::Position;
use bilbo_search::{analyze, chunk};
use bilbo_store::entries::{Generation, RecordRange};
use bilbo_store::id::Id;
use tantivy::DocSet;
use tantivy::TERMINATED;
use tantivy::index::SegmentComponent;
use tantivy::merge_policy::LogMergePolicy;
use tantivy::postings::Postings;
use tantivy::query::{BooleanQuery, EnableScoring, Occur, Query, RangeQuery, TermQuery};
use tantivy::schema::{
    BytesOptions, FAST, Field, FieldType, INDEXED, IndexRecordOption, STRING, Schema,
    TextFieldIndexing, TextOptions,
};
use tantivy::store::Compressor;
use tantivy::tokenizer::{PreTokenizedString, Token};
use tantivy::{DocId, IndexSettings, IndexWriter, SegmentReader, TantivyDocument, Term};

const MARKER_NAME: &str = "bilbo-index";
const MARKER: &[u8] = b"bilbo index\n";
const MANAGED: &str = ".managed.json";
const LIST_LIMIT: u64 = 64 * 1024 * 1024;

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

pub struct Options {
    /// The writer's memory, in bytes: tantivy takes 15 MB at least.
    pub memory: usize,
    /// A segment whose deleted share is over this is merged, where tantivy's default waits for all
    /// of it.
    pub merge_deleted_share: f32,
}

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
    length: Field,
    digest: Field,
}

struct Open {
    writer: IndexWriter<TantivyDocument>,
    fields: Fields,
}

/// Nothing touches the folder before the first call of a port method.
pub struct Tantivy {
    folder: PathBuf,
    options: Options,
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
        length: builder.add_u64_field("length", FAST),
        digest: builder.add_u64_field("digest", FAST),
    };
    (builder.build(), fields)
}

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
    pub fn at(folder: PathBuf, options: Options) -> Tantivy {
        Tantivy {
            folder,
            options,
            open: None,
        }
    }

    fn place(&self) -> Place<'_> {
        Place(&self.folder)
    }

    /// The writer holds the store's lock, so a folder with nothing to read is not looked at twice.
    fn extents_of(&self, entry: Id) -> Result<Vec<Extent>, IndexError> {
        let Look::Usable(opened) = self.place().readable(&|| {})? else {
            return Ok(Vec::new());
        };
        let read = |error: tantivy::TantivyError| self.failed("read", &error);
        let (_, fields) = schema();
        let term = Term::from_field_bytes(fields.entry, &entry.to_bytes());
        let query = TermQuery::new(term, IndexRecordOption::Basic);
        let weight = query
            .weight(EnableScoring::disabled_from_schema(&opened.index.schema()))
            .map_err(read)?;
        let mut found = Vec::new();
        for segment in &opened.readers {
            let mut scorer = weight.scorer(segment, 1.0).map_err(read)?;
            let mut doc = scorer.doc();
            while doc != TERMINATED {
                within(segment, doc)?;
                if !segment.is_deleted(doc) {
                    found.push(self.place().row(segment, doc)?);
                }
                doc = scorer.advance();
            }
        }
        found.sort_by_key(|row| (row.first, row.part));
        Ok(found.iter().map(Row::extent).collect())
    }

    fn failed(&self, action: &'static str, error: &dyn std::fmt::Display) -> IndexError {
        self.place().failed(action, error)
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

    /// No segment is opened and no component read: what a component holds cannot stop a rebuild. Only `clear` calls this.
    fn existing(&self, schema: &Schema) -> Result<Option<tantivy::Index>, IndexError> {
        match self.place().inspect()? {
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
                if self.place().managed_is_safe(&names)?
                    && names.iter().any(|name| name == "meta.json")
                {
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

    /// Only with `repair`, which `clear` alone passes, is a folder that is absent, empty or broken made into an empty index.
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
            .writer_with_num_threads(1, self.options.memory)
            .map_err(|error| self.failed("write", &error))?;
        let mut policy = LogMergePolicy::default();
        policy.set_del_docs_ratio_before_merge(self.options.merge_deleted_share);
        writer.set_merge_policy(Box::new(policy));
        Ok(Open { writer, fields })
    }

    fn whole(&self, schema: &Schema) -> Result<tantivy::Index, IndexError> {
        let names = match self.place().inspect()? {
            Folder::Marked(names) => names,
            Folder::Absent | Folder::Unmarked { .. } => {
                return Err(self.failed("write", &"it holds no index"));
            }
        };
        match self.place().usable(&names, schema, &|| {})? {
            Look::Usable(opened) => Ok(opened.index),
            Look::Missing | Look::Unusable => Err(self.failed("write", &"its index is broken")),
        }
    }

    fn bytes(&self) -> Result<u64, IndexError> {
        let Folder::Marked(mut names) = self.place().inspect()? else {
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

/// A folder judged for reading: nothing here creates, repairs or removes a file.
struct Place<'a>(&'a Path);

struct Opened {
    index: tantivy::Index,
    readers: Vec<SegmentReader>,
    payload: Option<String>,
}

enum Look {
    Usable(Opened),
    Missing,
    Unusable,
}

impl Place<'_> {
    fn failed(&self, action: &'static str, error: &dyn std::fmt::Display) -> IndexError {
        IndexError::Failed {
            action,
            path: self.0.to_path_buf(),
            error: error.to_string(),
        }
    }

    fn inspect(&self) -> Result<Folder, IndexError> {
        match fs::symlink_metadata(self.0) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => return Err(IndexError::Foreign(self.0.to_path_buf())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Folder::Absent),
            Err(error) => return Err(self.failed("read", &error)),
        }
        let listed = fs::read_dir(self.0).map_err(|error| self.failed("read", &error))?;
        let mut entries = Vec::new();
        for entry in listed {
            let entry = entry.map_err(|error| self.failed("read", &error))?;
            let plain = entry
                .file_type()
                .map_err(|error| self.failed("read", &error))?
                .is_file();
            let Ok(name) = entry.file_name().into_string() else {
                return Err(IndexError::Foreign(self.0.to_path_buf()));
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
            return Err(IndexError::Foreign(self.0.to_path_buf()));
        };
        let found = self.marker_bytes()?;
        if found == MARKER {
            let mut names = Vec::new();
            for (name, plain) in entries {
                if name == MARKER_NAME {
                    continue;
                }
                if !plain || !is_tantivy_name(&name) {
                    return Err(IndexError::Foreign(self.0.to_path_buf()));
                }
                names.push(name);
            }
            names.sort_unstable();
            Ok(Folder::Marked(names))
        } else if entries.len() == 1 && MARKER.starts_with(&found) {
            Ok(Folder::Unmarked { cut_marker: true })
        } else {
            Err(IndexError::Foreign(self.0.to_path_buf()))
        }
    }

    fn marker_bytes(&self) -> Result<Vec<u8>, IndexError> {
        let mut found = Vec::new();
        fs::File::open(self.0.join(MARKER_NAME))
            .and_then(|file| file.take(MARKER.len() as u64 + 1).read_to_end(&mut found))
            .map_err(|error| self.failed("read", &error))?;
        Ok(found)
    }

    /// tantivy deletes the paths this file lists, with no containment.
    fn managed_is_safe(&self, names: &[String]) -> Result<bool, IndexError> {
        if !names.iter().any(|name| name == MANAGED) {
            return Ok(true);
        }
        let mut bytes = Vec::new();
        fs::File::open(self.0.join(MANAGED))
            .and_then(|file| file.take(LIST_LIMIT + 1).read_to_end(&mut bytes))
            .map_err(|error| self.failed("read", &error))?;
        Ok(bytes.len() as u64 <= LIST_LIMIT && is_managed_list(&bytes))
    }

    /// An error when tantivy could not look, which says nothing about the content.
    fn usable(
        &self,
        names: &[String],
        schema: &Schema,
        between: &dyn Fn(),
    ) -> Result<Look, IndexError> {
        if !self.managed_is_safe(names)? || !names.iter().any(|name| name == "meta.json") {
            return Ok(Look::Unusable);
        }
        match fs::metadata(self.0.join("meta.json")) {
            Ok(meta) if meta.len() > LIST_LIMIT => return Ok(Look::Unusable),
            Ok(_) => {}
            Err(error) => return Err(self.failed("read", &error)),
        }
        let Some(index) = self.looked(tantivy::Index::open_in_dir(self.0))? else {
            return Ok(Look::Unusable);
        };
        let Some(metas) = self.looked(index.load_metas())? else {
            return Ok(Look::Unusable);
        };
        if index.schema() != *schema || metas.schema != *schema {
            return Ok(Look::Unusable);
        }
        between();
        let segments: Vec<tantivy::Segment> = metas
            .segments
            .iter()
            .map(|meta| index.segment(meta.clone()))
            .collect();
        for segment in &segments {
            if self.cut_short(segment)? {
                return Ok(Look::Unusable);
            }
        }
        let mut readers = Vec::with_capacity(segments.len());
        for segment in &segments {
            let Some(opened) = opens(segment) else {
                return Ok(Look::Unusable);
            };
            match opened {
                Ok(reader) if columns_agree(&reader, schema) => readers.push(reader),
                Ok(_) => return Ok(Look::Unusable),
                Err(error) if is_missing(&error) => return Ok(Look::Missing),
                Err(error) if is_broken(&error) => return Ok(Look::Unusable),
                Err(error) => return Err(self.failed("read", &error)),
            }
        }
        Ok(Look::Usable(Opened {
            index,
            readers,
            payload: metas.payload,
        }))
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
            let file = self.0.join(segment.relative_path(component));
            match fs::metadata(&file) {
                Ok(meta) if meta.len() < FOOTER_FIXED_BYTES => return Ok(true),
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(self.failed("read", &error)),
            }
        }
        Ok(false)
    }

    fn looked<T>(&self, result: tantivy::Result<T>) -> Result<Option<T>, IndexError> {
        match result {
            Ok(found) => Ok(Some(found)),
            Err(error) if is_broken(&error) => Ok(None),
            Err(error) => Err(self.failed("read", &error)),
        }
    }

    fn readable(&self, between: &dyn Fn()) -> Result<Look, IndexError> {
        match self.inspect()? {
            Folder::Absent | Folder::Unmarked { .. } => Ok(Look::Unusable),
            Folder::Marked(names) => self.usable(&names, &schema().0, between),
        }
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

/// Asked before `is_broken`, which a missing file also satisfies.
fn is_missing(error: &tantivy::TantivyError) -> bool {
    use tantivy::TantivyError;
    use tantivy::directory::error::OpenReadError;
    match error {
        TantivyError::IoError(error) => error.kind() == io::ErrorKind::NotFound,
        TantivyError::OpenReadError(OpenReadError::FileDoesNotExist(_)) => true,
        TantivyError::OpenReadError(OpenReadError::IoError { io_error, .. }) => {
            io_error.kind() == io::ErrorKind::NotFound
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

fn within(segment: &SegmentReader, doc: DocId) -> Result<(), IndexError> {
    if doc >= segment.max_doc() {
        return Err(IndexError::Inconsistent(
            "a posting names a document outside its segment",
        ));
    }
    Ok(())
}

fn columns_agree(reader: &SegmentReader, schema: &Schema) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let columns = reader.fast_fields();
        let max_doc = reader.max_doc();
        schema
            .fields()
            .filter(|(_, entry)| entry.is_fast())
            .all(|(_, entry)| {
                let name = entry.name();
                match entry.field_type() {
                    FieldType::U64(_) => columns
                        .u64(name)
                        .is_ok_and(|column| column.num_docs() == max_doc),
                    FieldType::I64(_) => columns
                        .i64(name)
                        .is_ok_and(|column| column.num_docs() == max_doc),
                    FieldType::Bytes(_) => columns
                        .bytes(name)
                        .is_ok_and(|column| column.is_some_and(|c| c.ords().num_docs() == max_doc)),
                    FieldType::Str(_) => columns
                        .str(name)
                        .is_ok_and(|column| column.is_some_and(|c| c.ords().num_docs() == max_doc)),
                    _ => true,
                }
            })
    }))
    .unwrap_or(false)
}

fn caught<T>(
    place: &Place<'_>,
    body: impl FnOnce() -> Result<T, IndexError>,
) -> Result<T, IndexError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(body))
        .unwrap_or_else(|_| Err(place.failed("read", &"its content could not be read")))
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
        Committed::at(&self.folder).position()
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
        let terms = analyze::terms(&doc.chunk.text);
        let length = terms.len() as u64;
        let tokens = terms
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
        document.add_u64(fields.length, length);
        document.add_u64(fields.digest, chunk::digest(&doc.chunk.text));
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
        caught(&self.place(), || self.extents_of(entry))
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

pub type Words = Vec<(String, u32)>;

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
    pub length: u64,
    pub digest: u64,
}

impl Row {
    fn extent(&self) -> Extent {
        Extent {
            generation: Generation(u32::try_from(self.generation).unwrap_or(u32::MAX)),
            records: RecordRange {
                first: self.first,
                count: self.count,
            },
            part: (self.parts > 0).then(|| {
                (
                    u32::try_from(self.part).unwrap_or(u32::MAX),
                    u32::try_from(self.parts).unwrap_or(u32::MAX),
                )
            }),
            date: self.date,
        }
    }
}

impl Place<'_> {
    fn row(
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
            length: number("length")?,
            digest: number("digest")?,
        })
    }
}

struct Loaded {
    readers: Vec<SegmentReader>,
    fields: Fields,
    offsets: Vec<u32>,
    bound: u32,
    payload: Option<String>,
    totals: OnceCell<Totals>,
}

/// Read through its segments with no searcher: `Index::reader` takes a lock and creates its file.
pub struct Committed {
    folder: PathBuf,
    between: Box<dyn Fn()>,
    loaded: RefCell<Option<Rc<Loaded>>>,
}

impl Committed {
    pub fn at(folder: impl AsRef<Path>) -> Committed {
        Committed::watched(folder.as_ref(), Box::new(|| {}))
    }

    /// `between` runs once a look, after the metadata is read and before any component of a
    /// segment is looked at.
    pub fn watched(folder: &Path, between: Box<dyn Fn()>) -> Committed {
        Committed {
            folder: folder.to_path_buf(),
            between,
            loaded: RefCell::new(None),
        }
    }

    fn place(&self) -> Place<'_> {
        Place(&self.folder)
    }

    /// Every method runs its whole body here: a tantivy panic raised while a component is read is
    /// a failed read, not an unwind into the caller.
    fn guard<T>(&self, body: impl FnOnce() -> Result<T, IndexError>) -> Result<T, IndexError> {
        let result = caught(&self.place(), body);
        if result.is_err() {
            self.loaded.borrow_mut().take();
        }
        result
    }

    fn load(&self) -> Result<Option<Rc<Loaded>>, IndexError> {
        if let Some(loaded) = self.loaded.borrow().as_ref() {
            return Ok(Some(Rc::clone(loaded)));
        }
        let place = self.place();
        let mut look = place.readable(&*self.between)?;
        if matches!(look, Look::Missing) {
            look = place.readable(&*self.between)?;
        }
        let Look::Usable(Opened {
            readers, payload, ..
        }) = look
        else {
            return Ok(None);
        };
        let mut offsets = Vec::with_capacity(readers.len());
        let mut bound = 0u32;
        for reader in &readers {
            offsets.push(bound);
            bound = bound
                .checked_add(reader.max_doc())
                .ok_or_else(|| place.failed("read", &"it holds too many documents"))?;
        }
        let loaded = Rc::new(Loaded {
            readers,
            fields: schema().1,
            offsets,
            bound,
            payload,
            totals: OnceCell::new(),
        });
        *self.loaded.borrow_mut() = Some(Rc::clone(&loaded));
        Ok(Some(loaded))
    }

    fn required(&self) -> Result<Rc<Loaded>, IndexError> {
        self.load()?
            .ok_or_else(|| self.place().failed("read", &"it holds no index"))
    }

    fn read_error(&self, error: &dyn std::fmt::Display) -> IndexError {
        self.place().failed("read", error)
    }

    /// Of the last commit, sorted.
    pub fn documents(&self) -> Result<Vec<Row>, IndexError> {
        self.guard(|| {
            let loaded = self.required()?;
            let mut rows = Vec::new();
            for segment in &loaded.readers {
                for doc in segment.doc_ids_alive() {
                    rows.push(self.place().row(segment, doc)?);
                }
            }
            rows.sort();
            Ok(rows)
        })
    }

    /// Live documents of the last commit, sorted.
    pub fn terms(&self) -> Result<Vec<(Row, Words)>, IndexError> {
        self.guard(|| {
            let loaded = self.required()?;
            let mut found: BTreeMap<(usize, DocId), Words> = BTreeMap::new();
            for (ordinal, segment) in loaded.readers.iter().enumerate() {
                let inverted = segment
                    .inverted_index(loaded.fields.text)
                    .map_err(|error| self.read_error(&error))?;
                let mut stream = inverted
                    .terms()
                    .stream()
                    .map_err(|error| self.read_error(&error))?;
                while stream.advance() {
                    let term = String::from_utf8_lossy(stream.key()).into_owned();
                    let mut postings = inverted
                        .read_postings_from_terminfo(stream.value(), IndexRecordOption::WithFreqs)
                        .map_err(|error| self.read_error(&error))?;
                    let mut doc = postings.doc();
                    while doc != TERMINATED {
                        within(segment, doc)?;
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
            for (ordinal, segment) in loaded.readers.iter().enumerate() {
                for doc in segment.doc_ids_alive() {
                    let mut words = found.remove(&(ordinal, doc)).unwrap_or_default();
                    words.sort();
                    terms.push((self.place().row(segment, doc)?, words));
                }
            }
            terms.sort();
            Ok(terms)
        })
    }

    fn live_postings(
        &self,
        loaded: &Loaded,
        field: Field,
        term: &str,
        option: IndexRecordOption,
        mut each: impl FnMut(usize, DocId, u32),
    ) -> Result<(), IndexError> {
        let term = Term::from_field_text(field, term);
        for (ordinal, segment) in loaded.readers.iter().enumerate() {
            let inverted = segment
                .inverted_index(field)
                .map_err(|error| self.read_error(&error))?;
            let Some(mut postings) = inverted
                .read_postings(&term, option)
                .map_err(|error| self.read_error(&error))?
            else {
                continue;
            };
            let mut doc = postings.doc();
            while doc != TERMINATED {
                within(segment, doc)?;
                if !segment.is_deleted(doc) {
                    let freq = if option.has_freq() {
                        postings.term_freq()
                    } else {
                        0
                    };
                    each(ordinal, doc, freq);
                }
                doc = postings.advance();
            }
        }
        Ok(())
    }

    fn totals_of(&self, loaded: &Loaded) -> Result<Totals, IndexError> {
        let mut documents = 0u64;
        let mut length = 0u64;
        for segment in &loaded.readers {
            documents += u64::from(segment.num_docs());
            let column = segment
                .fast_fields()
                .u64("length")
                .map_err(|error| self.read_error(&error))?;
            for doc in segment.doc_ids_alive() {
                length = length.saturating_add(column.first(doc).unwrap_or_default());
            }
        }
        let mut seen: HashSet<Vec<u8>> = HashSet::new();
        for segment in &loaded.readers {
            let inverted = segment
                .inverted_index(loaded.fields.entry)
                .map_err(|error| self.read_error(&error))?;
            let mut stream = inverted
                .terms()
                .stream()
                .map_err(|error| self.read_error(&error))?;
            let mut postings_held = 0u64;
            while stream.advance() {
                postings_held += u64::from(stream.value().doc_freq);
                if seen.contains(stream.key()) {
                    continue;
                }
                if segment.has_deletes() {
                    let mut postings = inverted
                        .read_postings_from_terminfo(stream.value(), IndexRecordOption::Basic)
                        .map_err(|error| self.read_error(&error))?;
                    let mut doc = postings.doc();
                    while doc != TERMINATED {
                        within(segment, doc)?;
                        if !segment.is_deleted(doc) {
                            break;
                        }
                        doc = postings.advance();
                    }
                    if doc == TERMINATED {
                        continue;
                    }
                }
                seen.insert(stream.key().to_vec());
            }
            if postings_held != u64::from(segment.max_doc()) {
                return Err(IndexError::Inconsistent(
                    "the entries' postings do not add up to the segment's documents",
                ));
            }
        }
        Ok(Totals {
            documents,
            entries: seen.len() as u64,
            length,
            bound: loaded.bound,
        })
    }
}

impl Lookup for Committed {
    fn position(&self) -> Result<Option<Position>, IndexError> {
        self.guard(|| {
            Ok(self
                .load()?
                .and_then(|loaded| loaded.payload.as_deref().and_then(Position::decode)))
        })
    }

    fn totals(&self) -> Result<Totals, IndexError> {
        self.guard(|| {
            let loaded = self.required()?;
            if let Some(totals) = loaded.totals.get() {
                return Ok(*totals);
            }
            let totals = self.totals_of(&loaded)?;
            let _ = loaded.totals.set(totals);
            Ok(totals)
        })
    }

    fn postings(&self, term: &str) -> Result<Vec<Posting>, IndexError> {
        self.guard(|| {
            let loaded = self.required()?;
            let mut found = Vec::new();
            let mut lengths = Vec::with_capacity(loaded.readers.len());
            for segment in &loaded.readers {
                lengths.push(
                    segment
                        .fast_fields()
                        .u64("length")
                        .map_err(|error| self.read_error(&error))?,
                );
            }
            self.live_postings(
                &loaded,
                loaded.fields.text,
                term,
                IndexRecordOption::WithFreqs,
                |ordinal, doc, freq| {
                    let length = lengths[ordinal].first(doc).unwrap_or_default();
                    found.push(Posting {
                        doc: loaded.offsets[ordinal] + doc,
                        freq,
                        length: u32::try_from(length).unwrap_or(u32::MAX),
                    });
                },
            )?;
            Ok(found)
        })
    }

    fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
        self.guard(|| {
            let loaded = self.required()?;
            let Filter::Kind(kind) = filter;
            let mut found = Vec::new();
            self.live_postings(
                &loaded,
                loaded.fields.kind,
                kind,
                IndexRecordOption::Basic,
                |ordinal, doc, _| {
                    found.push(loaded.offsets[ordinal] + doc);
                },
            )?;
            Ok(found)
        })
    }

    fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError> {
        self.guard(|| {
            let loaded = self.required()?;
            let mut found = Vec::with_capacity(docs.len());
            for &doc in docs {
                if doc >= loaded.bound {
                    return Err(self.read_error(&"a document number beyond the index"));
                }
                let ordinal = loaded.offsets.partition_point(|&offset| offset <= doc) - 1;
                let segment = &loaded.readers[ordinal];
                let row = self.place().row(segment, doc - loaded.offsets[ordinal])?;
                found.push(Found {
                    entry: Id::from_bytes(row.entry),
                    extent: row.extent(),
                    kind: row.kind,
                    digest: row.digest,
                });
            }
            Ok(found)
        })
    }
}

impl Tantivy {
    /// Of the last commit, sorted.
    pub fn documents(&self) -> Result<Vec<Row>, IndexError> {
        Committed::at(&self.folder).documents()
    }

    /// Live documents of the last commit, sorted.
    pub fn terms(&self) -> Result<Vec<(Row, Words)>, IndexError> {
        Committed::at(&self.folder).terms()
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
