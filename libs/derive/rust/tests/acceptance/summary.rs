use std::cell::RefCell;
use std::rc::Rc;

use bilbo_derive::plan::{Asked, Due, Plan, State, SummaryState};
use bilbo_derive::record;
use bilbo_derive::run::{Ended, Event, Which};
use bilbo_derive::scope::Standing;
use bilbo_derive::spool::Spool;
use bilbo_derive::summary::{
    self,
    asked::Asked as SummaryAsked,
    view::{Of, Sizes, sent},
};
use bilbo_derive::tags;
use bilbo_derive::view::Counter;
use bilbo_entries::reader::Reader;
use bilbo_entries::tokens::Estimate;
use bilbo_llm::model::{Cause, Outcome, Prompt, What};
use bilbo_store::entries::Entries;
use bilbo_store::id::Id;
use bilbo_store::labels::Tag;

use crate::fakes::{
    NOW, Plain, Script, World, answer, at, busy, cut, failed, id, refused, stopped, write_paragraph,
};

fn summarising() -> World {
    let mut world = World::new();
    world.summary = Some(summary::keys::defaults());
    world
}

fn file(world: &mut World, records: &[&str]) -> Id {
    world
        .instructions
        .retain(|instruction| instruction.kind != "rewrite");
    world.entry("rewrite", records)
}

fn state_of(plan: &Plan, entry: Id) -> SummaryState {
    plan.summaries
        .iter()
        .find(|(held, _)| *held == entry)
        .map(|(_, state)| *state)
        .expect("a summary state")
}

fn due_of(plan: &Plan, entry: Id) -> Option<&Due> {
    plan.due.iter().find(|due| due.parent.id == entry)
}

fn planned(plan: &Plan, entry: Id) -> &summary::Planned {
    due_of(plan, entry)
        .and_then(|due| due.summary.as_ref())
        .expect("a planned summary")
}

fn body(bytes: usize) -> String {
    "a".repeat(bytes)
}

fn counter() -> Counter<'static> {
    Counter {
        tokens: &Estimate,
        bound: 1 << 20,
    }
}

fn sizes(world: &World) -> Sizes {
    let section = world.summary.as_ref().expect("the summary is on");
    Sizes {
        window: section.window.value,
        most: section.most.value,
        head: world.section.passage_head.value,
    }
}

fn least(world: &mut World, tokens: usize) {
    world.summary.as_mut().expect("on").least.value = tokens;
}

#[test]
fn each_entry_gets_the_first_state_that_holds() {
    let mut world = summarising();
    world.readers.push(Box::new(Plain {
        kind: "paragraphs",
        appends_only: true,
        silent: "",
        preface: None,
    }));
    let unnamed = |world: &mut World, entry: Id| {
        world.table.names[0].1.retain(|held| *held != entry);
    };
    let owner = world.entry("plain", &["a"]);
    let stale = world.entry("plain", &["b"]);
    world.shelf.belong(stale, owner);
    world.table.standing = Some(Standing::NotCurrent(vec![stale]));
    let bare = world.entry("other", &["c"]);
    unnamed(&mut world, bare);
    let child = world.entry("plain", &["d"]);
    world.shelf.belong(child, owner);
    unnamed(&mut world, child);
    let outside = world.entry("plain", &["e"]);
    unnamed(&mut world, outside);
    world.name("out", &[outside]);
    let left = world.entry("plain", &["f"]);
    world.name("out", &[left]);
    world.shelf.seal(left, at(NOW));
    let growing = world.entry("plain", &["g"]);
    world.shelf.unseal(growing);
    world.notes(owner, 1, |_| {});
    let notes = world.shelf.notes_item(owner);
    let holder = file(&mut world, &["h"]);
    let kept = world.settle_summary(holder, None);
    let plan = world.plan(Asked::Every);
    for (entry, expected) in [
        (stale, SummaryState::Out(State::LabelsNotCurrent)),
        (bare, SummaryState::NoReader),
        (child, SummaryState::Out(State::BelongsToAnother)),
        (outside, SummaryState::Out(State::NotNamed)),
        (left, SummaryState::Out(State::LeftOut)),
        (growing, SummaryState::Out(State::StillGrowing)),
        (notes, SummaryState::Out(State::BelongsToAnother)),
        (kept, SummaryState::NoReader),
    ] {
        assert_eq!(state_of(&plan, entry), expected);
        assert!(
            due_of(&plan, entry).is_none_or(|due| due.summary.is_none()),
            "{entry}"
        );
    }
    for entry in [stale, child, outside, left, growing] {
        let SummaryState::Out(held) = state_of(&plan, entry) else {
            panic!("an entry of the kind plain is left out: {entry}");
        };
        assert!(plan.states.contains(&(entry, held)), "{entry}");
    }
}

#[test]
fn an_entry_of_a_kind_with_no_instruction_is_due_for_its_summary_alone() {
    let mut world = summarising();
    let first = body(199);
    let second = body(199);
    let entry = file(&mut world, &[&first, &second]);
    let plan = world.plan(Asked::Every);
    assert!(plan.states.contains(&(entry, State::NoInstruction)));
    assert_eq!(state_of(&plan, entry), SummaryState::Due);
    assert_eq!(plan.due.len(), 1);
    let due = &plan.due[0];
    assert!(due.windows.is_empty() && due.instruction.is_none() && due.tags.is_none());
    let records: Vec<&[u8]> = vec![first.as_bytes(), second.as_bytes()];
    let passages = Plain {
        kind: "rewrite",
        appends_only: false,
        silent: "",
        preface: None,
    }
    .passages(&records);
    let made = sent(&Of::Text(&passages), &sizes(&world), &counter());
    let summary = planned(&plan, entry);
    assert_eq!(summary.sent, counter().count(&made.material));
    assert_ne!(summary.sent, made.whole);
    assert!(matches!(summary.asking, Some((_, 0))));
    assert!(!summary.retire);
    assert_eq!(summary.made.as_ref(), Some(&made));
    assert_eq!(
        made.material,
        format!(
            "<entry>\n{}</entry>\nWrite the summary now, in at most 200 characters.\n",
            made.body()
        )
    );
    assert_eq!(made.body(), format!("{first}\n{second}\n"));
}

#[test]
fn an_entry_whose_windows_are_called_is_due_and_decided_after_them() {
    let mut world = summarising();
    let entry = world.entry("plain", &["aaaa", "bbbb"]);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Due);
    let due = due_of(&plan, entry).expect("a due");
    assert!(!due.windows.is_empty());
    let summary = planned(&plan, entry);
    assert_eq!(summary.asking, None);
    assert_eq!(summary.sent, 0);
    assert!(!summary.retire);
    assert_eq!(summary.head, world.head_of(entry));
    assert_ne!(summary.head.mark.hash, 0);
    assert_eq!(summary.made, None);
    assert_eq!(summary.notes, None);

    let mut world = summarising();
    let entry = world.entry("plain", &["aaaa"]);
    world.notes(entry, 1, |_| {});
    let digest = world.notes_digest(entry);
    world.shelf.grow(entry, &["bbbb"]);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Due);
    assert!(!due_of(&plan, entry).expect("a due").windows.is_empty());
    let summary = planned(&plan, entry);
    assert_eq!(summary.asking, None);
    assert_eq!(summary.notes, Some(digest));

    let mut world = summarising();
    let entry = world.entry("plain", &["aaaa"]);
    world.notes(entry, 1, |run| run.instruction = "other".to_owned());
    world.redo = true;
    let plan = world.plan(Asked::Every);
    let due = due_of(&plan, entry).expect("a due");
    assert!(due.opens);
    assert_eq!(planned(&plan, entry).notes, None);
}

#[test]
fn an_entry_with_complete_notes_and_no_summary_is_asked_with_no_window() {
    let mut world = summarising();
    let entry = world.entry("plain", &["aaaa"]);
    let text = body(400);
    world.notes_saying(entry, 1, &text, |_| {});
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Due);
    let due = due_of(&plan, entry).expect("a due");
    assert!(due.windows.is_empty());
    let paragraphs = [world.paragraph(entry, 1, 1, &text)];
    let made = sent(&Of::Notes(&paragraphs), &sizes(&world), &counter());
    let summary = planned(&plan, entry);
    assert!(matches!(summary.asking, Some((_, 0))));
    assert_eq!(summary.sent, counter().count(&made.material));
    assert_eq!(summary.made.as_ref(), Some(&made));
    assert_eq!(
        made.material,
        format!(
            "<notes>\n{}</notes>\nWrite the summary now, in at most 200 characters.\n",
            made.body()
        )
    );
    assert_eq!(made.body(), format!("r1 2023-11-14\n{text}\n\n"));
}

#[test]
fn notes_of_an_older_generation_alone_are_not_summarised() {
    let mut world = summarising();
    let entry = world.entry("folded", &["aaaa"]);
    world.notes(entry, 1, |_| {});
    world.shelf.regenerate(entry, &["----"]);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::NotComplete);
    assert!(due_of(&plan, entry).is_none());
}

#[test]
fn an_entry_with_a_run_record_and_no_note_is_not_summarised() {
    let mut world = summarising();
    let entry = world.entry("plain", &["aaaa"]);
    let run = world.run_record(entry, 1, 1);
    world.note_entry(entry, vec![record::write(&run).into_bytes()]);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::NoNote);
    assert!(due_of(&plan, entry).is_none());
}

