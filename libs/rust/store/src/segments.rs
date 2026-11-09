use std::ffi::OsString;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::FileExt;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use crate::capture::{self, Action};
use crate::catalog::{self, Catalog, Fresh, Place};
use crate::entries::{
    Capture, CaptureError, Captured, Change, Cursor, Entries, Generation, Head, Kept, Meta, Origin,
    RecordRange, StoreError,
};
use crate::files::{self, Create};
use crate::frame;
use crate::id::Id;
use crate::layout::Layout;
use crate::limits::{Compression, TearBudget};
use crate::meta;

pub type Hook = Box<dyn FnMut(Step) -> io::Result<()> + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// A writer holds the exclusive lock.
    Locked,
    /// A reader validated its paths and has not touched the store yet.
    Checked,
    /// The root passed the 0.20 guard; no folder is created yet.
    Guarded,
    /// The root or cache folder is about to be created.
    Creating,
    /// The root or cache folder exists and is not checked yet.
    Created,
    /// A new entry's meta frame is synced; its body frame is next.
    MetaSynced,
    /// A frame is written and not yet synced; a hook error here is a failed write.
    FrameWritten,
    /// A capture's frames are synced; its catalog commit is next.
    Appended,
    /// The marker is synced.
    SealMarked,
    /// The `.partial` copy is synced.
    SealCopied,
    /// The rename is done; the store root is not synced yet.
    SealRenamed,
    /// The store root is synced.
    SealSynced,
    /// The catalog names the new segment.
    SealCataloged,
    /// `open.seg` is cut to its header; the marker is next.
    SealCut,
    /// Recovery validated the sealed segment and synced the store root; it cuts `open.seg` next.
    RecoverySynced,
    /// Recovery cut `open.seg`; it removes the marker next.
    RecoveryCut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Waiting,
    CutTornTail {
        path: PathBuf,
        offset: u64,
        bytes: u64,
    },
    RebuiltCatalog {
        entries: u64,
        segments: u64,
    },
}

pub struct Options {
    pub seal_at: u64,
    pub compression: Compression,
    /// The frame bytes the search after a torn frame may hash, in multiples of the file's size.
    pub tear_budget: TearBudget,
    pub hook: Option<Hook>,
}

struct Writer {
    file: File,
    len: u64,
}

pub struct Store {
    layout: Layout,
    _lock: Option<File>,
    writer: Option<Writer>,
    catalog: Catalog,
    compressor: Option<zstd::bulk::Compressor<'static>>,
    options: Options,
    poisoned: bool,
}

fn open_seg(layout: &Layout) -> PathBuf {
    layout.state.join("open.seg")
}

fn lock_file(layout: &Layout) -> PathBuf {
    layout.state.join("store.lock")
}

fn marker_file(layout: &Layout) -> PathBuf {
    layout.state.join("sealing")
}

fn root_file(layout: &Layout) -> PathBuf {
    layout.state.join("root")
}

fn catalog_file(layout: &Layout) -> PathBuf {
    layout.cache.join("catalog.sqlite")
}

fn segment_file(layout: &Layout, name: &str) -> PathBuf {
    layout.root.join(format!("{name}.seg"))
}

fn partial_file(layout: &Layout, name: &str) -> PathBuf {
    layout.root.join(format!(".{name}.seg.partial"))
}

fn hook_io(hook: &mut Option<Hook>, step: Step) -> io::Result<()> {
    match hook {
        Some(hook) => hook(step),
        None => Ok(()),
    }
}

fn step(hook: &mut Option<Hook>, step: Step) -> Result<(), StoreError> {
    hook_io(hook, step).map_err(|_| StoreError::Stopped(format!("{step:?}")))
}

#[derive(Debug, Clone, Copy)]
enum Root {
    /// Compare with the record, if there is one.
    Check,
    /// Compare, and write the record when there is none.
    Record,
}

/// The state folder keeps the segments of one store root; another root must not share it.
/// A record that is empty or lacks its newline was torn: it counts as absent.
fn check_root(layout: &Layout, asked: &Path, mode: Root) -> Result<(), StoreError> {
    let path = root_file(layout);
    let whole = files::read_optional(&path)?.filter(|text| text.len() > 1 && text.ends_with(b"\n"));
    match whole {
        Some(text) => {
            let recorded = PathBuf::from(OsString::from_vec(text[..text.len() - 1].to_vec()));
            if recorded == asked {
                Ok(())
            } else {
                Err(StoreError::OtherRoot {
                    state: layout.state.clone(),
                    recorded,
                    asked: asked.to_path_buf(),
                })
            }
        }
        None if matches!(mode, Root::Record) => {
            let mut line = asked.as_os_str().as_bytes().to_vec();
            line.push(b'\n');
            let temporary = layout.state.join("root.tmp");
            files::write_synced(&temporary, &line, Create::Replace)?;
            fs::rename(&temporary, &path)
                .map_err(|error| files::io_err("rename", &temporary, error))?;
            files::sync_dir(&layout.state)
        }
        None => Ok(()),
    }
}

