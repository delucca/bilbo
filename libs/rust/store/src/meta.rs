use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::entries::Meta;
use crate::id::Id;
use crate::labels::{CaptureLabel, StoredLabels};

const ID: u8 = 1;
const KIND: u8 = 2;
const ORIGIN: u8 = 3;
const CAPTURED: u8 = 4;
const PARENT: u8 = 5;

pub fn millis(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_millis()).map_or(i64::MIN, |ms| -ms),
    }
}

/// Nanoseconds from the epoch, saturated past 2262: a stamp only needs to compare equal.
pub fn nanos(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_nanos()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_nanos()).map_or(i64::MIN, |ns| -ns),
    }
}

pub fn time_of_nanos(nanos: i64) -> SystemTime {
    let magnitude = Duration::from_nanos(nanos.unsigned_abs());
    if nanos >= 0 {
        UNIX_EPOCH + magnitude
    } else {
        UNIX_EPOCH - magnitude
    }
}

pub fn time_of(millis: i64) -> SystemTime {
    let magnitude = Duration::from_millis(millis.unsigned_abs());
    if millis >= 0 {
        UNIX_EPOCH + magnitude
    } else {
        UNIX_EPOCH - magnitude
    }
}

fn field(out: &mut Vec<u8>, tag: u8, value: &[u8]) -> Option<()> {
    out.push(tag);
    out.extend_from_slice(&u32::try_from(value.len()).ok()?.to_le_bytes());
    out.extend_from_slice(value);
    Some(())
}

/// `None` when a field would not fit its 32-bit length.
pub fn encode(meta: &Meta) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    field(&mut out, ID, &meta.id.to_bytes())?;
    field(&mut out, KIND, meta.kind.as_bytes())?;
    if let Some(label) = &meta.labels.capture {
        field(&mut out, ORIGIN, label.as_str().as_bytes())?;
    }
    field(&mut out, CAPTURED, &millis(meta.captured).to_le_bytes())?;
    if let Some(label) = meta.labels.id {
        field(&mut out, PARENT, &label.to_bytes())?;
    }
    Some(out)
}

fn fields(bytes: &[u8]) -> Result<Vec<(u8, &[u8])>, String> {
    let mut found = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        if rest.len() < 5 {
            return Err("a field is cut short".into());
        }
        let length = u32::from_le_bytes([rest[1], rest[2], rest[3], rest[4]]) as usize;
        let value = rest
            .get(5..5 + length)
            .ok_or_else(|| "a field is cut short".to_string())?;
        found.push((rest[0], value));
        rest = &rest[5 + length..];
    }
    Ok(found)
}

fn text(value: &[u8]) -> Result<String, String> {
    String::from_utf8(value.to_vec()).map_err(|_| "a field is not UTF-8".to_string())
}