#[test]
fn an_entry_under_least_is_short_only_until_it_has_a_summary() {
    let state_for = |records: &[&str], least_at: Option<usize>, kind: &str| {
        let mut world = summarising();
        if let Some(tokens) = least_at {
            least(&mut world, tokens);
        }
        world
            .instructions
            .retain(|instruction| instruction.kind != kind);
        let entry = world.entry(kind, records);
        let plan = world.plan(Asked::Every);
        state_of(&plan, entry)
    };
    assert_eq!(
        state_for(&[&body(395)], None, "rewrite"),
        SummaryState::Short
    );
    assert_eq!(state_for(&[&body(399)], None, "rewrite"), SummaryState::Due);
    assert_eq!(state_for(&["abc"], Some(0), "rewrite"), SummaryState::Due);
    assert_eq!(
        state_for(&["-a", "-b"], Some(0), "folded"),
        SummaryState::Short
    );
    assert_eq!(state_for(&[""], Some(0), "rewrite"), SummaryState::Short);
    let mut world = summarising();
    let entry = file(&mut world, &[&body(40)]);
    world.settle_summary(entry, None);
    world.shelf.regenerate(entry, &[&body(40)]);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Due);
    assert!(due_of(&plan, entry).is_some());
}

#[test]
fn a_summary_written_of_what_would_be_sent_settles_the_entry_with_no_read_of_it() {
    let mut world = summarising();
    let entry = file(&mut world, &[&body(399), &body(5)]);
    let item = world.settle_summary(entry, None);
    let again = world.summary_record(entry, None);
    world.shelf.append_summary(entry, &again);
    world.shelf.ranges.borrow_mut().clear();
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Written);
    assert!(plan.due.is_empty());
    assert_eq!(*world.shelf.ranges.borrow(), [(item, 1, 2, 1)]);

    let mut world = summarising();
    let chat = world.entry("plain", &["aaaa"]);
    world.notes(chat, 1, |_| {});
    let notes = world.notes_of_session(chat);
    let item = world.settle_summary(chat, Some(notes));
    let again = world.summary_record(chat, Some(notes));
    world.shelf.append_summary(chat, &again);
    let notes_item = world.shelf.notes_item(chat);
    world.shelf.ranges.borrow_mut().clear();
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, chat), SummaryState::Written);
    assert!(plan.due.is_empty());
    assert_eq!(
        *world.shelf.ranges.borrow(),
        [
            (notes_item, notes.generation.0, notes.records, 1),
            (item, 1, 2, 1)
        ]
    );
}

#[test]
fn an_edit_of_a_file_makes_its_summary_due_again() {
    let settled = || {
        let mut world = summarising();
        let entry = file(&mut world, &[&body(399), &body(5)]);
        world.settle_summary(entry, None);
        (world, entry)
    };
    let states = |edit: &dyn Fn(&mut World, Id)| {
        let (mut world, entry) = settled();
        edit(&mut world, entry);
        state_of(&world.plan(Asked::Every), entry)
    };
    assert_eq!(states(&|_, _| {}), SummaryState::Written);
    assert_eq!(
        states(&|world, entry| world.shelf.grow(entry, &["more"])),
        SummaryState::Due
    );
    assert_eq!(
        states(&|world, entry| world.shelf.regenerate(entry, &[&body(399), &body(5)])),
        SummaryState::Due
    );
    assert_eq!(
        states(&|world, entry| world.shelf.rewrite(entry, 1, &body(6))),
        SummaryState::Due
    );
}

#[test]
fn a_note_added_makes_a_summary_due_again_and_a_run_with_no_note_does_not() {
    let settled = || {
        let mut world = summarising();
        let entry = world.entry("plain", &["aaaa"]);
        world.notes(entry, 1, |_| {});
        let notes = world.notes_of_session(entry);
        world.settle_summary(entry, Some(notes));
        (world, entry)
    };
    let (mut world, entry) = settled();
    let run = world.run_record(entry, 1, 1);
    world
        .shelf
        .append_notes(entry, &[record::write(&run).into_bytes()]);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Written);
    let (mut world, entry) = settled();
    let paragraph = world.paragraph(entry, 1, 1, "Another paragraph.");
    let run = world.run_record(entry, 1, 1);
    world.shelf.append_notes(
        entry,
        &[
            write_paragraph(&paragraph),
            record::write(&run).into_bytes(),
        ],
    );
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Due);
    let due = due_of(&plan, entry).expect("a due");
    assert!(due.windows.is_empty() && due.summary.is_some());
}

#[test]
fn redo_asks_again_what_another_instruction_provider_or_model_wrote() {
    let settled = |change: &dyn Fn(&mut summary::record::Summary)| {
        let mut world = summarising();
        let entry = file(&mut world, &[&body(399), &body(5)]);
        let head = world.head_of(entry);
        let mut held = summary::record::Summary {
            text: "The garden log.".to_owned(),
            instruction: world.summary_id.clone(),
            provider: world.provider.clone(),
            model: "model-a".to_owned(),
            answered: None,
            made: NOW - 100,
            entry,
            generation: head.generation,
            records: head.records,
            content: head.mark.hash,
            notes: None,
            sent: 100,
            tokens_in: None,
            tokens_out: None,
        };
        change(&mut held);
        let made = id(200);
        world
            .shelf
            .put_summary(made, entry, &[summary::record::write(&held)]);
        (world, entry)
    };
    let both = |change: &dyn Fn(&mut summary::record::Summary)| {
        let (mut world, entry) = settled(change);
        let plain = state_of(&world.plan(Asked::Every), entry);
        world.redo = true;
        (plain, state_of(&world.plan(Asked::Every), entry))
    };
    assert_eq!(
        both(&|held| held.instruction = "other".to_owned()),
        (SummaryState::Written, SummaryState::Due)
    );
    assert_eq!(
        both(&|held| held.provider = "other".to_owned()),
        (SummaryState::Written, SummaryState::Due)
    );
    assert_eq!(
        both(&|held| held.model = "other".to_owned()),
        (SummaryState::Written, SummaryState::Due)
    );
    assert_eq!(
        both(&|_| {}),
        (SummaryState::Written, SummaryState::Written)
    );
}

#[test]
fn a_key_that_changed_asks_nothing_again() {
    let mut world = summarising();
    let entry = file(&mut world, &[&body(399), &body(5)]);
    let file_item = world.settle_summary(entry, None);
    let chat = world.entry("plain", &["aaaa"]);
    world.notes(chat, 1, |_| {});
    let notes = world.notes_of_session(chat);
    let chat_item = world.settle_summary(chat, Some(notes));
    let notes_item = world.shelf.notes_item(chat);
    {
        let section = world.summary.as_mut().expect("on");
        section.window.value = 1000;
        section.most.value = 40;
        section.least.value = 100_000;
    }
    world.shelf.ranges.borrow_mut().clear();
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Written);
    assert_eq!(state_of(&plan, chat), SummaryState::Written);
    assert!(plan.due.is_empty());
    assert_eq!(
        *world.shelf.ranges.borrow(),
        [
            (file_item, 1, 1, 1),
            (notes_item, notes.generation.0, notes.records, 1),
            (chat_item, 1, 1, 1)
        ]
    );
}

#[test]
fn an_asking_that_was_passed_over_is_not_due_until_what_is_sent_changes_or_redo() {
    let mut world = summarising();
    let entry = file(&mut world, &[&body(399), &body(5)]);
    let plan = world.plan(Asked::Every);
    let (number, _) = planned(&plan, entry).asking.expect("an asking");
    world.noted.pass(entry, number).expect("a pass");
    let before = world.noted.asks.get();
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::PassedOver);
    assert!(due_of(&plan, entry).is_none());
    assert_eq!(world.noted.asks.get() - before, 1);
    world.redo = true;
    let before = world.noted.asks.get();
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Due);
    assert_eq!(world.noted.asks.get() - before, 1);
    world.redo = false;
    world.shelf.grow(entry, &["more"]);
    let before = world.noted.asks.get();
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Due);
    assert_eq!(world.noted.asks.get() - before, 2);
}

#[test]
fn with_the_summary_off_nothing_is_planned_and_its_file_is_not_read() {
    let mut world = World::new();
    let entry = file(&mut world, &[&body(399), &body(5)]);
    let windowed = world.entry("plain", &["aaaa", "bbbb"]);
    let plan = world.plan(Asked::Every);
    assert!(plan.summaries.is_empty());
    assert_eq!(plan.due.len(), 1);
    let due = &plan.due[0];
    assert_eq!(due.parent.id, windowed);
    assert!(!due.windows.is_empty() && due.summary.is_none());
    assert!(plan.states.contains(&(entry, State::NoInstruction)));
    assert_eq!(world.noted.asks.get(), 0);
}

#[test]
fn an_entry_named_on_the_command_line_is_summarised_while_it_grows() {
    let mut world = summarising();
    let entry = file(&mut world, &[&body(399), &body(5)]);
    world.shelf.unseal(entry);
    let plan = world.plan(Asked::Every);
    assert_eq!(
        state_of(&plan, entry),
        SummaryState::Out(State::StillGrowing)
    );
    let plan = world.plan(Asked::Only(&[entry]));
    assert_eq!(state_of(&plan, entry), SummaryState::Due);
}

#[test]
fn an_entry_with_only_a_summary_due_takes_its_turn_by_its_id() {
    let mut world = summarising();
    let older = file(&mut world, &[&body(399), &body(5)]);
    let newer = world.entry("plain", &["aaaa", "bbbb"]);
    let order = |plan: &Plan| -> Vec<Id> { plan.due.iter().map(|due| due.parent.id).collect() };
    assert_eq!(order(&world.plan(Asked::Every)), [newer, older]);
    world.held.mark = Some(newer);
    assert_eq!(order(&world.plan(Asked::Every)), [older, newer]);
}

