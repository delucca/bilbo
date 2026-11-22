use bilbo_store::entries::{Capture, Entries};
use bilbo_store::id::Id;
use bilbo_store::labels::{CaptureLabel, Tags};
use bilbo_store::layout::Layout;
use bilbo_store::segments::{Options, Store};

pub fn noted(layout: &Layout, parent: Id, lines: &[Vec<u8>]) -> Id {
    let bytes: Vec<u8> = lines.concat();
    let mut ends = Vec::new();
    let mut end = 0;
    for line in lines {
        end += line.len();
        ends.push(end);
    }
    let label = CaptureLabel::new(format!("derive:{parent}")).expect("a capture label");
    let mut store = Store::open(layout, Options::default(), &mut |_| {}).expect("open the store");
    let captured = store
        .capture(&Capture {
            label: &label,
            kind: bilbo_cli::notes().kind,
            id: None,
            id_label: Some(parent),
            stamp: None,
            bytes: &bytes,
            ends: &ends,
            tags: &Tags::default(),
        })
        .expect("capture");
    store.close().expect("close");
    match captured {
        bilbo_store::entries::Captured::New(kept)
        | bilbo_store::entries::Captured::Appended(kept)
        | bilbo_store::entries::Captured::NewGeneration(kept) => kept.id,
        bilbo_store::entries::Captured::Unchanged(id) => id,
    }
}
