use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::SystemTime;

use bilbo_kinds::reader::{Passage, Reader};
use bilbo_kinds::tokens::Tokens;
use bilbo_store::entries::{Entries, Generation, Meta, RecordRange, StoreError};
use bilbo_store::id::Id;

use crate::chunk::{self, Chunk};
use crate::index::{Doc, Extent, Index, IndexError, millis};
use crate::work::{Error, Output};

/// How many entries a worker may be ahead of the one the index waits for.
const AHEAD: usize = 4;

/// What the index holds of an entry already, and what the reader promises.
struct Growth {
    rows: Vec<Extent>,
    meta_changed: bool,
    appends_only: bool,
}

/// What one entry gives the index, made off the thread that holds the store and the index.
struct Made {
    meta: Meta,
    generation: Generation,
    date: SystemTime,
    chunks: Vec<Chunk>,
    /// The first record from which the entry's documents in the index are replaced, in a growth.
    delete_from: Option<u64>,
}

struct Job {
    at: usize,
    meta: Meta,
    generation: Generation,
    records: Vec<Vec<u8>>,
    growth: Option<Growth>,
}

/// A made entry, or why the entry is left out.
type Outcome = Result<Made, (Id, &'static str)>;

fn make(job: Job, reader: &dyn Reader, tokens: &dyn Tokens, size: chunk::Size) -> Outcome {
    let records: Vec<&[u8]> = job.records.iter().map(Vec::as_slice).collect();
    let passages = reader.passages(&records);
    let date = passages
        .iter()
        .find_map(|passage| passage.at)
        .unwrap_or(job.meta.captured);
    let (delete_from, from) = match &job.growth {
        None => (None, 0),
        Some(growth) => {
            let (delete_from, from) = redo(growth, &passages, job.generation, date);
            (Some(delete_from), from)
        }
    };
    let Ok(chunks) = chunk::chunks(&passages[from..], tokens, size) else {
        return Err((job.meta.id, "the tokenizer could not split a text of it"));
    };
    Ok(Made {
        chunks,
        meta: job.meta,
        generation: job.generation,
        date,
        delete_from,
    })
}

/// From which record the documents of a grown entry are replaced, and the first passage to chunk
/// again: the whole entry unless the tail can be done again and give a rebuild's chunks.
fn redo(
    growth: &Growth,
    passages: &[Passage],
    generation: Generation,
    date: SystemTime,
) -> (u64, usize) {
    const WHOLE: (u64, usize) = (1, 0);
    let rows = &growth.rows;
    let Some(last) = rows.last() else {
        return WHOLE;
    };
    if !growth.appends_only
        || growth.meta_changed
        || rows.iter().any(|row| row.generation != generation)
        || rows.iter().any(|row| row.date != millis(date))
    {
        return WHOLE;
    }
    let first = match last.part {
        None => last,
        Some((back, parts)) => {
            let Some(row) = usize::try_from(back)
                .ok()
                .and_then(|back| rows.len().checked_sub(back + 1))
                .map(|at| &rows[at])
            else {
                return WHOLE;
            };
            if row.part != Some((0, parts)) {
                return WHOLE;
            }
            row
        }
    };
    let boundary = first.records.first;
    let starts: Vec<usize> = passages
        .iter()
        .enumerate()
        .filter(|(_, passage)| passage.range.first == boundary)
        .map(|(at, _)| at)
        .collect();
    let straddles = passages.iter().any(|passage| {
        passage.range.first < boundary && passage.range.first + passage.range.count > boundary
    });
    match starts[..] {
        [at] if !straddles => (boundary, at),
        _ => WHOLE,
    }
}

fn guarded(
    job: Job,
    reader: &dyn Reader,
    tokens: &dyn Tokens,
    size: chunk::Size,
) -> (usize, Outcome) {
    let (at, id) = (job.at, job.meta.id);
    // The closure borrows only shared references, and a panic leaves nothing of it in use.
    let outcome = match panic::catch_unwind(AssertUnwindSafe(|| make(job, reader, tokens, size))) {
        Ok(outcome) => outcome,
        Err(payload) => {
            // Its `Drop` is not ours to run: it could panic outside the catch.
            mem::forget(payload);
            Err((id, "its reader or the tokenizer panicked"))
        }
    };
    (at, outcome)
}

/// What a worker tells the thread that holds the index.
enum Msg {
    Done(usize, Outcome),
    /// The worker panicked outside `guarded`.
    Died,
}

struct Mourner(mpsc::Sender<Msg>);

impl Drop for Mourner {
    fn drop(&mut self) {
        if thread::panicking() {
            let _ = self.0.send(Msg::Died);
        }
    }
}

/// Takes the made entries as they arrive and adds them to the index in the order they were sent.
struct Adder<'i> {
    index: &'i mut dyn Index,
    waiting: BTreeMap<usize, Outcome>,
    next: usize,
    output: Output,
    /// The index holds documents of earlier runs: what an entry gives is added after deleting its own.
    grown: bool,
}