#[test]
fn records_that_render_nothing_at_the_end_leave_a_summary_written() {
    let settled = || {
        let mut world = summarising();
        let entry = world.entry("folded", &["aaaa"]);
        world.notes(entry, 1, |_| {});
        let notes = world.notes_of_session(entry);
        world.settle_summary(entry, Some(notes));
        (world, entry)
    };
    let (mut world, entry) = settled();
    world.shelf.grow(entry, &["-"]);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Written);
    assert!(plan.due.is_empty());
    let (mut world, entry) = settled();
    world.shelf.grow(entry, &["bbbb"]);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Due);
    assert!(!due_of(&plan, entry).expect("a due").windows.is_empty());
}

#[test]
fn an_entry_with_nothing_left_to_summarise_is_scheduled_for_a_retirement_and_for_no_call() {
    fn retiring(plan: &Plan, entry: Id) {
        assert_eq!(plan.due.len(), 1);
        let due = due_of(plan, entry).expect("a due");
        assert!(due.windows.is_empty() && due.tags.is_none());
        let summary = due.summary.as_ref().expect("a summary");
        assert!(summary.retire);
        assert_eq!(summary.asking, None);
        assert_eq!(summary.sent, 0);
        assert_eq!(summary.made, None);
    }
    let emptied_file = |kept: bool| {
        let mut world = summarising();
        world.noted.fail_read = true;
        let entry = file(&mut world, &[&body(399), &body(5)]);
        if kept {
            world.settle_summary(entry, None);
        }
        world.shelf.regenerate(entry, &[]);
        (world, entry)
    };
    let noted_chat = |kept: bool| {
        let mut world = summarising();
        world.noted.fail_read = true;
        let entry = world.entry("plain", &["aaaa"]);
        world.notes(entry, 1, |_| {});
        if kept {
            let notes = world.notes_of_session(entry);
            world.settle_summary(entry, Some(notes));
        }
        let run = world.run_record(entry, 1, 1);
        world
            .shelf
            .regenerate_notes(entry, &[record::write(&run).into_bytes()]);
        (world, entry)
    };
    let emptied_chat = |kept: bool| {
        let mut world = summarising();
        world.noted.fail_read = true;
        let entry = world.entry("plain", &["aaaa"]);
        world.notes(entry, 1, |_| {});
        if kept {
            let notes = world.notes_of_session(entry);
            world.settle_summary(entry, Some(notes));
        }
        world.shelf.regenerate(entry, &[]);
        (world, entry)
    };
    let (world, entry) = emptied_file(true);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Short);
    retiring(&plan, entry);
    let (world, entry) = noted_chat(true);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::NoNote);
    retiring(&plan, entry);
    let (world, entry) = emptied_chat(true);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::NotComplete);
    retiring(&plan, entry);
    for (world, entry, expected) in [
        {
            let (world, entry) = emptied_file(false);
            (world, entry, SummaryState::Short)
        },
        {
            let (world, entry) = noted_chat(false);
            (world, entry, SummaryState::NoNote)
        },
        {
            let (world, entry) = emptied_chat(false);
            (world, entry, SummaryState::NotComplete)
        },
    ] {
        let plan = world.plan(Asked::Every);
        assert_eq!(state_of(&plan, entry), expected);
        assert!(plan.due.is_empty());
    }
    let (mut world, entry) = emptied_file(true);
    let bare = world.head_of(entry);
    world.shelf.append_summary(
        entry,
        &summary::record::retirement(entry, bare.generation, 0, bare.mark.hash, NOW),
    );
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Short);
    assert!(plan.due.is_empty());
    let mut world = summarising();
    world.noted.fail_read = true;
    let entry = world.entry("folded", &["aaaa"]);
    world.notes(entry, 1, |_| {});
    let notes = world.notes_of_session(entry);
    world.settle_summary(entry, Some(notes));
    let run = world.run_record(entry, 1, 1);
    world
        .shelf
        .regenerate_notes(entry, &[record::write(&run).into_bytes()]);
    world.shelf.grow(entry, &["-"]);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::NoNote);
    assert!(plan.due.is_empty());
    let (mut world, entry) = emptied_file(true);
    world.shelf.regenerate(entry, &["", ""]);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Short);
    retiring(&plan, entry);
}

#[test]
fn a_windowed_entrys_stored_notes_are_scanned_once_for_its_tags_and_its_summary() {
    let ranges = |tags: bool, summary: bool| {
        let mut world = World::new();
        if tags {
            let mut section = bilbo_derive::tags::keys::defaults();
            section.enabled.value = true;
            section.pick.value = ["garden"]
                .iter()
                .map(|name| bilbo_store::labels::Tag::new(name).expect("a tag"))
                .collect();
            world.tags = Some(section);
        }
        if summary {
            world.summary = Some(summary::keys::defaults());
        }
        let entry = world.entry("plain", &["aaaa"]);
        world.notes(entry, 1, |_| {});
        world.shelf.grow(entry, &["bbbb"]);
        let notes_item = world.shelf.notes_item(entry);
        world.shelf.ranges.borrow_mut().clear();
        let plan = world.plan(Asked::Every);
        assert!(!due_of(&plan, entry).expect("a due").windows.is_empty());
        let held = world.shelf.ranges.borrow().clone();
        (held, notes_item)
    };
    let (plain, notes_item) = ranges(false, false);
    let scan = (notes_item, 1, 1, 2);
    let mut scanned = plain.clone();
    scanned.push(scan);
    assert_eq!(ranges(true, false).0, scanned);
    assert_eq!(ranges(false, true).0, scanned);
    assert_eq!(ranges(true, true).0, scanned);
}

fn lenient() -> World {
    let mut world = summarising();
    least(&mut world, 0);
    world
}

fn chat(world: &mut World, records: &[&str]) -> Id {
    let entry = world.entry("plain", records);
    world.notes(entry, records.len() as u64, |_| {});
    entry
}

fn is_summary(prompt: &Prompt) -> bool {
    prompt.instruction == summary::instruction::text()
}

fn is_tags(prompt: &Prompt) -> bool {
    prompt.instruction == tags::instruction::text()
}

fn first_record(prompt: &Prompt) -> u64 {
    let (_, records) = prompt
        .material
        .split_once("<records>\n#r")
        .expect("records");
    let digits: String = records.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().expect("a number")
}

fn scripted() -> Script {
    Script::new().always(|prompt, _| {
        if is_summary(prompt) {
            answer("The garden log.")
        } else if is_tags(prompt) {
            answer("garden")
        } else {
            answer(&format!(
                "r{}\nThe user asked about a topic.",
                first_record(prompt)
            ))
        }
    })
}

fn with_tags(world: &mut World) {
    let mut section = tags::keys::defaults();
    section.enabled.value = true;
    section.pick.value = [Tag::new("garden").expect("a tag")].into_iter().collect();
    world.tags = Some(section);
}

fn kinds(writer: &Script) -> Vec<&'static str> {
    writer
        .prompts
        .iter()
        .map(|prompt| {
            if is_summary(prompt) {
                "summary"
            } else if is_tags(prompt) {
                "tags"
            } else {
                "window"
            }
        })
        .collect()
}

fn blocks(world: &World) -> Vec<summary::block::Block> {
    world
        .summary_spool
        .files
        .iter()
        .filter_map(|(_, bytes)| summary::block::read(bytes).ok())
        .collect()
}

fn charges_of(world: &World, entry: Id, asking: u64) -> u8 {
    world.noted.charges(entry, asking).expect("charges")
}

fn asking_of(plan: &Plan, entry: Id) -> u64 {
    planned(plan, entry).asking.expect("an asking").0
}