fn binding(layout: &Layout) -> Result<catalog::Binding, StoreError> {
    Ok(catalog::Binding {
        root: canonical(&layout.root)?.as_os_str().as_bytes().to_vec(),
        state: canonical(&layout.state)?.as_os_str().as_bytes().to_vec(),
    })
}

/// bilbo 0.20 shares the default root; the migration task deletes this guard.
fn refuse_older(root: &Path, shown: &Path) -> Result<(), StoreError> {
    for name in ["notes", "library"] {
        let folder = root.join(name);
        match fs::symlink_metadata(&folder) {
            Ok(_) => return Err(StoreError::Older(shown.join(name))),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) => {}
            Err(error) => return Err(files::io_err("read", &folder, error)),
        }
    }
    Ok(())
}

/// The root as it is now, checked for 0.20's folders. A store uses this path from here on, never
/// the configured one: a link in the configured path could be repointed after it was checked.
/// The check is on a pathname, not a handle: each open checks the canonical root again once it
/// exists, and a root swapped for a link after that is the residual the folder guards accept.
fn guarded_root(configured: &Layout) -> Result<PathBuf, StoreError> {
    let root = files::resolve(&configured.root)?;
    refuse_older(&root, &configured.root)?;
    refuse_inside(configured, &root)?;
    Ok(root)
}

/// The root is one flat folder of segments: a state or cache folder in it, or the root itself,
/// would make the root hold more than segments. Compared lexically and with every link followed,
/// a dangling one included, so a link cannot hide the nesting.
fn refuse_inside(configured: &Layout, root: &Path) -> Result<(), StoreError> {
    let roots = [
        lexical(&configured.root),
        root.to_path_buf(),
        files::follow(&configured.root)?,
    ];
    for (folder, path) in [("state", &configured.state), ("cache", &configured.cache)] {
        let lexical = lexical(path);
        let followed = files::follow(&lexical)?;
        if roots
            .iter()
            .any(|inside| lexical.starts_with(inside) || followed.starts_with(inside))
        {
            return Err(StoreError::Inside {
                folder,
                path: path.clone(),
                root: configured.root.clone(),
            });
        }
    }
    Ok(())
}

/// `path` without `.` and without the components `..` cancels.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// A root that is gone is no store to a reader, not an I/O failure.
fn vanished(error: StoreError, configured: &Layout) -> StoreError {
    match error {
        StoreError::Io { error, .. } if error.kind() == io::ErrorKind::NotFound => {
            StoreError::Missing(format!("no store at {}", configured.root.display()))
        }
        other => other,
    }
}

/// Where `root` leads now. A reader binds and reads this path, so a link swapped in since the
/// last look is seen by the binding check.
fn current_root(root: &Path, configured: &Layout) -> Result<PathBuf, StoreError> {
    canonical(root).map_err(|error| vanished(error, configured))
}

/// Creates `dir` private if it is missing and returns it canonical, checked after the creation.
fn make_private(dir: &Path, hook: &mut Option<Hook>) -> Result<PathBuf, StoreError> {
    files::check_parents(dir)?;
    step(hook, Step::Creating)?;
    files::create_dir(dir)?;
    step(hook, Step::Created)?;
    files::check_private(dir)?;
    files::check_parents(dir)?;
    canonical(dir)
}

fn canonical(path: &Path) -> Result<PathBuf, StoreError> {
    fs::canonicalize(path).map_err(|error| files::io_err("read", path, error))
}

fn list_sealed(root: &Path) -> Result<Vec<(String, u64)>, StoreError> {
    let mut sealed = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| files::io_err("list", root, error))? {
        let entry = entry.map_err(|error| files::io_err("list", root, error))?;
        let name = entry.file_name();
        let Some(stem) = name.to_str().and_then(|name| name.strip_suffix(".seg")) else {
            continue;
        };
        if Id::parse(stem).is_none() {
            continue;
        }
        let length = entry
            .metadata()
            .map_err(|error| files::io_err("list", &entry.path(), error))?
            .len();
        sealed.push((stem.to_string(), length));
    }
    sealed.sort();
    Ok(sealed)
}

fn marker_name(text: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(text).ok()?.strip_suffix('\n')?;
    Id::parse(text).map(|_| text.to_string())
}

fn damaged(path: &Path, offset: u64, what: impl Into<String>) -> StoreError {
    StoreError::Damaged {
        path: path.to_path_buf(),
        offset,
        what: what.into(),
    }
}

fn header_fault(path: &Path, fault: frame::HeaderFault) -> StoreError {
    match fault {
        frame::HeaderFault::Bad(what) => damaged(path, 0, what),
        frame::HeaderFault::Newer(version) => StoreError::Newer {
            path: path.to_path_buf(),
            version,
        },
    }
}

fn first_difference(a: &[u8], b: &[u8]) -> u64 {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len())) as u64
}

