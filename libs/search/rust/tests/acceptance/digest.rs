use std::cell::RefCell;
use std::time::{Duration, SystemTime};

use bilbo_entries::reader::{Passage, Piece, Reader};
use bilbo_search::chunk::Size;
use bilbo_search::digest::Said;
use bilbo_search::digest::sessions::{Claimed, Sessions};
use bilbo_search::digest::{
    Chosen, First, List, Listed, Prompt, Request, Source, Summary, Turn, Way, Written, keys, list,
    turn,
};
use bilbo_search::index::IndexError;
use bilbo_search::lookup::{Edits, Filter, Found, Lookup, Near, Posting, Totals};
use bilbo_search::position::{Manifest, Position};
use bilbo_search::rank::{Query, Ranker};
use bilbo_search::recall::keys as ranking;
use bilbo_search::recall::{Error, Stand};
use bilbo_search::reindex;
use bilbo_search::tokenizer::Tokenizer;
use bilbo_search::work::Request as Work;
use bilbo_store::entries::{
    Capture, CaptureError, Captured, Change, Cursor, Entries, Generation, Head, Mark, Meta,
    RecordRange, StoreError,
};
use bilbo_store::id::{Id, Short};
use bilbo_store::labels::{CaptureLabel, Labels, Tag, Tagged};

use crate::fakes::{Bytes, Docs, Events, Lines, Log, Says, at, id};

const SIZE: Size = Size {
    tokens: 64,
    overlap: 8,
};

fn manifest() -> Manifest {
    Manifest::new(SIZE, "bytes/1", "lines/1")
}

fn tokenizer() -> Tokenizer {
    Tokenizer {
        tokens: Box::new(Bytes),
        id: "bytes/1",
    }
}

fn readers() -> Vec<Box<dyn Reader>> {
    vec![
        Box::new(Lines("lines")),
        Box::new(Lines("memo")),
        Box::new(Lines("jots")),
    ]
}

fn index_of(log: &Log) -> Docs {
    index_with(log, &readers())
}

fn index_with(log: &Log, readers: &[Box<dyn Reader>]) -> Docs {
    let mut index = Docs::default();
    reindex::run(
        &Work {
            chunk: SIZE,
            render: "lines/1".to_owned(),
            workers: 1,
            label_batch: 512,
            kept_unread: Vec::new(),
        },
        log,
        log,
        readers,
        &tokenizer(),
        &mut index,
    )
    .expect("a run");
    log.reads.borrow_mut().clear();
    index
}

fn list_of(kind: &str, most: usize, floor: f64) -> List<'static> {
    List {
        kind: kind.to_owned(),
        most,
        floor,
        line: None,
        old_after: None,
        lists_belonging: true,
        lists_parts: true,
    }
}

fn lists(wanted: &[(&str, usize)]) -> Vec<List<'static>> {
    wanted
        .iter()
        .map(|(kind, most)| list_of(kind, *most, 0.0))
        .collect()
}

#[derive(Default)]
struct Marks {
    seen: Vec<String>,
    refuse: bool,
}

impl Sessions for Marks {
    fn claim(&mut self, thread: &str) -> std::io::Result<Claimed> {
        if self.refuse {
            return Err(std::io::Error::other("the marks cannot be written"));
        }
        if self.seen.iter().any(|seen| seen == thread) {
            return Ok(Claimed::Seen);
        }
        self.seen.push(thread.to_owned());
        Ok(Claimed::Marked)
    }
}

fn first_of(thread: &str, text: &str) -> First {
    let prompt = Prompt::new(thread, text).expect("a prompt");
    match turn(prompt, &mut Marks::default()) {
        Turn::First(first) => first,
        other => panic!("a first prompt, got {other:?}"),
    }
}

const WIDTH: usize = 100;

struct Unasked;

impl Labels for Unasked {
    fn with_words(&self, _words: &[String]) -> Result<Vec<Id>, StoreError> {
        Err(refused())
    }

    fn belonging_to(&self, _entries: &[Id]) -> Result<Vec<Id>, StoreError> {
        Err(refused())
    }

    fn tagged(&self, _tag: &Tag) -> Result<Vec<Id>, StoreError> {
        Err(refused())
    }

    fn tags(&self) -> Result<Vec<Tagged>, StoreError> {
        Err(refused())
    }
}

const NOW: u64 = 1_700_000_000;

struct Ask<'a> {
    readers: Vec<Box<dyn Reader>>,
    width: usize,
    labels: &'a dyn Labels,
    at: SystemTime,
    thread: &'a str,
    way: Way<'a>,
}

impl Ask<'static> {
    fn plain() -> Ask<'static> {
        Ask {
            readers: readers(),
            width: WIDTH,
            labels: &Unasked,
            at: at(NOW),
            thread: "a-thread",
            way: Way {
                label: &kept_label,
                written: &|_| None,
                summary: None,
            },
        }
    }
}

fn ask_lists(
    entries: &dyn Entries,
    index: &dyn Lookup,
    wanted: &[List],
    text: &str,
    digest: &keys::Section,
) -> Result<Listed, bilbo_search::recall::Error> {
    ask_read(&Ask::plain(), entries, index, wanted, text, digest)
}

fn ask_read(
    setup: &Ask<'_>,
    entries: &dyn Entries,
    index: &dyn Lookup,
    wanted: &[List],
    text: &str,
    digest: &keys::Section,
) -> Result<Listed, bilbo_search::recall::Error> {
    list(
        &Request {
            first: &first_of(setup.thread, text),
            lists: wanted,
            manifest: &manifest(),
            digest,
            ranker: &ranking::defaults(),
            readers: &setup.readers,
            width: setup.width,
            labels: setup.labels,
            at: setup.at,
            way: setup.way,
        },
        index,
        entries,
        &Says(None),
    )
}

fn ask_by(
    entries: &dyn Entries,
    index: &dyn Lookup,
    wanted: &[(&str, usize)],
    text: &str,
    digest: &keys::Section,
) -> Result<Listed, bilbo_search::recall::Error> {
    ask_lists(entries, index, &lists(wanted), text, digest)
}

fn ask_over(log: &Log, index: &dyn Lookup, most: usize, floor: f64, text: &str) -> Chosen {
    let wanted = vec![list_of("lines", most, floor)];
    let listed = ask_lists(log, index, &wanted, text, &keys::defaults()).expect("a listing");
    listed.lists.into_iter().next().expect("a list")
}

fn over(score: f32) -> f64 {
    f64::from_bits(f64::from(score).to_bits() + 1)
}

fn ask(entries: &dyn Entries, index: &dyn Lookup, wanted: &[(&str, usize)], text: &str) -> Listed {
    ask_by(entries, index, wanted, text, &keys::defaults()).expect("a listing")
}

fn ids(chosen: &Chosen) -> Vec<Id> {
    chosen.rows.iter().map(|row| row.entry).collect()
}

fn plain(index: &dyn Lookup, text: &str) -> Vec<(Id, u32)> {
    let mut seen = Vec::new();
    let mut best = Vec::new();
    for (entry, _, _, bits) in plain_chunks(index, text) {
        if !seen.contains(&entry) {
            seen.push(entry);
            best.push((entry, bits));
        }
    }
    best
}

#[test]
fn a_note_is_listed_once_at_its_best_chunks_score() {
    let mut log = Log::new();
    log.put(
        1,
        "lines",
        &[
            "garden hose reel stored in the shed all winter long",
            "garden garden hose hose reel reel in the old shed here",
            "the garden hose is long and green and kept in the shed",
        ],
    );
    log.put(2, "lines", &["a garden path made of stones and bricks"]);
    let index = index_of(&log);
    assert_eq!(index.rows.len(), 4);
    let listed = ask(&log, &index, &[("lines", 6)], "garden hose");
    let rows = &listed.lists[0].rows;
    assert_eq!(ids(&listed.lists[0]), [id(1), id(2)]);
    let chunks = plain(&index, "garden hose");
    assert_eq!(rows[0].score.to_bits(), chunks[0].1);
    let best = index
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.entry == id(1))
        .count();
    assert_eq!(best, 3);
}

#[test]
fn notes_come_best_first_and_equal_scores_by_id() {
    let mut log = Log::new();
    log.put(5, "lines", &["garden shed"]);
    log.put(3, "lines", &["garden shed"]);
    log.put(4, "lines", &["garden shed garden shed"]);
    log.put(6, "lines", &["shed"]);
    let mut index = index_of(&log);
    index.rows.reverse();
    let listed = ask(&log, &index, &[("lines", 6)], "garden shed");
    let chosen = &listed.lists[0];
    let first = chosen.rows[0].score;
    assert!(first >= chosen.rows[1].score);
    assert_eq!(
        chosen.rows[1].score.to_bits(),
        chosen.rows[2].score.to_bits()
    );
    assert!(chosen.rows[2].score > chosen.rows[3].score);
    assert_eq!(ids(chosen), [id(4), id(3), id(5), id(6)]);
}

#[test]
fn at_most_a_lists_count_of_entries_are_taken() {
    let mut log = Log::new();
    for n in 1..=8 {
        log.put(n, "lines", &["garden hose reel"]);
    }
    let mut index = index_of(&log);
    index.rows.reverse();
    let few = ask(&log, &index, &[("lines", 3)], "garden hose");
    assert_eq!(ids(&few.lists[0]), [id(1), id(2), id(3)]);
    let all = ask(&log, &index, &[("lines", 8)], "garden hose");
    assert_eq!(ids(&all.lists[0]), (1..=8).map(id).collect::<Vec<_>>());
}

struct Shelves {
    notes: Log,
    both: Log,
}

fn shelves() -> Shelves {
    let notes = [
        (1, "carrot soup recipe with fresh herbs"),
        (2, "carrot cake recipe for the party"),
        (3, "carrot seeds are kept in the shed"),
        (4, "carat weight of the ring"),
        (5, "mortal remains of a snail"),
        (6, "mortal coil of a hose"),
        (7, "mortal combat of two beetles"),
        (8, "mortor and pestle in the kitchen"),
        (9, "the shed key is under the pot"),
        (10, "a pot for the garden shed"),
        (11, "garden hose near the shed"),
    ];
    let others = [
        (21, "carat weight of the jewel"),
        (22, "carat weight of the jewel"),
        (23, "carat weight of the jewel"),
        (24, "carat weight of the jewel"),
        (25, "carat weight of the jewel"),
        (26, "mortar mix for the wall"),
        (27, "mortor shell of the cannon"),
        (28, "mortor shell of the cannon"),
        (29, "mortor shell of the cannon"),
        (30, "mortor shell of the cannon"),
        (31, "mortor shell of the cannon"),
        (32, "recipe for a shed and a garden"),
        (33, "recipe for a shed and a garden hose"),
    ];
    let mut alone = Log::new();
    let mut both = Log::new();
    for (n, text) in notes {
        alone.put(n, "lines", &[text]);
        both.put(n, "lines", &[text]);
    }
    for (n, text) in others {
        both.put(n, "memo", &[text]);
    }
    Shelves { notes: alone, both }
}

const QUERIES: [&str; 5] = ["recipe", "jewel", "carot", "garden shed", "mortar"];

#[test]
fn the_list_scores_what_the_plain_ranker_scores_over_the_same_documents_alone() {
    let shelves = shelves();
    let alone = index_of(&shelves.notes);
    let both = index_of(&shelves.both);
    let mut named = Vec::new();
    for query in QUERIES {
        let wanted = plain(&alone, query);
        let listed = ask(&shelves.both, &both, &[("lines", 100)], query);
        assert_eq!(
            scores_of(
                listed.lists[0]
                    .rows
                    .iter()
                    .map(|row| (row.entry, row.score))
            ),
            wanted,
            "{query}"
        );
        named.push((
            query,
            wanted.iter().map(|(entry, _)| *entry).collect::<Vec<_>>(),
        ));
    }
    assert_eq!(
        named,
        [
            ("recipe", vec![id(2), id(1)]),
            ("jewel", vec![]),
            ("carot", vec![id(2), id(3), id(1), id(4)]),
            ("garden shed", vec![id(10), id(11), id(3), id(9)]),
            ("mortar", vec![id(5), id(6), id(7), id(8)]),
        ]
    );
    let by_pair = plain(&alone, "garden shed");
    assert_eq!(by_pair[0].0, id(10), "the two words in a row come first");
}

#[test]
fn a_list_holds_its_own_kind_and_no_other() {
    let mut log = Log::new();
    log.put(1, "lines", &["garden hose reel"]);
    log.put(2, "memo", &["garden hose garden hose garden hose"]);
    let index = index_of(&log);
    let only = ask(&log, &index, &[("lines", 6)], "garden hose");
    assert_eq!(ids(&only.lists[0]), [id(1)]);
    let both = ask(&log, &index, &[("lines", 6), ("memo", 6)], "garden hose");
    assert_eq!(ids(&both.lists[0]), [id(1)]);
    assert_eq!(ids(&both.lists[1]), [id(2)]);
    assert_eq!(both.lists[1].kind, "memo");
}

