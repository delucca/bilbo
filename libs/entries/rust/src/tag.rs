use std::collections::BTreeSet;

use bilbo_settings::key::{About as KeyAbout, Default, Key};
use bilbo_store::entries::{Entries, Meta, StoreError};
use bilbo_store::id::Id;
use bilbo_store::labels::{By, Labels, Standing, StoredLabels, Tag, Tags};

pub const CAP: Key<usize> = Key {
    about: KeyAbout {
        name: "tag.cap",
        variable: Some("BILBO_TAG_CAP"),
        default: Default::Toml("32"),
        allowed: "a whole number from 1 to 64",
        doc: "The most tags an entry may carry after a tag it did not carry is added: such an addition is refused past it, and a model's picks beyond it are left off. The tags an entry already carries are kept and read whatever their number.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|cap| usize::try_from(cap).ok())
            .filter(|cap| (1..=64).contains(cap))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

#[derive(Debug, Clone, Copy)]
pub enum Change<'a> {
    Person {
        add: &'a BTreeSet<Tag>,
        remove: &'a BTreeSet<Tag>,
    },
    Model {
        picks: &'a [Tag],
        takes_back: bool,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    pub tags: Tags,
    pub added: Vec<Tag>,
    pub taken: Vec<Tag>,
    pub left_off: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Full;

pub fn apply(tags: &Tags, change: &Change<'_>, cap: usize) -> Result<Applied, Full> {
    let mut applied = Applied {
        tags: tags.clone(),
        ..Applied::default()
    };
    match change {
        Change::Person { add, remove } => {
            for tag in *remove {
                if matches!(tags.standing(tag), Some(Standing::Carried(_))) {
                    applied.tags.set(tag.clone(), Some(Standing::Removed));
                    applied.taken.push(tag.clone());
                }
            }
            let mut new = false;
            for tag in *add {
                match tags.standing(tag) {
                    Some(Standing::Carried(By::Person)) => {}
                    Some(Standing::Carried(By::Model)) => {
                        applied
                            .tags
                            .set(tag.clone(), Some(Standing::Carried(By::Person)));
                    }
                    Some(Standing::Removed) | None => {
                        new = true;
                        applied
                            .tags
                            .set(tag.clone(), Some(Standing::Carried(By::Person)));
                        applied.added.push(tag.clone());
                    }
                }
            }
            if new && applied.tags.carried().count() > cap {
                return Err(Full);
            }
        }
        Change::Model { picks, takes_back } => {
            if *takes_back {
                for (tag, by) in tags.carried() {
                    if by == By::Model && !picks.contains(tag) {
                        applied.tags.set(tag.clone(), None);
                        applied.taken.push(tag.clone());
                    }
                }
            }
            let mut carried = applied.tags.carried().count();
            for tag in *picks {
                if applied.tags.standing(tag).is_some() {
                    continue;
                }
                if carried < cap {
                    applied
                        .tags
                        .set(tag.clone(), Some(Standing::Carried(By::Model)));
                    applied.added.push(tag.clone());
                    carried += 1;
                } else {
                    applied.left_off += 1;
                }
            }
        }
    }
    Ok(applied)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared {
    pub applied: Applied,
    pub wrote: bool,
    pub bytes: u64,
}

#[derive(Debug)]
pub enum Error {
    Full(Id),
    Gone(Id),
    Store(StoreError),
}

impl From<StoreError> for Error {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

pub fn declare(
    id: Id,
    change: &Change<'_>,
    cap: usize,
    entries: &mut dyn Entries,
) -> Result<Declared, Error> {
    let meta = entries.meta(id)?.ok_or(Error::Gone(id))?;
    let applied = apply(&meta.labels.tags, change, cap).map_err(|Full| Error::Full(id))?;
    let wrote = applied.tags != meta.labels.tags;
    let bytes = if wrote {
        entries.set_meta(
            id,
            &Meta {
                labels: StoredLabels {
                    tags: applied.tags.clone(),
                    ..meta.labels
                },
                ..meta
            },
        )?
    } else {
        0
    };
    Ok(Declared {
        applied,
        wrote,
        bytes,
    })
}

pub fn unused(tags: &BTreeSet<Tag>, labels: &dyn Labels) -> Result<Vec<Tag>, StoreError> {
    let used: BTreeSet<Tag> = labels
        .tags()?
        .into_iter()
        .map(|tagged| tagged.tag)
        .collect();
    Ok(tags.difference(&used).cloned().collect())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use bilbo_settings::file::File;
    use bilbo_settings::key::{Origin as From, Source};
    use bilbo_store::labels::{By, Standing, Tag, Tags};

    use super::{Applied, CAP, Change, Full, apply};

    const WIDE: usize = 10;

    fn tag(name: &str) -> Tag {
        Tag::new(name).expect("a tag")
    }

    fn tags(pairs: &[(&str, Option<Standing>)]) -> Tags {
        let mut tags = Tags::default();
        for (name, standing) in pairs {
            tags.set(tag(name), *standing);
        }
        tags
    }

    fn set(names: &[&str]) -> BTreeSet<Tag> {
        names.iter().map(|name| tag(name)).collect()
    }

    fn person() -> Option<Standing> {
        Some(Standing::Carried(By::Person))
    }

    fn model() -> Option<Standing> {
        Some(Standing::Carried(By::Model))
    }

    fn removed() -> Option<Standing> {
        Some(Standing::Removed)
    }

    fn universe() -> Vec<Tags> {
        let standings = [None, person(), model(), removed()];
        let mut all = Vec::new();
        for a in standings {
            for b in standings {
                for c in standings {
                    all.push(tags(&[("a", a), ("b", b), ("c", c)]));
                }
            }
        }
        all
    }

    fn pick_lists() -> Vec<Vec<Tag>> {
        let names = ["a", "b", "c"];
        let mut lists = vec![Vec::new()];
        let mut next = 0;
        while next < lists.len() {
            let list = lists[next].clone();
            for name in names {
                let name = tag(name);
                if !list.contains(&name) {
                    let mut longer = list.clone();
                    longer.push(name);
                    lists.push(longer);
                }
            }
            next += 1;
        }
        lists
    }

    fn every_model_case(mut check: impl FnMut(&Tags, &[Tag], bool, usize, &Applied)) {
        for before in universe() {
            for picks in pick_lists() {
                for takes_back in [false, true] {
                    for cap in 0..=3 {
                        let change = Change::Model {
                            picks: &picks,
                            takes_back,
                        };
                        let applied = apply(&before, &change, cap).expect("a model is never full");
                        check(&before, &picks, takes_back, cap, &applied);
                    }
                }
            }
        }
    }

    fn person_change(before: &Tags, add: &[&str], remove: &[&str]) -> Result<Applied, Full> {
        let (add, remove) = (set(add), set(remove));
        apply(
            before,
            &Change::Person {
                add: &add,
                remove: &remove,
            },
            WIDE,
        )
    }

    fn model_change(before: &Tags, picks: &[&str], takes_back: bool) -> Applied {
        let picks: Vec<Tag> = picks.iter().map(|name| tag(name)).collect();
        apply(
            before,
            &Change::Model {
                picks: &picks,
                takes_back,
            },
            WIDE,
        )
        .expect("never full")
    }

    fn applied(pairs: &[(&str, Option<Standing>)], added: &[&str], taken: &[&str]) -> Applied {
        Applied {
            tags: tags(pairs),
            added: added.iter().map(|name| tag(name)).collect(),
            taken: taken.iter().map(|name| tag(name)).collect(),
            left_off: 0,
        }
    }

    #[test]
    fn a_persons_change_gives_each_standing_what_the_table_says() {
        let absent = tags(&[]);
        let by_person = tags(&[("a", person())]);
        let by_model = tags(&[("a", model())]);
        let gone = tags(&[("a", removed())]);
        assert_eq!(
            person_change(&absent, &["a"], &[]),
            Ok(applied(&[("a", person())], &["a"], &[]))
        );
        assert_eq!(
            person_change(&by_person, &["a"], &[]),
            Ok(applied(&[("a", person())], &[], &[]))
        );
        assert_eq!(
            person_change(&by_model, &["a"], &[]),
            Ok(applied(&[("a", person())], &[], &[]))
        );
        assert_eq!(
            person_change(&gone, &["a"], &[]),
            Ok(applied(&[("a", person())], &["a"], &[]))
        );
        assert_eq!(
            person_change(&absent, &[], &["a"]),
            Ok(applied(&[], &[], &[]))
        );
        assert_eq!(
            person_change(&by_person, &[], &["a"]),
            Ok(applied(&[("a", removed())], &[], &["a"]))
        );
        assert_eq!(
            person_change(&by_model, &[], &["a"]),
            Ok(applied(&[("a", removed())], &[], &["a"]))
        );
        assert_eq!(
            person_change(&gone, &[], &["a"]),
            Ok(applied(&[("a", removed())], &[], &[]))
        );
    }

    #[test]
    fn a_models_change_gives_each_standing_what_the_table_says() {
        let absent = tags(&[]);
        let by_person = tags(&[("a", person())]);
        let by_model = tags(&[("a", model())]);
        let gone = tags(&[("a", removed())]);
        assert_eq!(
            model_change(&absent, &["a"], false),
            applied(&[("a", model())], &["a"], &[])
        );
        assert_eq!(model_change(&absent, &[], true), applied(&[], &[], &[]));
        assert_eq!(model_change(&absent, &[], false), applied(&[], &[], &[]));
        for (before, standing) in [(&by_person, person()), (&gone, removed())] {
            for (picks, takes_back) in [(&["a"][..], false), (&[][..], true), (&[][..], false)] {
                assert_eq!(
                    model_change(before, picks, takes_back),
                    applied(&[("a", standing)], &[], &[])
                );
            }
        }
        assert_eq!(
            model_change(&by_model, &["a"], false),
            applied(&[("a", model())], &[], &[])
        );
        assert_eq!(
            model_change(&by_model, &[], true),
            applied(&[], &[], &["a"])
        );
        assert_eq!(
            model_change(&by_model, &[], false),
            applied(&[("a", model())], &[], &[])
        );
    }

    #[test]
    fn a_models_change_never_touches_a_persons_tag_or_a_removed_name() {
        every_model_case(|before, _, _, _, applied| {
            for name in ["a", "b", "c"] {
                let name = tag(name);
                if let Some(Standing::Carried(By::Person) | Standing::Removed) =
                    before.standing(&name)
                {
                    assert_eq!(applied.tags.standing(&name), before.standing(&name));
                }
            }
        });
    }

    #[test]
    fn a_models_change_applied_twice_is_the_change_applied_once() {
        every_model_case(|_, picks, takes_back, cap, once| {
            let change = Change::Model { picks, takes_back };
            let twice = apply(&once.tags, &change, cap).expect("a model is never full");
            assert_eq!(twice.tags, once.tags);
        });
    }

    #[test]
    fn what_a_models_change_takes_back_and_what_it_adds_never_meet() {
        every_model_case(|_, picks, _, _, applied| {
            for name in &applied.added {
                assert!(!applied.taken.contains(name), "{name}");
                assert!(picks.contains(name), "{name}");
            }
            for name in &applied.taken {
                assert!(!picks.contains(name), "{name}");
            }
        });
    }

    #[test]
    fn the_cap_counts_what_is_carried_after_the_take_backs_and_leaves_the_last_picks_off() {
        let apply_at = |before: &Tags, picks: &[&str], takes_back: bool, cap: usize| {
            let picks: Vec<Tag> = picks.iter().map(|name| tag(name)).collect();
            apply(
                before,
                &Change::Model {
                    picks: &picks,
                    takes_back,
                },
                cap,
            )
            .expect("never full")
        };
        let by_model = tags(&[("a", model())]);
        let one = apply_at(&by_model, &["b"], true, 1);
        assert_eq!((one.tags, one.left_off), (tags(&[("b", model())]), 0));
        let two = apply_at(&by_model, &["b", "c"], true, 1);
        assert_eq!(
            (two.tags, two.added, two.left_off),
            (tags(&[("b", model())]), vec![tag("b")], 1)
        );
        let by_person = tags(&[("a", person())]);
        for takes_back in [false, true] {
            let kept = apply_at(&by_person, &["b"], takes_back, 1);
            assert_eq!(
                (kept.tags, kept.added, kept.left_off),
                (by_person.clone(), Vec::new(), 1)
            );
        }
        every_model_case(|before, _, _, cap, applied| {
            let carried = |tags: &Tags| tags.carried().count();
            assert!(carried(&applied.tags) <= cap.max(carried(before)));
        });
    }

    #[test]
    fn a_persons_addition_past_the_cap_is_full_and_a_removal_or_a_promotion_never_is() {
        let at = |before: &Tags, add: &[&str], remove: &[&str], cap: usize| {
            let (add, remove) = (set(add), set(remove));
            apply(
                before,
                &Change::Person {
                    add: &add,
                    remove: &remove,
                },
                cap,
            )
        };
        let full = tags(&[("a", person())]);
        assert_eq!(at(&full, &["b"], &[], 1), Err(Full));
        assert!(at(&full, &["a"], &[], 1).is_ok());
        assert!(at(&full, &[], &["a"], 1).is_ok());
        let gone = tags(&[("a", person()), ("b", removed())]);
        assert_eq!(at(&gone, &["b"], &[], 1), Err(Full));
        let by_model = tags(&[("a", model())]);
        let promoted = at(&by_model, &["a"], &[], 1).expect("a promotion");
        assert_eq!(promoted.tags, tags(&[("a", person())]));
        let over = tags(&[("a", person()), ("b", person()), ("c", person())]);
        let taken = at(&over, &[], &["a"], 1).expect("a removal");
        assert_eq!(taken.taken, vec![tag("a")]);
        let kept = at(&over, &["b"], &[], 1).expect("a carried name");
        assert_eq!(kept.tags, over);
        assert_eq!(at(&over, &["d"], &[], 1), Err(Full));
        let mixed = tags(&[("a", model()), ("b", person()), ("c", person())]);
        let promoted = at(&mixed, &["a"], &[], 1).expect("a promotion over the cap");
        assert_eq!(promoted.tags, over);
    }

    #[test]
    fn the_cap_is_a_whole_number_from_1_to_64_and_32_by_default() {
        let at = |text: Option<&str>| {
            let file = File::parse(Path::new("/c/config.toml"), b"").expect("toml");
            let var = |name: &str| {
                (name == "BILBO_TAG_CAP")
                    .then(|| text.map(Into::into))
                    .flatten()
            };
            CAP.resolve(&Source {
                var: &var,
                file: &file,
            })
            .map(|resolved| (resolved.value, resolved.origin))
            .map_err(|error| error.to_string())
        };
        assert_eq!(at(None), Ok((32, From::Default)));
        assert_eq!(at(Some("1")).map(|(cap, _)| cap), Ok(1));
        assert_eq!(at(Some("64")).map(|(cap, _)| cap), Ok(64));
        for text in ["0", "65", "some"] {
            assert_eq!(
                at(Some(text)),
                Err(format!(
                    "BILBO_TAG_CAP must be a whole number from 1 to 64, got \"{text}\""
                )),
                "{text}"
            );
        }
    }
}