fn u1_lines(world: &World) -> Vec<Vec<String>> {
    world
        .lines
        .0
        .iter()
        .filter(|line| line.starts_with("u1\t"))
        .map(|line| line.split('\t').map(str::to_owned).collect())
        .collect()
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

#[test]
fn the_call_carries_the_summarys_instruction_the_material_and_the_runs_limits() {
    let mut world = lenient();
    let entry = chat(&mut world, &["aaaa"]);
    let plan = world.plan(Asked::Every);
    let made = planned(&plan, entry).made.clone().expect("what is sent");
    let mut writer = Script::new().then(answer("The garden log."));
    world.calls(&mut writer, &plan);
    assert_eq!(writer.prompts.len(), 1);
    let prompt = &writer.prompts[0];
    assert_eq!(prompt.instruction, world.summary_text);
    let paragraphs = [world.paragraph(entry, 1, 1, "Earlier paragraph.")];
    let expected = sent(&Of::Notes(&paragraphs), &sizes(&world), &counter());
    assert_eq!(prompt.material, expected.material);
    assert_eq!(prompt.material, made.material);
    assert_eq!(writer.limits, [world.limits]);
}

#[test]
fn a_file_is_sent_its_own_text_between_entry_tags_and_nothing_else_of_it() {
    let mut world = lenient();
    file(&mut world, &["# The garden log", "Beds are raised."]);
    let mut writer = Script::new().then(answer("The garden log."));
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    assert_eq!(
        writer.prompts[0].material,
        "<entry>\n# The garden log\nBeds are raised.\n</entry>\nWrite the summary now, in at most 200 characters.\n"
    );
}

#[test]
fn an_entry_whose_last_window_is_answered_is_asked_for_its_tags_and_then_for_its_summary() {
    let mut world = lenient();
    with_tags(&mut world);
    let entry = world.entry("plain", &["aaaa"]);
    let mut writer = scripted();
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    assert_eq!(kinds(&writer), ["window", "tags", "summary"]);
    assert!(
        writer.prompts[2]
            .material
            .contains("The user asked about a topic.")
    );
    let counts = report.counts;
    assert_eq!((counts.entries, counts.calls, counts.notes), (1, 3, 1));
    let held = blocks(&world);
    assert_eq!(held.len(), 1);
    let head = world.head_of(entry);
    assert_eq!(
        (
            held[0].about.generation,
            held[0].about.records,
            held[0].about.content
        ),
        (head.generation, head.records, head.mark.hash)
    );
}

#[test]
fn an_entry_whose_window_ends_it_is_not_asked_for_its_summary() {
    for outcome in [
        failed(What::ToolUsed),
        refused(None),
        answer("I could not write notes."),
    ] {
        let mut world = lenient();
        let entry = world.entry("plain", &["aaaa", "bbbb"]);
        world.notes(entry, 1, |_| {});
        let mut writer = Script::new().then(outcome);
        let plan = world.plan(Asked::Every);
        world.calls(&mut writer, &plan);
        assert_eq!(writer.prompts.len(), 1);
        assert!(world.summary_spool.files.is_empty());
        assert!(world.noted.lines.is_empty());
    }
}

#[test]
fn a_tags_call_that_failed_does_not_keep_the_summary_from_being_asked() {
    let mut world = lenient();
    with_tags(&mut world);
    let entry = world.entry("plain", &["aaaa"]);
    let mut writer = Script::new().always(|prompt, _| {
        if is_tags(prompt) {
            refused(None)
        } else if is_summary(prompt) {
            answer("The garden log.")
        } else {
            answer("r1\nThe user asked about a topic.")
        }
    });
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    assert_eq!(kinds(&writer), ["window", "tags", "summary"]);
    assert_eq!(blocks(&world).len(), 1);
    assert_eq!(world.memo.charges_of(entry), 1);
}

#[test]
fn the_asking_of_the_run_that_wrote_the_notes_is_the_next_runs_asking() {
    let mut world = lenient();
    let entry = chat(&mut world, &["aaaa"]);
    world.shelf.grow(entry, &["bbbb"]);
    let mut writer = Script::new().always(|prompt, _| {
        if is_summary(prompt) {
            refused(None)
        } else {
            answer("r2\nThe user asked about b.")
        }
    });
    world.derive(&mut writer);
    assert_eq!(kinds(&writer), ["window", "summary"]);
    let asking = match world.noted.lines.as_slice() {
        [summary::asked::Line::Charged { asking, .. }] => *asking,
        other => panic!("{other:?}"),
    };
    let plan = world.plan(Asked::Every);
    assert_eq!(planned(&plan, entry).asking, Some((asking, 1)));
}

#[test]
fn an_answer_is_kept_as_one_block_of_the_summarys_spool_and_reaches_no_store() {
    let mut world = lenient();
    let entry = chat(&mut world, &["aaaa"]);
    let mut writer = Script::new().then(answer("The garden log."));
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    let held = blocks(&world);
    assert_eq!(world.summary_spool.files.len(), 1);
    assert_eq!(held.len(), 1);
    let block = &held[0];
    let head = world.head_of(entry);
    assert_eq!(block.form, summary::block::Form::Answer);
    assert_eq!(block.entry, entry);
    assert_eq!(
        (
            block.about.generation,
            block.about.records,
            block.about.content
        ),
        (head.generation, head.records, head.mark.hash)
    );
    assert_eq!(block.about.notes, Some(world.notes_digest(entry)));
    assert_eq!(block.instruction, world.summary_id);
    assert_eq!(
        (block.provider.as_str(), block.model.as_str()),
        ("fake", "model-a")
    );
    assert_eq!(block.text, "The garden log.");
    assert!(world.spooled.files.is_empty());
    assert!(world.summary_records(entry).is_none());
    assert!(world.noted.lines.is_empty());
}

#[test]
fn an_answer_bilbo_cannot_keep_is_a_failed_call_charged_at_once() {
    let long = "Z".repeat(256);
    for (text, why, word) in [
        (String::new(), summary::answer::Why::Empty, "empty"),
        (
            "One.\n\nTwo.".to_owned(),
            summary::answer::Why::Paragraphs,
            "paragraphs",
        ),
        (
            "It edited main.rs and lib.rs.".to_owned(),
            summary::answer::Why::Names,
            "names",
        ),
        (long, summary::answer::Why::Opaque, "opaque"),
    ] {
        let mut world = lenient();
        chat(&mut world, &["aaaa"]);
        chat(&mut world, &["bbbb"]);
        let plan = world.plan(Asked::Every);
        let first = plan.due[0].parent.id;
        let second = plan.due[1].parent.id;
        let mut writer = Script::new()
            .then(answer(&text))
            .then(answer("The garden log."));
        let seen = seeing(&mut world, &mut writer);
        let report = world.calls(&mut writer, &plan);
        assert_eq!(writer.prompts.len(), 2, "{word}");
        assert_eq!(
            order(&seen),
            [
                "call:writer".to_owned(),
                format!("charge:{first}"),
                "call:writer".to_owned()
            ]
        );
        assert_eq!(report.events, [Event::NoSummary { entry: first, why }]);
        assert_eq!(report.counts.failed, 1);
        assert_eq!(charges_of(&world, first, asking_of(&plan, first)), 1);
        let held = blocks(&world);
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].entry, second);
        let lines = u1_lines(&world);
        assert_eq!(lines[0][2], "failed");
        assert_eq!(lines[0][14], format!("no-summary {word}"));
    }
}

#[test]
fn a_refusal_of_a_summarys_call_is_charged_at_once() {
    let mut world = lenient();
    chat(&mut world, &["aaaa"]);
    chat(&mut world, &["bbbb"]);
    let plan = world.plan(Asked::Every);
    let first = plan.due[0].parent.id;
    let mut writer = Script::new()
        .then(refused(Some("policy")))
        .then(answer("The garden log."));
    let seen = seeing(&mut world, &mut writer);
    let report = world.calls(&mut writer, &plan);
    assert_eq!(writer.prompts.len(), 2);
    assert_eq!(
        order(&seen),
        [
            "call:writer".to_owned(),
            format!("charge:{first}"),
            "call:writer".to_owned()
        ]
    );
    assert_eq!(
        report.events,
        [Event::SummaryRefused {
            entry: first,
            category: Some("policy".to_owned())
        }]
    );
    assert_eq!(report.counts.refused, 1);
    assert_eq!(charges_of(&world, first, asking_of(&plan, first)), 1);
    let lines = u1_lines(&world);
    assert_eq!(
        (lines[0][2].as_str(), lines[0][14].as_str()),
        ("refused", "policy")
    );
}

fn seeing(world: &mut World, writer: &mut Script) -> Rc<RefCell<Vec<String>>> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    writer.seen = Some(Rc::clone(&seen));
    world.noted.seen = Some(Rc::clone(&seen));
    seen
}

fn order(seen: &Rc<RefCell<Vec<String>>>) -> Vec<String> {
    seen.borrow()
        .iter()
        .filter(|what| what.starts_with("call:") || what.starts_with("charge:"))
        .cloned()
        .collect()
}

fn three_chats(world: &mut World) -> Vec<Id> {
    for text in ["aaaa", "bbbb", "cccc"] {
        chat(world, &[text]);
    }
    let plan = world.plan(Asked::Every);
    plan.due.iter().map(|due| due.parent.id).collect()
}

#[test]
fn a_cut_summary_is_charged_once_the_writer_is_proved_whatever_proves_it() {
    let seconds: [(&str, Outcome); 4] = [
        ("kept", answer("The garden log.")),
        ("refused", refused(None)),
        ("empty", answer("")),
        ("failed", failed(What::ToolUsed)),
    ];
    for (name, second) in seconds {
        let mut world = lenient();
        world.section.failures.value = 2;
        chat(&mut world, &["aaaa"]);
        chat(&mut world, &["bbbb"]);
        let plan = world.plan(Asked::Every);
        let (first, next) = (plan.due[0].parent.id, plan.due[1].parent.id);
        let mut writer = Script::new().then(cut()).then(second);
        let seen = seeing(&mut world, &mut writer);
        let report = world.calls(&mut writer, &plan);
        assert_eq!(report.ended, Ended::Done, "{name}");
        let expected_order: Vec<String> = if name == "kept" {
            vec![
                "call:writer".to_owned(),
                "call:writer".to_owned(),
                format!("charge:{first}"),
            ]
        } else {
            vec![
                "call:writer".to_owned(),
                "call:writer".to_owned(),
                format!("charge:{next}"),
                format!("charge:{first}"),
            ]
        };
        assert_eq!(order(&seen), expected_order, "{name}");
        assert!(matches!(
            report.events.first(),
            Some(Event::SummaryFailed { cause, .. }) if cause.what == What::Cut
        ));
        assert_eq!(
            charges_of(&world, first, asking_of(&plan, first)),
            1,
            "{name}"
        );
        let expected = u8::from(name != "kept");
        assert_eq!(
            charges_of(&world, next, asking_of(&plan, next)),
            expected,
            "{name}"
        );
    }
    let mut world = lenient();
    world.section.failures.value = 2;
    let order = three_chats(&mut world);
    let plan = world.plan(Asked::Every);
    let mut writer = Script::new()
        .then(cut())
        .then(answer("The garden log."))
        .then(cut());
    let seen = seeing(&mut world, &mut writer);
    let report = world.calls(&mut writer, &plan);
    assert_eq!(report.ended, Ended::Done);
    let (a, b, c) = (order[0], order[1], order[2]);
    assert_eq!(
        crate::summary::order(&seen),
        [
            "call:writer".to_owned(),
            "call:writer".to_owned(),
            format!("charge:{a}"),
            "call:writer".to_owned(),
            format!("charge:{c}"),
        ]
    );
    assert_eq!(charges_of(&world, a, asking_of(&plan, a)), 1);
    assert_eq!(charges_of(&world, b, asking_of(&plan, b)), 0);
    assert_eq!(charges_of(&world, c, asking_of(&plan, c)), 1);
}