#[test]
fn a_note_the_store_no_longer_holds_is_passed_over() {
    let build = || {
        let mut log = Log::new();
        for n in 1..=3 {
            log.put(n, "lines", &["garden hose reel"]);
        }
        let index = index_of(&log);
        (log, index)
    };
    let (mut log, index) = build();
    log.rewrite(1, &[]);
    let listed = ask(&log, &index, &[("lines", 2)], "garden hose");
    assert_eq!(ids(&listed.lists[0]), [id(2), id(3)]);
    assert_eq!(listed.lists[0].gone, 1);

    let (mut log, index) = build();
    log.remove(1);
    let listed = ask(&log, &index, &[("lines", 2)], "garden hose");
    assert_eq!(ids(&listed.lists[0]), [id(2), id(3)]);
    assert_eq!(listed.lists[0].gone, 1);

    let (log, index) = build();
    let nameless = Nameless(&log, id(1), RefCell::new(Vec::new()));
    let listed = ask(&nameless, &index, &[("lines", 2)], "garden hose");
    assert_eq!(ids(&listed.lists[0]), [id(2), id(3)]);
    assert_eq!(listed.lists[0].gone, 1);
    assert!(
        !nameless.2.borrow().contains(&id(1)),
        "the newest extent of an entry with no meta is not asked"
    );

    let (log, index) = build();
    let listed = ask(&log, &index, &[("lines", 2)], "garden hose");
    assert_eq!(ids(&listed.lists[0]), [id(1), id(2)]);
    assert_eq!(listed.lists[0].gone, 0);

    let mut log = Log::new();
    log.put(
        1,
        "memo",
        &[
            "garden hose garden hose reel stored away",
            "garden hose garden hose coil kept aside",
        ],
    );
    for n in 2..=3 {
        log.put(n, "memo", &["garden hose reel with a few more words in it"]);
    }
    let index = index_with(&log, &speakers());
    assert_eq!(
        index.rows.iter().filter(|row| row.entry == id(1)).count(),
        2
    );
    log.rewrite(1, &[]);
    let found = asked(
        &log,
        &index,
        &[list_of("memo", 2, 0.0)],
        WIDTH,
        "garden hose",
    );
    assert_eq!(ids(&found[0]), [id(2), id(3)]);
    assert_eq!(found[0].gone, 1);
    assert!(!log.reads.borrow().contains(&id(1)));
}

struct Nameless<'a>(&'a Log, Id, RefCell<Vec<Id>>);

impl Entries for Nameless<'_> {
    fn capture(&mut self, input: &Capture<'_>) -> Result<Captured, CaptureError> {
        Err(CaptureError::Store(StoreError::Missing(format!(
            "not called with {} bytes",
            input.bytes.len()
        ))))
    }

    fn set_meta(&mut self, _id: Id, _meta: &Meta) -> Result<u64, StoreError> {
        Err(StoreError::Missing("not called".to_owned()))
    }

    fn read(
        &self,
        id: Id,
        generation: Generation,
        records: RecordRange,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        self.0.read(id, generation, records)
    }

    fn meta(&self, id: Id) -> Result<Option<Meta>, StoreError> {
        if id == self.1 {
            return Ok(None);
        }
        self.0.meta(id)
    }

    fn changes(&self, since: &Cursor) -> Result<(Vec<Change>, Cursor), StoreError> {
        self.0.changes(since)
    }

    fn by_capture(&self, label: &CaptureLabel) -> Result<Option<Head>, StoreError> {
        self.0.by_capture(label)
    }

    fn extent(
        &self,
        id: Id,
        generation: Option<Generation>,
    ) -> Result<Option<(Generation, u64)>, StoreError> {
        self.2.borrow_mut().push(id);
        self.0.extent(id, generation)
    }

    fn captures_with(&self, part: &str) -> Result<Vec<(Id, CaptureLabel)>, StoreError> {
        self.0.captures_with(part)
    }

    fn by_shorts(&self, shorts: &[Short]) -> Result<Vec<Id>, StoreError> {
        self.0.by_shorts(shorts)
    }
}

#[test]
fn a_prompt_longer_than_digest_query_is_searched_by_its_start() {
    let mut log = Log::new();
    log.put(1, "lines", &["garden tools"]);
    log.put(2, "lines", &["hose reel"]);
    let index = index_of(&log);
    let text = "garden \u{e7}a xxxx hose";
    let mut digest = keys::defaults();
    digest.query.value = 13;
    let cut = ask_by(&log, &index, &[("lines", 6)], text, &digest).expect("a listing");
    assert_eq!(ids(&cut.lists[0]), [id(1)]);
    digest.query.value = 8;
    let cut = ask_by(&log, &index, &[("lines", 6)], text, &digest).expect("a listing");
    assert_eq!(ids(&cut.lists[0]), [id(1)]);
    let whole = ask(&log, &index, &[("lines", 6)], text);
    assert_eq!(ids(&whole.lists[0]), [id(1), id(2)]);

    let mut log = Log::new();
    log.put(1, "lines", &["cafe au lait"]);
    log.put(2, "lines", &["hose reel"]);
    let index = index_of(&log);
    let mut digest = keys::defaults();
    digest.query.value = 4;
    let cut = ask_by(
        &log,
        &index,
        &[("lines", 6)],
        "caf\u{e9}teria hose",
        &digest,
    )
    .expect("a listing");
    assert_eq!(ids(&cut.lists[0]), [id(1)]);
}

#[test]
fn a_prompt_with_no_word_lists_nothing() {
    let mut log = Log::new();
    log.put(1, "lines", &["garden tools"]);
    log.put(2, "lines", &["anything at all about tools"]);
    let index = index_of(&log);
    log.put(3, "lines", &["a note written after the index"]);
    for text in ["   \n\t ", "!!! ???", "what did we should how anything"] {
        let listed = ask(&log, &index, &[("lines", 6)], text);
        assert_eq!(listed.stand, Stand::Behind(1), "{text:?}");
        assert_eq!(listed.lists.len(), 1);
        assert!(listed.lists[0].rows.is_empty(), "{text:?}");
    }
}

#[test]
fn no_index_and_another_manifest_list_nothing_and_say_which() {
    let mut log = Log::new();
    log.put(1, "lines", &["garden tools"]);
    let none = Docs::default();
    let spy = Spy::over(&none);
    let listed = ask(&log, &spy, &[("lines", 6)], "garden");
    assert_eq!(listed.stand, Stand::NoIndex);
    assert!(listed.lists.is_empty());
    assert_eq!(*spy.calls.borrow(), ["position"]);

    let mut index = index_of(&log);
    let other = Manifest::new(SIZE, "bytes/2", "lines/1");
    index.position = index.position.map(|position| Position {
        manifest: other,
        ..position
    });
    let spy = Spy::over(&index);
    let listed = ask(&log, &spy, &[("lines", 6)], "garden");
    assert_eq!(listed.stand, Stand::OtherManifest);
    assert!(listed.lists.is_empty());
    assert_eq!(*spy.calls.borrow(), ["position"]);
}

#[test]
fn the_notes_are_those_of_the_best_digest_candidates_chunks() {
    let mut log = Log::new();
    log.put(
        1,
        "lines",
        &[
            "garden hose garden hose garden hose reel here",
            "garden hose garden hose garden hose coil here",
        ],
    );
    log.put(2, "lines", &["a garden hose and a long wooden shed"]);
    let index = index_of(&log);
    assert_eq!(index.rows.len(), 3);
    let mut digest = keys::defaults();
    digest.candidates.value = 2;
    let few = ask_by(&log, &index, &[("lines", 6)], "garden hose", &digest).expect("a listing");
    assert_eq!(ids(&few.lists[0]), [id(1)]);
    digest.candidates.value = 3;
    let more = ask_by(&log, &index, &[("lines", 6)], "garden hose", &digest).expect("a listing");
    assert_eq!(ids(&more.lists[0]), [id(1), id(2)]);
}

#[test]
fn a_short_id_another_entry_shares_is_marked() {
    let mut first = [0u8; 16];
    first[0] = 1;
    first[15] = 5;
    let mut second = first;
    second[0] = 2;
    let mut log = Log::new();
    log.put_id(Id::from_bytes(first), "lines", &["garden hose one"]);
    log.put_id(Id::from_bytes(second), "lines", &["garden hose two"]);
    log.put(6, "lines", &["garden hose three"]);
    let index = index_of(&log);
    let listed = ask(&log, &index, &[("lines", 6)], "garden hose");
    assert_eq!(listed.lists[0].rows.len(), 3);
    for row in &listed.lists[0].rows {
        assert_eq!(row.shared_short, row.entry != id(6), "{}", row.entry);
    }

    let mut log = Log::new();
    log.put_id(Id::from_bytes(first), "lines", &["garden hose one"]);
    log.put_id(Id::from_bytes(second), "memo", &["garden hose two"]);
    log.put(6, "lines", &["garden hose three"]);
    let index = index_of(&log);
    let listed = ask(&log, &index, &[("lines", 6)], "garden hose");
    assert_eq!(listed.lists[0].rows.len(), 2);
    for row in &listed.lists[0].rows {
        assert_eq!(row.shared_short, row.entry != id(6), "{}", row.entry);
    }
}

struct Spy<'a> {
    inner: &'a Docs,
    kinds: RefCell<Vec<String>>,
    calls: RefCell<Vec<&'static str>>,
    shared: Option<Events>,
}

impl<'a> Spy<'a> {
    fn over(inner: &'a Docs) -> Spy<'a> {
        Spy {
            inner,
            kinds: RefCell::new(Vec::new()),
            calls: RefCell::new(Vec::new()),
            shared: None,
        }
    }

    fn called(&self, call: &'static str) {
        self.calls.borrow_mut().push(call);
        if let Some(events) = &self.shared {
            events.borrow_mut().push(call);
        }
    }
}

impl Lookup for Spy<'_> {
    fn position(&self) -> Result<Option<Position>, IndexError> {
        self.called("position");
        self.inner.position()
    }

    fn totals(&self) -> Result<Totals, IndexError> {
        self.called("totals");
        self.inner.totals()
    }

    fn postings(&self, term: &str) -> Result<Vec<Posting>, IndexError> {
        self.called("postings");
        self.inner.postings(term)
    }

    fn near(&self, term: &str, edits: Edits) -> Result<Vec<Near>, IndexError> {
        self.called("near");
        self.inner.near(term, edits)
    }

    fn pairs(&self, first: &str, second: &str) -> Result<Vec<Posting>, IndexError> {
        self.called("pairs");
        self.inner.pairs(first, second)
    }

    fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
        self.called("holding");
        if let Filter::Kind(kind) = filter {
            self.kinds.borrow_mut().push((*kind).to_owned());
        }
        self.inner.holding(filter)
    }

    fn length(&self, kind: &str) -> Result<u64, IndexError> {
        self.called("length");
        self.kinds.borrow_mut().push(kind.to_owned());
        self.inner.length(kind)
    }

    fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError> {
        self.called("found");
        self.inner.found(docs)
    }
}

#[test]
fn the_lists_come_in_the_order_handed_and_a_count_of_0_ranks_nothing() {
    let mut log = Log::new();
    log.put(1, "jots", &["garden hose reel"]);
    log.put(2, "memo", &["garden hose reel"]);
    log.put(3, "lines", &["garden hose reel"]);
    log.put(4, "lines", &["garden hose garden hose"]);
    log.put(5, "lines", &["garden hose reel for a shed"]);
    log.put(7, "jots", &["garden hose reel"]);
    log.put(8, "jots", &["garden hose reel"]);
    for n in 9..=11 {
        log.put(n, "lines", &["pot ring lid"]);
    }
    let index = index_of(&log);
    let spy = Spy::over(&index);
    let listed = ask(
        &log,
        &spy,
        &[("jots", 2), ("memo", 0), ("lines", 2)],
        "garden hose",
    );
    let kinds: Vec<&str> = listed
        .lists
        .iter()
        .map(|chosen| chosen.kind.as_str())
        .collect();
    assert_eq!(kinds, ["jots", "lines"]);
    assert_eq!(ids(&listed.lists[0]), [id(1), id(7)]);
    assert_eq!(ids(&listed.lists[1]), [id(4), id(3)]);
    assert!(listed.lists[1].rows[0].score > listed.lists[0].rows[0].score);
    assert!(!spy.kinds.borrow().iter().any(|kind| kind == "memo"));

    log.put(12, "lines", &["garden"]);
    let spy = Spy::over(&index);
    let off = ask(
        &log,
        &spy,
        &[("jots", 0), ("memo", 0), ("lines", 0)],
        "garden hose",
    );
    assert_eq!(off.stand, Stand::Behind(1));
    assert!(off.lists.is_empty());
    assert_eq!(*spy.calls.borrow(), ["position"]);
}

fn pool(kind: usize, n: usize) -> String {
    let words = [
        "garden", "shed", "hose", "carrot", "recipe", "mortal", "pot", "ring",
    ];
    let first = words[(n + kind) % words.len()];
    let second = words[(n * 3 + kind * 2 + 1) % words.len()];
    let third = words[(n * 5 + kind + 2) % words.len()];
    let odd = match kind {
        0 => "carat",
        1 => "mortor",
        _ => "gadget",
    };
    format!("{first} {second} {third} {odd}{}", n % 2)
}