fn apply_frame(
    batch: &mut catalog::Batch<'_>,
    segment: &str,
    path: &Path,
    offset: usize,
    frame: &frame::Frame<'_>,
) -> Result<(), StoreError> {
    let place = Place {
        segment,
        path,
        offset: offset as u64,
        length: frame.total as u64,
    };
    match frame.header.kind {
        frame::Kind::Meta => {
            let decoded = frame::unpack(frame.payload)
                .and_then(|plain| meta::decode(frame.header.entry, &plain))
                .map_err(|what| damaged(path, offset as u64, what))?;
            batch.meta(&decoded, &place)
        }
        frame::Kind::Body => {
            frame::unpack(frame.payload)
                .and_then(|plain| frame::split_records(&plain, frame.header.count).map(|_| ()))
                .map_err(|what| damaged(path, offset as u64, what))?;
            batch.body(&frame.header, &place)
        }
    }
}

fn apply_walk(
    batch: &mut catalog::Batch<'_>,
    segment: &str,
    path: &Path,
    bytes: &[u8],
    from: usize,
) -> Result<u64, StoreError> {
    let mut applied = 0;
    for item in frame::walk(bytes, from) {
        let (offset, frame) = item.map_err(|(at, fault)| damaged(path, at as u64, fault.what()))?;
        apply_frame(batch, segment, path, offset, &frame)?;
        applied += 1;
    }
    Ok(applied)
}

/// Fills an empty catalog from the sealed segments, then from the good frames of `open`.
fn populate(
    catalog: &mut Catalog,
    layout: &Layout,
    sealed: &[(String, u64)],
    open: Option<&[u8]>,
    budget: TearBudget,
) -> Result<u64, StoreError> {
    let mut batch = catalog.batch()?;
    let mut frames = 0;
    for (name, _) in sealed {
        let path = segment_file(layout, name);
        let bytes = files::read_file(&path)?;
        match frame::check_segment_header(&bytes) {
            Ok(frame::HeaderState::Whole) => {}
            Ok(frame::HeaderState::Torn) => {
                return Err(damaged(&path, 0, "the header is cut short"));
            }
            Err(fault) => return Err(header_fault(&path, fault)),
        }
        frames += apply_walk(&mut batch, name, &path, &bytes, frame::SEGMENT_HEADER)?;
        batch.set_segment(name, bytes.len() as u64)?;
    }
    if let Some(bytes) = open {
        let path = open_seg(layout);
        match frame::check_segment_header(bytes) {
            Ok(frame::HeaderState::Whole) => {
                let end = frame::good_end(bytes, budget)
                    .map_err(|damage| damaged(&path, damage.offset, damage.what))?;
                frames += apply_walk(
                    &mut batch,
                    catalog::OPEN,
                    &path,
                    &bytes[..end],
                    frame::SEGMENT_HEADER,
                )?;
                batch.set_segment(catalog::OPEN, end as u64)?;
            }
            Ok(frame::HeaderState::Torn) => batch.set_segment(catalog::OPEN, 0)?,
            Err(fault) => return Err(header_fault(&path, fault)),
        }
    }
    batch.commit()?;
    Ok(frames)
}

fn rebuild(
    layout: &Layout,
    binding: &catalog::Binding,
    sealed: &[(String, u64)],
    open: &[u8],
    budget: TearBudget,
    notice: &mut dyn FnMut(Notice),
) -> Result<Catalog, StoreError> {
    let mut catalog = Catalog::create(&catalog_file(layout), binding)?;
    let frames = populate(&mut catalog, layout, sealed, Some(open), budget)?;
    if frames > 0 {
        let (entries, segments) = catalog.counts()?;
        notice(Notice::RebuiltCatalog { entries, segments });
    }
    Ok(catalog)
}

fn load_catalog(
    layout: &Layout,
    budget: TearBudget,
    notice: &mut dyn FnMut(Notice),
) -> Result<Catalog, StoreError> {
    let binding = binding(layout)?;
    let sealed = list_sealed(&layout.root)?;
    let open = files::read_file(&open_seg(layout))?;
    match Catalog::open_current(&catalog_file(layout), &binding, &sealed, open.len() as u64) {
        Fresh::Current(catalog) => Ok(catalog),
        Fresh::CatchUp(mut catalog, from) => {
            let caught_up = (|| {
                let mut batch = catalog.batch()?;
                apply_walk(
                    &mut batch,
                    catalog::OPEN,
                    &open_seg(layout),
                    &open,
                    usize::try_from(from).unwrap_or(usize::MAX),
                )?;
                batch.set_segment(catalog::OPEN, open.len() as u64)?;
                batch.commit()
            })();
            match caught_up {
                Ok(()) => Ok(catalog),
                Err(_) => {
                    drop(catalog);
                    rebuild(layout, &binding, &sealed, &open, budget, notice)
                }
            }
        }
        Fresh::Rebuild => rebuild(layout, &binding, &sealed, &open, budget, notice),
    }
}