#[test]
fn a_stop_of_the_model_ends_the_run_and_charges_nothing() {
    let mut world = lenient();
    chat(&mut world, &["aaaa"]);
    chat(&mut world, &["bbbb"]);
    let plan = world.plan(Asked::Every);
    let first = plan.due[0].parent.id;
    let mut writer = Script::new()
        .then(stopped())
        .then(answer("The garden log."));
    let report = world.calls(&mut writer, &plan);
    assert!(matches!(
        &report.ended,
        Ended::Stopped(cause, Which::Writer) if cause.what == What::Unauthorized
    ));
    assert_eq!(writer.prompts.len(), 1);
    assert!(report.events.is_empty());
    assert_eq!(report.counts.failed, 0);
    assert_eq!(charges_of(&world, first, asking_of(&plan, first)), 0);
    assert!(world.noted.lines.is_empty());
    assert_eq!(u1_lines(&world)[0][2], "stopped");

    let mut world = lenient();
    chat(&mut world, &["aaaa"]);
    chat(&mut world, &["bbbb"]);
    let plan = world.plan(Asked::Every);
    let first = plan.due[0].parent.id;
    let rejected = Cause {
        status: Some(400),
        ..Cause::new(What::Rejected)
    };
    let mut writer = Script::new()
        .then(Outcome::Stopped(rejected))
        .then(answer("The garden log."));
    let seen = seeing(&mut world, &mut writer);
    let report = world.calls(&mut writer, &plan);
    assert_eq!(report.ended, Ended::Done);
    assert_eq!(writer.prompts.len(), 2);
    assert_eq!(report.counts.failed, 1);
    assert!(matches!(
        report.events.as_slice(),
        [Event::SummaryFailed { entry, cause }] if *entry == first && cause.what == What::Rejected
    ));
    assert_eq!(
        order(&seen),
        [
            "call:writer".to_owned(),
            "call:writer".to_owned(),
            format!("charge:{first}")
        ]
    );
    assert_eq!(charges_of(&world, first, asking_of(&plan, first)), 1);
    assert_eq!(u1_lines(&world)[0][2], "stopped");
}

#[test]
fn an_asking_at_derive_tries_is_passed_over_with_no_call_and_kept_as_a_pass() {
    let mut world = lenient();
    let entry = chat(&mut world, &["aaaa"]);
    let plan = world.plan(Asked::Every);
    let asking = asking_of(&plan, entry);
    for _ in 0..3 {
        world.noted.charge(entry, asking).expect("charged");
    }
    let plan = world.plan(Asked::Every);
    let mut writer = Script::new();
    let report = world.calls(&mut writer, &plan);
    assert!(writer.prompts.is_empty());
    assert_eq!(report.events, [Event::SummaryPassed { entry, charges: 3 }]);
    let held = blocks(&world);
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].form, summary::block::Form::Pass);
    assert_eq!(u1_lines(&world)[0][2], "passed");
    world.replay();
    assert_eq!(world.noted.passed(entry).expect("read"), Some(asking));
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::PassedOver);
}

#[test]
fn a_summarys_call_takes_one_of_derive_calls_and_a_spent_run_leaves_the_entry_due() {
    let mut world = lenient();
    world.section.calls.value = 1;
    let entry = world.entry("plain", &["aaaa"]);
    let mut writer = scripted();
    let done = world.derive(&mut writer);
    assert_eq!(done.report.ended, Ended::Calls);
    assert_eq!(writer.prompts.len(), 1);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Due);

    let mut world = lenient();
    world.section.calls.value = 1;
    file(&mut world, &["aaaa"]);
    world.entry("rewrite", &["bbbb"]);
    let mut writer = Script::new().always(|_, _| answer("The garden log."));
    let plan = world.plan(Asked::Every);
    let second = plan.due[1].parent.id;
    let report = world.calls(&mut writer, &plan);
    assert_eq!(writer.prompts.len(), 1);
    assert_eq!(report.counts.calls, 1);
    assert_eq!(report.ended, Ended::Calls);
    assert_eq!(blocks(&world).len(), 1);
    world.replay();
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, second), SummaryState::Due);
}