#[test]
fn each_list_scores_what_the_plain_ranker_scores_over_its_own_kind_alone() {
    let kinds = ["lines", "memo", "jots"];
    let mut listed_rows = 0;
    for sizes in [[12, 4, 3], [3, 12, 4], [4, 3, 12]] {
        let mut whole = Log::new();
        let mut separate = Vec::new();
        for (at, kind) in kinds.iter().enumerate() {
            let mut own = Log::new();
            for n in 0..sizes[at] {
                let entry = u8::try_from(at * 40 + n + 1).expect("a small id");
                let text = pool(at, n);
                own.put(entry, kind, &[&text]);
                whole.put(entry, kind, &[&text]);
            }
            separate.push(index_of(&own));
        }
        let index = index_of(&whole);
        for query in [
            "garden shed",
            "carot hose",
            "mortor recipe",
            "gadjet pot",
            "garden carat",
        ] {
            let listed = ask(
                &whole,
                &index,
                &[("lines", 100), ("memo", 100), ("jots", 100)],
                query,
            );
            let named: Vec<&str> = listed.lists.iter().map(|c| c.kind.as_str()).collect();
            assert_eq!(named, kinds, "{sizes:?} {query}");
            for (at, chosen) in listed.lists.iter().enumerate() {
                assert_eq!(chosen.kind, kinds[at]);
                listed_rows += chosen.rows.len();
                assert_eq!(
                    scores_of(chosen.rows.iter().map(|row| (row.entry, row.score))),
                    plain(&separate[at], query),
                    "{sizes:?} {query} {}",
                    kinds[at]
                );
            }
        }
    }
    assert!(listed_rows > 60, "{listed_rows}");
}

fn three_notes() -> (Log, Docs) {
    let mut log = Log::new();
    log.put(1, "lines", &["garden hose reel"]);
    log.put(2, "lines", &["garden"]);
    log.put(3, "lines", &["garden hose garden hose and a shed gate"]);
    let index = index_of(&log);
    (log, index)
}

#[test]
fn a_note_under_the_floor_is_held_back_and_one_at_it_is_listed() {
    let (log, index) = three_notes();
    let all = ask_over(&log, &index, 6, 0.0, "garden hose");
    assert_eq!(all.rows.len(), 3);
    let lowest = all.rows.last().expect("a row");
    let at = ask_over(&log, &index, 6, f64::from(lowest.score), "garden hose");
    assert_eq!(ids(&at), ids(&all));
    assert!(at.held_back.is_empty());
    let above = ask_over(&log, &index, 6, over(lowest.score), "garden hose");
    assert_eq!(ids(&above), ids(&all)[..2]);
    assert_eq!(above.held_back.len(), 1);
    assert_eq!(above.held_back[0].entry, lowest.entry);
    assert_eq!(above.held_back[0].score.to_bits(), lowest.score.to_bits());
}

#[test]
fn a_floor_of_0_lists_every_note_taken() {
    let (log, index) = three_notes();
    let chosen = ask_over(&log, &index, 2, 0.0, "garden hose");
    assert_eq!(chosen.rows.len(), 2);
    assert!(chosen.held_back.is_empty());
    let chosen = ask_over(&log, &index, 6, 0.0, "garden hose");
    assert_eq!(chosen.rows.len(), 3);
    assert!(chosen.held_back.is_empty());
}

#[test]
fn every_note_under_the_floor_lists_nothing_and_holds_them_back() {
    let (log, index) = three_notes();
    let all = ask_over(&log, &index, 6, 0.0, "garden hose");
    let chosen = ask_over(&log, &index, 6, 1000.0, "garden hose");
    assert!(chosen.rows.is_empty());
    assert_eq!(
        scores_of(chosen.held_back.iter().map(|held| (held.entry, held.score))),
        scores_of(all.rows.iter().map(|row| (row.entry, row.score))),
        "the notes taken, best first"
    );
    for (held, row) in chosen.held_back.iter().zip(&all.rows) {
        let Said::Passage { records, .. } = &row.said else {
            panic!("a row of a passage");
        };
        assert_eq!(held.records, Some(*records), "the winning chunk's range");
    }

    let (log, index) = two_turns(&[], false);
    let watched = Watched::over(&log);
    let held = asked(
        &watched,
        &index,
        &[list_of("lines", 6, 1000.0)],
        100,
        "turn eight",
    );
    assert!(held[0].rows.is_empty());
    assert_eq!(held[0].held_back.len(), 1);
    assert_eq!(
        held[0].held_back[0].records,
        Some(RecordRange { first: 3, count: 7 }),
        "a chunk that begins after the first record and holds several"
    );
    assert!(watched.reads.borrow().is_empty(), "nothing is read for it");
}

fn scores_of(rows: impl IntoIterator<Item = (Id, f32)>) -> Vec<(Id, u32)> {
    rows.into_iter()
        .map(|(entry, score)| (entry, score.to_bits()))
        .collect()
}

#[test]
fn the_floor_cuts_after_the_count() {
    let mut log = Log::new();
    let filler = "pad pad pad pad pad pad pad pad pad pad pad";
    for n in 1..=10u8 {
        let text = format!("garden {}", &filler[..usize::from(n) * 4 - 1]);
        log.put(n, "lines", &[&text]);
    }
    let index = index_of(&log);
    let all = ask_over(&log, &index, 10, 0.0, "garden");
    assert_eq!(all.rows.len(), 10);
    let fourth = all.rows[3].score;
    let chosen = ask_over(&log, &index, 6, over(fourth), "garden");
    assert_eq!(ids(&chosen), [id(1), id(2), id(3)]);
    assert_eq!(
        chosen
            .held_back
            .iter()
            .map(|row| row.entry)
            .collect::<Vec<_>>(),
        [id(4), id(5), id(6)]
    );
}

#[test]
fn the_first_prompt_of_a_session_is_first_once() {
    let mut marks = Marks::default();
    let again = |marks: &mut Marks, thread: &str, text: &str| {
        turn(Prompt::new(thread, text).expect("a prompt"), marks)
    };
    assert!(
        matches!(again(&mut marks, "one", "garden hose"), Turn::First(first) if first.thread() == "one")
    );
    assert!(matches!(
        again(&mut marks, "one", "garden hose"),
        Turn::Later
    ));
    assert!(matches!(
        again(&mut marks, "one", "another text"),
        Turn::Later
    ));
    assert!(
        matches!(again(&mut marks, "two", "garden hose"), Turn::First(first) if first.text() == "garden hose")
    );
    assert!(matches!(
        again(&mut marks, "one", "garden hose"),
        Turn::Later
    ));
    assert_eq!(marks.seen, ["one", "two"]);
}

#[test]
fn a_session_that_cannot_be_marked_is_not_first() {
    let mut marks = Marks {
        refuse: true,
        ..Marks::default()
    };
    let prompt = Prompt::new("one", "garden hose").expect("a prompt");
    match turn(prompt, &mut marks) {
        Turn::Unmarked(error) => assert_eq!(error.to_string(), "the marks cannot be written"),
        other => panic!("not marked, got {other:?}"),
    }
}

#[test]
fn a_prompt_needs_a_session_and_a_text() {
    assert!(Prompt::new("", "garden hose").is_none());
    assert!(Prompt::new("one", "").is_none());
    assert!(Prompt::new("", "").is_none());
    let prompt = Prompt::new("one", "garden hose").expect("a prompt");
    assert_eq!((prompt.thread(), prompt.text()), ("one", "garden hose"));
}

struct Speaks(&'static str, bool);

impl Reader for Speaks {
    fn kind(&self) -> &'static str {
        self.0
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        let mut passages = Vec::new();
        for (record, bytes) in (1..).zip(records) {
            let line = String::from_utf8_lossy(bytes);
            let line = line.trim_end_matches('\n');
            let (time, rest) = match line.strip_prefix('@').and_then(|rest| rest.split_once(' ')) {
                Some((seconds, rest)) => (seconds.parse().ok().map(at), rest),
                None => (None, line),
            };
            let (outline, text) = match rest.strip_prefix("> ") {
                Some(text) => (Some(text.to_owned()), text),
                None => (None, rest),
            };
            if text.is_empty() || text == "-" {
                continue;
            }
            passages.push(Passage {
                range: RecordRange {
                    first: record,
                    count: 1,
                },
                text: text.to_owned(),
                pieces: vec![Piece {
                    start: 0,
                    label: "line:".to_owned(),
                }],
                ends: vec![text.len()],
                at: time,
                outline,
            });
        }
        passages
    }

    fn appends_only(&self) -> bool {
        self.1
    }

    fn title(&self, records: &[&[u8]]) -> Option<String> {
        records
            .iter()
            .filter_map(|record| std::str::from_utf8(record).ok())
            .find_map(|line| line.trim_end().strip_prefix("# ").map(str::to_owned))
    }

    fn entry_labels(&self, records: &[&[u8]]) -> Vec<String> {
        records
            .iter()
            .filter_map(|record| std::str::from_utf8(record).ok())
            .filter_map(|line| line.trim_end().strip_prefix("dir ").map(str::to_owned))
            .collect()
    }
}

fn speakers() -> Vec<Box<dyn Reader>> {
    vec![
        Box::new(Speaks("lines", true)),
        Box::new(Speaks("memo", false)),
        Box::new(Speaks("jots", false)),
    ]
}

fn asked(
    log: &dyn Entries,
    index: &Docs,
    wanted: &[List],
    width: usize,
    text: &str,
) -> Vec<bilbo_search::digest::Chosen> {
    let setup = Ask {
        readers: speakers(),
        width,
        ..Ask::plain()
    };
    ask_read(&setup, log, index, wanted, text, &keys::defaults())
        .expect("a listing")
        .lists
}

fn said(chosen: &bilbo_search::digest::Chosen, entry: u8) -> &Said {
    &chosen
        .rows
        .iter()
        .find(|row| row.entry == id(entry))
        .unwrap_or_else(|| panic!("a row of entry {entry}"))
        .said
}

#[test]
fn a_row_is_its_readers_own_and_says_what_is_inside() {
    a_row_of_an_entry_holds_its_title();
    a_row_of_a_passage_names_its_records_its_day_its_folder_and_its_line();
    a_passage_of_two_turns_is_listed_at_the_line_before_its_first_record();
    a_list_with_a_line_of_its_own_gives_the_row_that_line_cut_no_more();
}

fn a_row_of_an_entry_holds_its_title() {
    let mut log = Log::new();
    log.put(
        1,
        "memo",
        &[
            "# A garden notebook with a rather long title",
            "garden hose",
        ],
    );
    log.put(2, "jots", &["garden hose only"]);
    let index = index_with(&log, &speakers());
    let wanted = [list_of("memo", 6, 0.0), list_of("jots", 6, 0.0)];
    let watched = Watched::over(&log);
    let held = asked(
        &watched,
        &index,
        &[list_of("memo", 6, 1000.0)],
        20,
        "garden hose",
    );
    assert!(
        watched.reads.borrow().is_empty(),
        "no read for a held-back file"
    );
    assert_eq!(held[0].held_back.len(), 1);
    assert_eq!(held[0].held_back[0].records, None, "an entry has no range");
    let watched = Watched::over(&log);
    let kinds = asked(&watched, &index, &wanted, 20, "garden hose");
    assert_eq!(
        *watched.reads.borrow(),
        [(id(1), whole(2)), (id(2), whole(1))],
        "one whole read for each row of an entry"
    );
    assert_eq!(
        *said(&kinds[0], 1),
        Said::Entry {
            title: Some("A garden notebook w\u{2026}".to_owned()),
            summary: None,
        }
    );
    assert_eq!(
        *said(&kinds[1], 2),
        Said::Entry {
            title: None,
            summary: None
        }
    );
    log.rewrite(1, &["# Another title", "garden hose"]);
    let watched = Watched::over(&log);
    let kinds = asked(
        &watched,
        &index,
        &[list_of("memo", 6, 0.0)],
        20,
        "garden hose",
    );
    assert_eq!(
        *watched.reads.borrow(),
        [(id(1), whole(2))],
        "one whole read of the newest generation"
    );
    assert_eq!(
        *said(&kinds[0], 1),
        Said::Entry {
            title: Some("Another title".to_owned()),
            summary: None,
        },
        "the title of the newest generation, though the chunk is the first's"
    );
}