/// Finishes or abandons a seal that an earlier writer began.
fn recover(layout: &Layout, hook: &mut Option<Hook>) -> Result<(), StoreError> {
    let marker = marker_file(layout);
    let Some(text) = files::read_optional(&marker)? else {
        return Ok(());
    };
    let Some(name) = marker_name(&text) else {
        files::remove_if_present(&marker)?;
        return files::sync_dir(&layout.state);
    };
    let sealed_path = segment_file(layout, &name);
    let Some(sealed) = files::read_optional(&sealed_path)? else {
        files::remove_if_present(&partial_file(layout, &name))?;
        files::sync_dir(&layout.root)?;
        files::remove_if_present(&marker)?;
        return files::sync_dir(&layout.state);
    };
    let open_path = open_seg(layout);
    let open = files::read_optional(&open_path)?.unwrap_or_default();
    if open.len() > frame::SEGMENT_HEADER {
        if open != sealed {
            return Err(damaged(
                &sealed_path,
                first_difference(&open, &sealed),
                "the sealed segment differs from open.seg",
            ));
        }
    } else {
        match frame::check_segment_header(&sealed) {
            Ok(frame::HeaderState::Whole) => {}
            Ok(frame::HeaderState::Torn) => {
                return Err(damaged(&sealed_path, 0, "the header is cut short"));
            }
            Err(fault) => return Err(header_fault(&sealed_path, fault)),
        }
        if let Some(Err((at, fault))) =
            frame::walk(&sealed, frame::SEGMENT_HEADER).find(Result::is_err)
        {
            return Err(damaged(&sealed_path, at as u64, fault.what()));
        }
    }
    files::sync_dir(&layout.root)?;
    step(hook, Step::RecoverySynced)?;
    if open.len() > frame::SEGMENT_HEADER {
        cut(&open_path, frame::SEGMENT_HEADER as u64)?;
    }
    step(hook, Step::RecoveryCut)?;
    files::remove_if_present(&marker)?;
    files::sync_dir(&layout.state)
}

fn cut(path: &Path, length: u64) -> Result<(), StoreError> {
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|error| files::io_err("cut", path, error))?;
    file.set_len(length)
        .and_then(|()| file.sync_all())
        .map_err(|error| files::io_err("cut", path, error))
}

fn prepare_open_seg(
    layout: &Layout,
    budget: TearBudget,
    notice: &mut dyn FnMut(Notice),
) -> Result<Writer, StoreError> {
    let path = open_seg(layout);
    match files::read_optional(&path)? {
        None => {
            files::write_synced(&path, &frame::segment_header(), Create::New)?;
            files::sync_dir(&layout.state)?;
        }
        Some(bytes) => match frame::check_segment_header(&bytes) {
            Err(fault) => return Err(header_fault(&path, fault)),
            Ok(frame::HeaderState::Torn) => {
                files::write_synced(&path, &frame::segment_header(), Create::Replace)?;
                notice(Notice::CutTornTail {
                    path: path.clone(),
                    offset: 0,
                    bytes: bytes.len() as u64,
                });
            }
            Ok(frame::HeaderState::Whole) => {
                let end = frame::good_end(&bytes, budget)
                    .map_err(|damage| damaged(&path, damage.offset, damage.what))?;
                if end < bytes.len() {
                    cut(&path, end as u64)?;
                    notice(Notice::CutTornTail {
                        path: path.clone(),
                        offset: end as u64,
                        bytes: (bytes.len() - end) as u64,
                    });
                }
            }
        },
    }
    let file = OpenOptions::new()
        .read(true)
        .append(true)
        .open(&path)
        .map_err(|error| files::io_err("write", &path, error))?;
    let len = file
        .metadata()
        .map_err(|error| files::io_err("read", &path, error))?
        .len();
    Ok(Writer { file, len })
}

struct Pending {
    bytes: Vec<u8>,
    pieces: Vec<(Piece, usize, usize)>,
}

enum Piece {
    Meta(Meta),
    Body(frame::Header),
}

impl Store {
    /// What both opens refuse before they create or write anything, so a caller with nothing to
    /// keep still reports a root it must not use. Creates nothing.
    pub fn check(layout: &Layout) -> Result<(), StoreError> {
        guarded_root(layout).map(|_| ())
    }