impl Adder<'_> {
    fn forget(&mut self, id: Id) -> Result<(), IndexError> {
        if self.grown {
            self.index.delete(id, 1)?;
        }
        Ok(())
    }

    fn take(&mut self, at: usize, outcome: Outcome) -> Result<(), IndexError> {
        self.waiting.insert(at, outcome);
        while let Some(made) = self.waiting.remove(&self.next) {
            self.next += 1;
            let made = match made {
                Ok(made) => made,
                Err((id, why)) => {
                    self.forget(id)?;
                    self.output.failed.push((id, why.to_owned()));
                    continue;
                }
            };
            if let Some(from) = made.delete_from {
                self.index.delete(made.meta.id, from)?;
            }
            for chunk in &made.chunks {
                self.index.add(&Doc {
                    entry: made.meta.id,
                    generation: made.generation,
                    kind: &made.meta.kind,
                    date: made.date,
                    chunk,
                })?;
            }
            self.output.entries += 1;
            self.output.chunks += made.chunks.len() as u64;
        }
        Ok(())
    }
}

/// Where the thread that reads the store hands its jobs to the workers.
struct Feed<'a, 'i, 'r> {
    jobs: mpsc::SyncSender<(Job, &'r dyn Reader)>,
    made: &'a mpsc::Receiver<Msg>,
    adder: &'a mut Adder<'i>,
    workers: usize,
    sent: usize,
}

impl<'r> Feed<'_, '_, 'r> {
    fn send(
        &mut self,
        meta: Meta,
        generation: Generation,
        records: Vec<Vec<u8>>,
        growth: Option<Growth>,
        reader: &'r dyn Reader,
    ) -> Result<(), Error> {
        let job = Job {
            at: self.sent,
            meta,
            generation,
            records,
            growth,
        };
        self.sent += 1;
        self.jobs.send((job, reader)).map_err(|_| Error::Workers)?;
        while self.sent - self.adder.next > self.workers * AHEAD {
            match self.made.recv() {
                Ok(Msg::Done(at, outcome)) => self.adder.take(at, outcome)?,
                Ok(Msg::Died) | Err(_) => return Err(Error::Workers),
            }
        }
        Ok(())
    }
}