fn a_row_of_a_passage_names_its_records_its_day_its_folder_and_its_line() {
    let mut log = Log::new();
    log.put(
        3,
        "lines",
        &[
            "@1700000000 > user: thin the carrot rows in the garden",
            "assistant: thin them to a hand apart at once now",
            "dir /home/me/src/garden",
            "dir /home/me/src/second-workspace-folder-with-a-long-name",
        ],
    );
    log.put(
        4,
        "lines",
        &["tomato stakes lean on the shed wall by the door"],
    );
    log.retouch(4, |held| held.captured = at(1_500_000_000));
    log.put(
        5,
        "lines",
        &[
            "plain words about hoses and reels kept in the shed",
            "> user: a later outline line",
        ],
    );
    log.put(
        7,
        "lines",
        &[
            "@1700000000 > user: first prompt about leeks and chives",
            "@1700100000 > user: second prompt about onions and kale",
            "assistant: replying with some words about radish stems",
        ],
    );
    let index = index_with(&log, &speakers());
    for row in &index.rows {
        assert_eq!(row.count, 1, "{row:?}");
    }
    let wanted = [list_of("lines", 6, 0.0)];
    let first = |text: &str, entry: u8| -> Said {
        let found = asked(&log, &index, &wanted, 20, text);
        said(&found[0], entry).clone()
    };
    let cut = "user: thin the carr\u{2026}".to_owned();
    assert_eq!(
        first("carrot rows garden", 3),
        Said::Passage {
            records: RecordRange { first: 1, count: 1 },
            day: "2023-11-14".to_owned(),
            folder: Some("/home/me/src/garden".to_owned()),
            line: Some(cut.clone()),
            written: None,
            summary: None,
        },
        "the folder is the first label of the whole generation, though its record stands after"
    );
    assert_eq!(
        first("hand apart", 3),
        Said::Passage {
            records: RecordRange { first: 2, count: 1 },
            day: "2023-11-14".to_owned(),
            folder: Some("/home/me/src/garden".to_owned()),
            line: Some(cut),
            written: None,
            summary: None,
        },
        "the nearest earlier passage with a time, and the outline line at or before the record"
    );
    assert_eq!(
        first("onions kale", 7),
        Said::Passage {
            records: RecordRange { first: 2, count: 1 },
            day: "2023-11-16".to_owned(),
            folder: None,
            line: Some("user: second prompt\u{2026}".to_owned()),
            written: None,
            summary: None,
        },
        "the passage's own day, not the entry's"
    );
    assert_eq!(
        first("radish stems", 7),
        Said::Passage {
            records: RecordRange { first: 3, count: 1 },
            day: "2023-11-16".to_owned(),
            folder: None,
            line: Some("user: second prompt\u{2026}".to_owned()),
            written: None,
            summary: None,
        },
        "the nearest earlier time, not the entry's own date"
    );
    assert_eq!(
        first("tomato stakes", 4),
        Said::Passage {
            records: RecordRange { first: 1, count: 1 },
            day: "2017-07-14".to_owned(),
            folder: None,
            line: None,
            written: None,
            summary: None,
        },
        "the entry's date when no passage has a time, and no line when the outline begins later"
    );
    assert_eq!(
        first("hoses reels", 5),
        Said::Passage {
            records: RecordRange { first: 1, count: 1 },
            day: "1970-01-01".to_owned(),
            folder: None,
            line: None,
            written: None,
            summary: None,
        }
    );
}

fn a_passage_of_two_turns_is_listed_at_the_line_before_its_first_record() {
    let mut log = Log::new();
    log.put(
        6,
        "lines",
        &[
            "first record of this entry is quite long, more or less",
            "second record is longer than the first, so it stands alone here",
            "> turn three",
            "x",
            "x",
            "x",
            "x",
            "> turn eight",
            "x",
        ],
    );
    let index = index_with(&log, &speakers());
    let rows: Vec<_> = index
        .rows
        .iter()
        .map(|row| (row.first, row.count))
        .collect();
    assert!(rows.contains(&(3, 7)), "{rows:?}");
    let wanted = [list_of("lines", 6, 0.0)];
    let found = asked(&log, &index, &wanted, 100, "turn eight");
    assert_eq!(
        *said(&found[0], 6),
        Said::Passage {
            records: RecordRange { first: 3, count: 7 },
            day: "1970-01-01".to_owned(),
            folder: None,
            line: Some("turn three".to_owned()),
            written: None,
            summary: None,
        }
    );
}

fn a_list_with_a_line_of_its_own_gives_the_row_that_line_cut_no_more() {
    let mut log = Log::new();
    let first = "> user: an opening record that is long enough to stand alone";
    let second = "> user: a later record, the one that matches, long enough too";
    log.put(3, "lines", &[first, second]);
    let index = index_with(&log, &speakers());
    let shows = |record: &[u8]| {
        Some(format!(
            "the way's line for {}",
            String::from_utf8_lossy(record).trim_end()
        ))
    };
    let wanted = [List {
        line: Some(&shows),
        ..list_of("lines", 6, 0.0)
    }];
    let found = asked(&log, &index, &wanted, 20, "matches");
    let Said::Passage { records, line, .. } = said(&found[0], 3) else {
        panic!("a row of a passage");
    };
    assert_eq!(*records, RecordRange { first: 2, count: 1 });
    assert_eq!(
        line.as_deref(),
        Some(format!("the way's line for {second}").as_str())
    );
}

struct Watched<'a> {
    log: &'a Log,
    reads: RefCell<Vec<(Id, RecordRange)>>,
    meta_fails: Option<Id>,
    extent_fails: Option<Id>,
    read_fails: Option<Id>,
    shorts_fail: bool,
    captures_fail: bool,
    belonging_fail: bool,
    capture_lookup_fails: Option<String>,
    extents_asked: RefCell<Vec<Id>>,
    lookups: RefCell<Vec<String>>,
    events: Events,
}

impl<'a> Watched<'a> {
    fn over(log: &'a Log) -> Watched<'a> {
        Watched {
            log,
            reads: RefCell::new(Vec::new()),
            meta_fails: None,
            extent_fails: None,
            read_fails: None,
            shorts_fail: false,
            captures_fail: false,
            belonging_fail: false,
            capture_lookup_fails: None,
            extents_asked: RefCell::new(Vec::new()),
            lookups: RefCell::new(Vec::new()),
            events: Events::default(),
        }
    }
}

fn refused() -> StoreError {
    StoreError::Missing("the double refuses".to_owned())
}

impl Entries for Watched<'_> {
    fn capture(&mut self, input: &Capture<'_>) -> Result<Captured, CaptureError> {
        Err(CaptureError::Store(StoreError::Missing(format!(
            "not called with {} bytes",
            input.bytes.len()
        ))))
    }

    fn set_meta(&mut self, _id: Id, _meta: &Meta) -> Result<u64, StoreError> {
        Err(StoreError::Missing("not called".to_owned()))
    }

    fn read(
        &self,
        id: Id,
        generation: Generation,
        records: RecordRange,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        self.events.borrow_mut().push("read");
        self.reads.borrow_mut().push((id, records));
        if self.read_fails == Some(id) {
            return Err(refused());
        }
        self.log.read(id, generation, records)
    }

    fn meta(&self, id: Id) -> Result<Option<Meta>, StoreError> {
        self.events.borrow_mut().push("meta");
        if self.meta_fails == Some(id) {
            return Err(refused());
        }
        self.log.meta(id)
    }

    fn changes(&self, since: &Cursor) -> Result<(Vec<Change>, Cursor), StoreError> {
        self.events.borrow_mut().push("changes");
        self.log.changes(since)
    }

    fn by_capture(&self, label: &CaptureLabel) -> Result<Option<Head>, StoreError> {
        self.events.borrow_mut().push("by_capture");
        self.lookups.borrow_mut().push(label.as_str().to_owned());
        if self.capture_lookup_fails.as_deref() == Some(label.as_str()) {
            return Err(refused());
        }
        Ok(self
            .log
            .entries
            .iter()
            .find(|held| held.capture.as_deref() == Some(label.as_str()))
            .map(|held| Head {
                id: held.id,
                generation: Generation(u32::try_from(held.generations.len()).expect("a few")),
                records: held
                    .generations
                    .last()
                    .map_or(0, |records| records.len() as u64),
                mark: Mark { end: 0, hash: 0 },
                stamp: None,
            }))
    }

    fn extent(
        &self,
        id: Id,
        generation: Option<Generation>,
    ) -> Result<Option<(Generation, u64)>, StoreError> {
        self.events.borrow_mut().push("extent");
        self.extents_asked.borrow_mut().push(id);
        if self.extent_fails == Some(id) {
            return Err(refused());
        }
        self.log.extent(id, generation)
    }

    fn captures_with(&self, part: &str) -> Result<Vec<(Id, CaptureLabel)>, StoreError> {
        self.events.borrow_mut().push("captures_with");
        if self.captures_fail {
            return Err(refused());
        }
        let mut found: Vec<(Id, CaptureLabel)> = self
            .log
            .entries
            .iter()
            .filter_map(|held| {
                let label = CaptureLabel::new(held.capture.clone()?)?;
                label
                    .as_str()
                    .to_ascii_lowercase()
                    .contains(&part.to_ascii_lowercase())
                    .then_some((held.id, label))
            })
            .collect();
        found.sort_by_key(|(id, _)| *id);
        Ok(found)
    }

    fn by_shorts(&self, shorts: &[Short]) -> Result<Vec<Id>, StoreError> {
        self.events.borrow_mut().push("by_shorts");
        if self.shorts_fail {
            return Err(refused());
        }
        self.log.by_shorts(shorts)
    }
}

impl Labels for Watched<'_> {
    fn with_words(&self, words: &[String]) -> Result<Vec<Id>, StoreError> {
        self.events.borrow_mut().push("with_words");
        self.log.with_words(words)
    }

    fn belonging_to(&self, entries: &[Id]) -> Result<Vec<Id>, StoreError> {
        self.events.borrow_mut().push("belonging_to");
        if self.belonging_fail {
            return Err(refused());
        }
        self.log.belonging_to(entries)
    }

    fn tagged(&self, tag: &Tag) -> Result<Vec<Id>, StoreError> {
        self.events.borrow_mut().push("tagged");
        self.log.tagged(tag)
    }

    fn tags(&self) -> Result<Vec<Tagged>, StoreError> {
        self.events.borrow_mut().push("tags");
        self.log.tags()
    }
}

fn whole(count: u64) -> RecordRange {
    RecordRange { first: 1, count }
}

fn two_kinds() -> (Log, Docs) {
    let mut log = Log::new();
    log.put(1, "lines", &["garden hose garden hose"]);
    log.put(2, "lines", &["garden hose reel"]);
    log.put(5, "memo", &["garden hose reel"]);
    log.put(6, "memo", &["garden hose garden hose reel"]);
    let index = index_with(&log, &speakers());
    (log, index)
}

fn four_gardens() -> (Log, Docs) {
    let mut log = Log::new();
    log.put(1, "lines", &["garden hose garden hose"]);
    log.put(2, "lines", &["garden hose reel"]);
    log.put(3, "lines", &["garden hose reel and more words here"]);
    log.put(
        4,
        "lines",
        &["garden hose reel and a few more words than that"],
    );
    let index = index_with(&log, &speakers());
    (log, index)
}

fn listed_by(
    watched: &Watched<'_>,
    index: &Docs,
    most: usize,
    floor: f64,
) -> bilbo_search::digest::Chosen {
    let setup = Ask {
        readers: speakers(),
        ..Ask::plain()
    };
    ask_read(
        &setup,
        watched,
        index,
        &[list_of("lines", most, floor)],
        "garden hose",
        &keys::defaults(),
    )
    .expect("a listing")
    .lists
    .remove(0)
}

#[test]
fn only_what_is_listed_is_read_and_a_passage_the_store_no_longer_holds_is_gone() {
    what_is_listed_is_read_once_and_whole();
    an_entry_whose_own_calls_fail_is_passed_over_and_one_whose_read_fails_is_left_out();
}