    pub fn open(
        configured: &Layout,
        mut options: Options,
        notice: &mut dyn FnMut(Notice),
    ) -> Result<Store, StoreError> {
        let root = guarded_root(configured)?;
        step(&mut options.hook, Step::Guarded)?;
        let root = make_private(&root, &mut options.hook)?;
        refuse_older(&root, &configured.root)?;
        let state = files::resolve(&configured.state)?;
        files::check_parents(&state)?;
        files::create_dir(&state)?;
        files::check_private(&state)?;
        files::check_parents(&state)?;
        let mut paths = Layout {
            root,
            state: canonical(&state)?,
            cache: configured.cache.clone(),
        };
        let layout = &paths;
        let lock_path = lock_file(layout);
        let lock = files::private_file()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| files::io_err("lock", &lock_path, error))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                notice(Notice::Waiting);
                lock.lock()
                    .map_err(|error| files::io_err("lock", &lock_path, error))?;
            }
            Err(TryLockError::Error(error)) => {
                return Err(files::io_err("lock", &lock_path, error));
            }
        }
        check_root(layout, &layout.root, Root::Check)?;
        paths.cache = make_private(&files::resolve(&configured.cache)?, &mut options.hook)?;
        let layout = &paths;
        check_root(layout, &layout.root, Root::Record)?;
        step(&mut options.hook, Step::Locked)?;
        recover(layout, &mut options.hook)?;
        let writer = prepare_open_seg(layout, options.tear_budget, notice)?;
        let catalog = load_catalog(layout, options.tear_budget, notice)?;
        Ok(Store {
            layout: layout.clone(),
            _lock: Some(lock),
            writer: Some(writer),
            catalog,
            compressor: Some(new_compressor(&layout.state, options.compression)?),
            options,
            poisoned: false,
        })
    }

    /// Holds a shared lock for its lifetime; `notice` hears `Waiting` when a writer holds the store.
    /// The options' hook hears `Step::Checked` once the paths are validated.
    pub fn open_read_only(
        configured: &Layout,
        mut options: Options,
        notice: &mut dyn FnMut(Notice),
    ) -> Result<Store, StoreError> {
        let root = guarded_root(configured)?;
        match fs::metadata(&configured.root) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(StoreError::Missing(format!(
                    "no store at {}",
                    configured.root.display()
                )));
            }
            Err(error) => return Err(files::io_err("read", &configured.root, error)),
        }
        step(&mut options.hook, Step::Guarded)?;
        let mut paths = Layout {
            root,
            state: files::resolve(&configured.state)?,
            cache: configured.cache.clone(),
        };
        files::check_parents(&paths.root)?;
        files::check_parents(&paths.state)?;
        files::check_private(&paths.root).map_err(|error| vanished(error, configured))?;
        files::check_private_if_present(&paths.state)?;
        paths.root = current_root(&paths.root, configured)?;
        refuse_older(&paths.root, &configured.root)?;
        check_root(&paths, &paths.root, Root::Check)?;
        step(&mut options.hook, Step::Checked)?;
        let lock_path = lock_file(&paths);
        let lock = match File::open(&lock_path) {
            Ok(file) => {
                match file.try_lock_shared() {
                    Ok(()) => {}
                    Err(TryLockError::WouldBlock) => {
                        notice(Notice::Waiting);
                        file.lock_shared()
                            .map_err(|error| files::io_err("lock", &lock_path, error))?;
                    }
                    Err(TryLockError::Error(error)) => {
                        return Err(files::io_err("lock", &lock_path, error));
                    }
                }
                Some(file)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(files::io_err("lock", &lock_path, error)),
        };
        paths.root = current_root(&paths.root, configured)?;
        refuse_older(&paths.root, &configured.root)?;
        let layout = &paths;
        if lock.is_some() {
            check_root(layout, &layout.root, Root::Check)?;
        }
        let sealed = list_sealed(&layout.root)?;
        let mut open = if lock.is_some() {
            files::read_optional(&open_seg(layout))?
        } else {
            None
        };
        if lock.is_some() {
            let named = files::read_optional(&marker_file(layout))?
                .as_deref()
                .and_then(marker_name);
            if let Some(name) = named {
                let path = segment_file(layout, &name);
                if let (Some(segment), Some(bytes)) =
                    (files::read_optional(&path)?, open.as_deref())
                    && bytes.len() > frame::SEGMENT_HEADER
                {
                    if bytes != segment {
                        return Err(damaged(
                            &path,
                            first_difference(bytes, &segment),
                            "the sealed segment differs from open.seg",
                        ));
                    }
                    open = None;
                }
            }
        }
        let mut catalog = Catalog::in_memory()?;
        populate(
            &mut catalog,
            layout,
            &sealed,
            open.as_deref(),
            options.tear_budget,
        )?;
        Ok(Store {
            layout: layout.clone(),
            _lock: lock,
            writer: None,
            catalog,
            compressor: None,
            options,
            poisoned: false,
        })
    }

    /// Seals `open.seg` when it holds a frame, then releases the lock.
    pub fn close(mut self) -> Result<(), StoreError> {
        let holds_frames = self
            .writer
            .as_ref()
            .is_some_and(|writer| writer.len > frame::SEGMENT_HEADER as u64);
        if holds_frames && !self.poisoned {
            self.seal()?;
        }
        Ok(())
    }

    fn writable(&self) -> Result<(), StoreError> {
        if self.writer.is_none() {
            return Err(StoreError::ReadOnly);
        }
        if self.poisoned {
            return Err(StoreError::Poisoned);
        }
        Ok(())
    }

    fn open_len(&self) -> u64 {
        self.writer.as_ref().map_or(0, |writer| writer.len)
    }

    fn path_of(&self, segment: &str) -> PathBuf {
        if segment == catalog::OPEN {
            open_seg(&self.layout)
        } else {
            segment_file(&self.layout, segment)
        }
    }

    /// Writes each frame at the end of `open.seg` and syncs it, then commits them all to the
    /// catalog. The store stays poisoned until the commit, or until a rollback that left the
    /// file as it was.
    fn persist(&mut self, pending: &Pending) -> Result<(), StoreError> {
        let path = open_seg(&self.layout);
        let Some(base) = self.writer.as_ref().map(|writer| writer.len) else {
            return Err(StoreError::ReadOnly);
        };
        self.poisoned = true;
        let last = pending.pieces.len().saturating_sub(1);
        for (index, (_, offset, length)) in pending.pieces.iter().enumerate() {
            let Some(writer) = self.writer.as_mut() else {
                return Err(StoreError::ReadOnly);
            };
            let written = writer
                .file
                .write_all(&pending.bytes[*offset..*offset + *length])
                .and_then(|()| hook_io(&mut self.options.hook, Step::FrameWritten))
                .and_then(|()| writer.file.sync_all());
            if let Err(error) = written {
                let rolled_back = writer
                    .file
                    .set_len(writer.len)
                    .and_then(|()| writer.file.sync_all())
                    .and_then(|()| writer.file.metadata())
                    .is_ok_and(|meta| meta.len() == writer.len);
                if rolled_back && index == 0 {
                    self.poisoned = false;
                }
                return Err(files::io_err("write", &path, error));
            }
            writer.len += *length as u64;
            if index < last {
                step(&mut self.options.hook, Step::MetaSynced)?;
            }
        }
        let end = self.open_len();
        step(&mut self.options.hook, Step::Appended)?;
        let mut batch = self.catalog.batch()?;
        for (piece, offset, length) in &pending.pieces {
            let place = Place {
                segment: catalog::OPEN,
                path: &path,
                offset: base + *offset as u64,
                length: *length as u64,
            };
            match piece {
                Piece::Meta(meta) => batch.meta(meta, &place)?,
                Piece::Body(header) => batch.body(header, &place)?,
            }
        }
        batch.set_segment(catalog::OPEN, end)?;
        batch.commit()?;
        self.poisoned = false;
        Ok(())
    }

    fn seal_if_full(&mut self) -> Result<(), StoreError> {
        if self.open_len() >= self.options.seal_at {
            self.seal()?;
        }
        Ok(())
    }

    fn seal(&mut self) -> Result<(), StoreError> {
        self.poisoned = true;
        self.seal_steps()?;
        self.poisoned = false;
        Ok(())
    }

    fn seal_steps(&mut self) -> Result<(), StoreError> {
        let layout = self.layout.clone();
        let sealed = list_sealed(&layout.root)?;
        let floor = sealed.last().and_then(|(name, _)| Id::parse(name));
        let id = Id::mint_after(floor).map_err(StoreError::Id)?;
        let name = id.to_string();
        let open_path = open_seg(&layout);
        let length = self.open_len();

        let marker = marker_file(&layout);
        files::write_synced(&marker, format!("{name}\n").as_bytes(), Create::Replace)?;
        files::sync_dir(&layout.state)?;
        step(&mut self.options.hook, Step::SealMarked)?;

        let mut bytes = files::read_file(&open_path)?;
        bytes.truncate(usize::try_from(length).unwrap_or(usize::MAX));
        let partial = partial_file(&layout, &name);
        files::write_synced(&partial, &bytes, Create::New)?;
        step(&mut self.options.hook, Step::SealCopied)?;

        let sealed_path = segment_file(&layout, &name);
        fs::rename(&partial, &sealed_path)
            .map_err(|error| files::io_err("rename", &partial, error))?;
        step(&mut self.options.hook, Step::SealRenamed)?;

        files::sync_dir(&layout.root)?;
        step(&mut self.options.hook, Step::SealSynced)?;

        let mut batch = self.catalog.batch()?;
        batch.seal(&name, length, frame::SEGMENT_HEADER as u64)?;
        batch.commit()?;
        step(&mut self.options.hook, Step::SealCataloged)?;

        cut(&open_path, frame::SEGMENT_HEADER as u64)?;
        if let Some(writer) = self.writer.as_mut() {
            writer.len = frame::SEGMENT_HEADER as u64;
        }
        step(&mut self.options.hook, Step::SealCut)?;

        files::remove_if_present(&marker)?;
        files::sync_dir(&layout.state)
    }

    fn pack(&mut self, plain: &[u8]) -> Result<Vec<u8>, StoreError> {
        let compressor = self.compressor.as_mut().ok_or(StoreError::ReadOnly)?;
        compressor
            .compress(plain)
            .map_err(|error| files::io_err("compress", &open_seg(&self.layout), error))
    }
}

