use bilbo_kinds::remember::{Output, Tally};

use crate::console::message::{Detail, Mark, Message};
use crate::console::view::View;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Totals {
    pub tally: Tally,
    pub claimed: u64,
    pub records: u64,
    pub raw_bytes: u64,
    pub stored_bytes: u64,
}

impl Totals {
    pub fn of(outputs: &[Output]) -> Totals {
        let mut totals = Totals::default();
        for output in outputs {
            let tally = output.tally();
            totals.tally.new += tally.new;
            totals.tally.appended += tally.appended;
            totals.tally.new_generation += tally.new_generation;
            totals.tally.unchanged += tally.unchanged;
            totals.tally.refused += tally.refused;
            totals.claimed += output.claimed;
            totals.records += output.records;
            totals.raw_bytes += output.raw_bytes;
            totals.stored_bytes += output.stored_bytes;
        }
        totals
    }
}

pub fn plain(output: &Output) -> String {
    plain_totals(&Totals::of(std::slice::from_ref(output)))
}

pub fn human(output: &Output, view: &View) -> String {
    human_totals(&Totals::of(std::slice::from_ref(output)), view)
}

pub fn counts(totals: &Totals) -> String {
    let tally = &totals.tally;
    format!(
        "new {}, appended {}, new generation {}, unchanged {}, refused {}",
        tally.new, tally.appended, tally.new_generation, tally.unchanged, tally.refused,
    )
}

pub fn plain_totals(totals: &Totals) -> String {
    format!(
        "{}\nrecords {}, raw bytes {}, stored bytes {}\n",
        counts(totals),
        totals.records,
        totals.raw_bytes,
        totals.stored_bytes,
    )
}

pub fn human_totals(totals: &Totals, view: &View) -> String {
    let tally = &totals.tally;
    let kept = tally.new + tally.appended + tally.new_generation;
    let mark = if totals.claimed == 0 || tally.refused > 0 {
        Mark::Warning
    } else if kept > 0 {
        Mark::Done
    } else {
        Mark::Info
    };
    let text = if totals.claimed == 0 {
        "Nothing to keep".to_owned()
    } else if kept > 0 {
        let noun = if kept == 1 { "entry" } else { "entries" };
        format!("Remembered {} {noun}", group(kept))
    } else {
        "Nothing new to remember".to_owned()
    };
    let message = Message {
        mark,
        text,
        details: vec![
            Detail::Text(format!(
                "{} new, {} appended, {} new generation, {} unchanged, {} refused",
                group(tally.new),
                group(tally.appended),
                group(tally.new_generation),
                group(tally.unchanged),
                group(tally.refused),
            )),
            Detail::Text(format!(
                "{} records, {} raw bytes, {} stored bytes",
                group(totals.records),
                group(totals.raw_bytes),
                group(totals.stored_bytes),
            )),
        ],
    };
    let mut text = message.human_lines(view).join("\n");
    text.push('\n');
    text
}

pub fn group(number: u64) -> String {
    let digits = number.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bilbo_kinds::remember::{Item, Outcome, Output};
    use bilbo_store::id::Id;

    use super::{Totals, group, human, plain, plain_totals};
    use crate::console::view::View;

    fn output(claimed: u64, outcomes: Vec<Outcome>) -> Output {
        Output {
            target: PathBuf::from("/notes"),
            kind: Some("markdown".to_owned()),
            claimed,
            items: outcomes
                .into_iter()
                .map(|outcome| Item {
                    path: PathBuf::from("/notes/a.md"),
                    outcome,
                })
                .collect(),
            records: 3,
            raw_bytes: 40,
            stored_bytes: 50,
        }
    }

    fn id() -> Id {
        Id::from_bytes([1; 16])
    }

    #[test]
    fn the_plain_output_is_two_lines_of_counts() {
        let output = output(
            3,
            vec![
                Outcome::New(id()),
                Outcome::Appended(id()),
                Outcome::Refused("x".to_owned()),
            ],
        );
        assert_eq!(
            plain(&output),
            "new 1, appended 1, new generation 0, unchanged 0, refused 1\n\
             records 3, raw bytes 40, stored bytes 50\n"
        );
    }

    #[test]
    fn numbers_on_a_terminal_are_grouped_by_thousands() {
        let grouped: Vec<String> = [0, 999, 1000, 1_234_567].into_iter().map(group).collect();
        assert_eq!(grouped, ["0", "999", "1,000", "1,234,567"]);
    }

    #[test]
    fn the_headline_and_mark_follow_what_was_kept() {
        let cases = [
            (output(0, vec![]), "\x1b[33m▲\x1b[39m  Nothing to keep\n"),
            (
                output(2, vec![Outcome::New(id()), Outcome::Refused("x".into())]),
                "\x1b[33m▲\x1b[39m  Remembered 1 entry\n",
            ),
            (
                output(2, vec![Outcome::New(id()), Outcome::Appended(id())]),
                "\x1b[32m◆\x1b[39m  Remembered 2 entries\n",
            ),
            (
                output(1, vec![Outcome::Unchanged(id())]),
                "\x1b[34m●\x1b[39m  Nothing new to remember\n",
            ),
        ];
        for (output, headline) in cases {
            let text = human(&output, &View::HUMAN);
            assert!(text.starts_with(headline), "{text:?} vs {headline:?}");
        }
    }

    #[test]
    fn the_totals_of_several_outputs_add_up() {
        let outputs = [
            output(2, vec![Outcome::New(id()), Outcome::Unchanged(id())]),
            output(1, vec![Outcome::Refused("x".to_owned())]),
        ];
        let totals = Totals::of(&outputs);
        assert_eq!(totals.claimed, 3);
        assert_eq!(
            plain_totals(&totals),
            "new 1, appended 0, new generation 0, unchanged 1, refused 1\n\
             records 6, raw bytes 80, stored bytes 100\n"
        );
        assert_eq!(
            plain_totals(&Totals::of(&[])),
            plain_totals(&Totals::default())
        );
    }
}
