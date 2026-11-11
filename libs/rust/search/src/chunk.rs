use std::fmt;

use bilbo_kinds::reader::Passage;
use bilbo_kinds::tokens::Tokens;
use bilbo_store::entries::RecordRange;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub records: RecordRange,
    /// The window's number from 0 and how many its passage was cut into; `None` for packed passages.
    pub part: Option<(u32, u32)>,
    pub tokens: usize,
    pub text: String,
}

/// The tokenizer gave no usable tokens for a text that is not empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unsplit;

impl fmt::Display for Unsplit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the tokenizer could not split a text")
    }
}

impl std::error::Error for Unsplit {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    /// The most tokens a chunk holds.
    pub tokens: usize,
    /// The tokens two consecutive windows of one passage share.
    pub overlap: usize,
}

/// A text is never handed to the tokenizer whole above this: one call costs about 180 bytes of
/// memory a byte of text. The real tokenizer's longest ordinary token is 128 bytes and a chunk
/// holds at most 8,192 tokens, so no chunk of a text over this size could fit anyway.
const MAX_ENCODE_BYTES: usize = 1 << 20;

/// Consecutive passages packed while they fit, a passage over the size as windows of its own.
/// A passage with no text is in no chunk. A text the tokenizer cannot split is `Unsplit`.
pub fn chunks(
    passages: &[Passage],
    tokens: &dyn Tokens,
    size: Size,
) -> Result<Vec<Chunk>, Unsplit> {
    chunks_within(passages, tokens, size, MAX_ENCODE_BYTES)
}

fn chunks_within(
    passages: &[Passage],
    tokens: &dyn Tokens,
    size: Size,
    bound: usize,
) -> Result<Vec<Chunk>, Unsplit> {
    let mut chunks = Vec::new();
    let mut packed: Option<Chunk> = None;
    for passage in passages.iter().filter(|passage| !passage.text.is_empty()) {
        let over = passage.text.len() > bound;
        let count = if over {
            0
        } else {
            counted(tokens, &passage.text)?
        };
        if over || count > size.tokens {
            chunks.extend(packed.take());
            chunks.extend(windows(passage, tokens, size, bound)?);
            continue;
        }
        if let Some(open) = &mut packed {
            if open.text.len() + 1 + passage.text.len() <= bound {
                let joined = format!("{}\n{}", open.text, passage.text);
                let together = counted(tokens, &joined)?;
                if together <= size.tokens {
                    open.text = joined;
                    open.tokens = together;
                    open.records = through(open.records, passage.range);
                    continue;
                }
            }
            chunks.extend(packed.take());
        }
        packed = Some(Chunk {
            records: passage.range,
            part: None,
            tokens: count,
            text: passage.text.clone(),
        });
    }
    chunks.extend(packed);
    Ok(chunks)
}

fn counted(tokens: &dyn Tokens, text: &str) -> Result<usize, Unsplit> {
    match tokens.count(text) {
        0 if !text.is_empty() => Err(Unsplit),
        count => Ok(count),
    }
}

fn through(first: RecordRange, last: RecordRange) -> RecordRange {
    let end = (last.first + last.count).max(first.first + first.count);
    RecordRange {
        first: first.first,
        count: end - first.first,
    }
}

/// The index of the piece that holds byte `at`: the first piece for a byte before it.
fn piece_at(passage: &Passage, at: usize) -> usize {
    passage
        .pieces
        .partition_point(|piece| piece.start <= at)
        .saturating_sub(1)
}

/// The records of `passage` that hold the bytes `from..to` of its text.
fn records_of(passage: &Passage, from: usize, to: usize) -> RecordRange {
    let last = passage.ends.len().saturating_sub(1);
    let first = passage.ends.partition_point(|end| *end <= from).min(last);
    let through = passage
        .ends
        .partition_point(|end| *end < to)
        .clamp(first, last);
    RecordRange {
        first: passage.range.first + first as u64,
        count: if passage.ends.is_empty() {
            passage.range.count
        } else {
            (through - first) as u64 + 1
        },
    }
}

/// The byte ranges `text` is tokenized in: none longer than `bound`, in order, each ending at the
/// last line feed of its second half, or else on a character.
fn stretches(text: &str, bound: usize) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < text.len() {
        if text.len() - at <= bound {
            found.push((at, text.len()));
            break;
        }
        let hard = text.floor_char_boundary(at + bound);
        let half = at + bound / 2;
        let mut end = text.as_bytes()[half..hard]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(hard, |line_feed| half + line_feed + 1);
        if end == at {
            end = at + text[at..].chars().next().map_or(1, char::len_utf8);
        }
        found.push((at, end));
        at = end;
    }
    found
}

