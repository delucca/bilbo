use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bilbo_store::layout::Layout;
use bilbo_store::segments::{Closed, Options, Store};

use crate::scratch::{Scratch, keep, open_seg, open_with, sealed};

fn since(layout: &Layout) -> std::path::PathBuf {
    layout.state.join("open.since")
}

fn closed(store: Store) {
    assert!(matches!(store.close().expect("close"), Closed::Released));
}

fn backdate(layout: &Layout, ago: Duration) {
    let at = SystemTime::now() - ago;
    let millis = at
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_millis();
    fs::write(since(layout), format!("{millis}\n")).expect("backdate");
}

#[test]
fn a_close_before_the_age_and_under_the_size_does_not_seal() {
    let scratch = Scratch::new("age_young");
    let layout = scratch.layout();
    let mut store = open_with(&layout, Options::default());
    keep(&mut store, "a.md", b"one\n");
    closed(store);
    assert!(sealed(&layout).is_empty());
    assert!(fs::metadata(open_seg(&layout)).expect("open.seg").len() > 12);
    assert!(since(&layout).exists(), "the first frame records its time");

    let mut store = open_with(&layout, Options::default());
    keep(&mut store, "b.md", b"two\n");
    closed(store);
    assert!(sealed(&layout).is_empty(), "a young open.seg stays open");
}

#[test]
fn a_close_after_the_age_seals_and_the_age_counts_from_the_first_frame() {
    let scratch = Scratch::new("age_old");
    let layout = scratch.layout();
    let mut store = open_with(&layout, Options::default());
    keep(&mut store, "a.md", b"one\n");
    closed(store);
    backdate(&layout, Duration::from_secs(61 * 60));
    let recorded = fs::read(since(&layout)).expect("the record");

    let mut store = open_with(&layout, Options::default());
    keep(&mut store, "b.md", b"two\n");
    assert_eq!(
        fs::read(since(&layout)).expect("the record"),
        recorded,
        "a later frame keeps the first frame's time"
    );
    closed(store);
    assert_eq!(sealed(&layout).len(), 1);
    assert_eq!(fs::metadata(open_seg(&layout)).expect("open.seg").len(), 12);
    assert!(!since(&layout).exists(), "a seal drops the record");
}

#[test]
fn a_lost_or_torn_age_record_counts_from_the_next_open() {
    for torn in [None, Some(&b"12"[..]), Some(&b"x\n"[..])] {
        let scratch = Scratch::new("age_lost");
        let layout = scratch.layout();
        let mut store = open_with(&layout, Options::default());
        keep(&mut store, "a.md", b"one\n");
        closed(store);
        match torn {
            None => fs::remove_file(since(&layout)).expect("lose the record"),
            Some(bytes) => fs::write(since(&layout), bytes).expect("tear the record"),
        }
        closed(open_with(&layout, Options::default()));
        assert!(sealed(&layout).is_empty(), "{torn:?}");
        let text = fs::read_to_string(since(&layout)).expect("written again");
        assert!(
            text.ends_with('\n') && text.trim_end().parse::<i64>().is_ok(),
            "{torn:?}"
        );
    }
}

#[test]
fn the_size_still_seals_a_young_open_seg() {
    let scratch = Scratch::new("age_size");
    let layout = scratch.layout();
    let mut store = open_with(
        &layout,
        Options {
            seal_at: 1,
            ..Options::default()
        },
    );
    keep(&mut store, "a.md", b"one\n");
    closed(store);
    assert_eq!(sealed(&layout).len(), 1);
}

#[test]
fn a_record_from_the_future_counts_from_the_next_open() {
    let day_ahead = SystemTime::now() + Duration::from_secs(24 * 60 * 60);
    let ahead = day_ahead
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_millis()
        .to_string();
    for record in [ahead, i64::MAX.to_string()] {
        let scratch = Scratch::new("age_future");
        let layout = scratch.layout();
        let mut store = open_with(&layout, Options::default());
        keep(&mut store, "a.md", b"one\n");
        closed(store);

        fs::write(since(&layout), format!("{record}\n")).expect("write the record");
        closed(open_with(&layout, Options::default()));
        assert!(sealed(&layout).is_empty(), "{record}");
        let text = fs::read_to_string(since(&layout)).expect("written again");
        let millis: u128 = text.trim_end().parse().expect("a time");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("after 1970")
            .as_millis();
        assert!(millis <= now, "{record}: {millis} is after {now}");

        backdate(&layout, Duration::from_secs(61 * 60));
        closed(open_with(&layout, Options::default()));
        assert_eq!(sealed(&layout).len(), 1, "{record}");
    }
}