pub fn decode(entry: Id, bytes: &[u8]) -> Result<Meta, String> {
    let mut found = fields(bytes)?.into_iter().peekable();
    let id = match found.next() {
        Some((ID, value)) => {
            let raw: [u8; 16] = value.try_into().map_err(|_| "the id is not 16 bytes")?;
            Id::from_bytes(raw)
        }
        _ => return Err("the meta has no id first".into()),
    };
    if id != entry {
        return Err("the meta names another entry".into());
    }
    let kind = match found.next() {
        Some((KIND, value)) => text(value)?,
        _ => return Err("the meta has no kind after its id".into()),
    };
    let capture = match found.next_if(|(tag, _)| *tag == ORIGIN) {
        Some((_, value)) => {
            Some(CaptureLabel::new(text(value)?).ok_or("the capture label is empty")?)
        }
        None => None,
    };
    let captured = match found.next() {
        Some((CAPTURED, value)) => {
            let raw: [u8; 8] = value.try_into().map_err(|_| "captured is not 8 bytes")?;
            time_of(i64::from_le_bytes(raw))
        }
        _ => return Err("the meta has no captured time in its place".into()),
    };
    let id_label = match found.next() {
        Some((PARENT, value)) => {
            let raw: [u8; 16] = value
                .try_into()
                .map_err(|_| "the id label is not 16 bytes")?;
            Some(Id::from_bytes(raw))
        }
        Some((tag, _)) => return Err(format!("unexpected meta field {tag}")),
        None => None,
    };
    if let Some((tag, _)) = found.next() {
        return Err(format!("unexpected meta field {tag}"));
    }
    Ok(Meta {
        id,
        kind,
        labels: StoredLabels {
            capture,
            id: id_label,
        },
        captured,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::labels::{CaptureLabel, StoredLabels};

    fn id() -> Id {
        Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("id")
    }

    fn id_label() -> Id {
        Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DF").expect("id")
    }

    fn meta(capture: Option<CaptureLabel>, id_label: Option<Id>) -> Meta {
        Meta {
            id: id(),
            kind: "markdown".into(),
            labels: StoredLabels {
                capture,
                id: id_label,
            },
            captured: time_of(1_762_560_000_123),
        }
    }

    #[test]
    fn a_meta_round_trips_with_and_without_a_capture_label_and_an_id_label() {
        let capture = CaptureLabel::file(Path::new("/notes/a.md"));
        for meta in [
            meta(capture.clone(), Some(id_label())),
            meta(capture, None),
            meta(None, Some(id_label())),
            meta(None, None),
        ] {
            let bytes = encode(&meta).expect("encode");
            assert_eq!(decode(id(), &bytes), Ok(meta));
        }
        let before_epoch = Meta {
            captured: time_of(-5),
            ..meta(None, None)
        };
        let bytes = encode(&before_epoch).expect("encode");
        assert_eq!(decode(id(), &bytes), Ok(before_epoch));
    }

    #[test]
    fn the_meta_frame_is_the_bytes_it_always_was() {
        let both = meta(
            CaptureLabel::file(Path::new("/notes/a.md")),
            Some(id_label()),
        );
        let mut expected: Vec<u8> = vec![1, 16, 0, 0, 0];
        expected.extend_from_slice(&id().to_bytes());
        expected.extend_from_slice(&[2, 8, 0, 0, 0]);
        expected.extend_from_slice(b"markdown");
        expected.extend_from_slice(&[3, 16, 0, 0, 0]);
        expected.extend_from_slice(b"file:/notes/a.md");
        expected.extend_from_slice(&[4, 8, 0, 0, 0]);
        expected.extend_from_slice(&1_762_560_000_123i64.to_le_bytes());
        expected.extend_from_slice(&[5, 16, 0, 0, 0]);
        expected.extend_from_slice(&id_label().to_bytes());
        assert_eq!(encode(&both), Some(expected.clone()));
        assert_eq!(decode(id(), &expected), Ok(both));
        let neither = meta(None, None);
        let mut plain: Vec<u8> = vec![1, 16, 0, 0, 0];
        plain.extend_from_slice(&id().to_bytes());
        plain.extend_from_slice(&[2, 8, 0, 0, 0]);
        plain.extend_from_slice(b"markdown");
        plain.extend_from_slice(&[4, 8, 0, 0, 0]);
        plain.extend_from_slice(&1_762_560_000_123i64.to_le_bytes());
        assert_eq!(encode(&neither), Some(plain.clone()));
        assert_eq!(decode(id(), &plain), Ok(neither));
    }

    #[test]
    fn a_meta_with_an_unknown_field_or_a_missing_one_is_damaged() {
        let good = encode(&meta(
            CaptureLabel::file(Path::new("/n/a.md")),
            Some(id_label()),
        ))
        .expect("encode");
        let mut unknown = good.clone();
        unknown.extend_from_slice(&[9, 0, 0, 0, 0]);
        let mut out_of_order = Vec::new();
        field(&mut out_of_order, KIND, b"markdown");
        field(&mut out_of_order, ID, &id().to_bytes());
        let mut no_kind = Vec::new();
        field(&mut no_kind, ID, &id().to_bytes());
        field(&mut no_kind, CAPTURED, &0i64.to_le_bytes());
        let mut no_captured = Vec::new();
        field(&mut no_captured, ID, &id().to_bytes());
        field(&mut no_captured, KIND, b"markdown");
        let mut short_id = Vec::new();
        field(&mut short_id, ID, &[1, 2, 3]);
        let mut other_id = Vec::new();
        field(&mut other_id, ID, &[7; 16]);
        field(&mut other_id, KIND, b"k");
        field(&mut other_id, CAPTURED, &0i64.to_le_bytes());
        let mut bad_utf8 = Vec::new();
        field(&mut bad_utf8, ID, &id().to_bytes());
        field(&mut bad_utf8, KIND, &[0xFF]);
        field(&mut bad_utf8, CAPTURED, &0i64.to_le_bytes());
        let mut empty_capture = Vec::new();
        field(&mut empty_capture, ID, &id().to_bytes());
        field(&mut empty_capture, KIND, b"k");
        field(&mut empty_capture, ORIGIN, b"");
        field(&mut empty_capture, CAPTURED, &0i64.to_le_bytes());
        let mut repeated_kind = good.clone();
        field(&mut repeated_kind, KIND, b"again");
        let mut short_time = Vec::new();
        field(&mut short_time, ID, &id().to_bytes());
        field(&mut short_time, KIND, b"k");
        field(&mut short_time, CAPTURED, &[1, 2]);
        let cut = good[..good.len() - 1].to_vec();
        let mut trailing = good.clone();
        trailing.push(1);
        let mut two_id_labels = good.clone();
        field(&mut two_id_labels, PARENT, &id_label().to_bytes());
        let mut short_id_label = encode(&meta(None, None)).expect("encode");
        field(&mut short_id_label, PARENT, &[1, 2, 3]);
        for (name, bytes) in [
            ("two id labels", two_id_labels),
            ("short id label", short_id_label.clone()),
            ("unknown tag", unknown),
            ("out of order", out_of_order),
            ("no kind", no_kind),
            ("no captured", no_captured),
            ("short id", short_id),
            ("another entry's id", other_id),
            ("bad utf-8", bad_utf8),
            ("empty capture label", empty_capture.clone()),
            ("repeated kind", repeated_kind),
            ("short time", short_time),
            ("cut field", cut),
            ("trailing byte", trailing),
            ("empty", Vec::new()),
        ] {
            assert!(decode(id(), &bytes).is_err(), "{name}");
        }
        assert_eq!(
            decode(id(), &empty_capture),
            Err("the capture label is empty".to_string())
        );
        assert_eq!(
            decode(id(), &short_id_label),
            Err("the id label is not 16 bytes".to_string())
        );
    }
}