/// Runs `work` for every job `produce` sends, on `workers` threads, and gives the results to
/// `adder` in the order the jobs were sent. A worker that dies is an error, never a wait.
fn pool<'r, W>(
    workers: usize,
    work: &W,
    adder: &mut Adder<'_>,
    produce: impl FnOnce(&mut Feed<'_, '_, 'r>) -> Result<(), Error>,
) -> Result<(), Error>
where
    W: Fn(Job, &dyn Reader) -> (usize, Outcome) + Sync,
{
    let (jobs, taken) = mpsc::sync_channel::<(Job, &'r dyn Reader)>(workers);
    let taken = Arc::new(Mutex::new(taken));
    let (done, made) = mpsc::channel::<Msg>();
    thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                let mourner = Mourner(done.clone());
                let taken = Arc::clone(&taken);
                scope.spawn(move || {
                    let done = &mourner.0;
                    loop {
                        let next = taken.lock().map(|taken| taken.recv());
                        let Ok(Ok((job, reader))) = next else {
                            break;
                        };
                        let (at, outcome) = work(job, reader);
                        if done.send(Msg::Done(at, outcome)).is_err() {
                            break;
                        }
                    }
                })
            })
            .collect();
        drop(done);
        drop(taken);
        let mut feed = Feed {
            jobs,
            made: &made,
            adder: &mut *adder,
            workers,
            sent: 0,
        };
        let produced = produce(&mut feed);
        let Feed { jobs, sent, .. } = feed;
        drop(jobs);
        let result = produced.and_then(|()| {
            for message in &made {
                match message {
                    Msg::Done(at, outcome) => adder.take(at, outcome)?,
                    Msg::Died => return Err(Error::Workers),
                }
            }
            if adder.next == sent {
                Ok(())
            } else {
                Err(Error::Workers)
            }
        });
        let joined: Vec<bool> = handles
            .into_iter()
            .map(|handle| handle.join().is_err())
            .collect();
        let died = joined.contains(&true);
        if died { Err(Error::Workers) } else { result }
    })
}

/// What fills an index: the entries to read, the readers, the chunker.
pub(crate) struct Fill<'a> {
    pub size: chunk::Size,
    pub workers: usize,
    pub entries: &'a dyn Entries,
    pub readers: &'a [Box<dyn Reader>],
    pub tokens: &'a dyn Tokens,
    /// `None` when the index was just emptied. Otherwise it holds the entries' documents of an
    /// earlier commit, and this has the ids whose metadata changed since.
    pub grown: Option<&'a BTreeSet<Id>>,
}

impl Fill<'_> {
    /// Adds the newest generation of every entry of `ids` in id order whatever `workers` is. In a
    /// growth an entry the store no longer has, or no reader reads, or that failed, loses its
    /// documents. Nothing is committed.
    pub(crate) fn run(&self, ids: &BTreeSet<Id>, index: &mut dyn Index) -> Result<Output, Error> {
        let (entries, readers, size, tokens) = (self.entries, self.readers, self.size, self.tokens);
        let mut adder = Adder {
            index,
            waiting: BTreeMap::new(),
            next: 0,
            output: Output::default(),
            grown: self.grown.is_some(),
        };
        pool(
            self.workers.max(1),
            &|job, reader| guarded(job, reader, tokens, size),
            &mut adder,
            |feed| {
                for &id in ids {
                    let Some(meta) = entries.meta(id)? else {
                        feed.adder.forget(id)?;
                        continue;
                    };
                    let Some(reader) = readers.iter().find(|reader| reader.kind() == meta.kind)
                    else {
                        *feed.adder.output.unread.entry(meta.kind).or_default() += 1;
                        feed.adder.forget(id)?;
                        continue;
                    };
                    let growth = match self.grown {
                        None => None,
                        Some(changed) => Some(Growth {
                            rows: feed.adder.index.extents(id)?,
                            meta_changed: changed.contains(&id),
                            appends_only: reader.appends_only(),
                        }),
                    };
                    match records_of(entries, id) {
                        Ok((generation, records)) => {
                            feed.send(meta, generation, records, growth, reader.as_ref())?;
                        }
                        Err(error) => {
                            feed.adder.forget(id)?;
                            feed.adder.output.failed.push((id, error.to_string()));
                        }
                    }
                }
                Ok(())
            },
        )?;
        let mut output = adder.output;
        output.failed.sort_by_key(|(id, _)| *id);
        Ok(output)
    }
}