#[test]
fn a_run_of_summaries_alone_prepares_the_model_once_and_asks_once_whether_it_is_bound() {
    let mut world = lenient();
    file(&mut world, &["aaaa"]);
    world.entry("rewrite", &["bbbb"]);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut writer = Script::new().always(|_, _| answer("The garden log."));
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
fn the_model_is_asked_once_whether_it_is_bound_for_the_tags_and_the_summary() {
    let mut world = lenient();
    with_tags(&mut world);
    chat(&mut world, &["aaaa"]);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut writer = scripted();
    writer.seen = Some(Rc::clone(&seen));
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    let held = seen.borrow();
    assert_eq!(
        held.iter().filter(|what| *what == "bound:writer").count(),
        1
    );
    let bound = held
        .iter()
        .position(|what| what == "bound:writer")
        .expect("bound");
    let call = held
        .iter()
        .position(|what| what == "call:writer")
        .expect("call");
    assert!(bound < call);
    assert_eq!(held.iter().filter(|what| *what == "call:writer").count(), 2);
}

#[test]
fn a_writer_that_is_not_bound_is_asked_no_summary_and_the_run_goes_on() {
    let mut world = lenient();
    world.entry("plain", &["aaaa"]);
    world.entry("plain", &["bbbb"]);
    let mut writer = Script::new()
        .then(answer("r1\nA note."))
        .then(answer("r1\nAnother note."));
    writer.refuse_bound = Some("unbound".to_owned());
    let seen = seeing(&mut world, &mut writer);
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    assert_eq!(report.ended, Ended::Done);
    assert_eq!(writer.prompts.len(), 2);
    assert_eq!(
        report.events,
        [Event::SummariesNotAsked {
            why: "unbound".to_owned()
        }]
    );
    assert_eq!(
        seen.borrow()
            .iter()
            .filter(|line| *line == "bound:writer")
            .count(),
        1
    );
    assert_eq!(report.counts.failed, 0);
    assert_eq!(world.spooled.blocks().len(), 2);
    assert!(world.summary_spool.files.is_empty());
    assert!(world.noted.lines.is_empty());
}

#[test]
fn an_unbound_writer_still_retires_and_spools_no_pass_after_the_warning() {
    let mut world = lenient();
    let charged = chat(&mut world, &["eeee"]);
    let emptied = |world: &mut World, entry: Id| {
        world.notes(entry, 1, |_| {});
        let notes = world.notes_of_session(entry);
        world.settle_summary(entry, Some(notes));
        let run = world.run_record(entry, 1, 1);
        world
            .shelf
            .regenerate_notes(entry, &[record::write(&run).into_bytes()]);
    };
    let scheduled = world.entry("plain", &["bbbb"]);
    emptied(&mut world, scheduled);
    let at_run_time = world.entry("plain", &["cccc", "dddd"]);
    emptied(&mut world, at_run_time);
    let tripping = world.entry("plain", &["aaaa"]);
    let plan = world.plan(Asked::Every);
    let asking = asking_of(&plan, charged);
    for _ in 0..3 {
        world.noted.charge(charged, asking).expect("charged");
    }
    let mut writer = Script::new().then(answer("r1\nA note.")).then(answer(""));
    writer.refuse_bound = Some("unbound".to_owned());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    assert_eq!(report.ended, Ended::Done);
    assert_eq!(
        report.events,
        [Event::SummariesNotAsked {
            why: "unbound".to_owned()
        }]
    );
    let held = blocks(&world);
    let retired: Vec<Id> = held
        .iter()
        .filter(|block| block.form == summary::block::Form::Retirement)
        .map(|block| block.entry)
        .collect();
    assert_eq!(held.len(), 2, "{held:?}");
    assert!(retired.contains(&scheduled) && retired.contains(&at_run_time));
    assert!(!retired.contains(&tripping));
    assert_eq!(charges_of(&world, charged, asking), 3);
    assert_eq!(world.replay().retired, 2);
}

#[test]
fn an_entry_short_or_with_no_note_after_its_windows_makes_no_call_and_says_nothing() {
    let quiet = |world: &World, writer: &Script, report: &bilbo_derive::run::Report| {
        assert_eq!(writer.prompts.len(), 1);
        assert!(report.events.is_empty());
        assert!(u1_lines(world).is_empty());
        assert!(world.summary_spool.files.is_empty());
    };
    let mut world = summarising();
    world.entry("plain", &["aaaa"]);
    let mut writer = Script::new().then(answer("r1\nA note of the garden"));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    quiet(&world, &writer, &report);

    let mut world = lenient();
    world.entry("plain", &["aaaa"]);
    let mut writer = Script::new().then(answer(""));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    quiet(&world, &writer, &report);

    let build = || {
        let mut world = lenient();
        let entry = world.entry("plain", &["aaaa"]);
        (world, entry)
    };
    let (mut twin, entry) = build();
    let mut writer = Script::new()
        .then(answer("r1\nA note."))
        .then(refused(None));
    twin.derive(&mut writer);
    let asking = match twin.noted.lines.as_slice() {
        [summary::asked::Line::Charged { asking, .. }] => *asking,
        other => panic!("{other:?}"),
    };
    let (mut world, entry) = {
        let (world, same) = build();
        assert_eq!(same, entry);
        (world, same)
    };
    world.noted.pass(entry, asking).expect("a pass");
    let mut writer = Script::new().then(answer("r1\nA note."));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    quiet(&world, &writer, &report);
}

#[test]
fn with_the_summary_on_the_requests_of_windows_and_tags_are_those_of_the_summary_off() {
    let run = |on: bool| {
        let mut world = World::new();
        with_tags(&mut world);
        if on {
            world.summary = Some(summary::keys::defaults());
            least(&mut world, 0);
        }
        world.entry("plain", &["aaaa"]);
        world.entry("plain", &["bbbb", "cccc"]);
        let mut writer = scripted();
        world.derive(&mut writer);
        writer
    };
    let off = run(false);
    let on = run(true);
    let keep = |writer: &Script| -> Vec<(Prompt, bilbo_llm::model::Limits)> {
        writer
            .prompts
            .iter()
            .zip(&writer.limits)
            .filter(|(prompt, _)| !is_summary(prompt))
            .map(|(prompt, limits)| (prompt.clone(), *limits))
            .collect()
    };
    assert_eq!(keep(&on), keep(&off));
    assert!(on.prompts.len() > off.prompts.len());
}

#[test]
fn a_busy_model_is_asked_again_and_every_attempt_counts() {
    let mut world = lenient();
    let entry = chat(&mut world, &["aaaa"]);
    let mut writer = Script::new()
        .then(busy(None))
        .then(answer("The garden log."));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    assert_eq!(report.counts.calls, 2);
    assert_eq!(
        *world.watch.slept.borrow(),
        [world.section.retry_wait.value]
    );
    let words: Vec<String> = u1_lines(&world)
        .iter()
        .map(|line| line[2].clone())
        .collect();
    assert_eq!(words, ["busy", "written"]);
    assert_eq!(charges_of(&world, entry, asking_of(&plan, entry)), 0);
    assert!(world.noted.lines.is_empty());
}

fn kept_run(world: &mut World, entry: Id, text: &'static str) -> bilbo_derive::run::Report {
    let mut writer = Script::new().then(answer(text));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    assert_eq!(writer.prompts.len(), 1, "{entry}");
    report
}

#[test]
fn a_replay_that_fails_after_a_capture_says_how_much_it_wrote() {
    let mut world = lenient();
    let entry = chat(&mut world, &["aaaa"]);
    kept_run(&mut world, entry, "The garden log.");
    let other = file(&mut world, &["aaaa", "bbbb"]);
    let pass = summary_block(&world, other, summary::block::Form::Pass, (77, None));
    world.summary_spool.keep(&pass).expect("kept");
    world.noted.fail_write = Some(world.noted.writes + 2);
    let Err(interrupted) = world.try_replay() else {
        panic!("the replay fails");
    };
    assert_eq!(interrupted.written, 1);
    assert!(world.summary_records(entry).is_some());
    assert_eq!(world.summary_spool.files.len(), 1);
    world.noted.fail_write = None;
    let replayed = world.replay();
    assert_eq!(replayed.written(), 0);
    assert_eq!(world.noted.passed(other).expect("read"), Some(77));
    let sum = bilbo_derive::run::Replayed {
        captured: 1,
        tagged: 2,
        summarised: 4,
        retired: 8,
        found: 16,
        notices: Vec::new(),
    };
    assert_eq!(sum.written(), 31);
}

#[test]
fn a_replay_keeps_the_summary_under_its_label_and_settles_the_entrys_lines() {
    let mut world = lenient();
    let entry = chat(&mut world, &["aaaa"]);
    let plan = world.plan(Asked::Every);
    let asking = asking_of(&plan, entry);
    kept_run(&mut world, entry, "The garden log.");
    world.noted.charge(entry, asking).expect("charged");
    let replayed = world.replay();
    assert_eq!(replayed.summarised, 1);
    let (head, records) = world.summary_records(entry).expect("a summary");
    let meta = world
        .shelf
        .meta(head.id)
        .expect("a lookup")
        .expect("a meta");
    assert_eq!(meta.kind, summary::KIND);
    assert_eq!(meta.labels.id, None);
    assert_eq!(records.len(), 1);
    let held = summary::record::read(&records[0]).expect("a summary record");
    let at = world.head_of(entry);
    let notes_head = world.shelf.by_capture_head(entry).expect("notes");
    assert_eq!(held.text, "The garden log.");
    assert_eq!(held.instruction, world.summary_id);
    assert_eq!(
        (held.provider.as_str(), held.model.as_str()),
        ("fake", "model-a")
    );
    assert_eq!(
        (held.generation, held.records, held.content),
        (at.generation, at.records, at.mark.hash)
    );
    assert_eq!(
        held.notes,
        Some(summary::record::Notes {
            generation: notes_head.0,
            records: notes_head.1,
            digest: world.notes_digest(entry)
        })
    );
    assert!(world.noted.lines.is_empty());
    assert!(world.summary_spool.files.is_empty() && world.spooled.files.is_empty());
}

#[test]
fn a_second_summary_is_appended_and_the_last_record_is_the_one_in_force() {
    let mut world = lenient();
    let entry = chat(&mut world, &["aaaa"]);
    kept_run(&mut world, entry, "The garden log.");
    world.replay();
    let paragraph = world.paragraph(entry, 1, 1, "A second paragraph.");
    let run = world.run_record(entry, 1, 1);
    world.shelf.append_notes(
        entry,
        &[
            write_paragraph(&paragraph),
            record::write(&run).into_bytes(),
        ],
    );
    let plan = world.plan(Asked::Every);
    assert!(due_of(&plan, entry).expect("a due").windows.is_empty());
    kept_run(&mut world, entry, "A second summary.");
    assert_eq!(world.replay().summarised, 1);
    let (head, records) = world.summary_records(entry).expect("a summary");
    assert_eq!(
        (head.generation, records.len()),
        (bilbo_store::entries::Generation(1), 2)
    );
    let last = summary::record::read(&records[1]).expect("a summary record");
    assert_eq!(last.text, "A second summary.");
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Written);
}

#[test]
fn a_block_is_dropped_when_the_entry_is_gone_or_changed_or_its_notes_changed() {
    type Edit = Box<dyn Fn(&mut World, Id)>;
    let rows: [(bool, Edit, &str); 4] = [
        (
            true,
            Box::new(|world, entry| world.shelf.remove(entry)),
            "the store no longer holds it",
        ),
        (
            true,
            Box::new(|world, entry| world.shelf.grow(entry, &["more"])),
            "the entry changed since it was asked",
        ),
        (
            false,
            Box::new(|world, entry| world.shelf.rewrite(entry, 0, "other")),
            "the entry changed since it was asked",
        ),
        (
            true,
            Box::new(|world, entry| {
                let other = write_paragraph(&world.paragraph(entry, 1, 1, "Changed."));
                world.shelf.rewrite_notes(entry, 0, &other);
            }),
            "its notes changed since it was asked",
        ),
    ];
    for (noted, edit, why) in rows {
        let mut world = lenient();
        let entry = if noted {
            chat(&mut world, &["aaaa"])
        } else {
            file(&mut world, &["aaaa", "bbbb"])
        };
        let mut writer = Script::new().then(answer("The garden log."));
        let plan = world.plan(Asked::Every);
        world.calls(&mut writer, &plan);
        assert_eq!(world.summary_spool.files.len(), 1);
        edit(&mut world, entry);
        let replayed = world.replay();
        assert_eq!(
            replayed.notices,
            [format!("dropped the summary kept for {entry}: {why}")]
        );
        assert_eq!(replayed.summarised, 0);
        assert!(world.summary_records(entry).is_none());
        assert!(world.summary_spool.files.is_empty() && world.spooled.files.is_empty());
    }
}

const MOVED: &str = "the store holds other notes for it now";

fn two_windows_run(world: &mut World) -> Id {
    world.section.window.value = 3;
    let parent = world.entry("plain", &["aaaa", "bbbb"]);
    let mut writer = scripted();
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    assert_eq!(world.summary_spool.files.len(), 1);
    let records = notes_of(
        world,
        parent,
        &[
            (1, "The user asked about a topic."),
            (2, "The user asked about a topic."),
        ],
    );
    world.note_entry(parent, records);
    parent
}