/// Windows of the passage's tokens, each starting `overlap` tokens before the last one's end. Every
/// window but the first opens with the label line of the piece it starts in, unless that line
/// leaves no room to move on. A window's edge inside a character moves out to the character, and a
/// window those bytes push over the size loses its last token until it fits. A text over `bound`
/// is windowed a stretch at a time, and no window crosses the end of a stretch.
fn windows(
    passage: &Passage,
    tokens: &dyn Tokens,
    size: Size,
    bound: usize,
) -> Result<Vec<Chunk>, Unsplit> {
    let found = cuts(passage, tokens, size, bound)?;
    let parts = u32::try_from(found.len()).unwrap_or(u32::MAX);
    Ok(found
        .into_iter()
        .zip(0..)
        .map(|(cut, part)| Chunk {
            records: records_of(passage, cut.from, cut.to),
            part: Some((part, parts)),
            tokens: cut.tokens,
            text: cut.text,
        })
        .collect())
}

/// A window and the bytes `from..to` of the passage's text it was cut from.
struct Cut {
    text: String,
    tokens: usize,
    from: usize,
    to: usize,
}

fn cuts(
    passage: &Passage,
    tokens: &dyn Tokens,
    size: Size,
    bound: usize,
) -> Result<Vec<Cut>, Unsplit> {
    let text = passage.text.as_str();
    let data = text.as_bytes();
    let label_of = |piece: usize| {
        passage
            .pieces
            .get(piece)
            .map_or("", |piece| piece.label.as_str())
    };
    let mut room: Vec<Option<Option<usize>>> = vec![None; passage.pieces.len().max(1)];
    let mut room_of = |piece: usize| -> Result<Option<usize>, Unsplit> {
        if let Some(found) = room[piece] {
            return Ok(found);
        }
        let label = label_of(piece);
        let cost = if label.len() >= bound {
            None
        } else {
            let cost = counted(tokens, &format!("{label}\n"))?;
            size.tokens
                .checked_sub(cost)
                .is_some_and(|width| width > size.overlap)
                .then_some(cost)
        };
        room[piece] = Some(cost);
        Ok(cost)
    };
    let width = room_of(piece_at(passage, 0))?
        .map_or(size.tokens, |cost| size.tokens - cost)
        .max(size.overlap + 1);
    let is_inside = |at: usize| at < data.len() && data[at] & 0xC0 == 0x80;
    let mut found: Vec<Cut> = Vec::new();
    for (begin, finish) in stretches(text, bound) {
        let ends = tokens.ends(&text[begin..finish]);
        if ends.last() != Some(&(finish - begin)) || ends.windows(2).any(|pair| pair[0] > pair[1]) {
            return Err(Unsplit);
        }
        let mut offsets = vec![begin];
        offsets.extend(ends.iter().map(|end| begin + end));
        let pieces = offsets.len() - 1;
        let mut start = 0;
        loop {
            let mut end = (start + width).min(pieces);
            let (body, count, from, to) = loop {
                let mut from = offsets[start];
                let mut to = offsets[end];
                while from > 0 && is_inside(from) {
                    from -= 1;
                }
                while is_inside(to) {
                    to += 1;
                }
                let piece = piece_at(passage, from);
                let label = label_of(piece);
                let labelled = from > 0 && room_of(piece)?.is_some();
                let body = if labelled {
                    format!("{label}\n{}", &text[from..to])
                } else {
                    text[from..to].to_owned()
                };
                let count = counted(tokens, &body)?;
                if count <= size.tokens || end <= start + 1 {
                    break (body, count, from, to);
                }
                end -= 1;
            };
            if from < to {
                found.push(Cut {
                    text: body,
                    tokens: count,
                    from,
                    to,
                });
            }
            if end >= pieces {
                break;
            }
            start = end.saturating_sub(size.overlap).max(start + 1);
        }
    }
    if found.is_empty() {
        return Err(Unsplit);
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use bilbo_kinds::reader::{Passage, Piece};
    use bilbo_kinds::tokens::Tokens;
    use bilbo_store::entries::RecordRange;

    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::{Chunk, Cut, MAX_ENCODE_BYTES, Size, Unsplit, stretches};

    fn chunks(passages: &[Passage], tokens: &dyn Tokens, size: Size) -> Vec<Chunk> {
        super::chunks(passages, tokens, size).expect("chunks")
    }

    /// A token every `0` bytes, wherever a character falls.
    struct Every(usize);

    impl Tokens for Every {
        fn ends(&self, text: &str) -> Vec<usize> {
            (1..=text.len().div_ceil(self.0))
                .map(|token| (token * self.0).min(text.len()))
                .collect()
        }
    }

    const SMALL: Size = Size {
        tokens: 12,
        overlap: 2,
    };

    fn range(first: u64, count: u64) -> RecordRange {
        RecordRange { first, count }
    }

    fn passage(record: u64, text: &str) -> Passage {
        let label = text.split_once(':').map_or(text, |(label, _)| label);
        labelled(range(record, 1), text, &[(0, &format!("{label}:"))])
    }

    fn labelled(range: RecordRange, text: &str, pieces: &[(usize, &str)]) -> Passage {
        Passage {
            range,
            text: text.to_owned(),
            pieces: pieces
                .iter()
                .map(|(start, label)| Piece {
                    start: *start,
                    label: (*label).to_owned(),
                })
                .collect(),
            ends: vec![text.len()],
            at: None,
            outline: None,
        }
    }

    fn packed(first: u64, count: u64, tokens: usize, text: &str) -> Chunk {
        Chunk {
            records: range(first, count),
            part: None,
            tokens,
            text: text.to_owned(),
        }
    }

    fn texts(chunks: &[Chunk]) -> Vec<&str> {
        chunks.iter().map(|chunk| chunk.text.as_str()).collect()
    }

    #[test]
    fn consecutive_passages_are_packed_while_they_fit() {
        let passages = [passage(1, "aaaa"), passage(2, "bbbb"), passage(4, "cccc")];
        assert_eq!(
            chunks(&passages, &Every(1), SMALL),
            [packed(1, 2, 9, "aaaa\nbbbb"), packed(4, 1, 4, "cccc")]
        );
        assert_eq!(chunks(&[], &Every(1), SMALL), []);
    }

    #[test]
    fn the_line_feed_between_two_passages_counts_toward_the_size() {
        let fits = [passage(1, "aaaa"), passage(2, "bbbbbbb")];
        assert_eq!(
            chunks(&fits, &Every(1), SMALL),
            [packed(1, 2, 12, "aaaa\nbbbbbbb")]
        );
        let over = [passage(1, "aaaa"), passage(2, "bbbbbbbb")];
        assert_eq!(
            chunks(&over, &Every(1), SMALL),
            [packed(1, 1, 4, "aaaa"), packed(2, 1, 8, "bbbbbbbb")]
        );
    }

    #[test]
    fn a_passage_of_exactly_the_size_is_one_chunk_and_one_token_more_is_windows() {
        let exact = chunks(&[passage(1, "u: 123456789")], &Every(1), SMALL);
        assert_eq!(exact, [packed(1, 1, 12, "u: 123456789")]);
        let over = chunks(&[passage(1, "u: 1234567890")], &Every(1), SMALL);
        assert_eq!(texts(&over), ["u: 123456", "u:\n567890"]);
        assert_eq!(over[0].part, Some((0, 2)));
        assert_eq!(over[1].part, Some((1, 2)));
    }

    #[test]
    fn a_packed_chunk_spans_from_its_first_record_to_its_last() {
        let passages = [
            labelled(range(1, 2), "aa", &[(0, "a")]),
            labelled(range(5, 3), "bb", &[(0, "b")]),
        ];
        assert_eq!(
            chunks(&passages, &Every(1), SMALL),
            [packed(1, 7, 5, "aa\nbb")]
        );
    }

    #[test]
    fn a_passage_with_no_text_is_in_no_chunk_and_breaks_none() {
        let passages = [passage(1, "aaaa"), passage(2, ""), passage(3, "bbbb")];
        assert_eq!(
            chunks(&passages, &Every(1), SMALL),
            [packed(1, 3, 9, "aaaa\nbbbb")]
        );
        assert_eq!(chunks(&[passage(1, "")], &Every(1), SMALL), []);
    }

    #[test]
    fn windows_share_the_overlap_and_every_later_one_opens_with_the_label_line() {
        let found = chunks(&[passage(7, "u: abcdefghijklmnopqrst")], &Every(1), SMALL);
        assert_eq!(
            texts(&found),
            ["u: abcdef", "u:\nefghijklm", "u:\nlmnopqrst"]
        );
        assert_eq!(
            found.iter().map(|chunk| chunk.tokens).collect::<Vec<_>>(),
            [9, 12, 12]
        );
        for (part, chunk) in found.iter().enumerate() {
            assert_eq!(chunk.part, Some((part as u32, 3)));
            assert_eq!(chunk.records, range(7, 1));
        }
    }

    #[test]
    fn a_window_over_the_size_closes_the_open_chunk_and_the_next_passage_starts_a_new_one() {
        let passages = [
            passage(1, "aaaa"),
            passage(2, "u: 1234567890"),
            passage(3, "bbbb"),
            passage(4, "cccc"),
        ];
        assert_eq!(
            texts(&chunks(&passages, &Every(1), SMALL)),
            ["aaaa", "u: 123456", "u:\n567890", "bbbb\ncccc"]
        );
    }

    #[test]
    fn a_window_takes_the_label_line_of_the_piece_it_starts_in_and_loses_tokens_to_fit_it() {
        let text = format!(
            "user: {}\ntool_result X:\n{}",
            "a".repeat(10),
            "b".repeat(30)
        );
        let passage = labelled(range(3, 1), &text, &[(0, "user:"), (17, "tool_result X:")]);
        let size = Size {
            tokens: 30,
            overlap: 2,
        };
        let found = chunks(&[passage], &Every(1), size);
        assert_eq!(
            texts(&found),
            [
                &text[..24],
                &format!("tool_result X:\n{}", &text[22..37]),
                &format!("tool_result X:\n{}", &text[35..50]),
                &format!("tool_result X:\n{}", &text[48..]),
            ]
        );
        assert_eq!(
            found.iter().map(|chunk| chunk.tokens).collect::<Vec<_>>(),
            [24, 30, 30, 29]
        );
    }

    #[test]
    fn a_window_that_starts_before_the_second_piece_keeps_the_first_label() {
        let text = format!("user: {}\ntl X:\nb", "a".repeat(20));
        let passage = labelled(range(1, 1), &text, &[(0, "user:"), (27, "tl X:")]);
        let found = chunks(&[passage], &Every(1), SMALL);
        let first = found
            .iter()
            .position(|chunk| chunk.text.starts_with("tl X:\n"))
            .expect("a window of the later piece");
        assert!(first >= 1);
        assert!(
            found[1..first]
                .iter()
                .all(|chunk| chunk.text.starts_with("user:\n"))
        );
        assert!(
            found[first..]
                .iter()
                .all(|chunk| chunk.text.starts_with("tl X:\n"))
        );
    }

    #[test]
    fn an_edge_inside_a_character_moves_out_and_a_window_it_pushes_over_loses_its_last_token() {
        let even = chunks(
            &[passage(1, &format!("u: {}", "é".repeat(10)))],
            &Every(1),
            SMALL,
        );
        assert_eq!(texts(&even), ["u: ééé", "u:\néééé", "u:\néééé", "u:\néé"]);
        assert_eq!(
            even.iter().map(|chunk| chunk.tokens).collect::<Vec<_>>(),
            [9, 11, 11, 7]
        );
        let odd = chunks(
            &[passage(1, &format!("u:{}", "é".repeat(10)))],
            &Every(1),
            SMALL,
        );
        assert_eq!(texts(&odd), ["u:éééé", "u:\néééé", "u:\néééé", "u:\néé"]);
        assert_eq!(odd[0].tokens, 10);
    }

    #[test]
    fn a_window_of_a_passage_of_several_records_names_the_records_its_bytes_are_in() {
        let mut section = labelled(range(10, 4), "u: abcdefghijklmnopqrstuv", &[(0, "u:")]);
        section.ends = vec![5, 10, 15, 25];
        let found = chunks(&[section], &Every(1), SMALL);
        assert_eq!(
            texts(&found),
            ["u: abcdef", "u:\nefghijklm", "u:\nlmnopqrst", "u:\nstuv"]
        );
        assert_eq!(
            found.iter().map(|chunk| chunk.records).collect::<Vec<_>>(),
            [range(10, 2), range(11, 3), range(12, 2), range(13, 1)]
        );
    }

    #[test]
    fn a_label_line_that_leaves_no_room_is_left_off_and_the_windows_still_end() {
        let long = format!("{}: {}", "l".repeat(20), "a".repeat(30));
        let found = chunks(&[passage(1, &long)], &Every(1), SMALL);
        assert_eq!(found[0].text, &long[..12]);
        assert_eq!(found[1].text, &long[10..22]);
        assert!(found.iter().all(|chunk| chunk.tokens <= 12));
        assert!(
            found
                .last()
                .is_some_and(|chunk| long.ends_with(&chunk.text))
        );

        let wide = Size {
            tokens: 12,
            overlap: 11,
        };
        let found = chunks(&[passage(1, "u: abcdefghijklmnopqrst")], &Every(1), wide);
        assert_eq!(found.len(), 12);
        assert_eq!(found[0].text, "u: abcdefghi");
        assert_eq!(found[11].text, "ijklmnopqrst");
    }

    #[test]
    fn a_tokenizer_that_cannot_split_a_text_fails_the_passages_and_never_panics() {
        struct Broken(Vec<usize>, usize);
        impl Tokens for Broken {
            fn ends(&self, _text: &str) -> Vec<usize> {
                self.0.clone()
            }
            fn count(&self, _text: &str) -> usize {
                self.1
            }
        }
        let text = "u: ééééé";
        for ends in [
            vec![],
            vec![4],
            vec![99, 99],
            vec![4, 2, 13],
            vec![0, 0, 0, 0],
        ] {
            let found = super::chunks(&[passage(1, text)], &Broken(ends.clone(), 99), SMALL);
            assert_eq!(found, Err(Unsplit), "{ends:?}");
        }
        let found = super::chunks(&[passage(1, "aaaa")], &Broken(vec![], 0), SMALL);
        assert_eq!(found, Err(Unsplit));
        assert_eq!(Unsplit.to_string(), "the tokenizer could not split a text");
    }

    #[test]
    fn a_later_label_that_leaves_no_room_is_left_off_its_windows_and_the_output_stays_small() {
        let long = "l".repeat(400);
        let text = format!("u: hi\n{long}: body");
        let passage = labelled(range(1, 1), &text, &[(0, "u:"), (6, &format!("{long}:"))]);
        let found = chunks(&[passage], &Every(1), SMALL);
        assert!(found.iter().all(|chunk| chunk.tokens <= 12));
        assert!(
            found
                .iter()
                .all(|chunk| !chunk.text.starts_with(&format!("{long}:\n")))
        );
        let total: usize = found.iter().map(|chunk| chunk.text.len()).sum();
        assert!(total < 3 * text.len(), "{total}");
        assert!(found.len() < text.len(), "{}", found.len());
    }

    #[test]
    fn a_label_that_fits_still_opens_its_window_beside_one_that_does_not() {
        let long = "l".repeat(20);
        let text = format!(
            "u: {}\n{long}: {}\nt: {}",
            "a".repeat(10),
            "b".repeat(30),
            "c".repeat(30)
        );
        let second = text.find(&long).expect("the second piece");
        let third = text.find("\nt:").expect("the third piece") + 1;
        let passage = labelled(
            range(1, 1),
            &text,
            &[(0, "u:"), (second, &format!("{long}:")), (third, "t:")],
        );
        let found = chunks(&[passage], &Every(1), SMALL);
        assert!(found.iter().all(|chunk| chunk.tokens <= 12));
        assert!(found.iter().any(|chunk| chunk.text.starts_with("t:\n")));
        assert!(
            found
                .iter()
                .all(|chunk| !chunk.text.starts_with(&format!("{long}:\n")))
        );
        assert!(found.iter().any(|chunk| chunk.text.starts_with("u:\n")));
    }

    /// A token a byte, noting the longest text `ends` and `count` were asked about and whether any
    /// text held `needle`.
    #[derive(Default)]
    struct Spy {
        longest_ends: AtomicUsize,
        longest_count: AtomicUsize,
        saw_needle: AtomicBool,
        needle: &'static str,
    }

    impl Spy {
        fn note(&self, text: &str) {
            if !self.needle.is_empty() && text.contains(self.needle) {
                self.saw_needle.store(true, Ordering::Relaxed);
            }
        }
    }

    impl Tokens for Spy {
        fn ends(&self, text: &str) -> Vec<usize> {
            self.note(text);
            self.longest_ends.fetch_max(text.len(), Ordering::Relaxed);
            (1..=text.len()).collect()
        }

        fn count(&self, text: &str) -> usize {
            self.note(text);
            self.longest_count.fetch_max(text.len(), Ordering::Relaxed);
            text.len()
        }
    }

    fn within(passages: &[Passage], tokens: &dyn Tokens, size: Size, bound: usize) -> Vec<Chunk> {
        super::chunks_within(passages, tokens, size, bound).expect("chunks")
    }

    #[test]
    fn packing_counts_the_joined_text_and_never_a_sum() {
        /// A token a byte, except that a line feed and the byte before it are one token when that
        /// byte is `.`, and a line feed and the byte after it are three tokens when that byte is `#`.
        struct Joined;
        impl Tokens for Joined {
            fn ends(&self, text: &str) -> Vec<usize> {
                let data = text.as_bytes();
                let mut ends = Vec::new();
                let mut at = 0;
                while at < data.len() {
                    if data[at] == b'.' && data.get(at + 1) == Some(&b'\n') {
                        at += 2;
                        ends.push(at);
                    } else if data[at] == b'\n' && data.get(at + 1) == Some(&b'#') {
                        ends.extend([at + 1, at + 2, at + 2]);
                        at += 2;
                    } else {
                        at += 1;
                        ends.push(at);
                    }
                }
                ends
            }
        }
        let size = Size {
            tokens: 9,
            overlap: 2,
        };
        let one = chunks(&[passage(1, "aaa."), passage(2, "bbbbb")], &Joined, size);
        assert_eq!(one, [packed(1, 2, 9, "aaa.\nbbbbb")]);
        let two = chunks(&[passage(1, "aaaa"), passage(2, "#bbb")], &Joined, size);
        assert_eq!(texts(&two), ["aaaa", "#bbb"]);
    }

    #[test]
    fn a_passage_over_the_bound_is_cut_at_line_feeds_and_no_longer_text_reaches_the_tokenizer() {
        let text: String = (0..10).map(|line| format!("line {line:04}xx\n")).collect();
        assert_eq!(text.len(), 120);
        let passage = labelled(range(1, 1), &text, &[(0, "l:")]);
        let spy = Spy::default();
        let found = within(std::slice::from_ref(&passage), &spy, SMALL, 32);
        assert!(spy.longest_ends.load(Ordering::Relaxed) <= 32);
        assert!(spy.longest_count.load(Ordering::Relaxed) <= 32 + "l:\n".len());
        let parts = u32::try_from(found.len()).expect("a few");
        for (at, chunk) in found.iter().enumerate() {
            assert_eq!(chunk.part, Some((u32::try_from(at).expect("a few"), parts)));
        }
        assert!(parts > 4);
        assert_covers(&text, &cuts_of(&passage, &spy, SMALL, 32), "ten lines");
        assert_eq!(
            stretches(&text, 32),
            (0..5).map(|at| (at * 24, at * 24 + 24)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_stretch_with_no_line_feed_ends_on_a_character() {
        let text = "é".repeat(100);
        let found = stretches(&text, 33);
        assert_eq!(found.first().map(|stretch| stretch.0), Some(0));
        assert_eq!(found.last().map(|stretch| stretch.1), Some(200));
        for (first, last) in &found {
            assert!(text.is_char_boundary(*first) && text.is_char_boundary(*last));
            assert!((1..=33).contains(&(last - first)));
        }
        assert!(found.windows(2).all(|pair| pair[0].1 == pair[1].0));
        let spy = Spy::default();
        let whole = passage(1, &text);
        within(std::slice::from_ref(&whole), &spy, SMALL, 33);
        assert!(spy.longest_ends.load(Ordering::Relaxed) <= 33);
        assert_covers(&text, &cuts_of(&whole, &spy, SMALL, 33), "no line feed");
    }

    #[test]
    fn a_line_feed_in_the_first_half_of_a_stretch_does_not_end_it() {
        let text = format!("aaaa\n{}", "b".repeat(35));
        assert_eq!(stretches(&text, 32), [(0, 32), (32, 40)]);
        let late = format!("{}\n{}", "a".repeat(20), "b".repeat(30));
        assert_eq!(stretches(&late, 32), [(0, 21), (21, 51)]);
    }

    #[test]
    fn the_first_window_of_a_later_stretch_opens_with_the_label_of_its_piece() {
        let text = format!("u: {}\ntl: {}\n", "a".repeat(20), "b".repeat(19));
        assert_eq!(stretches(&text, 32), [(0, 24), (24, 48)]);
        let passage = labelled(range(1, 1), &text, &[(0, "u:"), (24, "tl:")]);
        let found = within(&[passage], &Every(1), SMALL, 32);
        assert_eq!(found[0].text, &text[..9]);
        assert!(found[1..].iter().all(|chunk| chunk.text.contains(":\n")));
        let first = found
            .iter()
            .find(|chunk| chunk.text.starts_with("tl:\n"))
            .expect("a window of the later stretch");
        assert_eq!(first.text, format!("tl:\n{}", &text[24..32]));
        let before = found
            .iter()
            .position(|chunk| chunk.text.starts_with("tl:\n"))
            .expect("a window of the later stretch");
        assert!(found[before - 1].text.ends_with("aa\n"));
    }

    #[test]
    fn a_window_in_a_later_stretch_names_the_records_its_bytes_are_in() {
        let text: String = (0..4)
            .map(|n| format!("record {n} {}\n", "x".repeat(14)))
            .collect();
        assert_eq!(text.len(), 96);
        let mut section = labelled(range(10, 4), &text, &[(0, "u:")]);
        section.ends = vec![24, 48, 72, 96];
        let found = within(&[section], &Every(1), SMALL, 32);
        assert!(found.len() > 4);
        assert!(found.iter().all(|chunk| chunk.records.count == 1));
        let firsts: Vec<u64> = found.iter().map(|chunk| chunk.records.first).collect();
        assert!(firsts.windows(2).all(|pair| pair[0] <= pair[1]));
        for record in 0..4 {
            let mark = format!("record {record}");
            assert!(
                found
                    .iter()
                    .filter(|chunk| chunk.text.contains(&mark))
                    .all(|chunk| chunk.records.first == 10 + record),
                "{mark}"
            );
        }
        assert_eq!(firsts.last(), Some(&13));
    }

    #[test]
    fn a_passage_at_the_bound_is_chunked_as_before_and_one_byte_over_is_cut() {
        let wide = Size {
            tokens: 100,
            overlap: 2,
        };
        let spy = Spy::default();
        let at = within(&[passage(1, &"x".repeat(32))], &spy, wide, 32);
        assert_eq!(at.len(), 1);
        assert_eq!(at[0].part, None);
        let over = within(&[passage(1, &"x".repeat(33))], &spy, wide, 32);
        assert_eq!(
            over.iter().map(|chunk| chunk.part).collect::<Vec<_>>(),
            [Some((0, 2)), Some((1, 2))]
        );
        assert!(spy.longest_ends.load(Ordering::Relaxed) <= 32);
    }

    #[test]
    fn a_label_longer_than_the_bound_is_left_off_and_never_counted() {
        let label = "l".repeat(40);
        let text = "a".repeat(80);
        let passage = labelled(range(1, 1), &text, &[(0, &label)]);
        let spy = Spy {
            needle: "lll",
            ..Spy::default()
        };
        let found = within(std::slice::from_ref(&passage), &spy, SMALL, 32);
        assert!(!spy.saw_needle.load(Ordering::Relaxed));
        assert!(found.iter().all(|chunk| !chunk.text.contains("lll")));
        assert_covers(&text, &cuts_of(&passage, &spy, SMALL, 32), "a long label");
    }

    #[test]
    fn a_passage_over_the_bound_between_two_small_ones_closes_the_open_chunk_and_fails_nothing() {
        let big = format!("u: {}", "a".repeat(60));
        let passages = [
            passage(1, "aaaa"),
            passage(2, &big),
            passage(3, "bbbb"),
            passage(4, "cccc"),
        ];
        let found = within(&passages, &Every(1), SMALL, 32);
        assert_eq!(found[0], packed(1, 1, 4, "aaaa"));
        assert_eq!(found[found.len() - 1], packed(3, 2, 9, "bbbb\ncccc"));
        assert!(
            found[1..found.len() - 1]
                .iter()
                .all(|chunk| chunk.part.is_some())
        );
    }

    /// A token a byte, counting how many times it was handed exactly `needle`.
    struct Tally {
        needle: String,
        seen: AtomicUsize,
    }

    impl Tokens for Tally {
        fn ends(&self, text: &str) -> Vec<usize> {
            (1..=text.len()).collect()
        }

        fn count(&self, text: &str) -> usize {
            if text == self.needle {
                self.seen.fetch_add(1, Ordering::Relaxed);
            }
            text.len()
        }
    }

    #[test]
    fn a_long_label_is_counted_once_however_many_windows_start_in_its_piece() {
        let label = format!("{}:", "l".repeat(399));
        let text = format!("u: hi\n{label} {}", "b".repeat(1500));
        let passage = labelled(range(1, 1), &text, &[(0, "u:"), (6, &label)]);
        let tally = Tally {
            needle: format!("{label}\n"),
            seen: AtomicUsize::default(),
        };
        let found = within(&[passage], &tally, SMALL, MAX_ENCODE_BYTES);
        assert!(found.len() > 100, "{}", found.len());
        assert_eq!(tally.seen.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn the_piece_of_a_window_is_found_without_walking_the_pieces() {
        let text: String = (0..2000)
            .map(|at| char::from(b'a' + (at % 26) as u8))
            .collect();
        let labels: Vec<String> = (0..2000).map(|at| format!("{at}:")).collect();
        let pieces: Vec<(usize, &str)> = labels
            .iter()
            .enumerate()
            .map(|(at, label)| (at, label.as_str()))
            .collect();
        let passage = labelled(range(1, 1), &text, &pieces);
        let found = cuts_of(&passage, &Every(1), SMALL, MAX_ENCODE_BYTES);
        assert!(found.len() > 100);
        for cut in found.iter().filter(|cut| cut.from > 0) {
            let label = &pieces
                .iter()
                .rev()
                .find(|(start, _)| *start <= cut.from)
                .expect("a piece")
                .1;
            assert_eq!(cut.text, format!("{label}\n{}", &text[cut.from..cut.to]));
        }
    }

    /// Counting cannot show this: what was quadratic was hashing the label and walking the pieces,
    /// neither a call a fake sees. The bound is set where the quadratic versions take minutes.
    #[test]
    fn the_work_of_windows_does_not_grow_with_a_labels_length_or_the_number_of_pieces() {
        use std::time::{Duration, Instant};

        let label = "l".repeat(MAX_ENCODE_BYTES - 1);
        let text = "a".repeat(500_000);
        let long = labelled(range(1, 1), &text, &[(0, &label)]);
        let started = Instant::now();
        let found = chunks(&[long], &Every(1), SMALL);
        assert!(found.len() > 49_000, "{}", found.len());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );

        let text = "a".repeat(400_000);
        let pieces: Vec<(usize, &str)> = (0..text.len()).map(|at| (at, "p:")).collect();
        let many = labelled(range(1, 1), &text, &pieces);
        let started = Instant::now();
        let found = chunks(&[many], &Every(1), SMALL);
        assert!(found.len() > 39_000, "{}", found.len());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn two_passages_that_are_together_over_the_bound_are_not_joined_and_not_counted_together() {
        /// A token every 128 bytes, noting the longest text `count` was handed.
        struct Wide(AtomicUsize);
        impl Tokens for Wide {
            fn ends(&self, text: &str) -> Vec<usize> {
                Every(128).ends(text)
            }

            fn count(&self, text: &str) -> usize {
                self.0.fetch_max(text.len(), Ordering::Relaxed);
                text.len().div_ceil(128)
            }
        }
        let wide = Wide(AtomicUsize::default());
        let passages = [passage(1, &"a".repeat(40)), passage(2, &"b".repeat(40))];
        let size = Size {
            tokens: 10,
            overlap: 2,
        };
        let found = within(&passages, &wide, size, 64);
        assert_eq!(found.len(), 2);
        assert!(wide.0.load(Ordering::Relaxed) <= 64);
    }

    /// The windows of `passage` with the bytes of its text each was cut from.
    fn cuts_of(passage: &Passage, tokens: &dyn Tokens, size: Size, bound: usize) -> Vec<Cut> {
        super::cuts(passage, tokens, size, bound).expect("cuts")
    }

    /// Each window starts at or before where the ones before it ended, ends with the bytes it was
    /// cut from, and the last one ends the text.
    fn assert_covers(text: &str, cuts: &[Cut], case: &str) {
        let mut covered: usize = 0;
        for (at, cut) in cuts.iter().enumerate() {
            assert!(
                (at > 0 || cut.from == 0) && cut.from <= covered && cut.from < cut.to,
                "{case}: window {at} is {}..{} after byte {covered} of {text:?}",
                cut.from,
                cut.to
            );
            assert!(
                cut.text.ends_with(&text[cut.from..cut.to]),
                "{case}: {:?} is not from {}..{} of {text:?}",
                cut.text,
                cut.from,
                cut.to
            );
            covered = covered.max(cut.to);
        }
        assert_eq!(covered, text.len(), "{case}: {text:?}");
    }

    #[test]
    fn a_window_after_a_line_feed_under_an_empty_label_covers_the_line_feed() {
        let passage = labelled(range(1, 1), "a\nb", &[(0, "")]);
        let size = Size {
            tokens: 1,
            overlap: 0,
        };
        let found = cuts_of(&passage, &Every(1), size, MAX_ENCODE_BYTES);
        assert_eq!(
            found
                .iter()
                .map(|cut| (cut.from, cut.to))
                .collect::<Vec<_>>(),
            [(0, 1), (1, 2), (2, 3)]
        );
        assert_covers("a\nb", &found, "a line feed");
    }

    #[test]
    fn whatever_the_text_the_pieces_and_the_size_the_windows_end_and_cover_the_passage() {
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % bound as u64).unwrap_or(0)
        };
        let alphabet = ['a', ' ', '\n', 'é', '五', '🙂', ':'];
        for case in 0..4000 {
            let bounded = case % 2 == 1;
            let text: String = (0..next(120) + 1)
                .map(|_| alphabet[next(alphabet.len())])
                .collect();
            let mut pieces = vec![(0, "u:".to_owned())];
            for _ in 0..next(3) {
                pieces.push((next(text.len() + 4), "x".repeat(next(41))));
            }
            pieces.sort();
            let borrowed: Vec<(usize, &str)> = pieces
                .iter()
                .map(|(at, label)| (*at, label.as_str()))
                .collect();
            let mut passage = labelled(range(1, 3), &text, &borrowed);
            passage.ends = vec![next(text.len() + 1), next(text.len() + 1), text.len()];
            let size = Size {
                tokens: next(20) + 1,
                overlap: next(8),
            };
            let tokens = Every(next(4) + 1);
            let bound = if bounded {
                8 + next(57)
            } else {
                MAX_ENCODE_BYTES
            };
            let found = super::chunks_within(std::slice::from_ref(&passage), &tokens, size, bound)
                .expect("chunks");
            assert!(
                !found.is_empty() && found.len() <= text.len() + 1,
                "case {case}"
            );
            for chunk in &found {
                assert!(!chunk.text.is_empty(), "case {case}");
                assert!((1..=3).contains(&chunk.records.first), "case {case}");
                assert!(
                    chunk.records.first + chunk.records.count <= 4,
                    "case {case}"
                );
            }
            if found.iter().all(|chunk| chunk.part.is_none()) {
                assert_eq!(found.len(), 1, "case {case}");
                assert_eq!(found[0].text, text, "case {case}");
            } else {
                let cut = cuts_of(&passage, &tokens, size, bound);
                assert_covers(&text, &cut, &format!("case {case}"));
                assert_eq!(
                    cut.iter().map(|cut| &cut.text).collect::<Vec<_>>(),
                    found.iter().map(|chunk| &chunk.text).collect::<Vec<_>>(),
                    "case {case}"
                );
            }
            let total: usize = found.iter().map(|chunk| chunk.text.len()).sum();
            assert!(
                total <= 4 * text.len() + 64 * found.len(),
                "case {case}: {total} bytes from {}",
                text.len()
            );
            let last = &found[found.len() - 1].text;
            assert!(
                text.chars().last().is_some_and(|end| last.ends_with(end)),
                "case {case}: {last:?} does not end {text:?}"
            );
        }
    }
}