fn what_is_listed_is_read_once_and_whole() {
    let (log, index) = four_gardens();
    let watched = Watched::over(&log);
    let all = listed_by(&watched, &index, 4, 0.0);
    assert_eq!(ids(&all), [id(1), id(2), id(3), id(4)]);

    let watched = Watched::over(&log);
    let two = listed_by(&watched, &index, 2, 0.0);
    assert_eq!(ids(&two), [id(1), id(2)]);
    assert_eq!(
        *watched.reads.borrow(),
        [(id(1), whole(1)), (id(2), whole(1))]
    );

    let watched = Watched::over(&log);
    let held = listed_by(&watched, &index, 2, over(two.rows[1].score));
    assert_eq!(ids(&held), [id(1)]);
    assert_eq!(held.held_back.len(), 1);
    assert_eq!(*watched.reads.borrow(), [(id(1), whole(1))]);

    let mut long = Log::new();
    long.put(
        1,
        "lines",
        &[
            "garden hose reel stored in the shed all winter long",
            "garden hose coil kept by the gate until the spring comes",
        ],
    );
    let index = index_with(&long, &speakers());
    assert_eq!(index.rows.len(), 2);
    long.rewrite(1, &["garden hose reel stored in the shed all winter long"]);
    let watched = Watched::over(&long);
    let chosen = listed_by(&watched, &index, 6, 0.0);
    assert!(chosen.rows.is_empty());
    assert_eq!(chosen.gone, 2, "each chunk of the old generation");
    assert!(watched.reads.borrow().is_empty());

    let mut again = Log::new();
    again.put(
        1,
        "lines",
        &[
            "garden hose reel stored in the shed all winter long",
            "garden hose garden hose coil kept by the gate for spring",
        ],
    );
    let mut index = index_with(&again, &speakers());
    index.rows.sort();
    index.rows[1].first = 99;
    let watched = Watched::over(&again);
    let chosen = listed_by(&watched, &index, 6, 0.0);
    assert_eq!(ids(&chosen), [id(1)]);
    assert_eq!(
        chosen.gone, 1,
        "the chunk whose records the newest generation lacks"
    );
    match said(&chosen, 1) {
        Said::Passage { records, .. } => assert_eq!(*records, RecordRange { first: 1, count: 1 }),
        other => panic!("a row of a passage, got {other:?}"),
    }
    for (first, count) in [(2, 0), (2, 5), (3, 0)] {
        let mut index = index_with(&again, &speakers());
        index.rows.sort();
        index.rows[1].first = first;
        index.rows[1].count = count;
        let watched = Watched::over(&again);
        let chosen = listed_by(&watched, &index, 6, 0.0);
        assert_eq!(ids(&chosen), [id(1)], "{first} {count}");
        assert_eq!(
            chosen.gone, 1,
            "{first} {count}: an empty range or one past the records"
        );
        assert_eq!(
            records_of(&chosen, 1),
            RecordRange { first: 1, count: 1 },
            "{first} {count}"
        );
    }

    let mut rewritable = Log::new();
    rewritable.put(
        1,
        "memo",
        &["# A title", "garden hose", "more garden hose words"],
    );
    let index = index_with(&rewritable, &speakers());
    let watched = Watched::over(&rewritable);
    let setup = Ask {
        readers: speakers(),
        ..Ask::plain()
    };
    let found = ask_read(
        &setup,
        &watched,
        &index,
        &[list_of("memo", 6, 0.0)],
        "garden hose",
        &keys::defaults(),
    )
    .expect("a listing");
    assert_eq!(found.lists[0].rows.len(), 1);
    assert_eq!(*watched.reads.borrow(), [(id(1), whole(3))]);

    let mut log = Log::new();
    log.put(
        1,
        "lines",
        &[
            "garden hose garden hose reel stored away",
            "garden hose garden hose coil kept aside",
        ],
    );
    for n in 2..=4 {
        log.put(
            n,
            "lines",
            &["garden hose reel with a few more words in it"],
        );
    }
    let index = index_with(&log, &speakers());
    assert_eq!(
        index.rows.iter().filter(|row| row.entry == id(1)).count(),
        2
    );
    let watched = Watched::over(&log);
    let setup = Ask {
        readers: vec![Box::new(Speaks("memo", false))],
        ..Ask::plain()
    };
    let found = ask_read(
        &setup,
        &watched,
        &index,
        &[list_of("lines", 6, 0.0)],
        "garden hose",
        &keys::defaults(),
    )
    .expect("a listing");
    assert!(found.lists[0].rows.is_empty());
    assert_eq!(found.lists[0].gone, 4);
    assert!(watched.reads.borrow().is_empty());
}

fn an_entry_whose_own_calls_fail_is_passed_over_and_one_whose_read_fails_is_left_out() {
    let (log, index) = four_gardens();
    let mut watched = Watched::over(&log);
    watched.meta_fails = Some(id(1));
    let chosen = listed_by(&watched, &index, 2, 0.0);
    assert_eq!(ids(&chosen), [id(2), id(3)]);
    assert_eq!(chosen.gone, 1);

    let mut watched = Watched::over(&log);
    watched.extent_fails = Some(id(1));
    let chosen = listed_by(&watched, &index, 2, 0.0);
    assert_eq!(ids(&chosen), [id(2), id(3)]);
    assert_eq!(chosen.gone, 1);

    let mut watched = Watched::over(&log);
    watched.read_fails = Some(id(2));
    let chosen = listed_by(&watched, &index, 3, 0.0);
    assert_eq!(ids(&chosen), [id(1), id(3)]);
    assert_eq!(
        chosen.gone, 1,
        "nothing takes the place of an entry whose read failed"
    );
    assert_eq!(watched.reads.borrow().len(), 3);
}

#[test]
fn an_error_of_the_store_that_is_no_entrys_own_ends_the_run() {
    let (log, index) = two_kinds();
    let mut watched = Watched::over(&log);
    watched.shorts_fail = true;
    let mut spy = Spy::over(&index);
    spy.shared = Some(watched.events.clone());
    let setup = Ask {
        readers: speakers(),
        ..Ask::plain()
    };
    let result = ask_read(
        &setup,
        &watched,
        &spy,
        &[list_of("lines", 6, 0.0), list_of("memo", 6, 0.0)],
        "garden hose",
        &keys::defaults(),
    );
    match &result {
        Err(Error::Store(error)) => assert_eq!(error.to_string(), refused().to_string()),
        other => panic!("the store's own refusal, got {other:?}"),
    }
    let events = watched.events.borrow();
    let failed = events
        .iter()
        .position(|event| *event == "by_shorts")
        .expect("asked");
    let ranked = events
        .iter()
        .rposition(|event| *event == "found")
        .expect("ranked");
    assert!(ranked < failed, "{events:?}");
    assert!(
        events
            .iter()
            .take(failed)
            .filter(|event| **event == "found")
            .count()
            >= 2,
        "both lists ranked a candidate first: {events:?}"
    );
    assert!(!events.contains(&"read"), "{events:?}");
    drop(events);

    let uuid = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";
    let (mut log, index) = two_kinds();
    log.label(1, &format!("file:/p/proj/{uuid}.jsonl"));
    for (fails_captures, fails_belonging, called) in [
        (true, false, "captures_with"),
        (false, true, "belonging_to"),
    ] {
        let mut watched = Watched::over(&log);
        watched.captures_fail = fails_captures;
        watched.belonging_fail = fails_belonging;
        let mut spy = Spy::over(&index);
        spy.shared = Some(watched.events.clone());
        let setup = Ask {
            readers: speakers(),
            thread: uuid,
            labels: &watched,
            ..Ask::plain()
        };
        let result = ask_read(
            &setup,
            &watched,
            &spy,
            &[list_of("lines", 6, 0.0), list_of("memo", 6, 0.0)],
            "garden hose",
            &keys::defaults(),
        );
        match &result {
            Err(Error::Store(error)) => {
                assert_eq!(error.to_string(), refused().to_string(), "{called}");
            }
            other => panic!("{called}: the store's own refusal, got {other:?}"),
        }
        let events = watched.events.borrow();
        let failed = events
            .iter()
            .position(|event| *event == called)
            .expect("asked");
        let ranked = events.iter().rposition(|event| *event == "found");
        assert!(
            ranked.is_none_or(|ranked| ranked < failed),
            "{called}: {events:?}"
        );
        assert!(
            events.iter().filter(|event| **event == "postings").count() > 0
                && events.iter().position(|event| *event == "postings") < Some(failed),
            "every list was ranked first: {events:?}"
        );
        assert!(
            events
                .iter()
                .take(failed)
                .filter(|event| **event == "found")
                .count()
                >= 2,
            "both lists ranked a candidate first: {events:?}"
        );
        assert!(!events.contains(&"read"), "{events:?}");
    }

    let watched = Watched::over(&log);
    let setup = Ask {
        readers: speakers(),
        labels: &watched,
        ..Ask::plain()
    };
    let listed = ask_read(
        &setup,
        &watched,
        &index,
        &[list_of("lines", 6, 0.0)],
        "garden hose",
        &keys::defaults(),
    )
    .expect("a listing");
    assert_eq!(listed.lists[0].rows.len(), 2);
    let events = watched.events.borrow();
    assert!(!events.contains(&"captures_with") && !events.contains(&"belonging_to"));
    drop(events);

    for (wanted, text, lists_given) in [
        (
            vec![list_of("lines", 0, 0.0), list_of("memo", 0, 0.0)],
            "garden hose",
            0,
        ),
        (
            vec![list_of("lines", 6, 0.0), list_of("memo", 6, 0.0)],
            "zzzzzz",
            2,
        ),
    ] {
        let mut watched = Watched::over(&log);
        watched.captures_fail = true;
        watched.belonging_fail = true;
        let setup = Ask {
            readers: speakers(),
            thread: uuid,
            labels: &watched,
            ..Ask::plain()
        };
        let listed = ask_read(&setup, &watched, &index, &wanted, text, &keys::defaults())
            .expect("nothing to pass over, nothing asked");
        assert_eq!(listed.stand, Stand::Behind(1));
        assert_eq!(listed.lists.len(), lists_given);
        assert!(listed.lists.iter().all(|chosen| chosen.rows.is_empty()));
        let events = watched.events.borrow();
        assert!(
            !events.contains(&"captures_with") && !events.contains(&"belonging_to"),
            "{events:?}"
        );
    }
}

const DAY: Duration = Duration::from_secs(86_400);

fn long_garden() -> &'static str {
    "garden hose garden hose garden hose garden hose garden hose garden hose garden hose"
}

fn ruled(
    list: List<'static>,
    log: &Log,
    text: &str,
    run: SystemTime,
) -> bilbo_search::digest::Chosen {
    let index = index_with(log, &speakers());
    let setup = Ask {
        readers: speakers(),
        at: run,
        ..Ask::plain()
    };
    ask_read(&setup, log, &index, &[list], text, &keys::defaults())
        .expect("a listing")
        .lists
        .remove(0)
}

fn every_rule() -> List<'static> {
    List {
        old_after: Some(30 * DAY),
        lists_belonging: false,
        lists_parts: false,
        ..list_of("lines", 6, 0.0)
    }
}

type Chunk = (Id, u64, Option<(u32, u32)>, u32);

fn plain_chunks(index: &dyn Lookup, text: &str) -> Vec<Chunk> {
    let says = Says(None);
    let section = ranking::defaults();
    let ranked = section
        .bm25(index, &says)
        .rank(
            &Query {
                text: text.to_owned(),
                kinds: Vec::new(),
                apart: Vec::new(),
                entries: None,
            },
            1000,
        )
        .expect("ranked");
    ranked
        .candidates
        .iter()
        .map(|candidate| {
            (
                candidate.found.entry,
                candidate.found.extent.records.first,
                candidate.found.extent.part,
                candidate.score.to_bits(),
            )
        })
        .collect()
}

fn score_of(chosen: &bilbo_search::digest::Chosen, entry: u8) -> u32 {
    chosen
        .rows
        .iter()
        .find(|row| row.entry == id(entry))
        .expect("a row")
        .score
        .to_bits()
}

fn parts_only_list() -> List<'static> {
    List {
        lists_parts: false,
        ..list_of("lines", 6, 0.0)
    }
}

fn no_days() -> List<'static> {
    List {
        old_after: None,
        ..every_rule()
    }
}

fn records_of(chosen: &bilbo_search::digest::Chosen, entry: u8) -> RecordRange {
    match said(chosen, entry) {
        Said::Passage { records, .. } => *records,
        other => panic!("a row of a passage, got {other:?}"),
    }
}