#[test]
fn a_notes_group_that_is_dropped_takes_the_summarys_block_first() {
    let mut world = lenient();
    let parent = two_windows_run(&mut world);
    let replayed = world.replay();
    assert_eq!(
        replayed.notices,
        [
            format!("dropped a run kept for {parent}: {MOVED}"),
            format!("dropped the summary kept for {parent}: its notes changed since it was asked"),
        ]
    );
    assert!(world.summary_records(parent).is_none());
    assert!(world.spooled.files.is_empty() && world.summary_spool.files.is_empty());

    let mut world = lenient();
    let parent = two_windows_run(&mut world);
    let names: Vec<String> = world
        .spooled
        .files
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    assert!(!names.is_empty());
    world.summary_spool.fail_removal = Some(1);
    assert!(world.try_replay().is_err());
    let left: Vec<String> = world
        .spooled
        .files
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    assert_eq!(left, names);
    world.summary_spool.fail_removal = None;
    world.replay();
    assert!(world.summary_records(parent).is_none());
    assert!(world.spooled.files.is_empty() && world.summary_spool.files.is_empty());
}

fn summary_block(
    world: &World,
    entry: Id,
    form: summary::block::Form,
    (asking, notes): (u64, Option<u64>),
) -> Vec<u8> {
    let head = world.head_of(entry);
    summary::block::write(&summary::block::Block {
        entry,
        about: summary::asked::About {
            generation: head.generation,
            records: head.records,
            content: head.mark.hash,
            notes,
        },
        asking,
        form,
        instruction: world.summary_id.clone(),
        provider: "fake".to_owned(),
        model: "model-a".to_owned(),
        answered: None,
        made: NOW,
        sent: 5,
        tokens_in: None,
        tokens_out: None,
        text: String::new(),
    })
}

#[test]
fn a_pass_is_kept_as_a_pass_and_writes_nothing() {
    let mut world = lenient();
    let entry = file(&mut world, &["aaaa", "bbbb"]);
    world.settle_summary(entry, None);
    let bytes = summary_block(&world, entry, summary::block::Form::Pass, (77, None));
    world.summary_spool.keep(&bytes).expect("kept");
    world.shelf.ranges.borrow_mut().clear();
    world.shelf.lookups.borrow_mut().clear();
    let replayed = world.replay();
    assert!(world.shelf.ranges.borrow().is_empty());
    assert_eq!(*world.shelf.lookups.borrow(), [format!("file:/n/{entry}")]);
    assert_eq!(world.summary_records(entry).expect("a record").1.len(), 1);
    assert_eq!(world.noted.passed(entry).expect("read"), Some(77));
    assert_eq!(replayed.summarised, 0);
    assert!(replayed.notices.is_empty());

    let mut world = lenient();
    let entry = chat(&mut world, &["aaaa"]);
    let digest = world.notes_digest(entry);
    let bytes = summary_block(
        &world,
        entry,
        summary::block::Form::Pass,
        (78, Some(digest)),
    );
    world.summary_spool.keep(&bytes).expect("kept");
    let notes_item = world.shelf.notes_item(entry);
    world.shelf.ranges.borrow_mut().clear();
    world.shelf.lookups.borrow_mut().clear();
    let replayed = world.replay();
    assert_eq!(
        *world.shelf.lookups.borrow(),
        [format!("file:/n/{entry}"), format!("derive:{entry}")]
    );
    assert_eq!(*world.shelf.ranges.borrow(), [(notes_item, 1, 1, 2)]);
    assert_eq!(world.noted.passed(entry).expect("read"), Some(78));
    assert!(replayed.notices.is_empty());
}

#[test]
fn a_replay_stopped_between_the_capture_and_the_removal_changes_nothing_twice() {
    let mut world = lenient();
    let entry = chat(&mut world, &["aaaa"]);
    kept_run(&mut world, entry, "The garden log.");
    let bytes = world.summary_spool.files[0].1.clone();
    let first = world.replay();
    assert_eq!((first.summarised, first.found), (1, 0));
    world.summary_spool.keep(&bytes).expect("kept");
    let run = world.run_record(entry, 1, 1);
    world
        .shelf
        .append_notes(entry, &[record::write(&run).into_bytes()]);
    let replayed = world.replay();
    assert_eq!((replayed.summarised, replayed.found), (0, 1));
    let (_, records) = world.summary_records(entry).expect("a summary");
    assert_eq!(records.len(), 1);
    assert!(world.summary_spool.files.is_empty() && world.spooled.files.is_empty());
}

#[test]
fn the_record_names_the_notes_as_the_store_holds_them_once_they_are_kept() {
    let mut world = lenient();
    let entry = world.entry("plain", &["aaaa"]);
    let mut writer = scripted();
    world.derive(&mut writer);
    let (_, records) = world.summary_records(entry).expect("a summary");
    let held = summary::record::read(&records[0]).expect("a summary record");
    let notes_head = world.shelf.by_capture_head(entry).expect("notes");
    assert_eq!(
        held.notes,
        Some(summary::record::Notes {
            generation: notes_head.0,
            records: notes_head.1,
            digest: world.notes_digest(entry)
        })
    );
}

#[test]
fn what_an_earlier_run_left_in_the_summarys_spool_is_replayed_with_the_summary_off() {
    let mut world = lenient();
    let entry = chat(&mut world, &["aaaa"]);
    kept_run(&mut world, entry, "The garden log.");
    let second = file(&mut world, &["aaaa", "bbbb"]);
    world.summary = None;
    world.summary_spool.keep(b"not a block").expect("kept");
    let head = world.head_of(second);
    let two_lines = summary::block::write(&summary::block::Block {
        entry: second,
        about: summary::asked::About {
            generation: head.generation,
            records: head.records,
            content: head.mark.hash,
            notes: None,
        },
        asking: 5,
        form: summary::block::Form::Answer,
        instruction: world.summary_id.clone(),
        provider: "fake".to_owned(),
        model: "model-a".to_owned(),
        answered: None,
        made: NOW,
        sent: 5,
        tokens_in: None,
        tokens_out: None,
        text: "One.\n\nTwo.".to_owned(),
    });
    world.summary_spool.keep(&two_lines).expect("kept");
    let junk = world.summary_spool.files[1].0.clone();
    let long = world.summary_spool.files[2].0.clone();
    let replayed = world.replay();
    assert_eq!(replayed.summarised, 1);
    assert_eq!(
        replayed.notices,
        [
            format!("dropped a spool file bilbo cannot read: {junk}"),
            format!("dropped a spool file bilbo cannot read: {long}"),
        ]
    );
    assert!(world.summary_records(second).is_none());
    assert!(world.summary_spool.files.is_empty());
}

#[test]
fn an_answer_of_another_model_proves_nothing_of_the_writer() {
    let mut world = lenient();
    with_tags(&mut world);
    world.tags_model = "model-t".to_owned();
    let settled = chat(&mut world, &["aaaa"]);
    let notes = world.notes_of_session(settled);
    world.settle_summary(settled, Some(notes));
    let waiting = chat(&mut world, &["bbbb"]);
    let plan = world.plan(Asked::Every);
    let asking = asking_of(&plan, waiting);
    let mut writer = Script::new().then(cut());
    let mut tagger = Script::new().always(|_, _| answer("garden"));
    tagger.model = "model-t".to_owned();
    let report = world.calls_with(&mut writer, Some(&mut tagger), &plan);
    assert_eq!(report.ended, Ended::Done);
    assert_eq!(kinds(&tagger), ["tags", "tags"]);
    assert_eq!(kinds(&writer), ["summary"]);
    assert_eq!(charges_of(&world, waiting, asking), 0);
}

#[test]
fn a_kept_answer_is_in_the_spool_before_its_proof_charges_what_waits() {
    let mut world = lenient();
    chat(&mut world, &["aaaa"]);
    chat(&mut world, &["bbbb"]);
    world.noted.fail_write = Some(1);
    let plan = world.plan(Asked::Every);
    let mut writer = Script::new().then(cut()).then(answer("The garden log."));
    let second = plan.due[1].parent.id;
    let failed = world.try_calls(&mut writer, None, &plan);
    assert!(failed.is_err());
    let held = blocks(&world);
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].form, summary::block::Form::Answer);
    assert_eq!(held[0].entry, second);
    assert_eq!(held[0].text, "The garden log.");
    assert!(world.noted.lines.is_empty());
}

fn three_records(world: &mut World) -> Id {
    world.section.window.value = 3;
    let entry = world.entry("plain", &["aaaa", "bbbb", "cccc"]);
    world.notes(entry, 1, |_| {});
    with_tags(world);
    entry
}

