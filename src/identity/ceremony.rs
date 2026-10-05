//! The recovery phrase ceremony, drawn through the `Prompter` on the phrase's own screen.

use crate::host::prompt::{Choice, Prompter};
use crate::identity::phrase::{self, Phrase};
use std::fmt::Write;
use std::io;
use zeroize::Zeroizing;

/// The numbered words in three columns, then the owner fingerprint. Built in one pre-sized buffer,
/// so no temporary or reallocation leaves a word in freed memory.
fn body(phrase: &Phrase, fingerprint: &str) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::with_capacity(256));
    for row in 0..phrase::WORDS / 3 {
        for col in 0..3 {
            let n = col * (phrase::WORDS / 3) + row;
            let word = phrase::word(phrase[n]);
            let _ = if col < 2 {
                write!(out, "{:>2}. {word:<10}", n + 1)
            } else {
                write!(out, "{:>2}. {word}", n + 1)
            };
        }
        out.push('\n');
    }
    let _ = write!(out, "\nOwner fingerprint: {fingerprint}");
    out
}

fn show<P: Prompter>(p: &mut P, phrase: &Phrase, fingerprint: &str) -> io::Result<()> {
    p.note("Recovery phrase", &body(phrase, fingerprint))
}

fn any(_: &str) -> Result<(), String> {
    Ok(())
}

/// Shows `phrase` with the owner `fingerprint` on the phrase's screen, asks for the words at
/// `positions` (zero-based), and says so once it is gone. False when the user cancelled or has
/// not written it down; an interrupt is an `Err` of kind `Interrupted`.
pub fn confirm_written<P: Prompter>(
    p: &mut P,
    phrase: &Phrase,
    fingerprint: &str,
    positions: [usize; 3],
) -> io::Result<bool> {
    let confirmed = p.screen(|p| -> io::Result<bool> {
        show(p, phrase, fingerprint)?;
        if !p.confirm("Written down?", false)? {
            return Ok(false);
        }
        for pos in positions {
            let prompt = format!("Word {}", pos + 1);
            loop {
                let typed = Zeroizing::new(p.input(&prompt, "", any)?);
                if phrase::lookup(&typed) == Some(phrase[pos]) {
                    break;
                }
                p.warn(&format!("{prompt} does not match."))?;
                let choices = [
                    Choice::new("Try again", ""),
                    Choice::new("Show the phrase again", ""),
                    Choice::new("Cancel", "nothing is written"),
                ];
                match p.select("What now?", &choices, 0)? {
                    0 => {}
                    1 => show(p, phrase, fingerprint)?,
                    _ => return Ok(false),
                }
            }
        }
        Ok(true)
    })?;
    if confirmed {
        p.info("Recovery phrase confirmed")?;
    }
    Ok(confirmed)
}

/// Reads the 12 words on the phrase's screen, one by one, and returns the 128 bits they encode.
/// A failed checksum asks for the 12 again, with the typed words as defaults.
pub fn read_phrase<P: Prompter>(p: &mut P) -> io::Result<Zeroizing<[u8; 16]>> {
    p.screen(|p| {
        let mut typed: Vec<Zeroizing<String>> = Vec::new();
        loop {
            let mut indexes = Zeroizing::new([0u16; phrase::WORDS]);
            let mut next = Vec::with_capacity(phrase::WORDS);
            for i in 0..phrase::WORDS {
                let default = typed.get(i).map_or("", |w| w.as_str());
                let word = Zeroizing::new(p.input(
                    &format!("Word {}", i + 1),
                    default,
                    phrase::check_word,
                )?);
                indexes[i] = phrase::lookup(&word).expect("checked by check_word");
                next.push(word);
            }
            typed = next;
            match phrase::decode(&*indexes) {
                Ok(entropy) => return Ok(entropy),
                Err(m) => p.warn(&format!("{m}. Enter all 12 words again."))?,
            }
        }
    })
}