#[test]
fn a_list_passes_over_what_it_was_set_to_and_counts_each() {
    let mut log = Log::new();
    log.put(
        1,
        "lines",
        &[
            "garden hose garden hose garden hose garden hose reel",
            "garden hose garden hose garden hose garden hose coil",
            "garden hose garden hose garden hose garden hose gate",
        ],
    );
    log.put(
        5,
        "lines",
        &["garden hose reel and a few more words than that"],
    );
    log.belong(1, 5);
    let chunks = plain_chunks(&index_with(&log, &speakers()), "garden hose");
    let own_bits = |entry: u8| {
        chunks
            .iter()
            .find(|chunk| chunk.0 == id(entry))
            .map(|chunk| chunk.3)
            .expect("a chunk")
    };
    let belongs_only = List {
        lists_belonging: false,
        ..list_of("lines", 6, 0.0)
    };
    for by in [
        ruled(belongs_only, &log, "garden hose", at(NOW)),
        ruled(no_days(), &log, "garden hose", at(NOW)),
    ] {
        assert_eq!(ids(&by), [id(5)]);
        assert_eq!((by.belongs, by.part, by.own, by.old), (1, 0, 0, 0));
        assert_eq!(
            score_of(&by, 5),
            own_bits(5),
            "the surviving chunk's own score"
        );
    }
    let without = ruled(list_of("lines", 6, 0.0), &log, "garden hose", at(NOW));
    assert_eq!(ids(&without), [id(1), id(5)]);
    assert_eq!((without.belongs, without.part), (0, 0));
    let apart = ruled(parts_only_list(), &log, "garden hose", at(NOW));
    assert_eq!(ids(&apart), [id(1), id(5)]);
    assert_eq!(
        (apart.belongs, apart.part),
        (0, 0),
        "the switches are apart"
    );

    let mut log = Log::new();
    log.put(
        2,
        "lines",
        &[
            long_garden(),
            "garden shed and more words here and some others to dilute it",
        ],
    );
    log.put(3, "lines", &[long_garden()]);
    let index = index_with(&log, &speakers());
    assert!(index.rows.iter().filter(|row| row.part.is_some()).count() >= 4);
    let chunks = plain_chunks(&index, "garden hose");
    let mut taken_whole = Vec::new();
    let mut parts_met = 0;
    for (entry, _, part, _) in &chunks {
        if taken_whole.contains(entry) {
            continue;
        }
        if part.is_some() {
            parts_met += 1;
        } else {
            taken_whole.push(*entry);
        }
    }
    let whole_bits = chunks
        .iter()
        .find(|(entry, first, part, _)| *entry == id(2) && *first == 2 && part.is_none())
        .map(|chunk| chunk.3)
        .expect("the whole chunk");
    for parted in [
        ruled(parts_only_list(), &log, "garden hose", at(NOW)),
        ruled(no_days(), &log, "garden hose", at(NOW)),
    ] {
        assert_eq!(ids(&parted), [id(2)]);
        assert_eq!(records_of(&parted, 2), RecordRange { first: 2, count: 1 });
        assert_eq!(
            parted.part, parts_met,
            "every part met before the whole chunk"
        );
        assert_eq!((parted.belongs, parted.own, parted.old), (0, 0, 0));
        assert_eq!(
            score_of(&parted, 2),
            whole_bits,
            "the surviving chunk's own score"
        );
    }
    let belongs_only = ruled(
        List {
            lists_belonging: false,
            ..list_of("lines", 6, 0.0)
        },
        &log,
        "garden hose",
        at(NOW),
    );
    assert_eq!(ids(&belongs_only), [id(2), id(3)]);
    assert_eq!(
        records_of(&belongs_only, 2),
        RecordRange { first: 1, count: 1 }
    );
    assert_eq!((belongs_only.part, belongs_only.belongs), (0, 0));
    let plain = ruled(list_of("lines", 6, 0.0), &log, "garden hose", at(NOW));
    assert_eq!(ids(&plain), [id(2), id(3)]);
    assert_eq!(records_of(&plain, 2), RecordRange { first: 1, count: 1 });
    assert_eq!(plain.part, 0);

    let began = 1_600_000_000;
    let mut log = Log::new();
    log.put(4, "lines", &[&format!("@{began} garden hose reel")]);
    let days = |n: u64| at(began) + DAY * u32::try_from(n).expect("days");
    let old = |run: SystemTime| ruled(every_rule(), &log, "garden hose", run);
    assert_eq!(ids(&old(days(30))), [id(4)]);
    let late = old(days(30) + Duration::from_millis(1));
    assert!(late.rows.is_empty());
    assert_eq!(late.old, 1);
    let late = old(days(31));
    assert_eq!((late.rows.len(), late.old), (0, 1));
    let none = ruled(list_of("lines", 6, 0.0), &log, "garden hose", days(400));
    assert_eq!((ids(&none), none.old), (vec![id(4)], 0));

    let mut log = Log::new();
    log.put(9, "lines", &[&format!("@{began} garden hose reel")]);
    log.put(10, "lines", &["unrelated words only"]);
    log.belong(9, 10);
    let both = ruled(every_rule(), &log, "garden hose", days(40));
    assert!(both.rows.is_empty());
    assert_eq!((both.belongs, both.old), (1, 0));

    let mut log = Log::new();
    log.put(
        6,
        "lines",
        &[
            &format!("@{began} {}", long_garden()),
            "garden shed and more words here and some others to dilute it",
        ],
    );
    let stale = ruled(every_rule(), &log, "garden hose", days(40));
    assert_eq!((stale.rows.len(), stale.old, stale.part), (0, 1, 0));
    let fresh = ruled(every_rule(), &log, "garden hose", days(1));
    assert_eq!(ids(&fresh), [id(6)]);
    assert_eq!(fresh.part, 2, "the parts that outrank the whole chunk");
    assert_eq!(records_of(&fresh, 6), RecordRange { first: 2, count: 1 });

    let mut log = Log::new();
    log.put(
        7,
        "lines",
        &[
            "garden hose garden hose garden hose garden hose garden hose and so on past the first part, then more words follow here without matches at all today",
        ],
    );
    let index = index_with(&log, &speakers());
    assert!(index.rows.iter().all(|row| row.part.is_some()));
    let only = ruled(no_days(), &log, "garden hose", at(NOW));
    assert!(only.rows.is_empty());
    assert!(only.part >= 1, "the first part of a record is a part");

    let mut log = Log::new();
    log.put(
        8,
        "lines",
        &["garden hose reel", "garden only here and more padding words for the part, then a lot more words follow so that this record is long enough"],
    );
    let taken = ruled(no_days(), &log, "garden hose", at(NOW));
    assert_eq!(ids(&taken), [id(8)]);
    assert_eq!(taken.part, 0, "an entry already taken counts nothing more");
}

#[test]
fn the_prompts_own_session_and_what_belongs_to_it_are_in_no_list() {
    const UUID: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";
    let mut log = Log::new();
    for n in (1..=5).rev() {
        log.put(n, "lines", &["garden hose reel"]);
    }
    log.label(1, &format!("file:/p/proj/{}.jsonl", UUID.to_uppercase()));
    log.label(2, "file:/p/proj/other-a.jsonl");
    log.label(3, "file:/p/proj/other-b.jsonl");
    log.label(4, &format!("file:/p/proj/{UUID}.jsonl.bak"));
    log.label(5, &format!("file:/p/proj/{UUID}/subagents/agent-b.jsonl"));
    log.belong(2, 1);
    log.belong(3, 2);
    let index = index_with(&log, &speakers());
    let by = |thread: &str, list: List<'static>| {
        let watched = Watched::over(&log);
        let setup = Ask {
            readers: speakers(),
            thread,
            labels: &watched,
            ..Ask::plain()
        };
        ask_read(
            &setup,
            &watched,
            &index,
            &[list],
            "garden hose",
            &keys::defaults(),
        )
        .expect("a listing")
        .lists
        .remove(0)
    };
    let own = by(UUID, list_of("lines", 6, 0.0));
    assert_eq!(ids(&own), [id(4), id(5)]);
    assert_eq!(own.own, 3);
    let capitals = by(&UUID.to_uppercase(), list_of("lines", 6, 0.0));
    assert_eq!((ids(&capitals), capitals.own), (ids(&own), 3));
    for thread in ["a-thread", &UUID[..8]] {
        let all = by(thread, list_of("lines", 6, 0.0));
        assert_eq!(all.rows.len(), 5, "{thread}");
        assert_eq!(all.own, 0, "{thread}");
    }
    let set = by(
        UUID,
        List {
            lists_belonging: false,
            ..list_of("lines", 6, 0.0)
        },
    );
    assert_eq!((set.own, set.belongs), (3, 0), "own is judged first");
}

#[test]
fn what_is_passed_over_takes_no_place_and_is_counted_only_on_the_way() {
    let mut log = Log::new();
    log.put(1, "lines", &["garden hose garden hose garden hose"]);
    log.put(2, "lines", &["garden hose garden hose garden"]);
    log.put(3, "lines", &["garden hose reel"]);
    log.put(4, "lines", &["garden hose reel and a few more words"]);
    log.put(
        5,
        "lines",
        &["garden hose reel and a few more words than that one"],
    );
    log.put(9, "lines", &["bed"]);
    log.belong(1, 9);
    log.belong(2, 9);
    log.belong(5, 9);
    let list = List {
        lists_belonging: false,
        ..list_of("lines", 2, 0.0)
    };
    let chosen = ruled(list, &log, "garden hose", at(NOW));
    assert_eq!(ids(&chosen), [id(3), id(4)]);
    assert_eq!(chosen.belongs, 2, "the fifth entry was never met");
}

fn kept_label(entry: Id) -> CaptureLabel {
    CaptureLabel::new(format!("note:{entry}")).expect("a label")
}

fn written_by(record: &[u8]) -> Option<Written> {
    let line = String::from_utf8_lossy(record);
    let (from, text) = line.trim_end().split_once('|')?;
    let from = from
        .split(';')
        .filter_map(|part| {
            let mut fields = part.split(':');
            let entry: u8 = fields.next()?.parse().ok()?;
            let generation: u32 = fields.next()?.parse().ok()?;
            let (first, last) = fields.next()?.split_once('-')?;
            let (first, last): (u64, u64) = (first.parse().ok()?, last.parse().ok()?);
            Some(Source {
                entry: id(entry),
                generation: Generation(generation),
                records: RecordRange {
                    first,
                    count: last - first + 1,
                },
            })
        })
        .collect();
    Some(Written {
        from,
        text: text.to_owned(),
    })
}

fn two_turns(notes: &[&str], second: bool) -> (Log, Docs) {
    let mut log = Log::new();
    if second {
        log.put(2, "lines", &["turn eight again in a second entry"]);
    }
    log.put(
        1,
        "lines",
        &[
            "first record of this entry is quite long, more or less",
            "second record is longer than the first, so it stands alone here",
            "> turn three",
            "x",
            "x",
            "x",
            "x",
            "> turn eight",
            "x",
        ],
    );
    if !notes.is_empty() {
        log.put(20, "jots", notes);
        log.label(20, &format!("note:{}", id(1)));
    }
    let index = index_with(&log, &speakers());
    assert!(
        index
            .rows
            .iter()
            .any(|row| (row.entry, row.first, row.count) == (id(1), 3, 7))
    );
    (log, index)
}

fn listed_with(
    watched: &Watched<'_>,
    index: &Docs,
    kind: &str,
    text: &str,
) -> bilbo_search::digest::Chosen {
    let setup = Ask {
        readers: speakers(),
        way: Way {
            label: &kept_label,
            written: &written_by,
            summary: None,
        },
        ..Ask::plain()
    };
    ask_read(
        &setup,
        watched,
        index,
        &[list_of(kind, 6, 0.0)],
        text,
        &keys::defaults(),
    )
    .expect("a listing")
    .lists
    .remove(0)
}

fn written_of(chosen: &bilbo_search::digest::Chosen, entry: u8) -> Option<String> {
    match said(chosen, entry) {
        Said::Passage { written, .. } => written.clone(),
        other => panic!("a row of a passage, got {other:?}"),
    }
}