#[test]
fn a_window_answered_with_nothing_goes_on_and_the_summary_is_judged_on_the_notes_that_stand() {
    let script = |second: &'static str| {
        Script::new().always(move |prompt, _| {
            if is_summary(prompt) {
                answer("The garden log.")
            } else if is_tags(prompt) {
                answer("garden")
            } else if first_record(prompt) == 2 {
                answer("")
            } else {
                answer(second)
            }
        })
    };
    let non_summary = |writer: &Script| -> Vec<(Prompt, bilbo_llm::model::Limits)> {
        writer
            .prompts
            .iter()
            .zip(&writer.limits)
            .filter(|(prompt, _)| !is_summary(prompt))
            .map(|(prompt, limits)| (prompt.clone(), *limits))
            .collect()
    };
    let run = |on: bool, second: &'static str, settle: bool| {
        let mut world = World::new();
        if on {
            world.summary = Some(summary::keys::defaults());
            least(&mut world, 0);
        }
        let entry = three_records(&mut world);
        if settle {
            let notes = world.notes_of_session(entry);
            world.settle_summary(entry, Some(notes));
        }
        let mut writer = script(second);
        let done = world.derive(&mut writer);
        (world, writer, done.report)
    };
    let (_, writer, _) = run(true, "r3\nThe user asked about c.", false);
    let (_, off, _) = run(false, "r3\nThe user asked about c.", false);
    assert_eq!(kinds(&writer), ["window", "window", "tags", "summary"]);
    assert_eq!(non_summary(&writer), non_summary(&off));
    let material = &writer.prompts[3].material;
    assert!(
        material.contains("Earlier paragraph.") && material.contains("The user asked about c.")
    );

    let (world, writer, _) = run(true, "", false);
    let (_, off, _) = run(false, "", false);
    assert_eq!(kinds(&writer), ["window", "window", "tags", "summary"]);
    assert_eq!(non_summary(&writer), non_summary(&off));
    let stored = [world.paragraph(id(1), 1, 1, "Earlier paragraph.")];
    let expected = sent(&Of::Notes(&stored), &sizes(&world), &counter());
    assert_eq!(writer.prompts[3].material, expected.material);

    let (world, writer, report) = run(true, "", true);
    let (_, off, _) = run(false, "", true);
    assert_eq!(kinds(&writer), ["window", "window", "tags"]);
    assert_eq!(non_summary(&writer), non_summary(&off));
    assert!(world.summary_spool.files.is_empty());
    assert!(u1_lines(&world).is_empty());
    assert!(report.events.iter().all(|event| !matches!(
        event,
        Event::SummaryRefused { .. }
            | Event::SummaryFailed { .. }
            | Event::NoSummary { .. }
            | Event::SummaryPassed { .. }
    )));
}

#[test]
fn a_note_written_in_this_run_makes_the_summary_due_though_the_stores_notes_head_is_the_old_one() {
    let settled = || {
        let mut world = lenient();
        let entry = chat(&mut world, &["aaaa"]);
        let notes = world.notes_of_session(entry);
        world.settle_summary(entry, Some(notes));
        world.shelf.grow(entry, &["bbbb"]);
        (world, entry, notes)
    };
    let (mut world, entry, notes) = settled();
    let mut writer = Script::new().always(|prompt, _| {
        if is_summary(prompt) {
            answer("A second summary.")
        } else {
            answer("r2\nThe user asked about b.")
        }
    });
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    assert_eq!(kinds(&writer), ["window", "summary"]);
    let material = &writer.prompts[1].material;
    assert!(
        material.contains("Earlier paragraph.") && material.contains("The user asked about b.")
    );
    let held = blocks(&world);
    assert_eq!(held.len(), 1);
    assert_ne!(held[0].about.notes, Some(notes.digest));
    assert_eq!(
        world.shelf.by_capture_head(entry),
        Some((notes.generation, notes.records))
    );
    assert_eq!(world.replay().summarised, 1);
    assert_eq!(held[0].about.notes, Some(world.notes_digest(entry)));

    let (mut world, _, _) = settled();
    let mut writer = Script::new().always(|prompt, _| {
        if is_summary(prompt) {
            answer("A second summary.")
        } else {
            answer("")
        }
    });
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    assert_eq!(writer.prompts.len(), 1);
    assert!(world.summary_spool.files.is_empty());
    assert!(report.events.is_empty());
}

#[test]
fn a_retirement_makes_no_call_and_the_replay_appends_a_record_that_is_no_summary() {
    let mut world = lenient();
    let entry = file(&mut world, &[&body(399), &body(5)]);
    world.settle_summary(entry, None);
    world.shelf.regenerate(entry, &[]);
    let mut writer = Script::new();
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut writer, &plan);
    assert!(writer.prompts.is_empty());
    assert_eq!(report.counts, bilbo_derive::run::Counts::default());
    assert!(report.events.is_empty());
    let head = world.head_of(entry);
    let held = blocks(&world);
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].form, summary::block::Form::Retirement);
    assert_eq!(
        (
            held[0].about.generation,
            held[0].about.records,
            held[0].about.content
        ),
        (head.generation, 0, head.mark.hash)
    );
    let lines = u1_lines(&world);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0][2], "retired");
    let bytes = world.summary_spool.files[0].1.clone();
    world.noted.charge(entry, 9).expect("charged");
    let replayed = world.replay();
    assert_eq!(
        (replayed.retired, replayed.summarised, replayed.found),
        (1, 0, 0)
    );
    let (_, records) = world.summary_records(entry).expect("records");
    assert_eq!(records.len(), 2);
    assert_eq!(
        String::from_utf8(records[1].clone()).expect("text"),
        summary::record::retirement(entry, head.generation, 0, head.mark.hash, held[0].made)
    );
    assert!(summary::record::read(&records[1]).is_none());
    assert!(world.noted.lines.is_empty());
    assert!(world.summary_spool.files.is_empty() && world.spooled.files.is_empty());
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::Short);
    assert!(plan.due.is_empty());
    world.summary_spool.keep(&bytes).expect("kept");
    let again = world.replay();
    assert_eq!((again.retired, again.found), (0, 1));
    assert_eq!(world.summary_records(entry).expect("records").1.len(), 2);
}

#[test]
fn a_summary_is_retired_in_the_run_whose_passes_empty_its_notes() {
    let mut world = lenient();
    let entry = world.entry("plain", &["aaaa"]);
    world.notes(entry, 1, |run| run.instruction = "other".to_owned());
    let notes = world.notes_of_session(entry);
    world.settle_summary(entry, Some(notes));
    world.redo = true;
    let extent = world.extent_of(entry, 1, 1);
    world.held.charges.push((extent, 3));
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, entry), SummaryState::NoNote);
    let due = due_of(&plan, entry).expect("a due entry");
    assert_eq!(due.windows.len(), 1);
    assert!(due.windows[0].pass);
    assert!(due.summary.as_ref().expect("a summary").retire);
    let mut writer = Script::new();
    world.calls(&mut writer, &plan);
    assert!(writer.prompts.is_empty());
    let head = world.head_of(entry);
    let held = blocks(&world);
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].form, summary::block::Form::Retirement);
    assert_eq!(
        (
            held[0].about.generation,
            held[0].about.records,
            held[0].about.content
        ),
        (head.generation, head.records, head.mark.hash)
    );
    let replayed = world.replay();
    assert_eq!(replayed.retired, 1);
    let generations = world.shelf.notes_of(entry).expect("notes");
    let newest = generations.last().expect("a generation");
    assert_eq!(newest.len(), 1);
    let bare = newest[0].strip_suffix(b"\n").unwrap_or(&newest[0]);
    assert!(crate::fakes::read_paragraph(bare).is_none());
    let (_, records) = world.summary_records(entry).expect("records");
    assert!(summary::record::read(records.last().expect("a record")).is_none());
}

#[test]
fn a_session_whose_new_notes_hold_no_note_has_its_summary_retired_in_the_same_run() {
    let mut world = lenient();
    let entry = world.entry("plain", &["aaaa"]);
    world.notes(entry, 1, |run| run.instruction = "other".to_owned());
    let notes = world.notes_of_session(entry);
    world.settle_summary(entry, Some(notes));
    world.redo = true;
    let mut writer = Script::new().then(answer(""));
    let plan = world.plan(Asked::Every);
    world.calls(&mut writer, &plan);
    assert_eq!(writer.prompts.len(), 1);
    let head = world.head_of(entry);
    let held = blocks(&world);
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].form, summary::block::Form::Retirement);
    assert_eq!(
        (
            held[0].about.generation,
            held[0].about.records,
            held[0].about.content
        ),
        (head.generation, head.records, head.mark.hash)
    );
    world.replay();
    let notes_head = world.shelf.by_capture_head(entry).expect("notes");
    assert_eq!(notes_head.0, bilbo_store::entries::Generation(2));
    let generations = world.shelf.notes_of(entry).expect("notes");
    let newest = generations.last().expect("a generation");
    assert_eq!(newest.len(), 1);
    let bare = newest[0].strip_suffix(b"\n").unwrap_or(&newest[0]);
    assert!(crate::fakes::read_paragraph(bare).is_none());
    let (_, records) = world.summary_records(entry).expect("records");
    assert!(summary::record::read(records.last().expect("a record")).is_none());
}

#[test]
fn a_cut_that_waits_is_charged_only_when_the_run_ends_done_or_spent() {
    let run = |calls: usize, third: Option<Outcome>, failures: usize| {
        let mut world = lenient();
        world.section.calls.value = calls;
        world.section.failures.value = failures;
        chat(&mut world, &["aaaa"]);
        chat(&mut world, &["bbbb"]);
        if third.is_some() {
            chat(&mut world, &["cccc"]);
        }
        let plan = world.plan(Asked::Every);
        let order: Vec<Id> = plan.due.iter().map(|due| due.parent.id).collect();
        let mut writer = Script::new().then(answer("The garden log.")).then(cut());
        if let Some(outcome) = third {
            writer = writer.then(outcome);
        }
        let report = world.calls(&mut writer, &plan);
        let charged: Vec<u8> = order
            .iter()
            .map(|entry| charges_of(&world, *entry, asking_of(&plan, *entry)))
            .collect();
        (report.ended, charged)
    };
    assert_eq!(run(10, None, 5), (Ended::Done, vec![0, 1]));
    assert_eq!(
        run(2, Some(answer("The garden log.")), 5),
        (Ended::Calls, vec![0, 1, 0])
    );
    let (ended, charged) = run(10, Some(stopped()), 5);
    assert!(matches!(ended, Ended::Stopped(_, Which::Writer)));
    assert_eq!(charged, [0, 0, 0]);
    assert_eq!(
        run(10, Some(cut()), 2),
        (Ended::Failures(Which::Writer), vec![0, 0, 0])
    );
}