fn covered(catalog: &Catalog, found: u64, wanted: u64) -> Result<(), StoreError> {
    if found == wanted {
        Ok(())
    } else {
        Err(StoreError::Catalog {
            path: catalog.path().to_path_buf(),
            error: "its frames do not cover the records".into(),
        })
    }
}

fn new_compressor(
    near: &Path,
    level: Compression,
) -> Result<zstd::bulk::Compressor<'static>, StoreError> {
    zstd::bulk::Compressor::new(level.level())
        .map_err(|error| files::io_err("compress", near, error))
}

impl Pending {
    fn new() -> Pending {
        Pending {
            bytes: Vec::new(),
            pieces: Vec::new(),
        }
    }

    fn push(&mut self, piece: Piece, frame: &[u8]) {
        self.pieces.push((piece, self.bytes.len(), frame.len()));
        self.bytes.extend_from_slice(frame);
    }
}

impl Entries for Store {
    fn capture(&mut self, input: &Capture<'_>) -> Result<Captured, CaptureError> {
        self.writable()?;
        capture::validate(input.bytes, input.ends)
            .map_err(|bad| CaptureError::Records(bad.to_string()))?;
        let head = self.catalog.head_by_origin(input.origin)?;
        let id = match &head {
            Some(head) => head.id,
            None => match input.id {
                Some(id) => {
                    if let Some(by) = self.catalog.holder_of_id(id)? {
                        return Err(CaptureError::IdHeld { id, by });
                    }
                    id
                }
                None => {
                    let id = Id::mint().map_err(StoreError::Id)?;
                    if self.catalog.entry_exists(id)? {
                        return Err(StoreError::IdTaken(id).into());
                    }
                    id
                }
            },
        };
        let decision = capture::decide(head.as_ref(), input.bytes, input.ends);
        let (generation, first, from, prior_end, with_meta) = match decision.action {
            Action::Unchanged => return Ok(Captured::Unchanged(id)),
            Action::New { with_meta } => (1, 1, 0, 0, with_meta),
            Action::NewGeneration => {
                let next = head
                    .as_ref()
                    .and_then(|head| head.generation.0.checked_add(1))
                    .ok_or(StoreError::TooManyGenerations(id))?;
                (next, 1, 0, 0, false)
            }
            Action::Appended { from, prior_end } => {
                let head = head
                    .as_ref()
                    .ok_or_else(|| StoreError::Missing(format!("no entry {id}")))?;
                (head.generation.0, head.records + 1, from, prior_end, false)
            }
        };
        let records = &input.ends[from..];
        let count = records.len() as u64;
        let mut pending = Pending::new();
        if with_meta {
            let meta = Meta {
                id,
                kind: input.kind.to_string(),
                origin: Some(input.origin.clone()),
                captured: meta::time_of(meta::millis(SystemTime::now())),
                labels: input.labels.to_vec(),
            };
            let plain = meta::encode(&meta).ok_or(CaptureError::TooLarge)?;
            let payload = self.pack(&plain)?;
            let frame =
                frame::encode(&frame::Header::meta(id), &payload).ok_or(CaptureError::TooLarge)?;
            pending.push(Piece::Meta(meta), &frame);
        }
        let plain = frame::pack_records(input.bytes, records, prior_end);
        let payload = self.pack(&plain)?;
        let header = frame::Header::body(
            id,
            generation,
            first,
            count,
            decision.mark.end,
            decision.mark.hash,
        );
        let frame = frame::encode(&header, &payload).ok_or(CaptureError::TooLarge)?;
        pending.push(Piece::Body(header), &frame);
        self.persist(&pending)?;
        self.seal_if_full()?;
        let kept = Kept {
            id,
            generation: Generation(generation),
            records: RecordRange { first, count },
            raw_bytes: (input.bytes.len() - prior_end) as u64,
            stored_bytes: pending.bytes.len() as u64,
        };
        Ok(match decision.action {
            Action::New { .. } => Captured::New(kept),
            Action::NewGeneration => Captured::NewGeneration(kept),
            _ => Captured::Appended(kept),
        })
    }

