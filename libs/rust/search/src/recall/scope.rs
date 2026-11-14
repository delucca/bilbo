use bilbo_kinds::read::refs::{Ref, Target};
use bilbo_store::entries::{Entries, StoreError};
use bilbo_store::id::{Id, Short};
use bilbo_store::labels::Labels;

use crate::analyze;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Id(Id),
    Short(Short),
    Words(Vec<String>),
}

pub fn value(text: &str) -> Option<Value> {
    match Ref::parse(text).map(|parsed| parsed.target) {
        Ok(Target::Entry(id)) => Some(Value::Id(id)),
        Ok(Target::Short(short)) => Some(Value::Short(short)),
        Ok(Target::Session(_)) | Err(_) => {
            let words = analyze::label_words(text);
            (!words.is_empty()).then_some(Value::Words(words))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unnamed {
    Nothing,
    Several(Vec<Id>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    /// Sorted, distinct.
    pub entries: Vec<Id>,
    /// By value, in the order given.
    pub unnamed: Vec<(String, Unnamed)>,
    /// A value was read as words: the label tables were asked.
    pub worded: bool,
}

pub fn resolve(
    values: &[String],
    entries: &dyn Entries,
    labels: &dyn Labels,
) -> Result<Scope, StoreError> {
    let mut scope = Scope::default();
    let mut named: Vec<Id> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for given in values {
        if seen.contains(&given.as_str()) {
            continue;
        }
        seen.push(given);
        let words = match value(given) {
            None => {
                scope.unnamed.push((given.clone(), Unnamed::Nothing));
                continue;
            }
            Some(Value::Id(id)) => {
                if entries.meta(id)?.is_some() {
                    named.push(id);
                    continue;
                }
                analyze::label_words(given)
            }
            Some(Value::Short(short)) => match entries.by_shorts(&[short])?.as_slice() {
                [] => analyze::label_words(given),
                [only] => {
                    named.push(*only);
                    continue;
                }
                several => {
                    scope
                        .unnamed
                        .push((given.clone(), Unnamed::Several(several.to_vec())));
                    continue;
                }
            },
            Some(Value::Words(words)) => words,
        };
        scope.worded = true;
        let found = labels.with_words(&words)?;
        if found.is_empty() {
            scope.unnamed.push((given.clone(), Unnamed::Nothing));
        }
        named.extend(found);
    }
    named.sort_unstable();
    named.dedup();
    if !named.is_empty() {
        named.extend(labels.belonging_to(&named)?);
        named.sort_unstable();
        named.dedup();
    }
    scope.entries = named;
    Ok(scope)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::SystemTime;

    use bilbo_store::entries::{
        Capture, CaptureError, Captured, Change, Cursor, Entries, Generation, Head, Meta,
        RecordRange, StoreError,
    };
    use bilbo_store::id::{Id, Short};
    use bilbo_store::labels::{CaptureLabel, Labels, StoredLabels};

    use super::{Scope, Unnamed, Value, resolve, value};
    use crate::analyze::label_words;
    use crate::testing::{id, unused};

    fn twin(n: u8) -> Id {
        let mut bytes = [n; 16];
        bytes[0] = 0x02;
        Id::from_bytes(bytes)
    }

    struct Known(Vec<Id>);

    impl Entries for Known {
        fn capture(&mut self, _input: &Capture<'_>) -> Result<Captured, CaptureError> {
            Err(CaptureError::Store(unused()))
        }

        fn set_meta(&mut self, _id: Id, _meta: &Meta) -> Result<u64, StoreError> {
            Err(unused())
        }

        fn read(
            &self,
            _id: Id,
            _generation: Generation,
            _records: RecordRange,
        ) -> Result<Vec<Vec<u8>>, StoreError> {
            Err(unused())
        }

        fn meta(&self, id: Id) -> Result<Option<Meta>, StoreError> {
            Ok(self.0.contains(&id).then(|| Meta {
                id,
                kind: "note".to_owned(),
                labels: StoredLabels::default(),
                captured: SystemTime::UNIX_EPOCH,
            }))
        }

        fn changes(&self, _since: &Cursor) -> Result<(Vec<Change>, Cursor), StoreError> {
            Err(unused())
        }

        fn by_capture(&self, _label: &CaptureLabel) -> Result<Option<Head>, StoreError> {
            Err(unused())
        }

        fn extent(
            &self,
            _id: Id,
            _generation: Option<Generation>,
        ) -> Result<Option<(Generation, u64)>, StoreError> {
            Err(unused())
        }

        fn captures_with(&self, _part: &str) -> Result<Vec<(Id, CaptureLabel)>, StoreError> {
            Err(unused())
        }

        fn by_shorts(&self, shorts: &[Short]) -> Result<Vec<Id>, StoreError> {
            let mut found: Vec<Id> = self
                .0
                .iter()
                .copied()
                .filter(|id| shorts.iter().any(|short| short.ends(*id)))
                .collect();
            found.sort();
            Ok(found)
        }
    }

    #[derive(Default)]
    struct Shelf {
        texts: Vec<(Id, &'static str)>,
        below: Vec<Id>,
        belonging: RefCell<Vec<Vec<Id>>>,
        words: RefCell<Vec<Vec<String>>>,
    }

    impl Labels for Shelf {
        fn with_words(&self, words: &[String]) -> Result<Vec<Id>, StoreError> {
            self.words.borrow_mut().push(words.to_vec());
            let mut found: Vec<Id> = self
                .texts
                .iter()
                .filter(|(_, text)| {
                    label_words(text)
                        .windows(words.len())
                        .any(|run| run == words)
                })
                .map(|(entry, _)| *entry)
                .collect();
            found.sort();
            found.dedup();
            Ok(found)
        }

        fn belonging_to(&self, entries: &[Id]) -> Result<Vec<Id>, StoreError> {
            self.belonging.borrow_mut().push(entries.to_vec());
            Ok(self.below.clone())
        }
    }

    fn values(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|text| (*text).to_owned()).collect()
    }

    fn scope_of(shelf: &Shelf, known: &[Id], given: &[&str]) -> Scope {
        resolve(&values(given), &Known(known.to_vec()), shelf).expect("a scope")
    }

    #[test]
    fn a_value_is_an_id_a_short_id_or_words() {
        let whole = id(1).to_string();
        let short = id(1).short().to_string();
        assert_eq!(value(&whole), Some(Value::Id(id(1))));
        assert_eq!(value(&format!("bilbo:{whole}")), Some(Value::Id(id(1))));
        assert_eq!(value(&format!("{whole}#r3")), Some(Value::Id(id(1))));
        assert_eq!(value(&short), Some(Value::Short(id(1).short())));
        assert_eq!(
            value(&format!("{short}#r2-4")),
            Some(Value::Short(id(1).short()))
        );
        assert_eq!(
            value("7d1e4b9a-3c5f-4a2e-8b6d-0f9e1c2a3b4d"),
            Some(Value::Words(label_words(
                "7d1e4b9a-3c5f-4a2e-8b6d-0f9e1c2a3b4d"
            )))
        );
        assert_eq!(label_words("7d1e4b9a-3c5f-4a2e-8b6d-0f9e1c2a3b4d").len(), 5);
        assert_eq!(value("bilbo"), Some(Value::Words(label_words("bilbo"))));
        for none in [".", "--", "/", " "] {
            assert_eq!(value(none), None, "{none:?}");
        }
    }

    #[test]
    fn an_id_names_its_entry_and_what_belongs_to_it() {
        let shelf = Shelf {
            below: vec![id(8), id(9)],
            ..Shelf::default()
        };
        let scope = scope_of(&shelf, &[id(1)], &[&id(1).to_string()]);
        assert_eq!(scope.entries, [id(1), id(8), id(9)]);
        assert!(!scope.worded);
        assert!(scope.unnamed.is_empty());
    }

    #[test]
    fn what_belongs_is_asked_once_for_all_the_values() {
        let shelf = Shelf {
            texts: vec![(id(3), "shed"), (id(4), "shed"), (id(5), "shed")],
            ..Shelf::default()
        };
        let known = [id(1), id(2), id(3), id(4), id(5)];
        let scope = scope_of(
            &shelf,
            &known,
            &[&id(1).to_string(), &id(2).short().to_string(), "shed"],
        );
        assert_eq!(scope.entries, known);
        assert_eq!(
            shelf.belonging.borrow().as_slice(),
            [vec![id(1), id(2), id(3), id(4), id(5)]]
        );
        let empty = Shelf::default();
        let scope = scope_of(&empty, &known, &["nothing", &id(7).to_string()]);
        assert!(scope.entries.is_empty());
        assert!(empty.belonging.borrow().is_empty());
    }

    #[test]
    fn an_id_form_no_entry_has_is_read_as_words() {
        let shelf = Shelf {
            texts: vec![(id(2), "references")],
            ..Shelf::default()
        };
        let known = [id(1), id(2), id(3)];
        let scope = scope_of(&shelf, &known, &["REFERENCES"]);
        assert_eq!(scope.entries, [id(2)]);
        assert!(scope.worded);
        let loose = Id::from_bytes([5; 16]).to_string();
        let held = Shelf {
            texts: vec![(id(3), Box::leak(loose.clone().into_boxed_str()))],
            ..Shelf::default()
        };
        let scope = scope_of(&held, &known, &[&loose]);
        assert_eq!(scope.entries, [id(3)]);
        assert!(scope.worded);
        let none = scope_of(&Shelf::default(), &known, &["REFERENCES", &loose]);
        assert!(none.entries.is_empty());
        assert!(none.worded);
        assert_eq!(
            none.unnamed,
            [
                ("REFERENCES".to_owned(), Unnamed::Nothing),
                (loose, Unnamed::Nothing)
            ]
        );
    }

    #[test]
    fn an_id_an_entry_has_wins_over_the_same_word_in_a_label() {
        let short = id(1).short().to_string();
        let shelf = Shelf {
            texts: vec![(id(2), Box::leak(short.clone().into_boxed_str()))],
            ..Shelf::default()
        };
        let scope = scope_of(&shelf, &[id(1), id(2)], &[&short]);
        assert_eq!(scope.entries, [id(1)]);
        assert!(!scope.worded);
        assert!(shelf.words.borrow().is_empty());
    }

    #[test]
    fn a_value_with_a_lower_case_letter_is_never_an_id() {
        let lower = id(1).short().to_string().to_ascii_lowercase();
        let shelf = Shelf {
            texts: vec![(id(2), Box::leak(lower.clone().into_boxed_str()))],
            ..Shelf::default()
        };
        let known = [id(1), id(2)];
        let scope = scope_of(&shelf, &known, &[&lower]);
        assert_eq!(scope.entries, [id(2)]);
        assert!(scope.worded);
        let mut bytes = [0u8; 16];
        let digits = "1234567890"
            .bytes()
            .fold(0u64, |value, digit| (value << 5) | u64::from(digit - b'0'));
        bytes[8..].copy_from_slice(&digits.to_be_bytes());
        let numeric = Id::from_bytes(bytes);
        assert_eq!(numeric.short().to_string(), "1234567890");
        let scope = scope_of(&Shelf::default(), &[numeric], &["1234567890"]);
        assert_eq!(scope.entries, [numeric]);
        assert!(!scope.worded);
    }

    #[test]
    fn a_short_id_several_entries_end_with_names_nothing_and_lists_them() {
        let shelf = Shelf {
            texts: vec![(id(3), "x")],
            ..Shelf::default()
        };
        let text = id(1).short().to_string();
        let scope = scope_of(&shelf, &[id(1), twin(1), id(3)], &[&text, "x"]);
        let mut both = vec![id(1), twin(1)];
        both.sort();
        assert_eq!(scope.unnamed, [(text, Unnamed::Several(both))]);
        assert_eq!(scope.entries, [id(3)]);
        assert!(
            shelf.words.borrow().len() == 1,
            "the short id is not read as words"
        );
    }

    #[test]
    fn words_name_the_entries_whose_label_holds_them_and_what_belongs_to_those() {
        let shelf = Shelf {
            texts: vec![
                (id(1), "file:/home/me/src/shed"),
                (id(2), "file:/home/me/src/garden"),
            ],
            below: vec![id(7)],
            ..Shelf::default()
        };
        let scope = scope_of(&shelf, &[id(1), id(2), id(7)], &["src shed"]);
        assert_eq!(scope.entries, [id(1), id(7)]);
        assert!(scope.worded);
        assert_eq!(shelf.belonging.borrow().as_slice(), [vec![id(1)]]);
        let reversed = scope_of(&shelf, &[id(1)], &["shed src"]);
        assert!(reversed.entries.is_empty());
    }

    #[test]
    fn several_values_add_up() {
        let shelf = Shelf {
            texts: vec![(id(1), "shed"), (id(2), "garden")],
            ..Shelf::default()
        };
        let known = [id(1), id(2), id(3)];
        let scope = scope_of(&shelf, &known, &["shed", "garden", &id(3).to_string()]);
        assert_eq!(scope.entries, [id(1), id(2), id(3)]);
        let twice = scope_of(&shelf, &known, &["shed", "shed", "shed"]);
        assert_eq!(twice, scope_of(&shelf, &known, &["shed"]));
        let missing = scope_of(&shelf, &known, &["nowhere", "nowhere"]);
        assert_eq!(missing.unnamed.len(), 1);
    }

    #[test]
    fn a_value_that_names_nothing_is_reported_by_value_and_the_rest_go_on() {
        let shelf = Shelf {
            texts: vec![(id(1), "shed")],
            ..Shelf::default()
        };
        let scope = scope_of(&shelf, &[id(1)], &["greenhouse", "shed"]);
        assert_eq!(scope.entries, [id(1)]);
        assert_eq!(scope.unnamed, [("greenhouse".to_owned(), Unnamed::Nothing)]);
    }
}
