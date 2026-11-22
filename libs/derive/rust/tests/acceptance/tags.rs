use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Duration;

use bilbo_derive::plan::{Asked, TagsState};
use bilbo_derive::record;
use bilbo_derive::run::{Ended, Event, Which};
use bilbo_derive::spool::Spool;
use bilbo_derive::tags::asked::{Answered, Asked as TagsAsked, Line};
use bilbo_derive::tags::{self, answer::Why};
use bilbo_llm::model::{Cause, Limits, Prompt, What};
use bilbo_store::entries::{Entries, Generation};
use bilbo_store::id::Id;
use bilbo_store::labels::{By, Standing, Tag, Tags};

use crate::fakes::{Script, World, answer, busy, cut, failed, refused, stopped, write_paragraph};

fn tag(name: &str) -> Tag {
    Tag::new(name).expect("a tag")
}

fn names(list: &[&str]) -> BTreeSet<Tag> {
    list.iter().map(|name| tag(name)).collect()
}

fn tagging(pick: &[&str]) -> World {
    let mut world = World::new();
    let mut section = tags::keys::defaults();
    section.enabled.value = true;
    section.pick.value = names(pick);
    world.tags = Some(section);
    world
}

fn noted(world: &mut World, records: &[&str]) -> Id {
    let parent = world.entry("plain", records);
    let covers = records.len() as u64;
    world.notes(parent, covers, |_| {});
    parent
}

fn model_tags(world: &World, parent: Id) -> Vec<(String, By)> {
    world
        .shelf
        .tags_of(parent)
        .carried()
        .map(|(tag, by)| (tag.as_str().to_owned(), by))
        .collect()
}

fn blocks(world: &World) -> Vec<tags::block::Block> {
    world
        .spooled
        .files
        .iter()
        .filter_map(|(_, bytes)| tags::block::read(bytes).ok())
        .collect()
}

#[test]
fn with_the_tags_off_a_run_asks_the_model_and_the_tags_file_nothing_more() {
    let mut world = World::new();
    noted(&mut world, &["aaaa"]);
    let mut section = tags::keys::defaults();
    section.pick.value = names(&["garden"]);
    world.tags = Some(section);
    let mut writer = Script::new();
    let done = world.derive(&mut writer);
    assert!(writer.prompts.is_empty());
    assert_eq!(world.memo.asks.get(), 0);
    assert_eq!(done.report.counts.calls, 0);
    assert!(world.plan(Asked::Every).due.is_empty());

    let mut reference = World::new();
    reference.entry("plain", &["aaaa"]);
    let mut expected = Script::new().then(answer("r1\nThe user asked about a topic."));
    let plan = reference.plan(Asked::Every);
    reference.calls(&mut expected, &plan);

    let mut world = World::new();
    let carrying = world.entry("plain", &["aaaa"]);
    let mut held = Tags::default();
    held.set(tag("garden"), Some(Standing::Carried(By::Model)));
    world.shelf.put_tags(carrying, held);
    let mut section = tags::keys::defaults();
    section.pick.value = names(&["garden"]);
    world.tags = Some(section);
    let mut writer = Script::new().then(answer("r1\nThe user asked about a topic."));
    let plan = world.plan(Asked::Every);
    world.format_reads.set(0);
    world.calls(&mut writer, &plan);
    assert_eq!(world.spooled.blocks().len(), 1);
    assert_eq!(world.format_reads.get(), 2);
    assert_eq!(writer.prompts, expected.prompts);
    assert_eq!(writer.limits, expected.limits);
    assert_eq!(world.memo.asks.get(), 0);
}

#[test]
fn with_an_empty_list_no_tags_call_is_made() {
    let mut world = tagging(&[]);
    noted(&mut world, &["aaaa"]);
    let mut writer = Script::new();
    let mut tagger = Script::new();
    let plan = world.plan(Asked::Every);
    world.calls_with(&mut writer, Some(&mut tagger), &plan);
    assert!(writer.prompts.is_empty() && tagger.prompts.is_empty());
    assert_eq!(world.memo.asks.get(), 0);
    assert_eq!(tagger.prepared, 0);
}