    fn set_meta(&mut self, id: Id, meta: &Meta) -> Result<(), StoreError> {
        self.writable()?;
        if meta.id != id {
            return Err(StoreError::MetaMismatch {
                meta: meta.id,
                given: id,
            });
        }
        if !self.catalog.entry_exists(id)? {
            return Err(StoreError::Missing(format!("no entry {id}")));
        }
        if let Some(origin) = &meta.origin
            && let Some(other) = self.catalog.holder_of_origin(origin)?
            && other != id
        {
            return Err(StoreError::OriginHeld {
                origin: origin.clone(),
                by: other,
            });
        }
        let meta = Meta {
            captured: meta::time_of(meta::millis(meta.captured)),
            ..meta.clone()
        };
        let plain = meta::encode(&meta).ok_or(StoreError::MetaTooLarge)?;
        let payload = self.pack(&plain)?;
        let frame =
            frame::encode(&frame::Header::meta(id), &payload).ok_or(StoreError::MetaTooLarge)?;
        let mut pending = Pending::new();
        pending.push(Piece::Meta(meta), &frame);
        self.persist(&pending)?;
        self.seal_if_full()
    }

    fn read(
        &self,
        id: Id,
        generation: Generation,
        records: RecordRange,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        if !self.catalog.entry_exists(id)? {
            return Err(StoreError::Missing(format!("no entry {id}")));
        }
        let Some(total) = self.catalog.generation_records(id, generation.0)? else {
            return Err(StoreError::Missing(format!(
                "entry {id} has no generation {}",
                generation.0
            )));
        };
        if records.count == 0 {
            return Ok(Vec::new());
        }
        let last = records.first.checked_add(records.count - 1);
        let last = match last {
            Some(last) if records.first >= 1 && last <= total => last,
            _ => {
                return Err(StoreError::Missing(format!(
                    "entry {id} generation {} has {total} records, not {}-{}",
                    generation.0,
                    records.first,
                    records.first.saturating_add(records.count - 1)
                )));
            }
        };
        let mut out = Vec::new();
        for row in self.catalog.rows(id, generation.0, records.first, last)? {
            if row.segment != catalog::OPEN && Id::parse(&row.segment).is_none() {
                return Err(StoreError::Catalog {
                    path: self.catalog.path().to_path_buf(),
                    error: format!("a frame row names the segment {:?}", row.segment),
                });
            }
            let path = self.path_of(&row.segment);
            let file = File::open(&path).map_err(|error| files::io_err("read", &path, error))?;
            let file_length = file
                .metadata()
                .map_err(|error| files::io_err("read", &path, error))?
                .len();
            if row.offset < frame::SEGMENT_HEADER as u64 {
                return Err(damaged(
                    &path,
                    row.offset,
                    "the frame starts inside the segment header",
                ));
            }
            if row
                .offset
                .checked_add(row.length)
                .is_none_or(|end| end > file_length)
            {
                return Err(damaged(
                    &path,
                    row.offset,
                    "the frame is past the end of the file",
                ));
            }
            let mut length = [0u8; 4];
            let past_end = |error: io::Error| {
                if error.kind() == io::ErrorKind::UnexpectedEof {
                    damaged(&path, row.offset, "the frame is past the end of the file")
                } else {
                    files::io_err("read", &path, error)
                }
            };
            file.read_exact_at(&mut length, row.offset)
                .map_err(past_end)?;
            if 4 + u64::from(u32::from_le_bytes(length)) != row.length {
                return Err(damaged(
                    &path,
                    row.offset,
                    "the frame's length does not match the catalog",
                ));
            }
            let mut raw = vec![0u8; usize::try_from(row.length).unwrap_or(0)];
            file.read_exact_at(&mut raw, row.offset).map_err(past_end)?;
            let frame =
                frame::parse(&raw, 0).map_err(|fault| damaged(&path, row.offset, fault.what()))?;
            let header = frame.header;
            if header.kind != frame::Kind::Body
                || header.entry != id
                || header.generation != generation.0
                || header.first != row.first
                || header.count != row.count
            {
                return Err(damaged(
                    &path,
                    row.offset,
                    "the frame does not match the catalog",
                ));
            }
            let plain =
                frame::unpack(frame.payload).map_err(|what| damaged(&path, row.offset, what))?;
            let split = frame::split_records(&plain, header.count)
                .map_err(|what| damaged(&path, row.offset, what))?;
            for (index, record) in split.into_iter().enumerate() {
                let number = header.first + index as u64;
                if number >= records.first && number <= last {
                    out.push(record.to_vec());
                }
            }
        }
        covered(&self.catalog, out.len() as u64, records.count)?;
        Ok(out)
    }

