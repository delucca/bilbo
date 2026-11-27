use std::io;

use bilbo_store::id::Id;

use super::Rules;
use super::asked::{About, Asked, asking};
use super::record::Summary;
use super::view::{Of, Sent, Sizes, sent};
use crate::view::Counter;

#[must_use]
pub fn written(record: &Summary, about: &About, in_force: (&str, &str, &str), redo: bool) -> bool {
    let same = match about.notes {
        Some(digest) => record.notes.is_some_and(|notes| notes.digest == digest),
        None => {
            record.notes.is_none()
                && record.generation == about.generation
                && record.records == about.records
                && record.content == about.content
        }
    };
    let other = (
        record.instruction.as_str(),
        record.provider.as_str(),
        record.model.as_str(),
    ) != in_force;
    same && !(redo && other)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decided {
    Written,
    NoNote { retire: bool },
    Short { retire: bool },
    PassedOver,
    Due(Asking),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asking {
    pub number: u64,
    pub charges: u8,
    pub sent: usize,
    pub made: Sent,
}

pub struct Known<'a> {
    pub id: Id,
    pub about: About,
    pub of: Of<'a>,
    pub record: Option<&'a Summary>,
    pub covered: bool,
}

pub struct Force<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub redo: bool,
    pub head: f64,
    pub counter: &'a Counter<'a>,
}

