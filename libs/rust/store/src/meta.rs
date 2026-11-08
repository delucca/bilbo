use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::entries::{Meta, Origin};
use crate::id::Id;

const ID: u8 = 1;
const KIND: u8 = 2;
const ORIGIN: u8 = 3;
const CAPTURED: u8 = 4;
const LABEL: u8 = 5;

pub fn millis(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_millis()).map_or(i64::MIN, |ms| -ms),
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
    if let Some(origin) = &meta.origin {
        field(&mut out, ORIGIN, origin.as_str().as_bytes())?;
    }
    field(&mut out, CAPTURED, &millis(meta.captured).to_le_bytes())?;
    for label in &meta.labels {
        field(&mut out, LABEL, label.as_bytes())?;
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
    let origin = match found.next_if(|(tag, _)| *tag == ORIGIN) {
        Some((_, value)) => Some(Origin::new(text(value)?).ok_or("the origin is empty")?),
        None => None,
    };
    let captured = match found.next() {
        Some((CAPTURED, value)) => {
            let raw: [u8; 8] = value.try_into().map_err(|_| "captured is not 8 bytes")?;
            time_of(i64::from_le_bytes(raw))
        }
        _ => return Err("the meta has no captured time in its place".into()),
    };
    let mut labels = Vec::new();
    for (tag, value) in found {
        if tag != LABEL {
            return Err(format!("unexpected meta field {tag}"));
        }
        labels.push(text(value)?);
    }
    Ok(Meta {
        id,
        kind,
        origin,
        captured,
        labels,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn id() -> Id {
        Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("id")
    }

    fn meta(origin: Option<Origin>, labels: &[&str]) -> Meta {
        Meta {
            id: id(),
            kind: "markdown".into(),
            origin,
            captured: time_of(1_762_560_000_123),
            labels: labels.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn a_meta_round_trips_with_and_without_an_origin_and_labels() {
        let origin = Origin::file(Path::new("/notes/a.md"));
        for meta in [
            meta(origin.clone(), &["p", "q"]),
            meta(origin, &[]),
            meta(None, &["only"]),
            meta(None, &[]),
        ] {
            let bytes = encode(&meta).expect("encode");
            assert_eq!(decode(id(), &bytes), Ok(meta));
        }
        let before_epoch = Meta {
            captured: time_of(-5),
            ..meta(None, &[])
        };
        let bytes = encode(&before_epoch).expect("encode");
        assert_eq!(decode(id(), &bytes), Ok(before_epoch));
    }

    #[test]
    fn a_meta_with_an_unknown_field_or_a_missing_one_is_damaged() {
        let good = encode(&meta(Origin::file(Path::new("/n/a.md")), &["p"])).expect("encode");
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
        let mut empty_origin = Vec::new();
        field(&mut empty_origin, ID, &id().to_bytes());
        field(&mut empty_origin, KIND, b"k");
        field(&mut empty_origin, ORIGIN, b"");
        field(&mut empty_origin, CAPTURED, &0i64.to_le_bytes());
        let mut repeated_kind = good.clone();
        field(&mut repeated_kind, KIND, b"again");
        let mut short_time = Vec::new();
        field(&mut short_time, ID, &id().to_bytes());
        field(&mut short_time, KIND, b"k");
        field(&mut short_time, CAPTURED, &[1, 2]);
        let cut = good[..good.len() - 1].to_vec();
        let mut trailing = good.clone();
        trailing.push(1);
        for (name, bytes) in [
            ("unknown tag", unknown),
            ("out of order", out_of_order),
            ("no kind", no_kind),
            ("no captured", no_captured),
            ("short id", short_id),
            ("another entry's id", other_id),
            ("bad utf-8", bad_utf8),
            ("empty origin", empty_origin),
            ("repeated kind", repeated_kind),
            ("short time", short_time),
            ("cut field", cut),
            ("trailing byte", trailing),
            ("empty", Vec::new()),
        ] {
            assert!(decode(id(), &bytes).is_err(), "{name}");
        }
    }
}