fn records_of(entries: &dyn Entries, id: Id) -> Result<(Generation, Vec<Vec<u8>>), StoreError> {
    let Some((generation, count)) = entries.extent(id, None)? else {
        return Ok((Generation(0), Vec::new()));
    };
    if count == 0 {
        return Ok((generation, Vec::new()));
    }
    let records = entries.read(id, generation, RecordRange { first: 1, count })?;
    Ok((generation, records))
}

#[cfg(test)]
#[allow(clippy::needless_pass_by_value)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, SystemTime};

    use bilbo_kinds::reader::{Passage, Reader};
    use bilbo_store::entries::{Generation, Meta, RecordRange};
    use bilbo_store::id::Id;

    use super::{Adder, Job, Made, Outcome, pool};
    use crate::chunk::Chunk;
    use crate::index::{Doc, Extent, Index, IndexError, Stats};
    use crate::position::Position;
    use crate::work::Error;

    #[derive(Default)]
    struct Adds(Vec<(Id, String)>);

    impl Index for Adds {
        fn position(&mut self) -> Result<Option<Position>, IndexError> {
            Ok(None)
        }

        fn clear(&mut self) -> Result<(), IndexError> {
            Ok(())
        }

        fn add(&mut self, doc: &Doc<'_>) -> Result<(), IndexError> {
            self.0.push((doc.entry, doc.chunk.text.clone()));
            Ok(())
        }

        fn commit(&mut self, _position: &Position) -> Result<Stats, IndexError> {
            Ok(Stats { bytes: 0 })
        }

        fn extents(&mut self, _entry: Id) -> Result<Vec<Extent>, IndexError> {
            Ok(Vec::new())
        }

        fn delete(&mut self, _entry: Id, _from: u64) -> Result<(), IndexError> {
            Ok(())
        }

        fn rollback(&mut self) {}
    }

    fn id(n: u8) -> Id {
        Id::from_bytes([n; 16])
    }

    fn meta(n: u8) -> Meta {
        Meta {
            id: id(n),
            kind: "lines".to_owned(),
            origin: None,
            captured: SystemTime::UNIX_EPOCH,
            parent: None,
        }
    }

    fn made(n: u8, texts: &[&str]) -> Outcome {
        Ok(Made {
            meta: meta(n),
            generation: Generation(1),
            date: SystemTime::UNIX_EPOCH,
            delete_from: None,
            chunks: texts
                .iter()
                .map(|text| Chunk {
                    records: RecordRange { first: 1, count: 1 },
                    part: None,
                    tokens: 1,
                    text: (*text).to_owned(),
                })
                .collect(),
        })
    }

    fn adder(index: &mut Adds) -> Adder<'_> {
        Adder {
            index,
            waiting: Default::default(),
            next: 0,
            output: Default::default(),
            grown: false,
        }
    }

    #[test]
    fn results_that_arrive_out_of_order_are_added_in_the_order_they_were_sent() {
        let mut index = Adds::default();
        let mut adder = adder(&mut index);
        adder.take(1, made(1, &["b1", "b2"])).expect("a take");
        assert_eq!((adder.output.entries, adder.next), (0, 0));
        adder.take(0, made(0, &["a"])).expect("a take");
        adder.take(3, made(3, &["d"])).expect("a take");
        adder.take(2, Err((id(2), "why"))).expect("a take");
        let output = adder.output.clone();
        assert_eq!(
            index.0,
            [
                (id(0), "a".to_owned()),
                (id(1), "b1".to_owned()),
                (id(1), "b2".to_owned()),
                (id(3), "d".to_owned()),
            ]
        );
        assert_eq!((output.entries, output.chunks), (3, 4));
        assert_eq!(output.failed, [(id(2), "why".to_owned())]);
    }

    struct Nothing;

    impl Reader for Nothing {
        fn kind(&self) -> &'static str {
            "lines"
        }

        fn passages(&self, _records: &[&[u8]]) -> Vec<Passage> {
            Vec::new()
        }

        fn appends_only(&self) -> bool {
            true
        }
    }

    /// Runs `jobs` jobs through a pool whose workers do `work`, on a thread of its own so that a
    /// run that waits for ever fails by timing out.
    fn dying_pool(
        workers: usize,
        jobs: u8,
        work: fn(Job, &dyn Reader) -> (usize, Outcome),
    ) -> Result<(), Error> {
        let (sent, received) = mpsc::channel();
        thread::spawn(move || {
            let mut index = Adds::default();
            let mut adder = adder(&mut index);
            let result = pool(workers, &work, &mut adder, |feed| {
                for n in 0..jobs {
                    feed.send(meta(n), Generation(1), Vec::new(), None, &Nothing)?;
                }
                Ok(())
            });
            let _ = sent.send(result);
        });
        received
            .recv_timeout(Duration::from_secs(20))
            .expect("the pool ends")
    }

    fn first_job_kills_its_worker(job: Job, reader: &dyn Reader) -> (usize, Outcome) {
        assert!(job.at != 0, "a worker that dies");
        guarded_by_nothing(job, reader)
    }

    fn guarded_by_nothing(job: Job, _reader: &dyn Reader) -> (usize, Outcome) {
        (job.at, made(1, &[&job.meta.kind]))
    }

    fn every_job_kills_its_worker(job: Job, _reader: &dyn Reader) -> (usize, Outcome) {
        panic!("a worker that dies on job {}", job.at);
    }

    #[test]
    fn a_worker_that_dies_while_another_lives_stops_the_run_with_an_error() {
        let result = dying_pool(2, 12, first_job_kills_its_worker);
        assert!(matches!(result, Err(Error::Workers)), "{result:?}");
    }

    #[test]
    fn every_worker_dying_stops_the_run_with_an_error_and_no_panic_leaves_it() {
        let result = dying_pool(1, 12, every_job_kills_its_worker);
        assert!(matches!(result, Err(Error::Workers)), "{result:?}");
    }

    #[test]
    fn a_run_whose_workers_all_live_ends_ok() {
        let result = dying_pool(3, 40, guarded_by_nothing);
        assert!(result.is_ok(), "{result:?}");
    }

    mod redo {
        use std::time::{Duration, SystemTime};

        use bilbo_kinds::reader::{Passage, Piece};
        use bilbo_kinds::tokens::Tokens;
        use bilbo_store::entries::{Generation, RecordRange};

        use super::super::{Growth, redo};
        use crate::chunk::{self, Size};
        use crate::index::{Extent, millis};

        const SIZE: Size = Size {
            tokens: 12,
            overlap: 2,
        };

        struct Bytes;

        impl Tokens for Bytes {
            fn ends(&self, text: &str) -> Vec<usize> {
                (1..=text.len()).collect()
            }
        }

        fn one(record: u64, text: &str) -> Passage {
            Passage {
                range: RecordRange {
                    first: record,
                    count: 1,
                },
                text: text.to_owned(),
                pieces: vec![Piece {
                    start: 0,
                    label: "l:".to_owned(),
                }],
                ends: vec![text.len()],
                at: None,
                outline: None,
            }
        }

        /// A passage of `records` records of ten bytes each from record `first`.
        fn several(first: u64, records: u64) -> Passage {
            let text: String = (0..records)
                .map(|record| {
                    format!(
                        "{}{}",
                        char::from(b'a' + record as u8).to_string().repeat(9),
                        '\n'
                    )
                })
                .collect();
            Passage {
                range: RecordRange {
                    first,
                    count: records,
                },
                ends: (1..=records as usize).map(|record| record * 10).collect(),
                text,
                pieces: vec![Piece {
                    start: 0,
                    label: "l:".to_owned(),
                }],
                at: None,
                outline: None,
            }
        }

        fn rows(passages: &[Passage], date: SystemTime) -> Vec<Extent> {
            chunk::chunks(passages, &Bytes, SIZE)
                .expect("chunks")
                .into_iter()
                .map(|chunk| Extent {
                    generation: Generation(1),
                    records: chunk.records,
                    part: chunk.part,
                    date: millis(date),
                })
                .collect()
        }

        fn growth(rows: Vec<Extent>) -> Growth {
            Growth {
                rows,
                meta_changed: false,
                appends_only: true,
            }
        }

        const WHOLE: (u64, usize) = (1, 0);

        #[test]
        fn the_boundary_is_the_last_packed_chunks_first_record() {
            let old = [one(1, "aaaaa"), one(2, "bbbbb"), one(3, "ccccc")];
            let found = rows(&old, SystemTime::UNIX_EPOCH);
            assert_eq!(found.len(), 2);
            let new = [
                old[0].clone(),
                old[1].clone(),
                old[2].clone(),
                one(4, "ddddd"),
            ];
            assert_eq!(
                redo(&growth(found), &new, Generation(1), SystemTime::UNIX_EPOCH),
                (3, 2)
            );
        }

        #[test]
        fn the_boundary_is_the_first_record_of_a_last_chunk_that_packs_two_passages() {
            let old = [
                one(1, "aaaaa"),
                one(2, "bbbbb"),
                one(3, "ccccc"),
                one(4, "ddddd"),
            ];
            let found = rows(&old, SystemTime::UNIX_EPOCH);
            assert_eq!(
                found
                    .last()
                    .map(|row| (row.records.first, row.records.count)),
                Some((3, 2))
            );
            let new = [
                old[0].clone(),
                old[1].clone(),
                old[2].clone(),
                old[3].clone(),
                one(5, "eeeee"),
            ];
            assert_eq!(
                redo(&growth(found), &new, Generation(1), SystemTime::UNIX_EPOCH),
                (3, 2)
            );
        }

        #[test]
        fn the_boundary_of_a_windowed_last_passage_is_its_window_0() {
            let old = [one(1, "aaaaa"), several(2, 6)];
            let found = rows(&old, SystemTime::UNIX_EPOCH);
            let parts = found.iter().filter(|row| row.part.is_some()).count();
            assert!(parts > 3, "{parts}");
            let firsts: std::collections::BTreeSet<u64> =
                found.iter().map(|row| row.records.first).collect();
            assert!(firsts.len() > 3, "windows start in different records");
            let new = [old[0].clone(), old[1].clone(), one(8, "eeeee")];
            assert_eq!(
                redo(&growth(found), &new, Generation(1), SystemTime::UNIX_EPOCH),
                (2, 1)
            );
        }

        #[test]
        fn a_boundary_no_passage_starts_at_does_the_whole_entry_again() {
            let found = rows(&[one(1, "aaaaa"), one(3, "bbbbb")], SystemTime::UNIX_EPOCH);
            let straddling = [several(1, 3), one(4, "bbbbb")];
            assert_eq!(
                redo(
                    &growth(found.clone()),
                    &straddling,
                    Generation(1),
                    SystemTime::UNIX_EPOCH
                ),
                WHOLE
            );
            let lacking = [one(1, "aaaaa"), one(2, "bbbbb")];
            assert_eq!(
                redo(
                    &growth(found),
                    &lacking,
                    Generation(1),
                    SystemTime::UNIX_EPOCH
                ),
                WHOLE
            );
        }

        #[test]
        fn a_date_is_compared_as_the_index_holds_it() {
            let date = SystemTime::UNIX_EPOCH + Duration::new(100, 123_456_789);
            let old = [one(1, "aaaaa"), one(2, "bbbbbbbbbb")];
            let found = rows(&old, date);
            let new = [old[0].clone(), old[1].clone(), one(3, "ccccc")];
            assert_eq!(
                redo(&growth(found.clone()), &new, Generation(1), date),
                (2, 1)
            );
            let other = date + Duration::from_millis(1);
            assert_eq!(redo(&growth(found), &new, Generation(1), other), WHOLE);
        }
    }
}