pub fn decide(
    known: &Known<'_>,
    rules: &Rules<'_>,
    force: &Force<'_>,
    asked: &dyn Asked,
) -> io::Result<Decided> {
    let section = rules.section;
    let notes = match &known.of {
        Of::Notes(paragraphs) => Some(paragraphs.is_empty()),
        Of::Text(_) => None,
    };
    if let Some(record) = known.record
        && (notes.is_none() || known.about.notes.is_some())
        && written(
            record,
            &known.about,
            (rules.id, force.provider, force.model),
            force.redo,
        )
    {
        return Ok(Decided::Written);
    }
    if notes == Some(true) {
        return Ok(Decided::NoNote {
            retire: known.record.is_some() && known.covered,
        });
    }
    let sizes = Sizes {
        window: section.window.value,
        most: section.most.value,
        head: force.head,
    };
    let made = sent(&known.of, &sizes, force.counter);
    if made.empty {
        return Ok(Decided::Short {
            retire: known.record.is_some(),
        });
    }
    if known.record.is_none() && made.whole < section.least.value {
        return Ok(Decided::Short { retire: false });
    }
    let number = asking(
        known.id,
        &known.about,
        force.provider,
        force.model,
        rules.id,
        &made.material,
    );
    if !force.redo && asked.passed(known.id)? == Some(number) {
        return Ok(Decided::PassedOver);
    }
    Ok(Decided::Due(Asking {
        number,
        charges: asked.charges(known.id, number)?,
        sent: force.counter.count(&made.material),
        made,
    }))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io;

    use bilbo_entries::reader::Passage;
    use bilbo_entries::tokens::Estimate;
    use bilbo_store::entries::{Generation, RecordRange};
    use bilbo_store::id::Id;

    use super::{Asking, Decided, Force, Known, decide};
    use crate::summary::Rules;
    use crate::summary::asked::{About, Asked, asking};
    use crate::summary::keys;
    use crate::summary::record::Summary;
    use crate::summary::view::{Of, Sizes, sent};
    use crate::view::Counter;

    const ID: &str = "01M6ZGE4KTBYYH07AS2KVMSCAF";

    type Case<'a> = (
        &'a [Passage],
        Option<&'a Summary>,
        bool,
        Setting,
        Option<u64>,
        usize,
    );

    struct Double {
        passed: Option<u64>,
        charges: u8,
        reads: Cell<usize>,
        passes: Cell<usize>,
        fail: bool,
    }

    impl Double {
        fn new(passed: Option<u64>) -> Double {
            Double {
                passed,
                charges: 2,
                reads: Cell::new(0),
                passes: Cell::new(0),
                fail: false,
            }
        }

        fn read(&self) -> io::Result<()> {
            self.reads.set(self.reads.get() + 1);
            if self.fail {
                return Err(io::Error::other("a read that fails"));
            }
            Ok(())
        }
    }

    impl Asked for Double {
        fn passed(&self, _: Id) -> io::Result<Option<u64>> {
            self.passes.set(self.passes.get() + 1);
            self.read()?;
            Ok(self.passed)
        }

        fn charges(&self, _: Id, _: u64) -> io::Result<u8> {
            self.read()?;
            Ok(self.charges)
        }

        fn charge(&mut self, _: Id, _: u64) -> io::Result<()> {
            Ok(())
        }

        fn pass(&mut self, _: Id, _: u64) -> io::Result<()> {
            Ok(())
        }

        fn settle(&mut self, _: Id) -> io::Result<()> {
            Ok(())
        }
    }

    fn passage(text: &str) -> Passage {
        Passage {
            range: RecordRange { first: 1, count: 1 },
            text: text.to_owned(),
            pieces: Vec::new(),
            ends: vec![text.len()],
            at: None,
            outline: None,
        }
    }

    fn about(content: u128) -> About {
        About {
            generation: Generation(1),
            records: 1,
            content,
            notes: None,
        }
    }

    fn record(about: &About, model: &str) -> Summary {
        Summary {
            text: "The garden log.".to_owned(),
            instruction: "i1".to_owned(),
            provider: "fake".to_owned(),
            model: model.to_owned(),
            answered: None,
            made: 1,
            entry: Id::parse(ID).expect("id"),
            generation: about.generation,
            records: about.records,
            content: about.content,
            notes: None,
            sent: 1,
            tokens_in: None,
            tokens_out: None,
        }
    }

    struct Setting {
        section: keys::Section,
        redo: bool,
    }

    impl Setting {
        fn new() -> Setting {
            Setting {
                section: keys::defaults(),
                redo: false,
            }
        }

        fn at(least: usize) -> Setting {
            let mut setting = Setting::new();
            setting.section.least.value = least;
            setting
        }

        fn redo(mut self) -> Setting {
            self.redo = true;
            self
        }

        fn decide(
            &self,
            of: &Of<'_>,
            about: &About,
            record: Option<&Summary>,
            covered: bool,
            asked: &Double,
        ) -> io::Result<Decided> {
            let rules = Rules {
                section: &self.section,
                instruction: "text",
                id: "i1",
            };
            let counter = Counter {
                tokens: &Estimate,
                bound: 1 << 20,
            };
            let force = Force {
                provider: "fake",
                model: "model-a",
                redo: self.redo,
                head: 0.5,
                counter: &counter,
            };
            let of = match of {
                Of::Notes(paragraphs) => Of::Notes(paragraphs),
                Of::Text(passages) => Of::Text(passages),
            };
            let known = Known {
                id: Id::parse(ID).expect("id"),
                about: *about,
                of,
                record,
                covered,
            };
            decide(&known, &rules, &force, asked)
        }
    }

    fn text_of(setting: &Setting, bytes: usize, record: Option<&Summary>) -> Decided {
        let passages = [passage(&"a".repeat(bytes))];
        setting
            .decide(
                &Of::Text(&passages),
                &about(0xabcd),
                record,
                true,
                &Double::new(None),
            )
            .expect("decided")
    }

    #[test]
    fn rows_ten_to_fifteen_are_judged_in_their_order() {
        let base = about(0xabcd);
        let other = about(0x1234);
        let passages = [passage("abc")];
        let held = record(&base, "model-a");
        let setting = Setting::new();
        let on = |setting: &Setting, record: Option<&Summary>| {
            setting
                .decide(
                    &Of::Text(&passages),
                    &base,
                    record,
                    true,
                    &Double::new(None),
                )
                .expect("decided")
        };
        assert_eq!(on(&setting, Some(&held)), Decided::Written);
        let stale = record(&base, "other");
        assert!(matches!(
            on(&Setting::new().redo(), Some(&stale)),
            Decided::Due(_)
        ));
        assert_eq!(on(&Setting::new().redo(), Some(&held)), Decided::Written);

        let none: [Passage; 0] = [];
        for (covered, kept, retire) in [
            (true, true, true),
            (false, true, false),
            (true, false, false),
        ] {
            let held = record(&base, "model-a");
            let decided = setting
                .decide(
                    &Of::Notes(&[]),
                    &base,
                    kept.then_some(&held),
                    covered,
                    &Double::new(None),
                )
                .expect("decided");
            assert_eq!(decided, Decided::NoNote { retire });
        }

        let moved = record(&other, "model-a");
        for (kept, retire) in [(true, true), (false, false)] {
            let decided = setting
                .decide(
                    &Of::Text(&none),
                    &base,
                    kept.then_some(&moved),
                    true,
                    &Double::new(None),
                )
                .expect("decided");
            assert_eq!(decided, Decided::Short { retire });
        }

        let free = Setting::at(0);
        let blank = |bytes: &str, record: Option<&Summary>, setting: &Setting| {
            let passages = [passage(bytes)];
            setting
                .decide(
                    &Of::Text(&passages),
                    &base,
                    record,
                    true,
                    &Double::new(None),
                )
                .expect("decided")
        };
        assert_eq!(blank("", None, &free), Decided::Short { retire: false });
        assert_eq!(
            blank("", Some(&moved), &free),
            Decided::Short { retire: true }
        );
        let mut cut = Setting::at(0);
        cut.section.window.value = 1000;
        assert_eq!(
            blank(&" ".repeat(4004), None, &cut),
            Decided::Short { retire: false }
        );
        assert!(matches!(blank(" a ", None, &free), Decided::Due(_)));

        assert_eq!(
            text_of(&setting, 395, None),
            Decided::Short { retire: false }
        );
        let due = text_of(&setting, 399, None);
        let Decided::Due(first) = &due else {
            panic!("{due:?}");
        };
        assert!(matches!(
            text_of(&setting, 395, Some(&moved)),
            Decided::Due(_)
        ));

        let passages = [passage(&"a".repeat(399))];
        let held_as = |passed: Option<u64>, setting: &Setting| {
            setting
                .decide(
                    &Of::Text(&passages),
                    &base,
                    None,
                    true,
                    &Double::new(passed),
                )
                .expect("decided")
        };
        assert_eq!(held_as(Some(first.number), &setting), Decided::PassedOver);
        assert!(matches!(
            held_as(Some(first.number), &Setting::new().redo()),
            Decided::Due(_)
        ));
        assert!(matches!(
            held_as(Some(first.number.wrapping_add(1)), &setting),
            Decided::Due(_)
        ));

        let short = [passage(&"a".repeat(395))];
        let at_zero = match Setting::at(0)
            .decide(&Of::Text(&short), &base, None, true, &Double::new(None))
            .expect("decided")
        {
            Decided::Due(held) => held.number,
            other => panic!("{other:?}"),
        };
        let decided = setting
            .decide(
                &Of::Text(&short),
                &base,
                None,
                true,
                &Double::new(Some(at_zero)),
            )
            .expect("decided");
        assert_eq!(decided, Decided::Short { retire: false });

        let counter = Counter {
            tokens: &Estimate,
            bound: 1 << 20,
        };
        let sizes = Sizes {
            window: setting.section.window.value,
            most: setting.section.most.value,
            head: 0.5,
        };
        let made = sent(&Of::Text(&passages), &sizes, &counter);
        let number = asking(
            Id::parse(ID).expect("id"),
            &base,
            "fake",
            "model-a",
            "i1",
            &made.material,
        );
        assert_eq!(
            first,
            &Asking {
                number,
                charges: 2,
                sent: counter.count(&made.material),
                made,
            }
        );
    }

    #[test]
    fn the_charges_file_is_read_from_row_fourteen_on_and_not_for_a_pass_under_redo() {
        let base = about(0xabcd);
        let moved = record(&about(0x1234), "model-a");
        let held = record(&base, "model-a");
        let none: [Passage; 0] = [];
        let passages = [passage(&"a".repeat(399))];
        let short = [passage(&"a".repeat(395))];
        let setting = Setting::new();
        let number = match text_of(&setting, 399, None) {
            Decided::Due(held) => held.number,
            other => panic!("{other:?}"),
        };
        let one = [passage("abc")];
        let cases: Vec<Case<'_>> = vec![
            (&one, Some(&held), true, Setting::new(), None, 0),
            (&none, Some(&moved), true, Setting::new(), None, 0),
            (&short, None, true, Setting::new(), None, 0),
            (&passages, None, true, Setting::new(), Some(number), 1),
            (&passages, None, true, Setting::new(), None, 2),
            (
                &passages,
                None,
                true,
                Setting::new().redo(),
                Some(number),
                1,
            ),
        ];
        for (at, (passages, record, covered, setting, passed, reads)) in cases.iter().enumerate() {
            let double = Double::new(*passed);
            let decided = setting
                .decide(&Of::Text(passages), &base, *record, *covered, &double)
                .expect("decided");
            assert_eq!(double.reads.get(), *reads);
            if at == 5 {
                assert!(matches!(decided, Decided::Due(_)));
                assert_eq!(double.passes.get(), 0);
            }
        }
        let double = Double::new(None);
        setting
            .decide(&Of::Notes(&[]), &base, Some(&held), true, &double)
            .expect("decided");
        assert_eq!(double.reads.get(), 0);
        let mut failing = Double::new(None);
        failing.fail = true;
        assert!(
            setting
                .decide(&Of::Notes(&[]), &base, Some(&held), true, &failing)
                .is_ok()
        );
        for (at, (passages, record, covered, setting, passed, reads)) in cases.iter().enumerate() {
            let mut double = Double::new(*passed);
            double.fail = true;
            let decided = setting.decide(&Of::Text(passages), &base, *record, *covered, &double);
            if *reads == 0 {
                assert!(decided.is_ok(), "{at}");
            } else {
                assert!(decided.is_err(), "{at}");
            }
        }
    }
}