/// Shows the derived owner `fingerprint`. When `ask`, also asks whether it matches the one written
/// down with the phrase or shown on another device, defaulting to no; false when it does not.
pub fn confirm_fingerprint<P: Prompter>(
    p: &mut P,
    fingerprint: &str,
    ask: bool,
) -> io::Result<bool> {
    p.note("Owner fingerprint", fingerprint)?;
    if !ask {
        return Ok(true);
    }
    p.confirm(
        "Does it match the fingerprint written down with the phrase, or shown by `bilbo device` on another device?",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::script::{Answer, Script, text};
    use std::collections::HashSet;

    const FP: &str = "yb4b-5aju-v6zb-x2nm-nc5x-ompf";
    // legal winner thank year wave sausage worth useful legal winner thank yellow
    const ENTROPY: [u8; 16] = [0x7f; 16];
    const POSITIONS: [usize; 3] = [0, 5, 11];

    fn words() -> Vec<&'static str> {
        phrase::encode(&ENTROPY)
            .iter()
            .map(|&i| phrase::word(i))
            .collect()
    }

    fn run(answers: Vec<Answer>) -> (Script, io::Result<bool>) {
        let mut p = Script::new(answers);
        let r = confirm_written(&mut p, &phrase::encode(&ENTROPY), FP, POSITIONS);
        only_notes_hold_words(&p);
        (p, r)
    }

    fn right() -> Vec<Answer> {
        let w = words();
        vec![Answer::Yes, text(w[0]), text(w[5]), text(w[11])]
    }

    /// No shown text but a `note` holds a phrase word, in any case, or its first 4 letters.
    fn only_notes_hold_words(p: &Script) {
        let set: HashSet<&str> = words().into_iter().collect();
        let prefixes: HashSet<&str> = words().into_iter().map(|w| &w[..4]).collect();
        for line in p.shown.iter().filter(|l| !l.starts_with("note:")) {
            for token in line.split(|c: char| !c.is_ascii_alphabetic()) {
                let token = token.to_ascii_lowercase();
                assert!(!set.contains(token.as_str()), "{token} in {line}");
                if token.len() >= 4 {
                    assert!(!prefixes.contains(&token[..4]), "{token} in {line}");
                }
            }
        }
    }

    fn index_of(p: &Script, needle: &str) -> usize {
        p.shown.iter().position(|l| l.contains(needle)).unwrap()
    }

    #[test]
    fn shows_numbered_words_and_fingerprint_in_one_note() {
        let (p, r) = run(right());
        assert!(r.unwrap());
        let notes: Vec<&String> = p.shown.iter().filter(|l| l.starts_with("note:")).collect();
        assert_eq!(notes.len(), 1);
        for (i, w) in words().iter().enumerate() {
            assert!(notes[0].contains(&format!("{:>2}. {w}", i + 1)));
        }
        assert!(notes[0].contains(FP));
        assert!(p.saw("confirm: Written down? initial=false"));
        assert!(p.saw("info: Recovery phrase confirmed"));
        assert_eq!(p.left(), 0);
        let (begin, end) = (index_of(&p, "screen: begin"), index_of(&p, "screen: end"));
        for needle in ["note:", "confirm:", "input: Word 1 ", "input: Word 12 "] {
            let at = index_of(&p, needle);
            assert!(begin < at && at < end, "{needle} outside the screen");
        }
        assert!(end < index_of(&p, "info: Recovery phrase confirmed"));
    }

    #[test]
    fn asks_positions_by_number() {
        let (p, _) = run(right());
        for n in ["Word 1", "Word 6", "Word 12"] {
            assert!(p.saw(&format!("input: {n} ")));
        }
        assert!(!p.saw("input: Word 2 "));
    }

    #[test]
    fn a_prefix_in_any_case_matches() {
        let w = words();
        let (_, r) = run(vec![
            Answer::Yes,
            text(&format!(" {} ", w[0][..4].to_uppercase())),
            text(&w[5].to_uppercase()),
            text(&w[11][..4]),
        ]);
        assert!(r.unwrap());
    }

    #[test]
    fn a_wrong_word_then_the_right_one() {
        let w = words();
        let (p, r) = run(vec![
            Answer::Yes,
            text(w[1]),
            Answer::Select(0),
            text(&w[2][..4]),
            Answer::Select(0),
            text(w[0]),
            text(w[5]),
            text(w[11]),
        ]);
        assert!(r.unwrap());
        assert!(p.saw("warn: Word 1 does not match."));
        assert_eq!(p.left(), 0);
    }

    #[test]
    fn show_again_shows_the_phrase_a_second_time() {
        let w = words();
        let (p, r) = run(vec![
            Answer::Yes,
            text("zoo"),
            Answer::Select(1),
            text(w[0]),
            text(w[5]),
            text(w[11]),
        ]);
        assert!(r.unwrap());
        assert_eq!(p.shown.iter().filter(|l| l.starts_with("note:")).count(), 2);
        let end = index_of(&p, "screen: end");
        assert!(
            p.shown[..end]
                .iter()
                .filter(|l| l.starts_with("note:"))
                .count()
                == 2
        );
    }

    #[test]
    fn cancel_returns_false_and_confirms_nothing() {
        let (p, r) = run(vec![Answer::Yes, text("zoo"), Answer::Select(2)]);
        assert!(!r.unwrap());
        assert!(!p.saw("Recovery phrase confirmed"));
        assert_eq!(p.shown.last().unwrap(), "screen: end");
    }

    #[test]
    fn not_written_down_returns_false() {
        let (p, r) = run(vec![Answer::No]);
        assert!(!r.unwrap());
        assert!(!p.saw("input:"));
    }

    #[test]
    fn an_interrupt_is_an_error() {
        let (_, r) = run(vec![Answer::Yes, Answer::Interrupt]);
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::Interrupted);
    }

    fn all_twelve() -> Vec<Answer> {
        words().into_iter().map(text).collect()
    }

    #[test]
    fn reads_twelve_words_into_their_entropy() {
        let mut p = Script::new(all_twelve());
        let entropy = read_phrase(&mut p).unwrap();
        assert_eq!(*entropy, ENTROPY);
        assert!(p.saw("input: Word 12 "));
        assert_eq!(p.left(), 0);
        let (begin, end) = (index_of(&p, "screen: begin"), index_of(&p, "screen: end"));
        let inputs: Vec<usize> = (0..p.shown.len())
            .filter(|&i| p.shown[i].starts_with("input:"))
            .collect();
        assert_eq!(inputs.len(), 12);
        assert!(inputs.iter().all(|&i| begin < i && i < end));
    }

    #[test]
    fn reads_prefixes() {
        let mut p = Script::new(words().into_iter().map(|w| text(&w[..4])).collect());
        assert_eq!(*read_phrase(&mut p).unwrap(), ENTROPY);
    }

    #[test]
    fn an_unknown_word_is_asked_again_by_position() {
        let mut answers = all_twelve();
        answers.insert(2, text("notaword"));
        let mut p = Script::new(answers);
        assert_eq!(*read_phrase(&mut p).unwrap(), ENTROPY);
        assert!(p.saw("input refused: Word 3: not a word of the list"));
        assert!(!p.saw("notaword"));
        assert_eq!(p.left(), 0);
    }

    #[test]
    fn a_failed_checksum_asks_all_twelve_again_with_defaults() {
        let mut answers = all_twelve();
        answers[11] = text("zoo");
        answers.extend((0..11).map(|_| Answer::Default));
        answers.push(text(words()[11]));
        let mut p = Script::new(answers);
        assert_eq!(*read_phrase(&mut p).unwrap(), ENTROPY);
        assert!(p.saw("warn: the words do not make a valid phrase. Enter all 12 words again."));
        assert!(p.saw("input: Word 12 default=zoo"));
        assert!(p.saw("input: Word 1 default=legal"));
        assert_eq!(p.left(), 0);
    }

    #[test]
    fn a_failed_checksum_then_the_right_words() {
        let mut answers = all_twelve();
        answers[11] = text("zoo");
        answers.extend(all_twelve());
        let mut p = Script::new(answers);
        assert_eq!(*read_phrase(&mut p).unwrap(), ENTROPY);
        assert_eq!(p.left(), 0);
        let warns: Vec<&String> = p.shown.iter().filter(|l| l.starts_with("warn:")).collect();
        assert_eq!(warns.len(), 1);
        only_notes_hold_words_except_defaults(&p);
    }

    /// Defaults on the phrase's screen carry the typed words; warnings do not.
    fn only_notes_hold_words_except_defaults(p: &Script) {
        let set: HashSet<&str> = words().into_iter().collect();
        for line in p.shown.iter().filter(|l| l.starts_with("warn:")) {
            for token in line.split(|c: char| !c.is_ascii_alphabetic()) {
                assert!(!set.contains(token), "{token} in {line}");
            }
        }
    }

    #[test]
    fn the_fingerprint_question_defaults_to_no() {
        let mut p = Script::new(vec![Answer::Default]);
        assert!(!confirm_fingerprint(&mut p, FP, true).unwrap());
        assert!(p.saw(FP));
        assert!(p.saw("initial=false"));
    }

    #[test]
    fn the_fingerprint_matches() {
        let mut p = Script::new(vec![Answer::Yes]);
        assert!(confirm_fingerprint(&mut p, FP, true).unwrap());
    }

    #[test]
    fn no_question_when_a_manifest_vouches() {
        let mut p = Script::new(vec![]);
        assert!(confirm_fingerprint(&mut p, FP, false).unwrap());
        assert!(p.saw(FP));
        assert!(!p.saw("confirm:"));
    }
}