#[test]
fn a_passages_row_carries_what_was_written_from_its_records() {
    let says = |notes: &[&str]| -> Option<String> {
        let (log, index) = two_turns(notes, false);
        let watched = Watched::over(&log);
        let chosen = listed_with(&watched, &index, "lines", "turn eight");
        assert_eq!(ids(&chosen), [id(1)]);
        written_of(&chosen, 1)
    };
    assert_eq!(says(&["1:1:3-7|the note"]).as_deref(), Some("the note"));
    assert_eq!(
        says(&["1:1:9-9|short", "1:1:3-7|long"]).as_deref(),
        Some("long"),
        "the note that shares more"
    );
    assert_eq!(
        says(&["1:1:3-5|early", "1:1:6-8|late"]).as_deref(),
        Some("late"),
        "of equals the later record"
    );
    assert_eq!(says(&["1:2:3-7|other generation"]), None);
    assert_eq!(says(&["2:1:3-7|other entry"]), None);
    assert_eq!(says(&["|no source"]), None);
    assert_eq!(says(&["1:1:1-2|shares none"]), None);
    assert_eq!(
        says(&["1:1:5-6;1:1:5-6|twice", "1:1:3-5|three"]).as_deref(),
        Some("three"),
        "a range named twice counts once"
    );
    assert_eq!(
        says(&["1:1:3-7|the first", "1:1:9-9|the second"]).as_deref(),
        Some("the first")
    );
    let long = "w".repeat(150);
    assert_eq!(
        says(&[&format!("1:1:1-9|{long}")]).as_deref(),
        Some(long.as_str()),
        "a note of the whole entry stands whole, longer than the outline's width"
    );
    assert_eq!(
        says(&["1:1:3-4;1:1:6-7|two ranges", "1:1:5-7|one range of three"]).as_deref(),
        Some("two ranges"),
        "every range of a note counts"
    );
    assert_eq!(
        says(&["1:1:4-5|four to five", "1:1:1-3|one to three"]).as_deref(),
        Some("four to five"),
        "two shared records beat one, whatever the lengths of the ranges"
    );
    for notes in [
        ["a run record, no note", "1:1:3-7|the note"],
        ["1:1:3-7|the note", "a run record, no note"],
    ] {
        assert_eq!(says(&notes).as_deref(), Some("the note"), "{notes:?}");
    }
    let five_seven = |notes: &[&str]| -> Option<String> {
        let mut log = Log::new();
        log.put(
            1,
            "lines",
            &[
                &"x".repeat(40),
                &"x".repeat(60),
                &"x".repeat(60),
                &"x".repeat(63),
                "turn five",
                "turn six",
                "turn seven",
                &"x".repeat(63),
            ],
        );
        log.put(20, "jots", notes);
        log.label(20, &format!("note:{}", id(1)));
        let index = index_with(&log, &speakers());
        assert!(
            index
                .rows
                .iter()
                .any(|row| (row.entry, row.first, row.count) == (id(1), 5, 3))
        );
        let watched = Watched::over(&log);
        let chosen = listed_with(&watched, &index, "lines", "turn seven");
        written_of(&chosen, 1)
    };
    assert_eq!(
        five_seven(&["1:1:5-6;1:1:5-6|twice", "1:1:5-7|three"]).as_deref(),
        Some("three")
    );
    assert_eq!(
        five_seven(&["1:1:5-6;1:1:5-6|twice"]).as_deref(),
        Some("twice"),
        "two records are shared, once each"
    );

    let (log, index) = two_turns(&["1:1:3-7|the note"], false);
    let mut watched = Watched::over(&log);
    watched.extent_fails = Some(id(20));
    let chosen = listed_with(&watched, &index, "lines", "turn eight");
    assert_eq!(written_of(&chosen, 1).as_deref(), Some("the note"));
    assert_eq!(*watched.lookups.borrow(), [format!("note:{}", id(1))]);
    assert_eq!(
        watched
            .reads
            .borrow()
            .iter()
            .filter(|(entry, _)| *entry == id(20))
            .collect::<Vec<_>>(),
        [&(id(20), whole(1))],
        "one whole read of the notes' entry"
    );
    assert!(
        !watched.extents_asked.borrow().contains(&id(20)),
        "the lookup gave the generation and the count"
    );

    let (mut log, index) = two_turns(&["1:1:3-7|the older text"], false);
    log.rewrite(20, &["1:1:3-7|the newer text"]);
    let watched = Watched::over(&log);
    let chosen = listed_with(&watched, &index, "lines", "turn eight");
    assert_eq!(
        written_of(&chosen, 1).as_deref(),
        Some("the newer text"),
        "the notes are read at their own entry's newest generation"
    );

    let (log, index) = two_turns(&[], false);
    let watched = Watched::over(&log);
    let chosen = listed_with(&watched, &index, "lines", "turn eight");
    assert_eq!(written_of(&chosen, 1), None);
    assert!(
        watched
            .reads
            .borrow()
            .iter()
            .all(|(entry, _)| *entry == id(1)),
        "nothing read for a label that holds no entry"
    );

    let mut rewritable = Log::new();
    rewritable.put(1, "memo", &["# A title", "garden hose"]);
    let index = index_with(&rewritable, &speakers());
    let watched = Watched::over(&rewritable);
    let chosen = listed_with(&watched, &index, "memo", "garden hose");
    assert_eq!(chosen.rows.len(), 1);
    assert!(
        !watched.events.borrow().contains(&"by_capture"),
        "the store is not asked for a row of an entry"
    );

    let (control_log, control_index) = two_turns(&[], true);
    let control = listed_with(
        &Watched::over(&control_log),
        &control_index,
        "lines",
        "turn eight",
    );
    let (log, index) = two_turns(&["1:1:3-7|the note"], true);
    for (lookup_fails, read_fails) in [(true, None), (false, Some(id(20)))] {
        let mut watched = Watched::over(&log);
        watched.capture_lookup_fails = lookup_fails.then(|| format!("note:{}", id(1)));
        watched.read_fails = read_fails;
        let chosen = listed_with(&watched, &index, "lines", "turn eight");
        assert_eq!(
            chosen.rows, control.rows,
            "both rows whole and in order, beside a run with no notes"
        );
        let mut rows = ids(&chosen);
        rows.sort();
        assert_eq!(
            rows,
            [id(1), id(2)],
            "the row is there, and the one after it"
        );
        assert_eq!(written_of(&chosen, 1), None);
        assert_eq!(written_of(&chosen, 2), None);
        assert_eq!(
            (
                chosen.gone,
                chosen.own,
                chosen.belongs,
                chosen.old,
                chosen.part,
                chosen.shown
            ),
            (0, 0, 0, 0, 0, 0),
            "nothing is counted for a lookup that failed"
        );
    }

    let (mut log, index) = two_turns(&["1:1:3-7|first note"], true);
    log.put(21, "jots", &["2:1:1-1|second note"]);
    log.label(21, &format!("note:{}", id(2)));
    let whole_control = listed_with(&Watched::over(&log), &index, "lines", "turn eight");
    assert_eq!(written_of(&whole_control, 1).as_deref(), Some("first note"));
    assert_eq!(
        written_of(&whole_control, 2).as_deref(),
        Some("second note")
    );
    let first = whole_control.rows[0].entry;
    let notes = if first == id(1) { id(20) } else { id(21) };
    let mut wanted = whole_control.rows.clone();
    if let Said::Passage { written, .. } = &mut wanted[0].said {
        *written = None;
    }
    for (lookup_fails, read_fails) in [(true, None), (false, Some(notes))] {
        let mut watched = Watched::over(&log);
        watched.capture_lookup_fails = lookup_fails.then(|| format!("note:{first}"));
        watched.read_fails = read_fails;
        let chosen = listed_with(&watched, &index, "lines", "turn eight");
        assert_eq!(
            chosen.rows, wanted,
            "the first row has its four fields and the second its fifth"
        );
        assert_eq!(
            (
                chosen.gone,
                chosen.own,
                chosen.belongs,
                chosen.old,
                chosen.part,
                chosen.shown
            ),
            (0, 0, 0, 0, 0, 0)
        );
    }
}

#[test]
fn an_entry_written_from_a_listed_entry_is_not_listed_again() {
    let mut log = Log::new();
    log.put(1, "lines", &["garden hose garden hose"]);
    log.put(2, "jots", &["garden hose garden hose"]);
    log.put(3, "jots", &["garden hose reel and a few more words"]);
    log.belong(2, 1);
    let index = index_with(&log, &speakers());
    let both = |order: [&str; 2], floor: f64| {
        let wanted: Vec<List<'static>> = order
            .iter()
            .map(|kind| {
                if *kind == "lines" {
                    list_of(kind, 6, floor)
                } else {
                    list_of(kind, 1, 0.0)
                }
            })
            .collect();
        let setup = Ask {
            readers: speakers(),
            ..Ask::plain()
        };
        ask_read(
            &setup,
            &log,
            &index,
            &wanted,
            "garden hose",
            &keys::defaults(),
        )
        .expect("a listing")
        .lists
    };
    let listed = both(["lines", "jots"], 0.0);
    assert_eq!(ids(&listed[0]), [id(1)]);
    assert_eq!(ids(&listed[1]), [id(3)]);
    assert_eq!(listed[1].shown, 1);
    let held = both(["lines", "jots"], over(listed[0].rows[0].score));
    assert!(held[0].rows.is_empty());
    assert_eq!(held[0].held_back.len(), 1);
    assert_eq!(ids(&held[1]), [id(2)]);
    assert_eq!(held[1].shown, 0, "a list held back hides nothing");
    let second = |second: List<'static>, log: &Log| {
        let index = index_with(log, &speakers());
        let setup = Ask {
            readers: speakers(),
            ..Ask::plain()
        };
        ask_read(
            &setup,
            log,
            &index,
            &[list_of("lines", 6, 0.0), second],
            "garden hose",
            &keys::defaults(),
        )
        .expect("a listing")
        .lists
        .remove(1)
    };
    let passes = second(
        List {
            lists_belonging: false,
            ..list_of("jots", 1, 0.0)
        },
        &log,
    );
    assert_eq!(ids(&passes), [id(3)]);
    assert_eq!((passes.belongs, passes.shown), (1, 0));
    let mut dated = Log::new();
    dated.put(1, "lines", &["garden hose garden hose"]);
    dated.put(
        2,
        "jots",
        &[&format!("@{} garden hose garden hose", NOW - 90 * 86_400)],
    );
    dated.put(
        3,
        "jots",
        &[&format!("@{NOW} garden hose reel and a few more words")],
    );
    dated.belong(2, 1);
    let aged = second(
        List {
            old_after: Some(30 * DAY),
            ..list_of("jots", 1, 0.0)
        },
        &dated,
    );
    assert_eq!(ids(&aged), [id(3)]);
    assert_eq!((aged.old, aged.shown), (1, 0));
    let other_way = both(["jots", "lines"], 0.0);
    assert_eq!(ids(&other_way[0]), [id(2)]);
    assert_eq!(other_way[0].shown, 0);
    assert_eq!(ids(&other_way[1]), [id(1)]);
}

struct Counted<'a> {
    inner: &'a Docs,
    calls: RefCell<Vec<String>>,
    fail_at: Option<usize>,
    reading: RefCell<Option<String>>,
}

impl Counted<'_> {
    fn asked(&self, what: &str) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|call| call.as_str() == what)
            .count()
    }

    fn seen(&self, what: &str) -> Result<(), IndexError> {
        let mut calls = self.calls.borrow_mut();
        calls.push(what.to_owned());
        match self.fail_at {
            Some(at) if calls.len() >= at => Err(IndexError::Failed {
                action: "read",
                path: std::path::PathBuf::from("/c/index"),
                error: "refused".to_owned(),
            }),
            _ => Ok(()),
        }
    }
}

impl Lookup for Counted<'_> {
    fn position(&self) -> Result<Option<Position>, IndexError> {
        self.seen("position")?;
        self.inner.position()
    }

    fn totals(&self) -> Result<Totals, IndexError> {
        self.seen("totals")?;
        self.inner.totals()
    }

    fn postings(&self, term: &str) -> Result<Vec<Posting>, IndexError> {
        self.seen(&format!("postings:{term}"))?;
        self.inner.postings(term)
    }

    fn near(&self, term: &str, edits: Edits) -> Result<Vec<Near>, IndexError> {
        self.seen(&format!("near:{term}"))?;
        self.inner.near(term, edits)
    }

    fn pairs(&self, first: &str, second: &str) -> Result<Vec<Posting>, IndexError> {
        self.seen(&format!("pairs:{first}:{second}"))?;
        self.inner.pairs(first, second)
    }

    fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
        self.seen("holding")?;
        self.inner.holding(filter)
    }

    fn length(&self, kind: &str) -> Result<u64, IndexError> {
        self.seen("length")?;
        self.inner.length(kind)
    }

    fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError> {
        self.seen("found")?;
        let found = self.inner.found(docs)?;
        if let Some(first) = found.first() {
            self.reading
                .borrow_mut()
                .get_or_insert_with(|| first.kind.clone());
        }
        Ok(found)
    }
}

fn counted<'a>(index: &'a Docs, fail_at: Option<usize>) -> Counted<'a> {
    Counted {
        inner: index,
        calls: RefCell::new(Vec::new()),
        fail_at,
        reading: RefCell::new(None),
    }
}

#[test]
fn the_index_is_read_once_for_every_list_and_not_at_all_after_an_error() {
    let mut log = Log::new();
    log.put(1, "lines", &["garden hose reel"]);
    log.put(2, "lines", &["garden hose coil"]);
    log.put(3, "lines", &["garden hose cart"]);
    log.put(4, "memo", &["garden hose"]);
    log.put(5, "memo", &["garden hose rake"]);
    log.put(6, "jots", &["garden hose"]);
    let index = index_with(&log, &speakers());
    let counting = counted(&index, None);
    let wanted = lists(&[("jots", 6), ("memo", 6), ("lines", 6)]);
    let found = ask_lists(
        &log,
        &counting,
        &wanted,
        "garden hose reel",
        &keys::defaults(),
    )
    .expect("a listing");
    assert_eq!(
        counting.reading.borrow().as_deref(),
        Some("lines"),
        "the largest kind reads first, though it is the last list"
    );
    for word in ["garden", "hose", "reel"] {
        assert_eq!(counting.asked(&format!("postings:{word}")), 1, "{word}");
    }
    assert_eq!(
        counting.asked("near:reel"),
        1,
        "the near terms of a word two lists lack"
    );
    for list in &found.lists {
        let alone = counted(&index, None);
        let one = ask_lists(
            &log,
            &alone,
            &[wanted
                .iter()
                .find(|each| each.kind == list.kind)
                .cloned()
                .unwrap_or_else(|| list_of(&list.kind, 6, 0.0))],
            "garden hose reel",
            &keys::defaults(),
        )
        .expect("a listing");
        assert_eq!(list.rows, one.lists[0].rows, "{}", list.kind);
    }

    let mut log = Log::new();
    for (n, text) in [
        (1, "garden hose coil"),
        (2, "garden hose cart"),
        (3, "garden hose rake"),
    ] {
        log.put(n, "lines", &[text]);
    }
    log.put(4, "memo", &["hose reel kept"]);
    log.put(5, "memo", &["hose reel here"]);
    let index = index_with(&log, &speakers());
    let counting = counted(&index, None);
    let wanted = lists(&[("lines", 6), ("memo", 6)]);
    let found =
        ask_lists(&log, &counting, &wanted, "hose reel", &keys::defaults()).expect("a listing");
    assert_eq!(
        counting.asked("pairs:hose:reel"),
        1,
        "asked by the smaller list"
    );
    let alone = counted(&index, None);
    let memo = ask_lists(
        &log,
        &alone,
        &lists(&[("memo", 6)]),
        "hose reel",
        &keys::defaults(),
    )
    .expect("a listing");
    assert_eq!(found.lists[1].rows, memo.lists[0].rows);
    assert!(!found.lists[1].rows.is_empty());
    let mut own_lines = Log::new();
    let mut own_memo = Log::new();
    for (n, text) in [
        (1, "garden hose coil"),
        (2, "garden hose cart"),
        (3, "garden hose rake"),
    ] {
        own_lines.put(n, "lines", &[text]);
    }
    own_memo.put(4, "memo", &["hose reel kept"]);
    own_memo.put(5, "memo", &["hose reel here"]);
    for (at, own) in [(0, &own_lines), (1, &own_memo)] {
        let plain_alone = plain(&index_with(own, &speakers()), "hose reel");
        assert!(!plain_alone.is_empty());
        assert_eq!(
            scores_of(
                found.lists[at]
                    .rows
                    .iter()
                    .map(|row| (row.entry, row.score))
            ),
            plain_alone,
            "list {at}: ids in order and score bits are the plain ranker's over its own kind"
        );
    }

    let mut log = Log::new();
    log.put(1, "jots", &["garden hose"]);
    log.put(2, "jots", &["garden hose reel"]);
    log.put(3, "memo", &["garden hose"]);
    log.put(4, "memo", &["garden hose rake"]);
    let index = index_with(&log, &speakers());
    let ties = |order: &[(&str, usize)]| {
        let counting = counted(&index, None);
        let lists = ask_lists(
            &log,
            &counting,
            &lists(order),
            "garden hose",
            &keys::defaults(),
        )
        .expect("a listing")
        .lists;
        let reading = counting.reading.borrow().clone();
        (lists, reading)
    };
    let (forward, first_forward) = ties(&[("jots", 6), ("memo", 6)]);
    let (backward, first_backward) = ties(&[("memo", 6), ("jots", 6)]);
    assert_eq!(forward[0].rows, backward[1].rows);
    assert_eq!(forward[1].rows, backward[0].rows);
    assert_eq!(first_forward.as_deref(), Some("jots"));
    assert_eq!(first_backward.as_deref(), Some("memo"));
    let whole_error = |error: Result<Listed, Error>| match error {
        Err(Error::Index(IndexError::Failed {
            action,
            path,
            error,
        })) => (action, path, error),
        Err(other) => panic!("not the index's own error: {other}"),
        Ok(_) => panic!("a listing"),
    };
    let refused = (
        "read",
        std::path::PathBuf::from("/c/index"),
        "refused".to_owned(),
    );

    let failing = counted(&index, Some(2));
    let error = ask_lists(
        &log,
        &failing,
        &lists(&[("jots", 6), ("memo", 6)]),
        "garden hose",
        &keys::defaults(),
    );
    assert_eq!(whole_error(error), refused);
    assert_eq!(failing.calls.borrow().len(), 2, "no call after the error");

    let mid = counted(&index, Some(6));
    let error = ask_lists(
        &log,
        &mid,
        &lists(&[("jots", 6), ("memo", 6)]),
        "garden hose",
        &keys::defaults(),
    );
    assert_eq!(whole_error(error), refused);
    assert_eq!(
        mid.calls.borrow().len(),
        6,
        "no call after an error of a ranking"
    );
}

