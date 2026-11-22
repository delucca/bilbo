use bilbo_entries::read::refs::{Ref, Target};
use bilbo_store::entries::{Entries, StoreError};
use bilbo_store::id::{Id, Short};
use bilbo_store::labels::{Labels, NotATag, Tag};

use crate::analyze;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Id(Id),
    Short(Short),
    Tag(Tag),
    Words(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoValue {
    Blank,
    NotATag(NotATag),
}

pub fn value(text: &str) -> Result<Value, NoValue> {
    if let Some(written) = Tag::written(text) {
        return written.map(Value::Tag).map_err(NoValue::NotATag);
    }
    match Ref::parse(text).map(|parsed| parsed.target) {
        Ok(Target::Entry(id)) => Ok(Value::Id(id)),
        Ok(Target::Short(short)) => Ok(Value::Short(short)),
        Ok(Target::Session(_)) | Err(_) => {
            let words = analyze::label_words(text);
            if words.is_empty() {
                Err(NoValue::Blank)
            } else {
                Ok(Value::Words(words))
            }
        }
    }
}

fn tagged_by_words(words: &[String], labels: &dyn Labels) -> Result<Vec<Id>, StoreError> {
    let mut found = Vec::new();
    let mut last: Option<Tag> = None;
    for item in labels.tags()? {
        if last.as_ref() == Some(&item.tag) {
            continue;
        }
        let held = analyze::label_words(item.tag.as_str());
        if !words.is_empty() && held.windows(words.len()).any(|run| run == words) {
            found.extend(labels.tagged(&item.tag)?);
        }
        last = Some(item.tag);
    }
    Ok(found)
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
    pub unnamed: Vec<(String, Unnamed)>,
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
            Err(_) => {
                scope.unnamed.push((given.clone(), Unnamed::Nothing));
                continue;
            }
            Ok(Value::Tag(tag)) => {
                let found = labels.tagged(&tag)?;
                if found.is_empty() {
                    scope.unnamed.push((given.clone(), Unnamed::Nothing));
                }
                named.extend(found);
                continue;
            }
            Ok(Value::Id(id)) => {
                if entries.meta(id)?.is_some() {
                    named.push(id);
                    continue;
                }
                analyze::label_words(given)
            }
            Ok(Value::Short(short)) => match entries.by_shorts(&[short])?.as_slice() {
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
            Ok(Value::Words(words)) => words,
        };
        scope.worded = true;
        let mut found = labels.with_words(&words)?;
        found.extend(tagged_by_words(&words, labels)?);
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
    use bilbo_store::labels::{CaptureLabel, Labels, NotATag, StoredLabels, Tag, Tagged};

    use super::{NoValue, Scope, Unnamed, Value, resolve, value};
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
        carried: Vec<(&'static str, Id)>,
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

        fn tagged(&self, tag: &Tag) -> Result<Vec<Id>, StoreError> {
            let mut found: Vec<Id> = self
                .carried
                .iter()
                .filter(|(name, _)| *name == tag.as_str())
                .map(|(_, entry)| *entry)
                .collect();
            found.sort();
            Ok(found)
        }

        fn tags(&self) -> Result<Vec<Tagged>, StoreError> {
            let mut names: Vec<&str> = self.carried.iter().map(|(name, _)| *name).collect();
            names.sort_unstable();
            names.dedup();
            Ok(names
                .into_iter()
                .map(|name| Tagged {
                    tag: tag(name),
                    kind: "note".to_owned(),
                    entries: self
                        .carried
                        .iter()
                        .filter(|(held, _)| *held == name)
                        .count() as u64,
                })
                .collect())
        }
    }

    fn tag(name: &str) -> Tag {
        Tag::new(name).expect("a tag")
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
        assert_eq!(value(&whole), Ok(Value::Id(id(1))));
        assert_eq!(value(&format!("bilbo:{whole}")), Ok(Value::Id(id(1))));
        assert_eq!(value(&format!("{whole}#r3")), Ok(Value::Id(id(1))));
        assert_eq!(value(&short), Ok(Value::Short(id(1).short())));
        assert_eq!(
            value(&format!("{short}#r2-4")),
            Ok(Value::Short(id(1).short()))
        );
        assert_eq!(
            value("7d1e4b9a-3c5f-4a2e-8b6d-0f9e1c2a3b4d"),
            Ok(Value::Words(label_words(
                "7d1e4b9a-3c5f-4a2e-8b6d-0f9e1c2a3b4d"
            )))
        );
        assert_eq!(label_words("7d1e4b9a-3c5f-4a2e-8b6d-0f9e1c2a3b4d").len(), 5);
        assert_eq!(value("bilbo"), Ok(Value::Words(label_words("bilbo"))));
        for none in [".", "--", "/", " "] {
            assert_eq!(value(none), Err(NoValue::Blank), "{none:?}");
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

    #[test]
    fn a_value_is_a_tag_by_its_scheme_then_an_id_then_words() {
        let digits = "00000000000000000000000001";
        assert_eq!(value("tag:rust"), Ok(Value::Tag(tag("rust"))));
        assert_eq!(
            value(digits),
            Ok(Value::Id(Id::parse(digits).expect("an id")))
        );
        assert_eq!(value(&format!("tag:{digits}")), Ok(Value::Tag(tag(digits))));
        assert_eq!(value("rust"), Ok(Value::Words(label_words("rust"))));
    }

    #[test]
    fn a_tag_scheme_with_no_tags_name_is_no_value_and_never_words() {
        for (text, rest) in [("tag:Rust", "Rust"), ("tag:", ""), ("tag:a b", "a b")] {
            assert_eq!(
                value(text),
                Err(NoValue::NotATag(NotATag(rest.to_owned()))),
                "{text:?}"
            );
        }
        assert_eq!(value("."), Err(NoValue::Blank));
    }

    #[test]
    fn a_tag_names_exactly_its_carriers_and_what_belongs_to_them() {
        let shelf = Shelf {
            carried: vec![("carpentry", id(1)), ("carpentry-docs", id(2))],
            texts: vec![(id(3), "file:/home/me/src/carpentry")],
            below: vec![id(7)],
            ..Shelf::default()
        };
        let scope = scope_of(&shelf, &[id(1), id(2), id(3), id(7)], &["tag:carpentry"]);
        assert_eq!(scope.entries, [id(1), id(7)]);
        assert!(!scope.worded);
        assert!(scope.unnamed.is_empty());
        assert_eq!(shelf.belonging.borrow().as_slice(), [vec![id(1)]]);
        assert!(shelf.words.borrow().is_empty());
    }

    #[test]
    fn a_tag_no_entry_carries_names_nothing_and_does_not_fall_back() {
        let shelf = Shelf {
            texts: vec![(id(1), "shed")],
            ..Shelf::default()
        };
        let scope = scope_of(&shelf, &[id(1)], &["tag:shed"]);
        assert!(scope.entries.is_empty());
        assert_eq!(scope.unnamed, [("tag:shed".to_owned(), Unnamed::Nothing)]);
        assert!(shelf.words.borrow().is_empty());
    }

    #[test]
    fn words_name_the_entries_of_a_tag_that_holds_them_too() {
        let shelf = Shelf {
            carried: vec![("rust", id(1)), ("rust-async", id(2)), ("trust", id(3))],
            texts: vec![(id(4), "file:/home/me/src/rust")],
            ..Shelf::default()
        };
        let scope = scope_of(&shelf, &[id(1), id(2), id(3), id(4)], &["rust"]);
        assert_eq!(scope.entries, [id(1), id(2), id(4)]);
        assert!(scope.worded);
        assert!(scope.unnamed.is_empty());
    }
}
