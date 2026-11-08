use std::io::Read;

use xxhash_rust::xxh3;

use crate::id::Id;

pub const MAGIC: &[u8; 8] = b"BILBOSEG";
pub const FORMAT: u32 = 1;
pub const SEGMENT_HEADER: usize = 12;
pub const FRAME_HEADER: usize = 77;
const MIN_LENGTH: u32 = 73;

pub fn segment_header() -> [u8; SEGMENT_HEADER] {
    let mut header = [0u8; SEGMENT_HEADER];
    header[..8].copy_from_slice(MAGIC);
    header[8..].copy_from_slice(&FORMAT.to_le_bytes());
    header
}

#[derive(Debug, PartialEq, Eq)]
pub enum HeaderState {
    Whole,
    /// Shorter than a header and a prefix of one.
    Torn,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HeaderFault {
    Bad(String),
    Newer(u32),
}

pub fn check_segment_header(bytes: &[u8]) -> Result<HeaderState, HeaderFault> {
    if bytes.len() < SEGMENT_HEADER {
        return if segment_header().starts_with(bytes) {
            Ok(HeaderState::Torn)
        } else {
            Err(HeaderFault::Bad("not a bilbo segment".into()))
        };
    }
    if &bytes[..8] != MAGIC {
        return Err(HeaderFault::Bad("not a bilbo segment".into()));
    }
    let version = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    match version {
        FORMAT => Ok(HeaderState::Whole),
        0 => Err(HeaderFault::Bad("segment format 0".into())),
        _ => Err(HeaderFault::Newer(version)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Meta,
    Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    pub entry: Id,
    pub generation: u32,
    pub first: u64,
    pub count: u64,
    pub end: u64,
    pub hash: u128,
}

impl Header {
    pub fn meta(entry: Id) -> Header {
        Header {
            kind: Kind::Meta,
            entry,
            generation: 0,
            first: 0,
            count: 0,
            end: 0,
            hash: 0,
        }
    }

    pub fn body(
        entry: Id,
        generation: u32,
        first: u64,
        count: u64,
        end: u64,
        hash: u128,
    ) -> Header {
        Header {
            kind: Kind::Body,
            entry,
            generation,
            first,
            count,
            end,
            hash,
        }
    }
}

/// XXH3-64 of the whole frame but its own field.
fn checksum(frame: &[u8]) -> u64 {
    let mut hasher = xxh3::Xxh3::new();
    hasher.update(&frame[..8]);
    hasher.update(&frame[16..]);
    hasher.digest()
}

/// The low 32 bits of XXH3-64 over the length and the fields from the type on: it lets a
/// search for a frame after damage reject a position by hashing 77 bytes, not a payload.
fn header_sum(frame: &[u8]) -> u32 {
    let mut hasher = xxh3::Xxh3::new();
    hasher.update(&frame[..4]);
    hasher.update(&frame[16..FRAME_HEADER]);
    hasher.digest() as u32
}

/// `None` when the frame would not fit the 32-bit length field.
pub fn encode(header: &Header, payload: &[u8]) -> Option<Vec<u8>> {
    let length = u32::try_from(payload.len() + (MIN_LENGTH as usize)).ok()?;
    let mut frame = Vec::with_capacity(FRAME_HEADER + payload.len());
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(&[0u8; 12]);
    frame.push(match header.kind {
        Kind::Meta => 1,
        Kind::Body => 2,
    });
    frame.extend_from_slice(&header.entry.to_bytes());
    frame.extend_from_slice(&header.generation.to_le_bytes());
    frame.extend_from_slice(&header.first.to_le_bytes());
    frame.extend_from_slice(&header.count.to_le_bytes());
    frame.extend_from_slice(&header.end.to_le_bytes());
    frame.extend_from_slice(&header.hash.to_le_bytes());
    frame.extend_from_slice(payload);
    let head = header_sum(&frame);
    frame[4..8].copy_from_slice(&head.to_le_bytes());
    let sum = checksum(&frame);
    frame[8..16].copy_from_slice(&sum.to_le_bytes());
    Some(frame)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameFault {
    IncompleteLength,
    Below(u32),
    RunsPast(u32),
    Checksum,
    HeaderSum,
    /// A good checksum and a bad field: damaged, never torn.
    Field(String),
}

impl FrameFault {
    pub fn what(&self) -> String {
        match self {
            FrameFault::IncompleteLength => "incomplete length".into(),
            FrameFault::Below(length) => format!("length {length} below {MIN_LENGTH}"),
            FrameFault::RunsPast(length) => format!("length {length} runs past the end"),
            FrameFault::Checksum => "checksum mismatch".into(),
            FrameFault::HeaderSum => "header checksum mismatch".into(),
            FrameFault::Field(what) => what.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Frame<'a> {
    pub header: Header,
    pub payload: &'a [u8],
    pub total: usize,
}

fn le_u32(bytes: &[u8]) -> u32 {
    let mut raw = [0u8; 4];
    raw.copy_from_slice(&bytes[..4]);
    u32::from_le_bytes(raw)
}

fn le_u64(bytes: &[u8]) -> u64 {
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&bytes[..8]);
    u64::from_le_bytes(raw)
}

fn le_u128(bytes: &[u8]) -> u128 {
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&bytes[..16]);
    u128::from_le_bytes(raw)
}

/// Checks the frame that starts at `at`: its length, its checksum and its fields.
pub fn parse(bytes: &[u8], at: usize) -> Result<Frame<'_>, FrameFault> {
    let rest = bytes.get(at..).unwrap_or(&[]);
    if rest.len() < 4 {
        return Err(FrameFault::IncompleteLength);
    }
    let length = le_u32(rest);
    if length < MIN_LENGTH {
        return Err(FrameFault::Below(length));
    }
    let total = 4 + length as usize;
    if total > rest.len() {
        return Err(FrameFault::RunsPast(length));
    }
    let frame = &rest[..total];
    if header_sum(frame) != le_u32(&frame[4..]) {
        return Err(FrameFault::HeaderSum);
    }
    if checksum(frame) != le_u64(&frame[8..]) {
        return Err(FrameFault::Checksum);
    }
    let kind = match frame[16] {
        1 => Kind::Meta,
        2 => Kind::Body,
        other => return Err(FrameFault::Field(format!("unknown frame type {other}"))),
    };
    let mut entry = [0u8; 16];
    entry.copy_from_slice(&frame[17..33]);
    let header = Header {
        kind,
        entry: Id::from_bytes(entry),
        generation: le_u32(&frame[33..]),
        first: le_u64(&frame[37..]),
        count: le_u64(&frame[45..]),
        end: le_u64(&frame[53..]),
        hash: le_u128(&frame[61..]),
    };
    match kind {
        Kind::Meta => {
            if header.generation != 0
                || header.first != 0
                || header.count != 0
                || header.end != 0
                || header.hash != 0
            {
                return Err(FrameFault::Field(
                    "a meta frame carries record fields".into(),
                ));
            }
        }
        Kind::Body => {
            if header.generation < 1 || header.first < 1 {
                return Err(FrameFault::Field(
                    "a body frame needs a generation and a first record of at least 1".into(),
                ));
            }
        }
    }
    Ok(Frame {
        header,
        payload: &frame[FRAME_HEADER..],
        total,
    })
}

/// Frames from `from` on, ending after the first fault.
pub fn walk(bytes: &[u8], from: usize) -> Walk<'_> {
    Walk {
        bytes,
        at: from,
        done: false,
    }
}

pub struct Walk<'a> {
    bytes: &'a [u8],
    at: usize,
    done: bool,
}

impl<'a> Iterator for Walk<'a> {
    type Item = Result<(usize, Frame<'a>), (usize, FrameFault)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.at >= self.bytes.len() {
            return None;
        }
        let at = self.at;
        match parse(self.bytes, at) {
            Ok(frame) => {
                self.at += frame.total;
                Some(Ok((at, frame)))
            }
            Err(fault) => {
                self.done = true;
                Some(Err((at, fault)))
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Damage {
    pub offset: u64,
    pub what: String,
}

/// The end of the good frames of an open segment: the file's length when it is whole, else the
/// start of its torn last frame. A bad frame followed by a good last frame is damage, not a tear.
pub fn good_end(bytes: &[u8]) -> Result<usize, Damage> {
    for item in walk(bytes, SEGMENT_HEADER) {
        if let Err((at, fault)) = item {
            if matches!(fault, FrameFault::Field(_))
                || !matches!(
                    search_after(bytes, at, bytes.len().saturating_mul(4)),
                    Search::Nothing
                )
            {
                return Err(Damage {
                    offset: at as u64,
                    what: fault.what(),
                });
            }
            return Ok(at);
        }
    }
    Ok(bytes.len())
}

enum Search {
    Found,
    Nothing,
    /// Too many bytes to checksum: the safe answer is the same as for `Found`.
    OutOfBudget,
}

/// Looks for a frame that passes its checksum after `bad`, wherever it ends. A position is
/// rejected by its length, then by its 77-byte header sum; only a header that sums gets its frame
/// hashed. Rejecting a position hashes at most 77 bytes, so those checks are linear in the file
/// by construction and cost no budget; the frame bytes hashed after a header sum matches are
/// capped at `budget`, so a crafted file cannot make the search quadratic (random bytes pass the
/// header sum about once per 2^32 positions).
fn search_after(bytes: &[u8], bad: usize, mut budget: usize) -> Search {
    let len = bytes.len();
    for q in (bad + 1)..len {
        if q + FRAME_HEADER > len {
            break;
        }
        let length = le_u32(&bytes[q..]) as usize;
        if length < MIN_LENGTH as usize || q + 4 + length > len {
            continue;
        }
        if header_sum(&bytes[q..q + FRAME_HEADER]) != le_u32(&bytes[q + 4..]) {
            continue;
        }
        let cost = 4 + length;
        if cost > budget {
            return Search::OutOfBudget;
        }
        budget -= cost;
        if checksum(&bytes[q..q + cost]) == le_u64(&bytes[q + 8..]) {
            return Search::Found;
        }
    }
    Search::Nothing
}

pub fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value & 0x7F) as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

pub fn get_varint(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut value: u64 = 0;
    for shift in (0..70).step_by(7) {
        let byte = *bytes.get(*at)?;
        *at += 1;
        let low = u64::from(byte & 0x7F);
        if shift == 63 && low > 1 {
            return None;
        }
        value |= low << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// The records `ends` (offsets in `bytes`, the first starting at `from`) as a body payload.
pub fn pack_records(bytes: &[u8], ends: &[usize], from: usize) -> Vec<u8> {
    let mut plain = Vec::with_capacity(bytes.len() - from + ends.len());
    let mut start = from;
    for &end in ends {
        put_varint(&mut plain, (end - start) as u64);
        start = end;
    }
    plain.extend_from_slice(&bytes[from..]);
    plain
}

pub fn split_records(plain: &[u8], count: u64) -> Result<Vec<&[u8]>, String> {
    let mut at = 0;
    let mut lengths = Vec::new();
    for _ in 0..count {
        let length = get_varint(plain, &mut at).ok_or("the record lengths are cut short")?;
        if length == 0 {
            return Err("a record has length 0".into());
        }
        lengths.push(length);
        if lengths.len() > plain.len() {
            return Err("more records than bytes".into());
        }
    }
    let mut records = Vec::with_capacity(lengths.len());
    let mut start = at;
    for length in lengths {
        let end = usize::try_from(length)
            .ok()
            .and_then(|length| start.checked_add(length))
            .filter(|&end| end <= plain.len())
            .ok_or("the records are longer than the payload")?;
        records.push(&plain[start..end]);
        start = end;
    }
    if start != plain.len() {
        return Err("the payload has bytes beyond its records".into());
    }
    Ok(records)
}

/// Format 1 allows exactly one zstd frame that records its content size; `decode_all` and
/// `bulk::decompress` both accept frames concatenated after the first, so neither is used alone.
/// The output grows as data really decodes: the recorded size is a claim to check, never an
/// allocation to make.
pub fn unpack(payload: &[u8]) -> Result<Vec<u8>, String> {
    match zstd::zstd_safe::find_frame_compressed_size(payload) {
        Ok(size) if size == payload.len() => {}
        _ => return Err("the payload is not exactly one zstd frame".into()),
    }
    let Ok(Some(size)) = zstd::zstd_safe::get_frame_content_size(payload) else {
        return Err("the payload does not record its size".into());
    };
    let bad = || "the payload does not decompress".to_string();
    let decoder = zstd::stream::read::Decoder::with_buffer(payload)
        .map_err(|_| bad())?
        .single_frame();
    let mut plain = Vec::new();
    decoder
        .take(size.saturating_add(1))
        .read_to_end(&mut plain)
        .map_err(|_| bad())?;
    if plain.len() as u64 == size {
        Ok(plain)
    } else {
        Err(bad())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> Id {
        Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("id")
    }

    fn compressor() -> zstd::bulk::Compressor<'static> {
        zstd::bulk::Compressor::new(19).expect("compressor")
    }

    fn body(first: u64, text: &[u8]) -> Vec<u8> {
        let payload = compressor().compress(text).expect("pack");
        encode(&Header::body(id(), 1, first, 1, 5, 9), &payload).expect("encode")
    }

    fn file(frames: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = segment_header().to_vec();
        for frame in frames {
            bytes.extend_from_slice(frame);
        }
        bytes
    }

    fn resum(frame: &mut [u8]) {
        let head = header_sum(frame);
        frame[4..8].copy_from_slice(&head.to_le_bytes());
        let sum = checksum(frame);
        frame[8..16].copy_from_slice(&sum.to_le_bytes());
    }

    #[test]
    fn the_segment_header_is_the_magic_and_format_1() {
        let header = segment_header();
        assert_eq!(&header[..8], b"BILBOSEG");
        assert_eq!(&header[8..], &[1, 0, 0, 0]);
        assert_eq!(check_segment_header(&header), Ok(HeaderState::Whole));
    }

    #[test]
    fn a_frame_header_has_the_documented_layout() {
        let header = Header::body(
            id(),
            7,
            11,
            3,
            400,
            0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10,
        );
        let frame = encode(&header, b"payload").expect("encode");
        assert_eq!(frame.len(), 77 + 7);
        assert_eq!(le_u32(&frame[0..]), 73 + 7);
        let mut head = xxh3::Xxh3::new();
        head.update(&frame[0..4]);
        head.update(&frame[16..77]);
        assert_eq!(le_u32(&frame[4..]), head.digest() as u32);
        let mut hasher = xxh3::Xxh3::new();
        hasher.update(&frame[0..8]);
        hasher.update(&frame[16..]);
        assert_eq!(le_u64(&frame[8..]), hasher.digest());
        assert_eq!(frame[16], 2);
        assert_eq!(&frame[17..33], &id().to_bytes());
        assert_eq!(le_u32(&frame[33..]), 7);
        assert_eq!(le_u64(&frame[37..]), 11);
        assert_eq!(le_u64(&frame[45..]), 3);
        assert_eq!(le_u64(&frame[53..]), 400);
        assert_eq!(
            &frame[61..77],
            &0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10u128.to_le_bytes()
        );
        assert_eq!(&frame[77..], b"payload");
    }

    #[test]
    fn meta_and_body_frames_round_trip() {
        let meta = encode(&Header::meta(id()), b"m").expect("meta");
        let body = encode(&Header::body(id(), 2, 5, 4, 77, 88), b"b").expect("body");
        let bytes = file(&[meta.clone(), body.clone()]);
        let frames: Vec<_> = walk(&bytes, SEGMENT_HEADER)
            .collect::<Result<_, _>>()
            .expect("walk");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].0, 12);
        assert_eq!(frames[0].1.header, Header::meta(id()));
        assert_eq!(frames[0].1.payload, b"m");
        assert_eq!(frames[1].0, 12 + meta.len());
        assert_eq!(frames[1].1.header, Header::body(id(), 2, 5, 4, 77, 88));
        assert_eq!(frames[1].1.payload, b"b");
        assert_eq!(frames[1].1.total, body.len());
    }

    #[test]
    fn record_lengths_round_trip_through_varints() {
        for value in [1u64, 127, 128, 16383, 16384, u64::from(u32::MAX), u64::MAX] {
            let mut out = Vec::new();
            put_varint(&mut out, value);
            let mut at = 0;
            assert_eq!(get_varint(&out, &mut at), Some(value));
            assert_eq!(at, out.len());
        }
        let mut at = 0;
        assert_eq!(get_varint(&[0x80], &mut at), None);
        let mut at = 0;
        assert_eq!(get_varint(&[0xFF; 11], &mut at), None);
    }

    #[test]
    fn records_pack_and_split_back_to_the_same_bytes() {
        let bytes = b"ab\ncd\nef";
        let plain = pack_records(bytes, &[3, 6, 8], 0);
        let records = split_records(&plain, 3).expect("split");
        assert_eq!(records, vec![&b"ab\n"[..], b"cd\n", b"ef"]);
        let tail = pack_records(bytes, &[6, 8], 3);
        assert_eq!(
            split_records(&tail, 2).expect("split"),
            vec![&b"cd\n"[..], b"ef"]
        );
        assert!(split_records(&plain, 2).is_err());
        assert!(split_records(&plain, 4).is_err());
    }

    #[test]
    fn a_torn_last_frame_is_cut_wherever_the_file_ends() {
        let frames = [body(1, b"one"), body(2, b"two"), body(3, b"three")];
        let whole = file(&frames);
        let boundaries = [
            12,
            12 + frames[0].len(),
            12 + frames[0].len() + frames[1].len(),
            whole.len(),
        ];
        for length in 12..=whole.len() {
            let kept = boundaries
                .iter()
                .copied()
                .filter(|&b| b <= length)
                .max()
                .unwrap_or(12);
            assert_eq!(good_end(&whole[..length]), Ok(kept), "cut at {length}");
        }
    }

    #[test]
    fn a_bad_last_frame_is_a_torn_tail() {
        let first = body(1, b"one");
        let last = body(2, b"two");
        let keep = 12 + first.len();
        let mut flipped = file(&[first.clone(), last.clone()]);
        let at = flipped.len() - 1;
        flipped[at] ^= 0xFF;
        let mut huge = file(&[first.clone(), last.clone()]);
        huge[keep..keep + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut zero = file(&[first.clone(), last]);
        zero[keep..keep + 4].copy_from_slice(&0u32.to_le_bytes());
        let mut zeros = file(std::slice::from_ref(&first));
        zeros.extend_from_slice(&[0u8; 4096]);
        let mut stray = file(&[first]);
        stray.extend_from_slice(&[7, 7]);
        for (name, bytes) in [
            ("flipped", flipped),
            ("huge", huge),
            ("zero", zero),
            ("zeros", zeros),
            ("stray", stray),
        ] {
            assert_eq!(good_end(&bytes), Ok(keep), "{name}");
        }
    }

    #[test]
    fn a_bad_frame_before_a_good_last_frame_is_damaged_not_torn() {
        let frames = [body(1, b"one"), body(2, b"two"), body(3, b"three")];
        let second = 12 + frames[0].len();
        let damage = |what: &str| {
            Err(Damage {
                offset: second as u64,
                what: what.into(),
            })
        };

        let mut flipped = file(&frames);
        flipped[second + 80] ^= 0xFF;
        assert_eq!(good_end(&flipped), damage("checksum mismatch"));

        let mut huge = file(&frames);
        huge[second..second + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            good_end(&huge),
            damage(&format!("length {} runs past the end", u32::MAX))
        );

        let mut zero = file(&frames);
        zero[second..second + 4].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(good_end(&zero), damage("length 0 below 73"));

        let mut junk = file(&frames[..1]);
        junk.extend_from_slice(&[0xAA; 10]);
        junk.extend_from_slice(&frames[1]);
        junk.extend_from_slice(&frames[2]);
        assert_eq!(
            good_end(&junk),
            damage(&format!("length {} runs past the end", 0xAAAA_AAAAu32))
        );
    }

    #[test]
    fn the_search_after_damage_costs_linear_time() {
        let mut bytes = segment_header().to_vec();
        bytes.resize(SEGMENT_HEADER + 64 * 1024 * 1024, 1);
        let started = std::time::Instant::now();
        assert_eq!(good_end(&bytes), Ok(SEGMENT_HEADER));
        assert!(started.elapsed().as_secs() < 60, "{:?}", started.elapsed());
    }

    fn pseudo_random(len: usize, mut seed: u64) -> Vec<u8> {
        (0..len)
            .map(|_| {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (seed >> 56) as u8
            })
            .collect()
    }

    #[test]
    fn a_large_torn_frame_of_random_bytes_is_cut_and_every_good_frame_is_kept() {
        let good = [body(1, b"a"), body(2, b"b"), body(3, b"c")];
        let mut bytes = file(&good);
        let keep = bytes.len();
        let big = encode(
            &Header::body(id(), 1, 4, 1, 5, 9),
            &pseudo_random(8 * 1024 * 1024, 7),
        )
        .expect("encode");
        bytes.extend_from_slice(&big[..big.len() / 2]);
        let started = std::time::Instant::now();
        assert_eq!(good_end(&bytes), Ok(keep));
        assert!(started.elapsed().as_secs() < 30, "{:?}", started.elapsed());
    }

    #[test]
    fn crafted_headers_that_sum_run_the_search_out_of_budget_and_are_refused() {
        let len = 8 * 1024 * 1024;
        let mut bytes = segment_header().to_vec();
        bytes.resize(len, 0);
        let template = body(1, b"x");
        for slot in 0..8 {
            let at = 100 + slot * FRAME_HEADER;
            let mut header = template[..FRAME_HEADER].to_vec();
            header[..4].copy_from_slice(&((len - at - 4) as u32).to_le_bytes());
            let head = header_sum(&header);
            header[4..8].copy_from_slice(&head.to_le_bytes());
            bytes[at..at + FRAME_HEADER].copy_from_slice(&header);
        }
        let started = std::time::Instant::now();
        assert_eq!(
            good_end(&bytes),
            Err(Damage {
                offset: SEGMENT_HEADER as u64,
                what: "length 0 below 73".into()
            })
        );
        assert!(started.elapsed().as_secs() < 30, "{:?}", started.elapsed());
    }

    #[test]
    fn rejecting_headers_costs_no_budget() {
        let mut bytes = file(&[body(1, b"a")]);
        let keep = bytes.len();
        let big = encode(
            &Header::body(id(), 1, 2, 1, 5, 9),
            &pseudo_random(8 * 1024 * 1024, 11),
        )
        .expect("encode");
        bytes.extend_from_slice(&big[..big.len() / 2]);
        let plausible = ((keep + 1)..bytes.len())
            .filter(|&q| {
                q + FRAME_HEADER <= bytes.len() && {
                    let length = le_u32(&bytes[q..]) as usize;
                    length >= MIN_LENGTH as usize && q + 4 + length <= bytes.len()
                }
            })
            .count();
        let old_charge = plausible * FRAME_HEADER;
        let budget = old_charge / 2;
        assert!(budget > 0 && budget < old_charge, "{plausible} positions");
        assert!(matches!(
            search_after(&bytes, keep, budget),
            Search::Nothing
        ));
        assert_eq!(good_end(&bytes), Ok(keep));
    }

    #[test]
    fn a_last_frame_with_a_whole_header_that_does_not_sum_is_a_tear() {
        let frames = [body(1, b"one"), body(2, b"two")];
        let keep = file(&frames[..1]).len();
        let mut bad = frames[1].clone();
        bad[53] ^= 0xFF;
        assert_eq!(parse(&bad, 0).err(), Some(FrameFault::HeaderSum));
        let mut bytes = file(&frames[..1]);
        bytes.extend_from_slice(&bad);
        assert_eq!(good_end(&bytes), Ok(keep));
        let mut cut = file(&frames[..1]);
        cut.extend_from_slice(&bad[..FRAME_HEADER]);
        assert_eq!(good_end(&cut), Ok(keep));
    }

    #[test]
    fn a_header_torn_mid_way_is_a_tear() {
        let frames = [body(1, b"one"), body(2, b"two")];
        let keep = file(&frames[..1]).len();
        for cut in 1..FRAME_HEADER {
            let mut bytes = file(&frames[..1]);
            bytes.extend_from_slice(&frames[1][..cut]);
            assert_eq!(
                good_end(&bytes),
                Ok(keep),
                "cut {cut} bytes into the header"
            );
        }
    }

    #[test]
    fn a_header_sum_that_does_not_match_is_a_fault_even_when_the_checksum_does() {
        let mut frame = body(1, b"one");
        frame[4] ^= 0xFF;
        let sum = checksum(&frame);
        frame[8..16].copy_from_slice(&sum.to_le_bytes());
        let bytes = file(&[frame]);
        assert_eq!(parse(&bytes, 12).err(), Some(FrameFault::HeaderSum));
        assert_eq!(FrameFault::HeaderSum.what(), "header checksum mismatch");
    }

    #[test]
    fn a_good_header_with_a_bad_payload_before_a_good_frame_is_damaged() {
        let frames = [body(1, b"one"), body(2, b"two"), body(3, b"three")];
        let mut bytes = file(&frames);
        let second = 12 + frames[0].len();
        bytes[second + FRAME_HEADER] ^= 0xFF;
        assert_eq!(parse(&bytes, second).err(), Some(FrameFault::Checksum));
        assert_eq!(
            good_end(&bytes),
            Err(Damage {
                offset: second as u64,
                what: "checksum mismatch".into()
            })
        );
    }

    #[test]
    fn a_payload_never_chooses_its_own_allocation() {
        let mut claim = vec![0x28, 0xB5, 0x2F, 0xFD, 0xC0, 0x00];
        claim.extend_from_slice(&(1u64 << 62).to_le_bytes());
        claim.extend_from_slice(&[0x01, 0x00, 0x00]);
        assert_eq!(
            zstd::zstd_safe::find_frame_compressed_size(&claim),
            Ok(claim.len())
        );
        assert!(matches!(
            zstd::zstd_safe::get_frame_content_size(&claim),
            Ok(Some(size)) if size == 1u64 << 62
        ));
        assert_eq!(
            unpack(&claim),
            Err("the payload does not decompress".to_string())
        );
    }

    #[test]
    fn a_double_fault_with_a_good_frame_between_is_damaged_not_torn() {
        let frames = [body(1, b"a"), body(2, b"b"), body(3, b"c"), body(4, b"d")];
        let second = 12 + frames[0].len();
        let mut bytes = file(&frames);
        bytes[second + 80] ^= 0xFF;
        let last = bytes.len() - frames[3].len();
        bytes.truncate(last + 40);
        assert_eq!(
            good_end(&bytes),
            Err(Damage {
                offset: second as u64,
                what: "checksum mismatch".into()
            })
        );
        let mut only_torn = file(&frames[..3]);
        only_torn.extend_from_slice(&frames[3][..40]);
        assert_eq!(good_end(&only_torn), Ok(only_torn.len() - 40));
    }

    #[test]
    fn a_frame_with_a_good_checksum_and_a_bad_field_is_damaged() {
        let good = body(1, b"one");
        let mut unknown_type = good.clone();
        unknown_type[16] = 9;
        resum(&mut unknown_type);
        let mut generation_zero = good.clone();
        generation_zero[33..37].copy_from_slice(&0u32.to_le_bytes());
        resum(&mut generation_zero);
        let mut first_zero = good.clone();
        first_zero[37..45].copy_from_slice(&0u64.to_le_bytes());
        resum(&mut first_zero);
        let mut meta_with_count = encode(&Header::meta(id()), b"m").expect("meta");
        meta_with_count[45..53].copy_from_slice(&1u64.to_le_bytes());
        resum(&mut meta_with_count);
        for (name, frame) in [
            ("unknown type", unknown_type),
            ("generation 0", generation_zero),
            ("first 0", first_zero),
            ("meta with a count", meta_with_count),
        ] {
            let bytes = file(&[frame]);
            let damage = good_end(&bytes).expect_err(name);
            assert_eq!(damage.offset, 12, "{name}");
            assert!(
                matches!(parse(&bytes, 12), Err(FrameFault::Field(_))),
                "{name}"
            );
        }
    }

    #[test]
    fn a_payload_must_be_exactly_one_zstd_frame() {
        let one = zstd::bulk::compress(b"hello", 19).expect("compress");
        assert_eq!(unpack(&one).expect("one frame"), b"hello");
        assert_eq!(
            unpack(&compressor().compress(b"").expect("empty")).expect("empty"),
            b""
        );

        let mut two = one.clone();
        two.extend_from_slice(&zstd::bulk::compress(b"world", 19).expect("compress"));
        let mut trailing = one.clone();
        trailing.push(0);
        let no_size = zstd::stream::encode_all(&b"hello"[..], 19).expect("encode_all");
        let truncated = one[..one.len() - 2].to_vec();
        for (name, payload) in [
            ("two frames", two),
            ("a trailing byte", trailing),
            ("no content size", no_size),
            ("truncated", truncated),
            ("empty", Vec::new()),
        ] {
            let bytes =
                file(&[encode(&Header::body(id(), 1, 1, 1, 5, 9), &payload).expect("encode")]);
            assert!(parse(&bytes, 12).is_ok(), "{name}: the checksum is good");
            assert!(unpack(&payload).is_err(), "{name}");
        }
    }

    #[test]
    fn a_newer_format_is_newer_and_a_wrong_magic_is_damaged() {
        let mut newer = segment_header();
        newer[8..].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(check_segment_header(&newer), Err(HeaderFault::Newer(2)));
        let mut wrong = segment_header();
        wrong[0] = b'X';
        assert!(matches!(
            check_segment_header(&wrong),
            Err(HeaderFault::Bad(_))
        ));
        assert_eq!(
            check_segment_header(&segment_header()[..5]),
            Ok(HeaderState::Torn)
        );
        assert_eq!(check_segment_header(&[]), Ok(HeaderState::Torn));
        assert!(matches!(
            check_segment_header(b"NOPE"),
            Err(HeaderFault::Bad(_))
        ));
    }
}