fn kept_summary(entry: Id) -> CaptureLabel {
    CaptureLabel::new(format!("kept:{entry}")).expect("a label")
}

fn summary_text(record: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(record).trim_end().to_owned();
    (!text.is_empty()).then_some(text)
}

fn listed_summarised(
    watched: &Watched<'_>,
    index: &Docs,
    summary: Option<Summary<'_>>,
    wanted: &[List],
    text: &str,
) -> Vec<Chosen> {
    let setup = Ask {
        readers: speakers(),
        way: Way {
            label: &kept_label,
            written: &written_by,
            summary,
        },
        ..Ask::plain()
    };
    ask_read(&setup, watched, index, wanted, text, &keys::defaults())
        .expect("a listing")
        .lists
}

fn summary_of(chosen: &Chosen, entry: u8) -> Option<String> {
    match said(chosen, entry) {
        Said::Entry { summary, .. } | Said::Passage { summary, .. } => summary.clone(),
    }
}

#[test]
fn a_row_carries_the_summary_kept_for_its_entry() {
    let (mut log, _) = two_turns(&["1:1:3-7|the note"], false);
    log.put(2, "memo", &["# A garden notebook", "garden hose"]);
    log.put(3, "memo", &["# Another notebook", "garden hose too"]);
    log.put(21, "jots", &["The lines summary."]);
    log.label(21, &format!("kept:{}", id(1)));
    log.put(22, "jots", &["The notebook summary."]);
    log.label(22, &format!("kept:{}", id(2)));
    let index = index_with(&log, &speakers());
    let watched = Watched::over(&log);
    let way = Summary {
        label: &kept_summary,
        text: &summary_text,
    };
    let chosen = listed_summarised(
        &watched,
        &index,
        Some(way),
        &[list_of("lines", 6, 0.0)],
        "turn eight",
    );
    match said(&chosen[0], 1) {
        Said::Passage {
            written, summary, ..
        } => {
            assert_eq!(written.as_deref(), Some("the note"));
            assert_eq!(summary.as_deref(), Some("The lines summary."));
        }
        other => panic!("a row of a passage, got {other:?}"),
    }
    let chosen = listed_summarised(
        &watched,
        &index,
        Some(way),
        &[list_of("memo", 6, 0.0)],
        "garden hose",
    );
    assert!(matches!(said(&chosen[0], 2), Said::Entry { .. }));
    assert!(matches!(said(&chosen[0], 3), Said::Entry { .. }));
    assert_eq!(
        summary_of(&chosen[0], 2).as_deref(),
        Some("The notebook summary.")
    );
    assert_eq!(summary_of(&chosen[0], 3), None);
}

#[test]
fn the_last_record_of_the_summarys_entry_is_the_one_asked() {
    let (mut log, _) = two_turns(&[], false);
    log.put(
        21,
        "jots",
        &["first summary", "second summary", "third summary"],
    );
    log.label(21, &format!("kept:{}", id(1)));
    let index = index_with(&log, &speakers());
    let asked_of: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let blind = std::cell::Cell::new(false);
    let text = |record: &[u8]| {
        asked_of
            .borrow_mut()
            .push(String::from_utf8_lossy(record).trim_end().to_owned());
        if blind.get() && record.starts_with(b"third") {
            None
        } else {
            summary_text(record)
        }
    };
    let way = Summary {
        label: &kept_summary,
        text: &text,
    };
    let watched = Watched::over(&log);
    let chosen = listed_summarised(
        &watched,
        &index,
        Some(way),
        &[list_of("lines", 6, 0.0)],
        "turn eight",
    );
    assert_eq!(summary_of(&chosen[0], 1).as_deref(), Some("third summary"));
    assert_eq!(*asked_of.borrow(), ["third summary"]);
    let summary_reads: Vec<(Id, RecordRange)> = watched
        .reads
        .borrow()
        .iter()
        .filter(|(entry, _)| *entry == id(21))
        .copied()
        .collect();
    assert_eq!(
        summary_reads,
        [(id(21), RecordRange { first: 3, count: 1 })]
    );
    blind.set(true);
    asked_of.borrow_mut().clear();
    let chosen = listed_summarised(
        &Watched::over(&log),
        &index,
        Some(way),
        &[list_of("lines", 6, 0.0)],
        "turn eight",
    );
    assert_eq!(summary_of(&chosen[0], 1), None);
    assert_eq!(*asked_of.borrow(), ["third summary"]);
}

#[test]
fn with_no_way_to_a_summary_none_is_looked_up() {
    let (mut log, _) = two_turns(&["1:1:3-7|the note"], false);
    log.put(2, "memo", &["# A garden notebook", "garden hose"]);
    log.put(21, "jots", &["The lines summary."]);
    log.label(21, &format!("kept:{}", id(1)));
    log.put(22, "jots", &["The notebook summary."]);
    log.label(22, &format!("kept:{}", id(2)));
    let index = index_with(&log, &speakers());
    let watched = Watched::over(&log);
    let chosen = listed_summarised(
        &watched,
        &index,
        None,
        &[list_of("lines", 6, 0.0), list_of("memo", 6, 0.0)],
        "turn eight garden hose",
    );
    for list in &chosen {
        for row in &list.rows {
            let (Said::Entry { summary, .. } | Said::Passage { summary, .. }) = &row.said;
            assert_eq!(*summary, None);
        }
    }
    assert_eq!(ids(&chosen[0]), [id(1)]);
    assert_eq!(ids(&chosen[1]), [id(2)]);
    assert_eq!(*watched.lookups.borrow(), [format!("note:{}", id(1))]);
    assert!(
        watched
            .reads
            .borrow()
            .iter()
            .all(|(entry, _)| *entry != id(21) && *entry != id(22))
    );
}

#[test]
fn a_lookup_or_a_read_of_a_summary_that_fails_gives_the_row_without_it_and_counts_nothing() {
    let (mut log, _) = two_turns(&[], true);
    let before = index_with(&log, &speakers());
    let way = Summary {
        label: &kept_summary,
        text: &summary_text,
    };
    let control = listed_summarised(
        &Watched::over(&log),
        &before,
        Some(way),
        &[list_of("lines", 6, 0.0)],
        "turn eight",
    )
    .remove(0);
    assert_eq!(summary_of(&control, 1), None);
    log.put(21, "jots", &["The first summary."]);
    log.label(21, &format!("kept:{}", id(1)));
    log.put(22, "jots", &["The second summary."]);
    log.label(22, &format!("kept:{}", id(2)));
    let index = index_with(&log, &speakers());
    let whole = listed_summarised(
        &Watched::over(&log),
        &index,
        Some(way),
        &[list_of("lines", 6, 0.0)],
        "turn eight",
    )
    .remove(0);
    assert_eq!(summary_of(&whole, 1).as_deref(), Some("The first summary."));
    for (lookup_fails, read_fails) in [(true, None), (false, Some(id(21)))] {
        let mut watched = Watched::over(&log);
        watched.capture_lookup_fails = lookup_fails.then(|| format!("kept:{}", id(1)));
        watched.read_fails = read_fails;
        let chosen = listed_summarised(
            &watched,
            &index,
            Some(way),
            &[list_of("lines", 6, 0.0)],
            "turn eight",
        )
        .remove(0);
        assert_eq!(summary_of(&chosen, 1), None);
        assert_eq!(
            summary_of(&chosen, 2).as_deref(),
            Some("The second summary.")
        );
        let mut without = chosen.rows.clone();
        for row in &mut without {
            let (Said::Entry { summary, .. } | Said::Passage { summary, .. }) = &mut row.said;
            *summary = None;
        }
        assert_eq!(without, control.rows);
        assert_eq!(
            (
                chosen.gone,
                chosen.own,
                chosen.belongs,
                chosen.old,
                chosen.part,
                chosen.shown
            ),
            (
                control.gone,
                control.own,
                control.belongs,
                control.old,
                control.part,
                control.shown
            )
        );
    }
}

#[test]
fn a_summary_is_looked_up_for_listed_rows_alone_and_once_each() {
    const UUID: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";
    let mut log = Log::new();
    log.put(1, "lines", &["garden hose reel"]);
    log.put(2, "lines", &["garden hose reel"]);
    log.put(3, "lines", &["garden"]);
    log.label(1, &format!("file:/p/proj/{UUID}.jsonl"));
    for (n, summary) in [(1u8, 21u8), (2, 22), (3, 23)] {
        log.put(summary, "jots", &["a summary"]);
        log.label(summary, &format!("kept:{}", id(n)));
    }
    let index = index_with(&log, &speakers());
    let scores = {
        let watched = Watched::over(&log);
        let chosen = listed_summarised(
            &watched,
            &index,
            None,
            &[list_of("lines", 6, 0.0)],
            "garden hose reel",
        )
        .remove(0);
        chosen
            .rows
            .iter()
            .map(|row| (row.entry, f64::from(row.score)))
            .collect::<Vec<_>>()
    };
    let high = scores
        .iter()
        .find(|(entry, _)| *entry == id(2))
        .expect("two")
        .1;
    let low = scores
        .iter()
        .find(|(entry, _)| *entry == id(3))
        .expect("three")
        .1;
    assert!(high > low);
    let floor = (high + low) / 2.0;
    let labels: RefCell<Vec<Id>> = RefCell::new(Vec::new());
    let label = |entry: Id| {
        labels.borrow_mut().push(entry);
        kept_summary(entry)
    };
    let way = Summary {
        label: &label,
        text: &summary_text,
    };
    let watched = Watched::over(&log);
    let setup = Ask {
        readers: speakers(),
        thread: UUID,
        labels: &watched,
        way: Way {
            label: &kept_label,
            written: &|_| None,
            summary: Some(way),
        },
        ..Ask::plain()
    };
    let chosen = ask_read(
        &setup,
        &watched,
        &index,
        &[list_of("lines", 6, floor)],
        "garden hose reel",
        &keys::defaults(),
    )
    .expect("a listing")
    .lists
    .remove(0);
    assert_eq!(ids(&chosen), [id(2)]);
    assert_eq!((chosen.own, chosen.held_back.len()), (1, 1));
    assert_eq!(*labels.borrow(), [id(2)]);
    let summary_lookups: Vec<String> = watched
        .lookups
        .borrow()
        .iter()
        .filter(|label| label.starts_with("kept:"))
        .cloned()
        .collect();
    assert_eq!(summary_lookups, [format!("kept:{}", id(2))]);
    let reads: Vec<(Id, RecordRange)> = watched
        .reads
        .borrow()
        .iter()
        .filter(|(entry, _)| [id(21), id(22), id(23)].contains(entry))
        .copied()
        .collect();
    assert_eq!(reads, [(id(22), RecordRange { first: 1, count: 1 })]);
}