#[test]
fn an_entry_whose_last_window_is_answered_is_asked_for_tags_right_after() {
    let mut world = tagging(&["garden"]);
    world.section.window.value = 3;
    let first = world.entry("plain", &["aaaa"]);
    let second = world.entry("plain", &["bbbb"]);
    let mut writer = Script::new().always(|prompt, _| {
        if prompt.instruction == tags::instruction::text() {
            answer("garden")
        } else if prompt.material.contains("bbbb") {
            answer("r1\nThe user asked about bbbb.")
        } else {
            answer("r1\nThe user asked about aaaa.")
        }
    });
    let done = world.derive(&mut writer);
    assert_eq!(done.report.counts.calls, 4);
    let seen: Vec<(bool, bool, bool)> = writer
        .prompts
        .iter()
        .map(|prompt| {
            (
                prompt.instruction == tags::instruction::text(),
                prompt.material.contains("bbbb"),
                prompt.material.contains("aaaa"),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            (false, true, false),
            (true, true, false),
            (false, false, true),
            (true, false, true)
        ]
    );
    assert_eq!(done.replayed.tagged, 2);
    for parent in [first, second] {
        assert_eq!(
            model_tags(&world, parent),
            [("garden".to_owned(), By::Model)]
        );
    }
}

#[test]
fn an_entry_with_a_window_that_failed_is_not_asked_for_tags() {
    let mut world = tagging(&["garden"]);
    let parent = world.entry("plain", &["aaaa", "bbbb"]);
    world.section.window.value = 3;
    world.notes(parent, 1, |_| {});
    let mut writer = Script::new().then(failed(What::Rejected));
    let done = world.derive(&mut writer);
    assert_eq!(writer.prompts.len(), 1);
    assert_eq!(done.report.counts.failed, 1);
}

#[test]
fn an_entry_with_complete_notes_and_no_answer_is_asked_with_no_window() {
    let mut world = tagging(&["garden"]);
    let parent = noted(&mut world, &["aaaa"]);
    let mut writer = Script::new().then(answer("garden"));
    let plan = world.plan(Asked::Every);
    assert_eq!(plan.due.len(), 1);
    assert!(plan.due[0].windows.is_empty());
    let report = world.calls(&mut writer, &plan);
    assert_eq!(report.counts.calls, 1);
    assert_eq!(writer.prompts[0].instruction, tags::instruction::text());
    let replayed = world.replay();
    assert_eq!(replayed.tagged, 1);
    assert_eq!(
        model_tags(&world, parent),
        [("garden".to_owned(), By::Model)]
    );
    assert!(blocks(&world).is_empty());
    assert!(world.memo.lines.iter().any(
        |line| matches!(line, Line::Answered(answered) if answered.entry == parent && answered.names == names(&["garden"]))
    ));
}

#[test]
fn an_entry_with_a_run_record_and_no_note_is_not_asked() {
    let mut world = tagging(&["garden"]);
    let parent = world.entry("plain", &["aaaa"]);
    let run = world.run_record(parent, 1, 1);
    let records = vec![bilbo_derive::record::write(&run).into_bytes()];
    world.note_entry(parent, records);
    let plan = world.plan(Asked::Every);
    assert!(plan.due.is_empty());
    assert!(plan.tags.contains(&(parent, TagsState::NoNote)));
}

#[test]
fn notes_of_an_older_generation_alone_are_not_asked_about() {
    let mut world = tagging(&["garden"]);
    let parent = world.entry("folded", &["aaaa"]);
    world.notes(parent, 1, |_| {});
    world.shelf.regenerate(parent, &["----"]);
    assert_eq!(
        world.shelf.extent(parent, None).unwrap().unwrap().0,
        Generation(2)
    );
    let plan = world.plan(Asked::Every);
    assert!(plan.due.is_empty());
    assert!(plan.tags.contains(&(parent, TagsState::NotComplete)));
    assert_eq!(world.memo.asks.get(), 0);
    let mut writer = Script::new();
    let report = world.calls(&mut writer, &plan);
    assert_eq!(report.counts.calls, 0);
    assert!(writer.prompts.is_empty());
}

#[test]
fn an_entry_with_no_record_is_not_asked_and_nothing_is_read_for_it() {
    let mut world = tagging(&["garden"]);
    let parent = world.entry("plain", &[]);
    windows_noted(&mut world, parent, &[(1, "A note.")]);
    let notes = world.shelf.notes_item(parent);
    let plan = world.plan(Asked::Every);
    assert!(plan.due.is_empty());
    assert!(plan.tags.contains(&(parent, TagsState::NotComplete)));
    assert_eq!(world.memo.asks.get(), 0);
    assert_eq!(world.shelf.reads_of(notes), 0);
}

#[test]
fn a_name_with_no_answer_asks_again_from_all_the_notes() {
    let mut world = tagging(&["a", "b"]);
    let parent = world.entry("plain", &["aaaa", "bbbb"]);
    windows_noted(
        &mut world,
        parent,
        &[(1, "First note."), (2, "Second note.")],
    );
    let notes_head = world
        .shelf
        .by_capture_head(parent)
        .expect("the notes' head");
    world
        .memo
        .answer(
            &Answered {
                entry: parent,
                notes: notes_head,
                instruction: world.tags_id.clone(),
                provider: "fake".to_owned(),
                model: "model-a".to_owned(),
                names: names(&["a"]),
            },
            1,
        )
        .unwrap();
    let mut writer = Script::new().then(answer("b"));
    let plan = world.plan(Asked::Every);
    assert_eq!(plan.due.len(), 1);
    world.calls(&mut writer, &plan);
    assert_eq!(writer.prompts.len(), 1);
    assert_eq!(
        writer.prompts[0].material,
        "<notes>\nr1 2023-11-14\nFirst note.\n\nr2 2023-11-14\nSecond note.\n\n</notes>\n<tags>\na\nb\n</tags>\nWrite the tags for these notes now, one a line, at most 3, or (none).\n"
    );
    world.replay();
    let line = world
        .memo
        .lines
        .iter()
        .find_map(|line| match line {
            Line::Answered(answered) => Some(answered.names.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(line, names(&["a", "b"]));
}

#[test]
fn an_entry_its_answer_settles_costs_no_read_of_its_notes() {
    let mut world = tagging(&["a"]);
    let parent = noted(&mut world, &["aaaa"]);
    let notes_head = world.shelf.by_capture_head(parent).unwrap();
    world
        .memo
        .answer(
            &Answered {
                entry: parent,
                notes: notes_head,
                instruction: world.tags_id.clone(),
                provider: "fake".to_owned(),
                model: "model-a".to_owned(),
                names: names(&["a"]),
            },
            1,
        )
        .unwrap();
    world.shelf.reads.borrow_mut().clear();
    world.shelf.ranges.borrow_mut().clear();
    let plan = world.plan(Asked::Every);
    assert!(plan.due.is_empty());
    assert!(plan.tags.contains(&(parent, TagsState::Asked)));
    assert_eq!(world.shelf.reads.borrow().len(), 1);
    let notes = world.shelf.notes_item(parent);
    assert_eq!(
        *world.shelf.ranges.borrow(),
        [(notes, notes_head.0.0, notes_head.1, 1)]
    );
}

#[test]
fn a_tags_call_takes_one_of_derive_calls() {
    let mut world = tagging(&["garden"]);
    world.section.calls.value = 2;
    world.entry("plain", &["aaaa"]);
    world.entry("plain", &["bbbb"]);
    let mut writer = Script::new().always(|prompt, _| {
        if prompt.instruction == tags::instruction::text() {
            answer("garden")
        } else {
            answer("r1\nA note.")
        }
    });
    let done = world.derive(&mut writer);
    assert_eq!(done.report.ended, Ended::Calls);
    assert_eq!(writer.prompts.len(), 2);
}

#[test]
fn an_entry_whose_last_window_is_passed_over_is_still_asked_for_tags() {
    let mut world = tagging(&["garden"]);
    let parent = world.entry("plain", &["aaaa", "bbbb"]);
    world.section.window.value = 1;
    world.section.tries.value = 1;
    world.notes(parent, 1, |_| {});
    let plan = world.plan(Asked::Every);
    let extent = plan.due[0].windows[0].extent;
    world.held.charges.push((extent, 1));
    let plan = world.plan(Asked::Every);
    assert!(plan.due[0].windows.iter().all(|planned| planned.pass));
    let mut writer = Script::new().then(answer("garden"));
    let report = world.calls(&mut writer, &plan);
    assert_eq!(report.counts.passed, 1);
    assert_eq!(writer.prompts.len(), 1);
    assert_eq!(writer.prompts[0].instruction, tags::instruction::text());
}

#[test]
fn the_tags_call_carries_the_tags_instruction_the_material_and_the_runs_limits() {
    let mut world = tagging(&["garden"]);
    noted(&mut world, &["aaaa"]);
    let mut writer = Script::new().then(answer("garden"));
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    assert_eq!(writer.prompts[0].instruction, tags::instruction::text());
    assert_eq!(
        writer.prompts[0].material,
        "<notes>\nr1 2023-11-14\nEarlier paragraph.\n\n</notes>\n<tags>\ngarden\n</tags>\nWrite the tags for these notes now, one a line, at most 3, or (none).\n"
    );
    assert_eq!(writer.limits[0], world.limits);
}

#[test]
fn an_answer_with_picks_spools_one_block_and_reaches_no_store() {
    let mut world = tagging(&["garden", "bilbo"]);
    let parent = noted(&mut world, &["aaaa"]);
    let mut writer = Script::new().then(answer("garden\nbilbo"));
    let plan = world.plan(Asked::Every);
    let touched = world.shelf.touched.get();
    world.calls(&mut writer, &plan);
    assert_eq!(world.shelf.touched.get(), touched);
    assert_eq!(world.spooled.files.len(), 1);
    let held = blocks(&world);
    assert_eq!(held.len(), 1);
    let about = tags::asked::About {
        generation: Generation(1),
        records: 1,
        notes: world.notes_digest(parent),
    };
    let material = "<notes>\nr1 2023-11-14\nEarlier paragraph.\n\n</notes>\n<tags>\nbilbo\ngarden\n</tags>\nWrite the tags for these notes now, one a line, at most 3, or (none).\n";
    assert_eq!(writer.prompts[0].material, material);
    let asking = tags::asked::asking(parent, &about, "fake", "model-a", &world.tags_id, material);
    assert_eq!(
        held[0],
        tags::block::Block {
            entry: parent,
            about,
            picks: vec![tag("garden"), tag("bilbo")],
            takes_back: false,
            names: names(&["bilbo", "garden"]),
            instruction: world.tags_id.clone(),
            provider: "fake".to_owned(),
            model: "model-a".to_owned(),
            asking,
            passed: false,
        }
    );
}

#[test]
fn the_none_line_ends_the_asking_with_no_pick_and_is_no_failure() {
    let mut world = tagging(&["garden"]);
    noted(&mut world, &["aaaa"]);
    let mut writer = Script::new().then(answer("(none)"));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    assert_eq!(report.counts.failed, 0);
    assert!(blocks(&world)[0].picks.is_empty());
    assert!(
        world
            .lines
            .0
            .iter()
            .any(|line| line.starts_with("t1\t") && line.split('\t').nth(2) == Some("none"))
    );
}

#[test]
fn an_answer_that_names_no_tag_is_a_failed_call_charged_at_once() {
    for (text, why, word) in [
        ("", Why::Empty, "empty"),
        ("```\n-", Why::Marks, "marks"),
        ("zzz", Why::Strays, "strays"),
    ] {
        let mut world = tagging(&["garden"]);
        let parent = noted(&mut world, &["aaaa"]);
        let mut writer = Script::new().then(answer(text));
        let plan = world.plan(Asked::Every);
        let report = world.calls(&mut writer, &plan);
        assert_eq!(report.counts.failed, 1);
        assert_eq!(
            report.events,
            [Event::NoTag { entry: parent, why }],
            "{text:?}"
        );
        assert_eq!(world.memo.charges_of(parent), 1);
        assert!(blocks(&world).is_empty());
        let line = world.lines.0.last().unwrap();
        assert_eq!(
            line.split('\t').nth(15),
            Some(format!("no-tag {word}").as_str())
        );
    }
}

#[test]
fn an_answer_that_says_both_is_a_failed_call_of_its_own_kind() {
    let mut world = tagging(&["garden"]);
    let parent = noted(&mut world, &["aaaa"]);
    let mut writer = Script::new().then(answer("garden\n(none)"));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    assert_eq!(
        report.events,
        [Event::NoTag {
            entry: parent,
            why: Why::Both
        }]
    );
    assert!(blocks(&world).is_empty());
}

#[test]
fn a_refusal_of_a_tags_call_is_charged_at_once() {
    let mut world = tagging(&["garden"]);
    let parent = noted(&mut world, &["aaaa"]);
    let mut writer = Script::new().then(refused(Some("policy")));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    assert_eq!(report.counts.refused, 1);
    assert_eq!(world.memo.charges_of(parent), 1);
    assert_eq!(
        report.events,
        [Event::TagsRefused {
            entry: parent,
            category: Some("policy".to_owned())
        }]
    );
}

#[test]
fn a_cut_answer_of_a_tags_call_waits_for_proof_as_a_windows_does() {
    let mut world = tagging(&["garden"]);
    let parent = noted(&mut world, &["aaaa"]);
    let mut writer = Script::new().then(cut());
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    assert_eq!(world.memo.charges_of(parent), 0);

    let mut world = tagging(&["garden"]);
    let tagged = noted(&mut world, &["aaaa"]);
    world.entry("plain", &["bbbb"]);
    let mut writer = Script::new().always(|prompt, _| {
        if prompt.instruction == tags::instruction::text() {
            cut()
        } else {
            answer("r1\nA note.")
        }
    });
    world.derive(&mut writer);
    assert_eq!(world.memo.charges_of(tagged), 1);
}

#[test]
fn an_asking_at_derive_tries_is_passed_over_with_no_call_and_kept_as_a_pass() {
    let mut world = tagging(&["garden"]);
    world.section.tries.value = 2;
    let parent = noted(&mut world, &["aaaa"]);
    let mut first = Script::new().then(refused(None));
    let plan = world.plan(Asked::Every);
    world.calls(&mut first, &plan);
    let asking = match world.memo.lines.last() {
        Some(Line::Charged { asking, .. }) => *asking,
        other => panic!("{other:?}"),
    };
    world.memo.lines.clear();
    world.memo.lines.push(Line::Charged {
        entry: parent,
        asking,
        charges: 3,
    });
    let mut writer = Script::new();
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    assert!(writer.prompts.is_empty());
    assert_eq!(
        report.events,
        [Event::TagsPassed {
            entry: parent,
            charges: 3
        }]
    );
    assert!(blocks(&world)[0].passed);
    let line = world.lines.0.last().unwrap();
    assert_eq!(line.split('\t').nth(2), Some("passed"));
    world.replay();
    assert!(world.memo.lines.contains(&Line::Passed {
        entry: parent,
        asking
    }));
}

#[test]
fn a_charge_is_of_one_asking() {
    let mut world = tagging(&["garden"]);
    let parent = noted(&mut world, &["aaaa"]);
    world.provider = "a".to_owned();
    for _ in 0..2 {
        let mut writer = Script::new().then(refused(None));
        world.derive(&mut writer);
    }
    let plan = world.plan(Asked::Every);
    let (under_a, charges) = asking_of(&plan);
    assert_eq!(charges, 2);
    assert_eq!(world.memo.charges(parent, under_a).unwrap(), 2);
    world.provider = "b".to_owned();
    let plan = world.plan(Asked::Every);
    let (under_b, charges) = asking_of(&plan);
    assert_ne!(under_a, under_b);
    assert_eq!(charges, 0);
    assert_eq!(world.memo.charges(parent, under_b).unwrap(), 0);
}

#[test]
fn the_tags_own_model_is_prepared_before_the_first_tags_call_and_never_in_a_run_with_none() {
    let mut world = tagging(&["garden"]);
    world.entry("plain", &["aaaa"]);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut writer = Script::new().then(answer("r1\nA note."));
    writer.seen = Some(Rc::clone(&seen));
    let mut tagger = Script::new().then(answer("garden"));
    tagger.name = "tagger";
    tagger.seen = Some(Rc::clone(&seen));
    let plan = world.plan(Asked::Every);
    world.calls_with(&mut writer, Some(&mut tagger), &plan);
    assert_eq!(
        *seen.borrow(),
        [
            "prepare:writer",
            "call:writer",
            "prepare:tagger",
            "bound:tagger",
            "call:tagger"
        ]
    );
    let mut quiet = World::new();
    quiet.entry("plain", &["aaaa"]);
    let mut quiet_tagger = Script::new();
    let mut quiet_writer = Script::new().then(failed(What::Rejected));
    let plan = quiet.plan(Asked::Every);
    quiet.calls_with(&mut quiet_writer, Some(&mut quiet_tagger), &plan);
    assert_eq!(quiet_tagger.prepared, 0);
}

#[test]
fn the_shared_model_is_asked_whether_it_is_bound_at_the_first_tags_call_and_not_before() {
    let mut world = tagging(&["garden"]);
    world.entry("plain", &["aaaa"]);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut writer = Script::new().always(|prompt, _| {
        if prompt.instruction == tags::instruction::text() {
            answer("garden")
        } else {
            answer("r1\nA note.")
        }
    });
    writer.seen = Some(Rc::clone(&seen));
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    assert_eq!(
        *seen.borrow(),
        [
            "prepare:writer",
            "call:writer",
            "bound:writer",
            "call:writer"
        ]
    );

    let mut world = tagging(&["garden"]);
    world.section.window.value = 3;
    world.section.calls.value = 2;
    world.entry("plain", &["aaaa", "bbbb"]);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut writer = scripted("garden");
    writer.seen = Some(Rc::clone(&seen));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    assert_eq!(report.ended, Ended::Calls);
    assert_eq!(
        *seen.borrow(),
        ["prepare:writer", "call:writer", "call:writer"]
    );
}

#[test]
fn a_tags_call_the_budget_cannot_pay_prepares_and_binds_nothing() {
    let mut world = tagging(&["garden"]);
    world.section.calls.value = 1;
    world.entry("plain", &["aaaa"]);
    let mut writer = Script::new().then(answer("r1\nA note."));
    let mut tagger = Script::new();
    tagger.refuse_bound = Some("unbound".to_owned());
    let seen = Rc::new(RefCell::new(Vec::new()));
    tagger.seen = Some(Rc::clone(&seen));
    let plan = world.plan(Asked::Every);
    let report = world.calls_with(&mut writer, Some(&mut tagger), &plan);
    assert_eq!(report.ended, Ended::Calls);
    assert!(seen.borrow().is_empty());

    let mut world = tagging(&["garden"]);
    world.section.calls.value = 1;
    world.entry("plain", &["aaaa"]);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut writer = Script::new().then(answer("r1\nA note."));
    writer.refuse_bound = Some("unbound".to_owned());
    writer.seen = Some(Rc::clone(&seen));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    assert_eq!(report.ended, Ended::Calls);
    assert_eq!(*seen.borrow(), ["prepare:writer", "call:writer"]);
}

#[test]
fn a_run_of_tags_calls_alone_prepares_the_shared_model_once_before_its_first_call() {
    let mut world = tagging(&["garden"]);
    noted(&mut world, &["aaaa"]);
    noted(&mut world, &["bbbb"]);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut writer = Script::new().always(|_, _| answer("garden"));
    writer.seen = Some(Rc::clone(&seen));
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    assert_eq!(
        *seen.borrow(),
        [
            "prepare:writer",
            "bound:writer",
            "call:writer",
            "call:writer"
        ]
    );
}

#[test]
fn a_model_that_is_not_bound_ends_the_run_with_the_notes_spooled() {
    let mut world = tagging(&["garden"]);
    world.entry("plain", &["aaaa"]);
    let mut writer = Script::new().then(answer("r1\nA note."));
    let mut tagger = Script::new();
    tagger.refuse_bound = Some("unbound".to_owned());
    let plan = world.plan(Asked::Every);
    let report = world.calls_with(&mut writer, Some(&mut tagger), &plan);
    assert_eq!(
        report.ended,
        Ended::Prepare("unbound".to_owned(), Which::Tagger)
    );
    assert_eq!(report.counts.calls, 1);
    assert_eq!(world.spooled.blocks().len(), 1);

    let mut world = tagging(&["garden"]);
    world.entry("plain", &["aaaa"]);
    let mut writer = Script::new().then(answer("r1\nA note."));
    writer.refuse_bound = Some("unbound".to_owned());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    assert_eq!(
        report.ended,
        Ended::Prepare("unbound".to_owned(), Which::Writer)
    );
}

#[test]
fn a_prepare_of_the_tags_model_that_fails_ends_the_run_with_the_notes_spooled() {
    let mut world = tagging(&["garden"]);
    world.entry("plain", &["aaaa"]);
    let mut writer = Script::new().then(answer("r1\nA note."));
    let mut tagger = Script::new();
    tagger.refuse_prepare = Some("no key".to_owned());
    let plan = world.plan(Asked::Every);
    let report = world.calls_with(&mut writer, Some(&mut tagger), &plan);
    assert_eq!(
        report.ended,
        Ended::Prepare("no key".to_owned(), Which::Tagger)
    );
    assert_eq!(world.spooled.blocks().len(), 1);
}

#[test]
fn a_stop_of_the_tags_model_ends_the_run_and_says_whose() {
    let mut world = tagging(&["garden"]);
    let parent = noted(&mut world, &["aaaa"]);
    let mut writer = Script::new();
    let mut tagger = Script::new().then(stopped());
    let plan = world.plan(Asked::Every);
    let report = world.calls_with(&mut writer, Some(&mut tagger), &plan);
    assert!(matches!(report.ended, Ended::Stopped(_, Which::Tagger)));
    assert_eq!(world.memo.charges_of(parent), 0);
}

#[test]
fn one_models_answer_never_charges_the_other_models_slot() {
    let mut world = tagging(&["garden"]);
    world.section.window.value = 3;
    world.entry("plain", &["bbbb"]);
    let tagged = noted(&mut world, &["aaaa"]);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut writer = Script::new().always(|_, _| answer("r1\nA note."));
    writer.seen = Some(Rc::clone(&seen));
    let mut tagger = Script::new().always(|_, _| failed(What::Rejected));
    tagger.name = "tagger";
    tagger.seen = Some(Rc::clone(&seen));
    let plan = world.plan(Asked::Every);
    assert_eq!(plan.due[0].parent.id, tagged);
    let report = world.calls_with(&mut writer, Some(&mut tagger), &plan);
    assert_eq!(report.ended, Ended::Done);
    assert_eq!(
        *seen.borrow(),
        [
            "prepare:writer",
            "prepare:tagger",
            "bound:tagger",
            "call:tagger",
            "call:writer",
            "call:tagger"
        ]
    );
    assert_eq!(world.memo.charges_of(tagged), 0);

    let mut world = tagging(&["garden"]);
    world.section.window.value = 3;
    noted(&mut world, &["aaaa"]);
    world.entry("plain", &["bbbb"]);
    let mut writer = Script::new().then(failed(What::Unreadable));
    let mut tagger = Script::new().then(answer("garden"));
    let plan = world.plan(Asked::Every);
    let extent = plan.due[0].windows[0].extent;
    let report = world.calls_with(&mut writer, Some(&mut tagger), &plan);
    assert_eq!(report.ended, Ended::Done);
    assert_eq!(writer.prompts.len(), 1);
    assert_eq!(tagger.prompts.len(), 1);
    assert_eq!(world.held.of(&extent), 0);
}

#[test]
fn with_one_model_the_evidence_is_one_as_today() {
    let mut world = tagging(&["garden"]);
    let tagged = noted(&mut world, &["aaaa"]);
    let mut writer = Script::new().always(|prompt, _| {
        if prompt.instruction == tags::instruction::text() {
            cut()
        } else {
            answer("r1\nA note.")
        }
    });
    world.entry("plain", &["bbbb"]);
    world.derive(&mut writer);
    assert_eq!(world.memo.charges_of(tagged), 1);
}

#[test]
fn a_replay_declares_the_picks_as_a_models_and_ends_the_asking_in_one_write() {
    let mut world = tagging(&["garden"]);
    let parent = noted(&mut world, &["aaaa"]);
    let mut writer = Script::new().then(answer("garden"));
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    let replayed = world.replay();
    assert_eq!(replayed.tagged, 1);
    assert_eq!(
        model_tags(&world, parent),
        [("garden".to_owned(), By::Model)]
    );
    assert!(blocks(&world).is_empty());
    assert_eq!(
        world
            .memo
            .lines
            .iter()
            .filter(|line| matches!(line, Line::Answered(_)))
            .count(),
        1
    );
}

fn picked(world: &mut World, parent: Id, picks: &[&str], takes_back: bool) {
    let block = tags::block::Block {
        entry: parent,
        about: tags::asked::About {
            generation: Generation(1),
            records: world
                .shelf
                .extent(parent, None)
                .expect("extent")
                .expect("held")
                .1,
            notes: world.notes_digest(parent),
        },
        picks: picks.iter().map(|name| tag(name)).collect(),
        takes_back,
        names: names(picks),
        instruction: world.tags_id.clone(),
        provider: "fake".to_owned(),
        model: "model-a".to_owned(),
        asking: 7,
        passed: false,
    };
    world
        .spooled
        .keep(&tags::block::write(&block))
        .expect("kept");
}

#[test]
fn a_replay_leaves_a_persons_tag_and_a_removed_name_as_they_are() {
    let mut world = tagging(&["a", "b"]);
    let parent = noted(&mut world, &["aaaa"]);
    let mut held = Tags::default();
    held.set(tag("a"), Some(Standing::Carried(By::Person)));
    held.set(tag("b"), Some(Standing::Removed));
    world.shelf.put_tags(parent, held.clone());
    picked(&mut world, parent, &["a", "b"], false);
    let replayed = world.replay();
    assert_eq!(replayed.tagged, 0);
    assert_eq!(world.shelf.tags_of(parent), held);
    assert!(world.shelf.set_metas.is_empty());
}

#[test]
fn an_entry_that_grew_since_the_plan_drops_the_tags_block_alone_and_keeps_the_notes() {
    let mut world = tagging(&["a"]);
    let parent = noted(&mut world, &["aaaa"]);
    picked(&mut world, parent, &["a"], false);
    world.shelf.grow(parent, &["bbbb"]);
    let replayed = world.replay();
    assert_eq!(replayed.tagged, 0);
    assert_eq!(
        replayed.notices,
        [format!(
            "dropped the tags kept for {parent}: the entry changed since they were asked"
        )]
    );
    assert!(blocks(&world).is_empty());
    assert!(world.memo.lines.is_empty());
}

#[test]
fn an_entry_the_store_no_longer_holds_drops_the_tags_block() {
    let mut world = tagging(&["a"]);
    let parent = noted(&mut world, &["aaaa"]);
    picked(&mut world, parent, &["a"], false);
    let mut gone = tags::block::read(&world.spooled.files[0].1).unwrap();
    gone.entry = Id::from_bytes([200; 16]);
    world.spooled.files[0].1 = tags::block::write(&gone);
    let other = noted(&mut world, &["bbbb"]);
    picked(&mut world, other, &["a"], false);
    let replayed = world.replay();
    assert_eq!(
        replayed.notices,
        [format!(
            "dropped the tags kept for {}: the store no longer holds it",
            gone.entry
        )]
    );
    assert_eq!(model_tags(&world, other), [("a".to_owned(), By::Model)]);
    assert!(
        world
            .memo
            .lines
            .iter()
            .any(|line| matches!(line, Line::Answered(answered) if answered.entry == other))
    );
    assert!(blocks(&world).is_empty());
    assert!(world.spooled.files.is_empty());
}

#[test]
fn redo_takes_back_a_models_tag_the_new_answer_does_not_pick_and_no_other() {
    let mut world = tagging(&["a", "b"]);
    world.cap = 2;
    let parent = noted(&mut world, &["aaaa"]);
    let mut held = Tags::default();
    held.set(tag("a"), Some(Standing::Carried(By::Model)));
    held.set(tag("p"), Some(Standing::Carried(By::Person)));
    world.shelf.put_tags(parent, held);
    picked(&mut world, parent, &["b"], true);
    world.replay();
    let mut now = model_tags(&world, parent);
    now.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        now,
        [("b".to_owned(), By::Model), ("p".to_owned(), By::Person)]
    );
}

#[test]
fn a_run_that_is_no_redo_takes_nothing_back() {
    let mut world = tagging(&["a", "b"]);
    let parent = noted(&mut world, &["aaaa"]);
    let mut held = Tags::default();
    held.set(tag("a"), Some(Standing::Carried(By::Model)));
    world.shelf.put_tags(parent, held);
    let mut writer = Script::new().then(answer("b"));
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    assert!(!blocks(&world)[0].takes_back);
    world.replay();
    let mut now = model_tags(&world, parent);
    now.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        now,
        [("a".to_owned(), By::Model), ("b".to_owned(), By::Model)]
    );
}

#[test]
fn a_pass_takes_back_nothing_and_is_kept_as_a_pass() {
    let mut world = tagging(&["a"]);
    let parent = noted(&mut world, &["aaaa"]);
    let mut held = Tags::default();
    held.set(tag("a"), Some(Standing::Carried(By::Model)));
    world.shelf.put_tags(parent, held.clone());
    picked(&mut world, parent, &[], true);
    let mut block = tags::block::read(&world.spooled.files[0].1).unwrap();
    block.passed = true;
    world.spooled.files[0].1 = tags::block::write(&block);
    world.replay();
    assert_eq!(world.shelf.tags_of(parent), held);
    assert!(world.memo.lines.contains(&Line::Passed {
        entry: parent,
        asking: 7
    }));
}

#[test]
fn picks_the_cap_leaves_no_room_for_are_said_with_their_count() {
    let mut world = tagging(&["a", "b"]);
    world.cap = 1;
    let parent = noted(&mut world, &["aaaa"]);
    let mut held = Tags::default();
    held.set(tag("a"), Some(Standing::Carried(By::Person)));
    world.shelf.put_tags(parent, held);
    picked(&mut world, parent, &["b"], false);
    let replayed = world.replay();
    assert_eq!(
        replayed.notices,
        [format!("left 1 tags off {parent}: tag.cap is 1")]
    );
    assert!(
        world
            .memo
            .lines
            .iter()
            .any(|line| matches!(line, Line::Answered(_)))
    );
}

#[test]
fn a_tags_block_an_earlier_run_left_is_replayed_with_the_tags_off() {
    let mut world = World::new();
    let parent = world.entry("plain", &["aaaa", "bbbb"]);
    windows_noted(
        &mut world,
        parent,
        &[(1, "First note."), (2, "Second note.")],
    );
    picked(&mut world, parent, &["a"], false);
    let replayed = world.replay();
    assert_eq!(replayed.tagged, 1);
    assert_eq!(model_tags(&world, parent), [("a".to_owned(), By::Model)]);
    let head = world
        .shelf
        .by_capture_head(parent)
        .expect("the notes' head");
    assert_eq!(
        world.memo.lines,
        [Line::Answered(Answered {
            entry: parent,
            notes: head,
            instruction: world.tags_id.clone(),
            provider: "fake".to_owned(),
            model: "model-a".to_owned(),
            names: names(&["a"]),
        })]
    );
    assert_eq!(world.memo.answers, 1);
}

#[test]
fn each_models_streak_is_its_own_and_either_ends_the_run() {
    let mut world = tagging(&["garden"]);
    world.section.failures.value = 2;
    noted(&mut world, &["aaaa"]);
    noted(&mut world, &["bbbb"]);
    let mut writer = Script::new();
    let mut tagger = Script::new().always(|_, _| failed(What::Rejected));
    let plan = world.plan(Asked::Every);
    let report = world.calls_with(&mut writer, Some(&mut tagger), &plan);
    assert_eq!(report.ended, Ended::Failures(Which::Tagger));

    let mut world = tagging(&["garden"]);
    world.section.failures.value = 2;
    world.section.window.value = 3;
    world.entry("plain", &["bbbb"]);
    let waiting = noted(&mut world, &["aaaa"]);
    let plan = world.plan(Asked::Every);
    let both = [plan.due[0].parent.id, plan.due[1].parent.id];
    let mut writer = Script::new().always(|_, _| answer("r1\nThe user asked about a topic."));
    let mut tagger = Script::new().always(|_, _| failed(What::Unreadable));
    let report = world.calls_with(&mut writer, Some(&mut tagger), &plan);
    assert_eq!(report.ended, Ended::Failures(Which::Tagger));
    assert_eq!(writer.prompts.len(), 1);
    assert_eq!(tagger.prompts.len(), 2);
    assert_eq!(world.memo.charges_of(both[0]), 0);
    assert_eq!(world.memo.charges_of(both[1]), 0);
    assert_eq!(both[0], waiting);
}

fn asking_of(plan: &bilbo_derive::plan::Plan) -> (u64, u8) {
    assert_eq!(plan.due.len(), 1);
    plan.due[0]
        .tags
        .as_ref()
        .expect("tags planned")
        .asking
        .expect("an asking")
}

fn first_record(prompt: &Prompt) -> u64 {
    let (_, records) = prompt
        .material
        .split_once("<records>\n#r")
        .expect("records");
    let digits: String = records.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().expect("a number")
}

fn scripted(picks: &'static str) -> Script {
    Script::new().always(move |prompt, _| {
        if prompt.instruction == tags::instruction::text() {
            answer(picks)
        } else {
            answer(&format!(
                "r{}\nThe user asked about a topic.",
                first_record(prompt)
            ))
        }
    })
}

fn notes_of(world: &World, parent: Id, notes: &[(u64, &str)]) -> Vec<Vec<u8>> {
    notes
        .iter()
        .flat_map(|(at, text)| {
            [
                write_paragraph(&world.paragraph(parent, *at, *at, text)),
                record::write(&world.run_record(parent, *at, *at)).into_bytes(),
            ]
        })
        .collect()
}

fn windows_noted(world: &mut World, parent: Id, notes: &[(u64, &str)]) -> Id {
    let records = notes_of(world, parent, notes);
    world.note_entry(parent, records)
}

fn kept_notes(world: &World) -> Vec<Vec<u8>> {
    world
        .spooled
        .blocks()
        .into_iter()
        .flat_map(|block| block.records)
        .collect()
}

fn no_frame_no_line_no_block(world: &World) {
    assert!(world.shelf.set_metas.is_empty());
    assert!(world.memo.lines.is_empty());
    assert!(blocks(world).is_empty());
}

#[test]
fn a_note_added_asks_again_from_all_the_notes_that_fit() {
    let mut world = tagging(&["a"]);
    let parent = world.entry("plain", &["aaaa", "bbbb"]);
    windows_noted(
        &mut world,
        parent,
        &[(1, "First note."), (2, "Second note.")],
    );
    let head = world.shelf.by_capture_head(parent).expect("notes");
    assert_eq!(head, (Generation(1), 4));
    world
        .memo
        .answer(
            &Answered {
                entry: parent,
                notes: (Generation(1), 2),
                instruction: world.tags_id.clone(),
                provider: "fake".to_owned(),
                model: "model-a".to_owned(),
                names: names(&["a"]),
            },
            1,
        )
        .unwrap();
    let plan = world.plan(Asked::Every);
    assert_eq!(plan.due.len(), 1);
    assert!(plan.due[0].windows.is_empty());
    let mut writer = Script::new().then(answer("a"));
    world.calls(&mut writer, &plan);
    assert_eq!(writer.prompts.len(), 1);
    let both = [
        world.paragraph(parent, 1, 1, "First note."),
        world.paragraph(parent, 2, 2, "Second note."),
    ];
    assert_eq!(
        writer.prompts[0].material,
        tags::view::material(&both, &names(&["a"]), 3)
    );
}

#[test]
fn redo_asks_again_for_another_model_before_anything_settles() {
    for redo in [true, false] {
        let mut world = tagging(&["a"]);
        world.redo = redo;
        world.tags_model = "m2".to_owned();
        let parent = noted(&mut world, &["aaaa"]);
        let head = world.shelf.by_capture_head(parent).expect("notes");
        world
            .memo
            .answer(
                &Answered {
                    entry: parent,
                    notes: head,
                    instruction: world.tags_id.clone(),
                    provider: "fake".to_owned(),
                    model: "m1".to_owned(),
                    names: names(&["a"]),
                },
                1,
            )
            .unwrap();
        let plan = world.plan(Asked::Every);
        let mut writer = Script::new().always(|_, _| answer("a"));
        world.calls(&mut writer, &plan);
        assert_eq!(writer.prompts.len(), usize::from(redo), "redo {redo}");
    }
}

#[test]
fn an_entry_named_on_the_command_line_is_asked_while_it_grows() {
    let mut world = tagging(&["garden"]);
    world.section.window.value = 3;
    let parent = world.entry("plain", &["aaaa"]);
    world.shelf.unseal(parent);
    assert!(world.plan(Asked::Every).due.is_empty());
    let plan = world.plan(Asked::Only(&[parent]));
    assert_eq!(plan.due.len(), 1);
    let mut writer = scripted("garden");
    let report = world.calls(&mut writer, &plan);
    assert_eq!(report.counts.calls, 2);
    let order: Vec<bool> = writer
        .prompts
        .iter()
        .map(|prompt| prompt.instruction == tags::instruction::text())
        .collect();
    assert_eq!(order, [false, true]);
}

#[test]
fn with_the_tags_on_each_windows_prompt_and_limits_are_those_of_the_tags_off() {
    let run = |on: bool| {
        let mut world = if on {
            tagging(&["garden"])
        } else {
            World::new()
        };
        world.section.window.value = 3;
        world.entry("plain", &["aaaa", "bbbb"]);
        world.entry("plain", &["cccc"]);
        let mut writer = scripted("garden");
        world.derive(&mut writer);
        let windows: Vec<(Prompt, Limits)> = writer
            .prompts
            .iter()
            .zip(&writer.limits)
            .filter(|(prompt, _)| prompt.instruction != tags::instruction::text())
            .map(|(prompt, limits)| (prompt.clone(), *limits))
            .collect();
        (windows, writer.prompts.len())
    };
    let (off, off_calls) = run(false);
    let (on, on_calls) = run(true);
    assert_eq!(off.len(), 3);
    assert_eq!(off_calls, 3);
    assert_eq!(on_calls, 5);
    assert_eq!(on, off);
}

#[test]
fn every_attempt_of_a_tags_call_is_handed_the_same_limits_a_retry_included() {
    let mut world = tagging(&["garden"]);
    world.limits = Limits {
        answer_tokens: 16_001,
        timeout: Duration::from_secs(301),
        response: (4 << 20) + 1,
        poll: Duration::from_millis(6),
    };
    noted(&mut world, &["aaaa"]);
    let mut writer = Script::new()
        .then(busy(Some(Duration::from_secs(1))))
        .then(busy(Some(Duration::from_secs(1))))
        .then(answer("garden"));
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    assert_eq!(writer.limits, [world.limits; 3]);
    assert_eq!(writer.prompts.len(), 3);
    assert_eq!(writer.prompts[0], writer.prompts[1]);
    assert_eq!(writer.prompts[0], writer.prompts[2]);
    assert_eq!(writer.prompts[0].instruction, tags::instruction::text());
}

#[test]
fn a_passed_asking_is_made_again_by_redo_and_by_a_name_with_no_answer_each_with_no_charge() {
    let mut world = tagging(&["a"]);
    world.section.tries.value = 1;
    let parent = noted(&mut world, &["aaaa"]);
    let mut writer = Script::new().then(refused(None));
    world.derive(&mut writer);
    let (first, charges) = asking_of(&world.plan(Asked::Every));
    assert_eq!(charges, 1);
    let mut idle = Script::new();
    let done = world.derive(&mut idle);
    assert_eq!(
        done.report.events,
        [Event::TagsPassed {
            entry: parent,
            charges: 1
        }]
    );
    assert!(world.memo.lines.contains(&Line::Passed {
        entry: parent,
        asking: first
    }));

    world.tags.as_mut().unwrap().pick.value = names(&["a", "b"]);
    let plan = world.plan(Asked::Every);
    let (second, charges) = asking_of(&plan);
    assert_ne!(first, second);
    assert_eq!(charges, 0);
    let mut writer = Script::new().then(refused(None));
    world.calls(&mut writer, &plan);
    assert_eq!(writer.prompts.len(), 1);
    assert!(world.memo.lines.contains(&Line::Charged {
        entry: parent,
        asking: second,
        charges: 1
    }));

    world.tags.as_mut().unwrap().pick.value = names(&["a"]);
    let plan = world.plan(Asked::Every);
    assert!(plan.due.is_empty());
    assert!(plan.tags.contains(&(parent, TagsState::PassedOver)));
    world.redo = true;
    let plan = world.plan(Asked::Every);
    assert_eq!(asking_of(&plan), (first, 0));
    let mut writer = Script::new().then(answer("a"));
    world.calls(&mut writer, &plan);
    assert_eq!(writer.prompts.len(), 1);
}

#[test]
fn an_asking_charged_in_the_run_that_wrote_its_notes_is_the_next_runs_asking() {
    for earlier in [true, false] {
        let mut world = tagging(&["garden"]);
        world.section.window.value = 3;
        let parent = if earlier {
            let parent = world.entry("plain", &["aaaa", "bbbb"]);
            world.notes(parent, 1, |_| {});
            parent
        } else {
            world.entry("plain", &["aaaa"])
        };
        let mut writer = Script::new().always(|prompt, _| {
            if prompt.instruction == tags::instruction::text() {
                refused(None)
            } else {
                answer(&format!(
                    "r{}\nThe user asked about a topic.",
                    first_record(prompt)
                ))
            }
        });
        let done = world.derive(&mut writer);
        assert_eq!(done.report.counts.refused, 1, "{earlier}");
        let charged = world
            .memo
            .lines
            .iter()
            .find_map(|line| match line {
                Line::Charged {
                    entry,
                    asking,
                    charges: 1,
                } if *entry == parent => Some(*asking),
                _ => None,
            })
            .expect("a charge");
        let plan = world.plan(Asked::Every);
        assert!(plan.due[0].windows.is_empty());
        assert_eq!(asking_of(&plan), (charged, 1), "{earlier}");
    }
}

#[test]
fn a_run_of_tags_calls_alone_is_scheduled_and_marked() {
    let mut world = tagging(&["garden"]);
    world.section.calls.value = 1;
    noted(&mut world, &["aaaa"]);
    noted(&mut world, &["bbbb"]);
    let plan = world.plan(Asked::Every);
    assert_eq!(plan.due.len(), 2);
    assert!(plan.due.iter().all(|due| due.windows.is_empty()));
    let mut writer = Script::new().then(failed(What::Unreadable));
    let report = world.calls(&mut writer, &plan);
    assert_eq!(report.counts.calls, 1);
    assert_eq!(world.held.mark, Some(plan.due[0].parent.id));
}

#[test]
fn a_failure_that_waits_is_not_charged_when_its_model_stops_the_run() {
    for (entries, stop, charges) in [(3, true, 0), (2, false, 1)] {
        let mut world = tagging(&["garden"]);
        for text in ["aaaa", "bbbb", "cccc"].into_iter().take(entries) {
            noted(&mut world, &[text]);
        }
        let plan = world.plan(Asked::Every);
        let waiting = plan.due[1].parent.id;
        let mut writer = Script::new();
        let mut tagger = Script::new()
            .then(answer("garden"))
            .then(failed(What::Unreadable));
        if stop {
            tagger = tagger.then(stopped());
        }
        let report = world.calls_with(&mut writer, Some(&mut tagger), &plan);
        assert_eq!(
            matches!(report.ended, Ended::Stopped(_, Which::Tagger)),
            stop
        );
        assert_eq!(report.ended == Ended::Done, !stop);
        assert_eq!(world.memo.charges_of(waiting), charges);
    }
}

fn tagged_run(world: &mut World) -> Id {
    world.section.window.value = 3;
    let parent = world.entry("plain", &["aaaa"]);
    let mut writer = scripted("garden");
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    assert_eq!(world.spooled.files.len(), 2);
    parent
}

#[test]
fn a_replay_stopped_at_each_step_is_finished_by_the_next_and_changes_nothing_twice() {
    for stop in 0..3 {
        let mut world = tagging(&["garden"]);
        let parent = tagged_run(&mut world);
        match stop {
            0 => world.spooled.fail_removal = Some(1),
            1 => world.memo.fail_answer = Some(1),
            _ => world.spooled.fail_removal = Some(2),
        }
        assert!(world.try_replay().is_err(), "stop {stop}");
        world.spooled.fail_removal = None;
        world.memo.fail_answer = None;
        world.replay();
        assert_eq!(world.shelf.set_metas.len(), 1, "stop {stop}");
        assert_eq!(
            model_tags(&world, parent),
            [("garden".to_owned(), By::Model)]
        );
        assert_eq!(
            world
                .memo
                .lines
                .iter()
                .filter(|line| matches!(line, Line::Answered(_)))
                .count(),
            1,
            "stop {stop}"
        );
        assert!(world.spooled.files.is_empty(), "stop {stop}");
    }
}

#[test]
fn a_tag_a_person_removed_between_a_stop_and_the_replay_stays_removed() {
    let mut world = tagging(&["garden"]);
    let parent = tagged_run(&mut world);
    world.memo.fail_answer = Some(1);
    assert!(world.try_replay().is_err());
    assert_eq!(world.shelf.set_metas.len(), 1);
    assert_eq!(
        model_tags(&world, parent),
        [("garden".to_owned(), By::Model)]
    );
    let mut held = world.shelf.tags_of(parent);
    held.set(tag("garden"), Some(Standing::Removed));
    world.shelf.put_tags(parent, held.clone());
    world.replay();
    assert_eq!(world.shelf.tags_of(parent), held);
    assert_eq!(world.shelf.set_metas.len(), 1);
    assert!(model_tags(&world, parent).is_empty());
    assert!(
        world
            .memo
            .lines
            .iter()
            .any(|line| matches!(line, Line::Answered(_)))
    );
    assert!(world.spooled.files.is_empty());
}

#[test]
fn a_take_back_that_waited_leaves_a_tag_a_person_added_meanwhile() {
    for stopped_after_declare in [false, true] {
        let mut world = tagging(&["a", "b"]);
        world.redo = true;
        let parent = noted(&mut world, &["aaaa"]);
        let mut held = Tags::default();
        held.set(tag("a"), Some(Standing::Carried(By::Model)));
        world.shelf.put_tags(parent, held);
        let mut writer = Script::new().then(answer("b"));
        let plan = world.plan(Asked::Every);
        world.calls(&mut writer, &plan);
        assert!(blocks(&world)[0].takes_back);
        if stopped_after_declare {
            world.memo.fail_answer = Some(1);
            assert!(world.try_replay().is_err());
            assert_eq!(model_tags(&world, parent), [("b".to_owned(), By::Model)]);
        }
        let mut mine = world.shelf.tags_of(parent);
        mine.set(tag("a"), Some(Standing::Carried(By::Person)));
        world.shelf.put_tags(parent, mine);
        world.replay();
        let mut now = model_tags(&world, parent);
        now.sort_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(
            now,
            [("a".to_owned(), By::Person), ("b".to_owned(), By::Model)],
            "{stopped_after_declare}"
        );
    }
}

#[test]
fn an_entry_written_again_since_the_plan_drops_the_tags_block() {
    let mut world = tagging(&["a"]);
    let parent = noted(&mut world, &["aaaa"]);
    picked(&mut world, parent, &["a"], false);
    world.shelf.regenerate(parent, &["zzzz"]);
    assert_eq!(
        world.shelf.extent(parent, None).unwrap().unwrap(),
        (Generation(2), 1)
    );
    let replayed = world.replay();
    assert_eq!(replayed.tagged, 0);
    assert_eq!(
        replayed.notices,
        [format!(
            "dropped the tags kept for {parent}: the entry changed since they were asked"
        )]
    );
    no_frame_no_line_no_block(&world);
}

const MOVED: &str = "the store holds other notes for it now";

fn moved_notices(parent: Id) -> Vec<String> {
    vec![
        format!("dropped a run kept for {parent}: {MOVED}"),
        format!("dropped the tags kept for {parent}: its notes changed since they were asked"),
    ]
}

fn dropped(world: &mut World, parent: Id) {
    let replayed = world.replay();
    assert_eq!(replayed.notices, moved_notices(parent));
    assert_eq!(replayed.tagged, 0);
    no_frame_no_line_no_block(world);
    assert!(world.spooled.files.is_empty());
}

fn two_windows(world: &mut World, source: &[&str]) -> Id {
    world.section.window.value = 3;
    world.entry("plain", source)
}

fn run_two(world: &mut World) {
    let mut writer = scripted("garden");
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
}

#[test]
fn other_notes_where_ours_would_be_drop_the_tags_block() {
    let mut world = tagging(&["garden"]);
    let parent = two_windows(&mut world, &["aaaa", "bbbb"]);
    run_two(&mut world);
    assert_eq!(world.spooled.files.len(), 3);
    windows_noted(&mut world, parent, &[(1, "Other one."), (2, "Other two.")]);
    dropped(&mut world, parent);

    let mut world = tagging(&["garden"]);
    let parent = world.entry("plain", &["aaaa", "bbbb"]);
    world.notes(parent, 2, |_| {});
    world.shelf.regenerate(parent, &["cccc", "dddd"]);
    world.section.window.value = 3;
    run_two(&mut world);
    assert!(world.spooled.blocks().iter().all(|block| block.opens));
    let records = notes_of(&world, parent, &[(1, "Other one."), (2, "Other two.")]);
    world.shelf.regenerate_notes(parent, &records);
    dropped(&mut world, parent);

    let mut world = tagging(&["garden"]);
    let parent = two_windows(&mut world, &["aaaa", "bbbb", "cccc"]);
    world.notes(parent, 1, |_| {});
    run_two(&mut world);
    assert!(world.spooled.blocks().iter().all(|block| !block.opens));
    let records = notes_of(&world, parent, &[(2, "Other two."), (3, "Other three.")]);
    world.shelf.append_notes(parent, &records);
    dropped(&mut world, parent);

    let mut world = tagging(&["garden"]);
    world.tags.as_mut().unwrap().notes.value = 1;
    let parent = two_windows(&mut world, &["aaaa", "bbbb", "cccc"]);
    windows_noted(&mut world, parent, &[(1, "Old one."), (2, "Old two.")]);
    let mut writer = scripted("garden");
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    let sent = writer
        .prompts
        .iter()
        .find(|prompt| prompt.instruction == tags::instruction::text())
        .expect("a tags call");
    assert!(!sent.material.contains("Old one."));
    assert!(!sent.material.contains("Old two."));
    world.spooled.fail_removal = Some(1);
    assert!(world.try_replay().is_err());
    world.spooled.fail_removal = None;
    assert_eq!(blocks(&world).len(), 1);
    let rewritten = notes_of(&world, parent, &[(1, "Changed one.")]);
    world.shelf.rewrite_notes(parent, 0, &rewritten[0]);
    let replayed = world.replay();
    assert_eq!(
        replayed.notices,
        [format!(
            "dropped the tags kept for {parent}: its notes changed since they were asked"
        )]
    );
    no_frame_no_line_no_block(&world);
    assert!(world.spooled.files.is_empty());
}

fn another_run_kept(world: &mut World, parent: Id) {
    let mut records: Vec<Vec<u8>> = Vec::new();
    for block in world.spooled.blocks() {
        records.extend(block.records.iter().cloned());
    }
    assert_eq!(records.len(), 4);
    let mut first = record::read(&records[1]).expect("a run record");
    first.made += 1;
    records[1] = record::write(&first).into_bytes();
    let bare = |record: &[u8]| record.strip_suffix(b"\n").unwrap_or(record).to_vec();
    let records: Vec<Vec<u8>> = records.iter().map(|record| bare(record)).collect();
    world.note_entry(parent, records);
}

#[test]
fn the_same_notes_under_another_runs_record_drop_the_tags_block() {
    let mut world = tagging(&["garden"]);
    let parent = two_windows(&mut world, &["aaaa", "bbbb"]);
    run_two(&mut world);
    another_run_kept(&mut world, parent);
    let held = kept_notes(&world);
    assert_eq!(held.len(), 4);
    dropped(&mut world, parent);

    let mut world = tagging(&["garden"]);
    let parent = two_windows(&mut world, &["aaaa", "bbbb"]);
    run_two(&mut world);
    another_run_kept(&mut world, parent);
    world.spooled.fail_removal = Some(2);
    assert!(world.try_replay().is_err());
    world.spooled.fail_removal = None;
    assert!(blocks(&world).is_empty());
    assert_eq!(world.spooled.blocks().len(), 2);
    let replayed = world.replay();
    assert_eq!(
        replayed.notices,
        [format!("dropped a run kept for {parent}: {MOVED}")]
    );
    no_frame_no_line_no_block(&world);
    assert!(world.spooled.files.is_empty());
}

#[test]
fn the_mark_is_the_entry_of_the_first_call_counted_and_not_of_a_free_pass() {
    let mut world = tagging(&["garden"]);
    world.section.calls.value = 1;
    world.section.tries.value = 2;
    noted(&mut world, &["cccc"]);
    let bees = world.entry("plain", &["bbbb"]);
    windows_noted(&mut world, bees, &[(1, "Note about bees.")]);
    noted(&mut world, &["aaaa"]);
    let plan = world.plan(Asked::Every);
    let order: Vec<Id> = plan.due.iter().map(|due| due.parent.id).collect();
    assert_eq!(order[1], bees);
    let (passed, _) = plan.due[0].tags.as_ref().unwrap().asking.unwrap();
    world.memo.lines.push(Line::Charged {
        entry: order[0],
        asking: passed,
        charges: 2,
    });
    let mut writer = Script::new().then(failed(What::Unreadable));
    let report = world.calls(&mut writer, &plan);
    assert_eq!(writer.prompts.len(), 1);
    assert!(writer.prompts[0].material.contains("Note about bees."));
    assert_eq!(
        report.events,
        [
            Event::TagsPassed {
                entry: order[0],
                charges: 2
            },
            Event::TagsFailed {
                entry: order[1],
                cause: Cause::new(What::Unreadable)
            }
        ]
    );
    assert_eq!(world.held.mark, Some(order[1]));
    let next = world.plan(Asked::Every);
    assert_eq!(next.due[0].parent.id, order[2]);
}

#[test]
fn the_tags_call_sends_the_instruction_of_the_rules_and_numbers_its_asking_by_that_id() {
    let text = "Pick the tags of the notes.\n";
    let mut world = tagging(&["garden"]);
    world.tags_text = text.to_owned();
    world.tags_id = bilbo_derive::instruction::id(text);
    let parent = noted(&mut world, &["aaaa"]);
    let mut writer = Script::new().then(answer("garden"));
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    assert_eq!(writer.prompts.len(), 1);
    assert_eq!(writer.prompts[0].instruction, text);
    let held = blocks(&world);
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].instruction, world.tags_id);
    let about = tags::asked::About {
        generation: Generation(1),
        records: 1,
        notes: world.notes_digest(parent),
    };
    let asking = |id: &str| {
        tags::asked::asking(
            parent,
            &about,
            "fake",
            "model-a",
            id,
            &writer.prompts[0].material,
        )
    };
    assert_eq!(held[0].asking, asking(&world.tags_id));
    assert_ne!(held[0].asking, asking(&tags::instruction::id()));
}