    fn meta(&self, id: Id) -> Result<Option<Meta>, StoreError> {
        self.catalog.meta(id)
    }

    fn changes(&self, since: &Cursor) -> Result<(Vec<Change>, Cursor), StoreError> {
        let id = self.catalog.id();
        let from = if since.catalog() == id {
            since.seq()
        } else {
            0
        };
        let (changes, last) = self.catalog.changes_since(from)?;
        Ok((changes, Cursor::at(id, last)))
    }

    fn by_origin(&self, origin: &Origin) -> Result<Option<Head>, StoreError> {
        self.catalog.head_by_origin(origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_the_frames_do_not_cover_name_the_catalog_in_use() {
        let in_memory = Catalog::in_memory().expect("catalog");
        assert_eq!(covered(&in_memory, 2, 2).ok(), Some(()));
        assert_eq!(
            covered(&in_memory, 1, 2).err().map(|e| e.to_string()),
            Some("cannot use the catalog :memory:: its frames do not cover the records".into())
        );
    }

    #[test]
    fn the_compressor_packs_at_the_level_it_is_given() {
        let mut text = Vec::new();
        let mut seed = 12345u32;
        for _ in 0..40_000 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            text.extend_from_slice(format!("word{} ", (seed >> 16) % 900).as_bytes());
        }
        let level = Options::default().compression;
        let mut compressor = new_compressor(Path::new("/s"), level).expect("compressor");
        let packed = compressor.compress(&text).expect("compress");
        assert_eq!(
            packed,
            zstd::bulk::compress(&text, level.level()).expect("default")
        );
        assert_ne!(packed, zstd::bulk::compress(&text, 3).expect("level 3"));
        let three = Compression::try_from(3).expect("in range");
        let mut fast = new_compressor(Path::new("/s"), three).expect("compressor");
        assert_eq!(
            fast.compress(&text).expect("compress"),
            zstd::bulk::compress(&text, 3).expect("level 3")
        );
    }
}
