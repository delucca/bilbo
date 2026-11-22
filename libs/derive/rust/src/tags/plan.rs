use std::collections::BTreeSet;

use bilbo_store::entries::Generation;
use bilbo_store::labels::Tag;

use super::asked::Answered;

pub struct Seen<'a> {
    pub answered: Option<&'a Answered>,
    pub passed: Option<u64>,
    pub added: bool,
    pub asking: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Due,
    Settled,
    PassedOver,
}

fn lacks(answered: &Answered, list: &BTreeSet<Tag>) -> bool {
    list.iter().any(|tag| !answered.names.contains(tag))
}

fn other(answered: &Answered, in_force: (&str, &str, &str)) -> bool {
    (
        answered.instruction.as_str(),
        answered.provider.as_str(),
        answered.model.as_str(),
    ) != in_force
}

#[must_use]
pub fn unsettled(
    answered: Option<&Answered>,
    passed: Option<u64>,
    head: (Generation, u64),
    list: &BTreeSet<Tag>,
    in_force: (&str, &str, &str),
    redo: bool,
) -> bool {
    let Some(answered) = answered else {
        return true;
    };
    lacks(answered, list)
        || answered.notes != head
        || (redo && (other(answered, in_force) || passed.is_some()))
}

#[must_use]
pub fn state(
    seen: &Seen<'_>,
    list: &BTreeSet<Tag>,
    in_force: (&str, &str, &str),
    redo: bool,
) -> State {
    let due = seen.answered.is_none_or(|answered| {
        seen.added
            || lacks(answered, list)
            || (redo && (other(answered, in_force) || seen.passed == Some(seen.asking)))
    });
    if !due {
        State::Settled
    } else if !redo && seen.passed == Some(seen.asking) {
        State::PassedOver
    } else {
        State::Due
    }
}

#[must_use]
pub fn names(seen: &Seen<'_>, asked: &BTreeSet<Tag>) -> BTreeSet<Tag> {
    match seen.answered {
        Some(answered) if !seen.added => answered.names.union(asked).cloned().collect(),
        _ => asked.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;
    use bilbo_store::labels::Tag;

    use super::{Seen, State, names, state, unsettled};
    use crate::tags::asked::Answered;

    const FORCE: (&str, &str, &str) = ("i", "p", "m");

    fn list(names: &[&str]) -> BTreeSet<Tag> {
        names
            .iter()
            .map(|name| Tag::new(name).expect("a tag"))
            .collect()
    }

    fn answered(held: &[&str]) -> Answered {
        Answered {
            entry: Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DB").expect("id"),
            notes: (Generation(1), 5),
            instruction: "i".to_owned(),
            provider: "p".to_owned(),
            model: "m".to_owned(),
            names: list(held),
        }
    }

    fn seen(answered: Option<&Answered>, passed: Option<u64>, added: bool) -> Seen<'_> {
        Seen {
            answered,
            passed,
            added,
            asking: 7,
        }
    }

    #[test]
    fn tags_are_due_by_what_the_answers_leave_unasked() {
        let held = answered(&["a", "b"]);
        assert_eq!(
            state(&seen(None, None, false), &list(&["a"]), FORCE, false),
            State::Due
        );
        for (names, added, expected) in [
            (&["a"][..], false, State::Settled),
            (&["a", "c"][..], false, State::Due),
            (&["a"][..], true, State::Due),
        ] {
            assert_eq!(
                state(&seen(Some(&held), None, added), &list(names), FORCE, false),
                expected,
                "{names:?} {added}"
            );
        }
    }

    #[test]
    fn nothing_settles_an_entry_before_its_answer_is_compared() {
        let held = answered(&["a"]);
        let head = (Generation(1), 5);
        let same = |list: &BTreeSet<Tag>, head, force, passed, redo| {
            unsettled(Some(&held), passed, head, list, force, redo)
        };
        assert!(unsettled(None, None, head, &list(&["a"]), FORCE, false));
        assert!(same(&list(&["a", "b"]), head, FORCE, None, false));
        assert!(same(&list(&["a"]), (Generation(1), 6), FORCE, None, false));
        assert!(!same(&list(&["a"]), head, FORCE, None, false));
        for force in [("j", "p", "m"), ("i", "q", "m"), ("i", "p", "n")] {
            assert!(same(&list(&["a"]), head, force, None, true), "{force:?}");
            assert!(!same(&list(&["a"]), head, force, None, false), "{force:?}");
        }
        assert!(same(&list(&["a"]), head, FORCE, Some(1), true));
        assert!(!same(&list(&["a"]), head, FORCE, Some(1), false));
    }

    #[test]
    fn a_name_put_back_asks_nothing_and_a_run_that_kept_no_note_leaves_the_names() {
        let held = answered(&["a", "b"]);
        for names in [&["a"][..], &["a", "b"][..]] {
            assert_eq!(
                state(&seen(Some(&held), None, false), &list(names), FORCE, false),
                State::Settled
            );
        }
        assert_eq!(
            names(&seen(Some(&held), None, false), &list(&["a", "c"])),
            list(&["a", "b", "c"])
        );
        assert_eq!(
            names(&seen(Some(&held), None, true), &list(&["a", "c"])),
            list(&["a", "c"])
        );
        assert_eq!(names(&seen(None, None, false), &list(&["a"])), list(&["a"]));
    }

    #[test]
    fn a_passed_asking_is_not_made_again_and_another_asking_is() {
        let ask = list(&["a"]);
        assert_eq!(
            state(&seen(None, Some(7), false), &ask, FORCE, false),
            State::PassedOver
        );
        assert_eq!(
            state(&seen(None, Some(8), false), &ask, FORCE, false),
            State::Due
        );
    }

    #[test]
    fn redo_asks_again_for_another_instruction_provider_or_model_or_a_pass_and_for_nothing_else() {
        let held = answered(&["a"]);
        let ask = list(&["a"]);
        for force in [("j", "p", "m"), ("i", "q", "m"), ("i", "p", "n")] {
            assert_eq!(
                state(&seen(Some(&held), None, false), &ask, force, true),
                State::Due,
                "{force:?}"
            );
        }
        assert_eq!(
            state(&seen(Some(&held), None, false), &ask, FORCE, true),
            State::Settled
        );
        let covered = answered(&["a", "b", "c"]);
        let both = list(&["a", "b"]);
        assert_eq!(
            state(&seen(Some(&covered), Some(7), false), &both, FORCE, true),
            State::Due
        );
        assert_eq!(
            state(&seen(Some(&covered), Some(7), false), &both, FORCE, false),
            State::Settled
        );
        for redo in [true, false] {
            assert_eq!(
                state(&seen(Some(&covered), Some(8), false), &both, FORCE, redo),
                State::Settled
            );
        }
    }

    #[test]
    fn only_an_answer_adds_names() {
        let held = answered(&["a"]);
        let ask = list(&["a", "b"]);
        let found = seen(Some(&held), Some(8), false);
        assert_eq!(state(&found, &ask, FORCE, false), State::Due);
        assert_eq!(held.names, list(&["a"]));
        assert_eq!(names(&found, &ask), list(&["a", "b"]));
    }
}
