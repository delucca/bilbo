use bilbo_store::entries::{Capture, Captured, Entries};
use bilbo_store::id::Id;
use bilbo_store::labels::{CaptureLabel, Tags};
use bilbo_store::layout::Layout;
use bilbo_store::segments::{Options, Store};

pub fn kept(layout: &Layout, parent: Id, text: &str) -> Id {
    let record = format!(
        "{{\"summary\":{{\"text\":\"{text}\",\"instruction\":\"7fdf9f0d93f443e3\",\"by\":{{\"provider\":\"anthropic\",\"model\":\"claude-test-1\"}},\"made\":1,\"of\":{{\"entry\":\"{parent}\",\"generation\":1,\"records\":1,\"content\":\"00000000000000000000000000000000\"}},\"sent\":1}}}}\n"
    );
    let label = CaptureLabel::new(format!("summary:{parent}")).expect("a capture label");
    let mut store = Store::open(layout, Options::default(), &mut |_| {}).expect("open the store");
    let captured = store
        .capture(&Capture {
            label: &label,
            kind: "summary",
            id: None,
            id_label: None,
            stamp: None,
            bytes: record.as_bytes(),
            ends: &[record.len()],
            tags: &Tags::default(),
        })
        .expect("capture");
    store.close().expect("close");
    match captured {
        Captured::New(kept) | Captured::Appended(kept) | Captured::NewGeneration(kept) => kept.id,
        Captured::Unchanged(id) => id,
    }
}
