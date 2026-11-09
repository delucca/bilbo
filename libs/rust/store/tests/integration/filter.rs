use bilbo_store::entries::{Capture, Captured, Entries};
use xxhash_rust::xxh3;

use crate::scratch::{Scratch, ends, id_of, open, origin, read_records};

fn base64(length: usize) -> String {
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        .chars()
        .cycle()
        .take(length)
        .collect()
}

fn capture(store: &mut bilbo_store::segments::Store, bytes: &[u8]) -> Captured {
    let origin = origin("s.jsonl");
    store
        .capture(&Capture {
            origin: &origin,
            kind: "session",
            id: None,
            parent: None,
            stamp: None,
            bytes,
            ends: &ends(bytes),
        })
        .expect("capture")
}

#[test]
fn an_opaque_string_is_kept_as_its_placeholder_and_its_line_keeps_its_number() {
    let scratch = Scratch::new("filter");
    let mut store = open(&scratch.layout());
    let image = base64(900);
    let sha = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b";
    let lines = [
        "{\"type\":\"user\",\"text\":\"plant the beans\"}\n".to_owned(),
        format!("{{\"type\":\"user\",\"data\":\"{image}\"}}\n"),
        format!("{{\"type\":\"assistant\",\"commit\":\"{sha}\"}}\n"),
    ];
    let bytes = lines.concat();
    let captured = capture(&mut store, bytes.as_bytes());
    let Captured::New(kept) = captured else {
        panic!("expected New, got {captured:?}");
    };
    assert_eq!(kept.raw_bytes, bytes.len() as u64);
    let records = store
        .read(
            kept.id,
            bilbo_store::entries::Generation(1),
            bilbo_store::entries::RecordRange { first: 1, count: 3 },
        )
        .expect("read");
    let placeholder = format!(
        "[opaque: 900 chars, xxh3 {:016x}]",
        xxh3::xxh3_64(image.as_bytes())
    );
    assert_eq!(records[0], lines[0].as_bytes());
    assert_eq!(
        records[1],
        format!("{{\"type\":\"user\",\"data\":\"{placeholder}\"}}\n").as_bytes()
    );
    assert_eq!(records[2], lines[2].as_bytes(), "a commit SHA passes");

    assert!(matches!(
        capture(&mut store, bytes.as_bytes()),
        Captured::Unchanged(_)
    ));
    let grown = format!(
        "{bytes}{{\"type\":\"user\",\"more\":\"{}\"}}\n",
        base64(300)
    );
    let appended = capture(&mut store, grown.as_bytes());
    assert!(matches!(appended, Captured::Appended(_)), "{appended:?}");
    let all = read_records(&store, id_of(&appended), 1, 4);
    assert!(
        all.ends_with(
            format!(
                "{{\"type\":\"user\",\"more\":\"[opaque: 300 chars, xxh3 {:016x}]\"}}\n",
                xxh3::xxh3_64(base64(300).as_bytes())
            )
            .as_bytes()
        )
    );
}

#[test]
fn a_markdown_line_passes_the_filter_whatever_it_holds() {
    let scratch = Scratch::new("filter_markdown");
    let mut store = open(&scratch.layout());
    let text = format!("# notes\n\"{}\" is a key\n", base64(400));
    let id = id_of(&capture(&mut store, text.as_bytes()));
    assert_eq!(read_records(&store, id, 1, 2), text.as_bytes());
}
