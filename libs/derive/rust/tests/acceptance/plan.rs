use std::time::Duration;

use bilbo_derive::plan::{Asked, Plan, State};
use bilbo_derive::record::Run;
use bilbo_derive::run::Ended;
use bilbo_derive::scope::{Standing, Untrusted};
use bilbo_store::entries::Generation;
use bilbo_store::id::Id;

use crate::fakes::{NOW, Script, World, answer, at, no_note, write_paragraph};

type Case = (bool, fn(&mut Run), State);

fn state_of(plan: &Plan, entry: Id) -> State {
    plan.states
        .iter()
        .find(|(held, _)| *held == entry)
        .map(|(_, state)| *state)
        .expect("a state")
}

fn unnamed(world: &mut World, entry: Id) {
    world.table.names[0].1.retain(|held| *held != entry);
}

#[test]
fn an_entry_is_due_when_nothing_stands_against_it() {
    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta", "gamma"]);
    let plan = world.plan(Asked::Every);
    assert_eq!(plan.states, [(parent, State::Due)]);
    assert_eq!(plan.due.len(), 1);
    let due = &plan.due[0];
    assert_eq!(due.windows.len(), 1);
    let planned = &due.windows[0];
    assert_eq!(planned.window.text, "#r1 alpha\n#r2 beta\n#r3 gamma\n");
    assert_eq!(
        (
            planned.window.first,
            planned.window.last,
            planned.window.sent
        ),
        (1, 3, 9)
    );
    assert!(!planned.pass);
    assert_eq!(planned.extent, world.extent_of(parent, 1, 3));
    assert!(!due.opens);
    assert_eq!(due.held, None);
    assert_eq!(due.last, None);
    assert_eq!(due.parent.id, parent);
}

#[test]
fn each_entry_gets_the_first_state_that_holds() {
    let mut world = World::new();
    let due = world.entry("plain", &["a", "b"]);
    let covered = world.entry("plain", &["a", "b"]);
    world.notes(covered, 2, |_| {});
    let passed = world.entry("plain", &["a", "b"]);
    world.notes(passed, 2, |run| run.passes = 1);
    let growing = world.entry("plain", &["a"]);
    world.shelf.seal(growing, at(NOW - 60));
    let left_out = world.entry("plain", &["a"]);
    world.name("out", &[left_out]);
    let not_named = world.entry("plain", &["a"]);
    unnamed(&mut world, not_named);
    let belongs = world.entry("plain", &["a"]);
    world.shelf.belong(belongs, due);
    let no_instruction = world.entry("other", &["a"]);
    let stale = world.entry("plain", &["a"]);
    let neighbours = [
        ("stale and no instruction", "other", State::LabelsNotCurrent),
        ("no instruction and belongs", "other", State::NoInstruction),
        ("belongs and not named", "plain", State::BelongsToAnother),
        ("not named and left out", "plain", State::NotNamed),
        ("left out and still growing", "plain", State::LeftOut),
        ("still growing and covered", "plain", State::StillGrowing),
        ("passed over and due", "plain", State::Due),
    ];
    let mut met = Vec::new();
    for (index, (_, kind, _)) in neighbours.iter().enumerate() {
        let records: &[&str] = if index == 6 {
            &["a", "b", "c"]
        } else {
            &["a", "b"]
        };
        let entry = world.entry(kind, records);
        match index {
            0 => {}
            1 => world.shelf.belong(entry, due),
            2 => {
                world.shelf.belong(entry, due);
                unnamed(&mut world, entry);
            }
            3 => {
                unnamed(&mut world, entry);
                world.name("out", &[entry]);
            }
            4 => {
                world.name("out", &[entry]);
                world.shelf.unseal(entry);
            }
            5 => {
                world.shelf.unseal(entry);
                world.notes(entry, 2, |_| {});
            }
            _ => {
                world.notes(entry, 2, |run| run.passes = 1);
            }
        }
        met.push(entry);
    }
    let stale_pair = met[0];
    world.table.standing = Some(Standing::NotCurrent(vec![stale, stale_pair]));
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, due), State::Due);
    assert_eq!(state_of(&plan, covered), State::Covered);
    assert_eq!(state_of(&plan, passed), State::PassedOver);
    assert_eq!(state_of(&plan, growing), State::StillGrowing);
    assert_eq!(state_of(&plan, left_out), State::LeftOut);
    assert_eq!(state_of(&plan, not_named), State::NotNamed);
    assert_eq!(state_of(&plan, belongs), State::BelongsToAnother);
    assert_eq!(state_of(&plan, no_instruction), State::NoInstruction);
    assert_eq!(state_of(&plan, stale), State::LabelsNotCurrent);
    for ((what, _, state), entry) in neighbours.iter().zip(&met) {
        assert_eq!(state_of(&plan, *entry), *state, "{what}");
    }
    let words: Vec<&str> = plan.states.iter().map(|(_, state)| state.words()).collect();
    for word in [
        "labels not current",
        "no instruction",
        "belongs to another",
        "not named",
        "left out",
        "still growing",
        "covered",
        "passed over",
        "due",
    ] {
        assert!(words.contains(&word), "{word}");
    }
}

#[test]
fn a_kind_with_no_instruction_is_never_due() {
    let mut world = World::new();
    let bare = world.entry("other", &["a"]);
    let plain = world.entry("plain", &["a"]);
    world
        .instructions
        .retain(|instruction| instruction.kind != "plain");
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, bare), State::NoInstruction);
    assert_eq!(state_of(&plan, plain), State::NoInstruction);
    assert!(plan.due.is_empty());
}

#[test]
fn an_entry_named_by_derive_not_in_is_left_out_whatever_derive_in_says() {
    let mut world = World::new();
    let both = world.entry("plain", &["a"]);
    world.name("out", &[both]);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, both), State::LeftOut);
    assert!(plan.due.is_empty());
}

#[test]
fn an_entry_that_belongs_to_another_is_never_due() {
    let mut world = World::new();
    let parent = world.entry("plain", &["a"]);
    let child = world.entry("plain", &["b"]);
    world.shelf.belong(child, parent);
    world.notes(parent, 1, |_| {});
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, child), State::BelongsToAnother);
    assert_eq!(state_of(&plan, parent), State::Covered);
    let notes = plan
        .states
        .iter()
        .filter(|(held, _)| *held != parent && *held != child)
        .count();
    assert_eq!(notes, 1);
    assert!(plan.due.is_empty());
}

#[test]
fn an_entry_whose_labels_are_not_current_is_not_sent() {
    let mut world = World::new();
    let failed = world.entry("plain", &["a"]);
    let child = world.entry("plain", &["b"]);
    world.shelf.belong(child, failed);
    unnamed(&mut world, child);
    world.table.standing = Some(Standing::NotCurrent(vec![failed, child]));
    let stale = State::LabelsNotCurrent;
    for (asked, states) in [
        (Asked::Every, vec![(failed, stale), (child, stale)]),
        (Asked::Only(&[failed]), vec![(failed, stale)]),
        (Asked::Only(&[child]), vec![(child, stale)]),
    ] {
        let plan = world.plan(asked);
        assert_eq!(plan.states, states);
        assert!(plan.due.is_empty());
    }
}

#[test]
fn nothing_is_due_while_the_labels_cannot_be_trusted() {
    let mut world = World::new();
    world.entry("plain", &["a"]);
    for why in [Untrusted::NoIndex, Untrusted::OtherBilbo, Untrusted::Behind] {
        world.table.standing = Some(Standing::Untrusted(why));
        let plan = world.plan(Asked::Every);
        assert_eq!(plan.untrusted, Some(why));
        assert!(plan.states.is_empty());
        assert!(plan.due.is_empty());
    }
}

#[test]
fn an_entry_sealed_less_than_quiet_ago_is_still_growing() {
    let minutes = |count: u64| Duration::from_secs(count * 60);
    let now = at(NOW);
    let cases = [
        (minutes(30), Some(minutes(30)), State::Due),
        (
            minutes(30),
            Some(minutes(30) - Duration::from_millis(1)),
            State::StillGrowing,
        ),
        (minutes(31), Some(minutes(30)), State::StillGrowing),
        (minutes(30), None, State::StillGrowing),
        (Duration::ZERO, Some(Duration::ZERO), State::Due),
        (Duration::ZERO, None, State::StillGrowing),
    ];
    for (quiet, ago, state) in cases {
        let mut world = World::new();
        let entry = world.entry("plain", &["a"]);
        world.section.quiet.value = quiet;
        match ago {
            Some(ago) => world.shelf.seal(entry, now - ago),
            None => world.shelf.unseal(entry),
        }
        assert_eq!(
            state_of(&world.plan(Asked::Every), entry),
            state,
            "{quiet:?} {ago:?}"
        );
    }
}

#[test]
fn a_named_entry_skips_the_test_of_quiet_and_no_other() {
    let mut world = World::new();
    let entry = world.entry("plain", &["a"]);
    let other = world.entry("plain", &["b"]);
    world.shelf.unseal(entry);
    world.shelf.unseal(other);
    let plan = world.plan(Asked::Only(&[entry]));
    assert_eq!(plan.states, [(entry, State::Due)]);
    assert_eq!(
        state_of(&world.plan(Asked::Every), entry),
        State::StillGrowing
    );
    world.name("out", &[entry]);
    assert_eq!(
        state_of(&world.plan(Asked::Only(&[entry])), entry),
        State::LeftOut
    );
    unnamed(&mut world, other);
    assert_eq!(
        state_of(&world.plan(Asked::Only(&[other])), other),
        State::NotNamed
    );
}

#[test]
fn refs_that_were_given_and_name_nothing_consider_nothing() {
    let mut world = World::new();
    let one = world.entry("plain", &["a"]);
    let two = world.entry("plain", &["b"]);
    let none = world.plan(Asked::Only(&[]));
    assert_eq!(none.states, []);
    assert!(none.due.is_empty());
    let every = world.plan(Asked::Every);
    assert_eq!(state_of(&every, one), State::Due);
    assert_eq!(state_of(&every, two), State::Due);
}

#[test]
fn an_entry_with_no_passage_after_its_frontier_is_covered_and_costs_no_call() {
    let mut world = World::new();
    let parent = world.entry("folded", &["a", "b", "-c", "-d", "-e"]);
    world.notes(parent, 2, |_| {});
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, parent), State::Covered);
    assert!(plan.due.is_empty());
    assert_eq!(world.shelf.reads_of(parent), 1);
}

#[test]
fn an_entry_that_renders_nothing_is_covered_and_has_no_note_entry() {
    let mut world = World::new();
    let parent = world.entry("folded", &["-a", "-b"]);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, parent), State::Covered);
    assert!(plan.due.is_empty());
    assert!(world.shelf.notes_of(parent).is_none());
}

#[test]
fn an_entry_whose_count_is_its_frontier_is_not_read() {
    let mut world = World::new();
    let parent = world.entry("plain", &["a", "b", "c"]);
    world.notes(parent, 3, |_| {});
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, parent), State::Covered);
    assert_eq!(world.shelf.reads_of(parent), 0);
    let other = world.entry("plain", &["a", "b", "c"]);
    world.notes(other, 2, |_| {});
    world.plan(Asked::Every);
    assert_eq!(world.shelf.reads_of(other), 1);
}

#[test]
fn a_grown_entry_is_due_from_the_first_passage_after_its_frontier() {
    let mut world = World::new();
    let parent = world.entry("plain", &["a", "b", "c", "d"]);
    world.notes(parent, 2, |_| {});
    let plan = world.plan(Asked::Every);
    let due = &plan.due[0];
    let window = &due.windows[0].window;
    assert_eq!(window.text, "#r3 c\n#r4 d\n");
    assert_eq!((window.first, window.last), (3, 4));
    assert!(!due.opens);
    assert_eq!(due.held, Some((Generation(1), 2)));
    assert_eq!(due.last.as_ref().map(|run| run.last), Some(2));
    assert_eq!(due.stored.len(), 1);
    assert_eq!(due.stored[0].text, "Earlier paragraph.");
}

#[test]
fn a_grown_entry_of_a_reader_that_does_not_append_only_is_due_from_record_1() {
    let mut world = World::new();
    let parent = world.entry("rewrite", &["a", "b", "c", "d"]);
    world.notes(parent, 2, |run| run.total = 2);
    let plan = world.plan(Asked::Every);
    let due = &plan.due[0];
    assert!(due.opens);
    assert_eq!(
        (due.windows[0].window.first, due.windows[0].window.last),
        (1, 4)
    );
    assert_eq!(due.held, Some((Generation(1), 2)));
    assert_eq!(due.last, None);
    assert!(due.stored.is_empty());
}

#[test]
fn a_parent_with_a_new_generation_is_due_from_record_1() {
    let mut world = World::new();
    let parent = world.entry("plain", &["a", "b"]);
    world.notes(parent, 2, |_| {});
    world.shelf.regenerate(parent, &["x", "y", "z"]);
    let plan = world.plan(Asked::Every);
    let due = &plan.due[0];
    assert!(due.opens);
    assert_eq!(due.parent.generation, Generation(2));
    assert_eq!(
        (due.windows[0].window.first, due.windows[0].window.last),
        (1, 3)
    );
    assert_eq!(due.windows[0].extent, world.extent_of(parent, 1, 3));
    assert!(due.stored.is_empty());
}

#[test]
fn redo_makes_due_what_any_run_wrote_under_another_instruction_provider_or_model() {
    let cases: [Case; 8] = [
        (true, |run| run.same = false, State::Due),
        (true, |_| {}, State::Covered),
        (
            true,
            |run| run.instruction = "0000000000000001".to_owned(),
            State::Due,
        ),
        (true, |run| run.provider = "other".to_owned(), State::Due),
        (true, |run| run.model = "other".to_owned(), State::Due),
        (false, |run| run.same = false, State::Covered),
        (
            false,
            |run| run.instruction = "0000000000000001".to_owned(),
            State::Covered,
        ),
        (false, |_| {}, State::Covered),
    ];
    for (index, (redo, edit, state)) in cases.into_iter().enumerate() {
        let mut world = World::new();
        world.redo = redo;
        let parent = world.entry("plain", &["a", "b"]);
        world.notes(parent, 2, edit);
        let plan = world.plan(Asked::Every);
        assert_eq!(state_of(&plan, parent), state, "case {index}");
        if state == State::Due {
            assert!(plan.due[0].opens, "case {index}");
            assert_eq!(plan.due[0].windows[0].window.first, 1, "case {index}");
        }
    }
}

#[test]
fn every_due_entry_is_planned_greatest_id_first_and_a_charged_window_is_a_pass() {
    let mut world = World::new();
    let all: Vec<Id> = (0..4).map(|_| world.entry("plain", &["a"])).collect();
    world.section.calls.value = 2;
    let plan = world.plan(Asked::Every);
    let ids: Vec<Id> = plan.due.iter().map(|due| due.parent.id).collect();
    let want: Vec<Id> = all.iter().rev().copied().collect();
    assert_eq!(ids, want);
    assert!(plan.due.iter().all(|due| due.windows.len() == 1));
    let mut script = Script::new()
        .then(answer("r1\nThe user asked about a topic."))
        .then(answer("r1\nThe user asked about a topic."));
    let report = world.calls(&mut script, &plan);
    assert_eq!(script.prompts.len(), 2);
    assert_eq!(report.ended, Ended::Calls);
    let mut world = World::new();
    let parent = world.entry("plain", &["aaaa", "bbbb"]);
    world.section.window.value = 3;
    world.section.calls.value = 1;
    let first = world.extent_of(parent, 1, 1);
    for _ in 0..3 {
        use bilbo_derive::tries::Tries;
        world.held.charge(&first).expect("charge");
    }
    let plan = world.plan(Asked::Every);
    let windows = &plan.due[0].windows;
    assert_eq!(windows.len(), 2);
    assert!(windows[0].pass);
    assert!(!windows[1].pass);
    world.section.tries.value = 4;
    let plan = world.plan(Asked::Every);
    let windows = &plan.due[0].windows;
    assert_eq!(windows.len(), 2);
    assert!(!windows[0].pass);
    assert_eq!(windows[0].extent, first);
}

#[test]
fn a_charge_for_another_extent_counts_for_nothing() {
    let mut world = World::new();
    let texts = vec!["abcd"; 100];
    let parent = world.entry("plain", &texts);
    world.section.window.value = 30;
    let small = world.extent_of(parent, 1, 10);
    let whole = world.extent_of(parent, 1, 100);
    let mut script = Script::new().always(|_, _| no_note());
    for expected in [1, 2] {
        let done = world.derive(&mut script);
        assert_eq!(done.report.counts.failed, 1);
        assert_eq!(world.held.of(&small), expected);
    }
    world.section.window.value = 300;
    let plan = world.plan(Asked::Every);
    assert_eq!(plan.due[0].windows.len(), 1);
    assert_eq!(plan.due[0].windows[0].extent, whole);
    assert!(!plan.due[0].windows[0].pass);
    assert_eq!(world.held.of(&whole), 0);
    world.derive(&mut script);
    assert_eq!(world.held.of(&whole), 1);
    assert_eq!(world.held.of(&small), 2);
}

#[test]
fn redo_takes_a_passed_over_stretch_again() {
    let mut world = World::new();
    let parent = world.entry("plain", &["a", "b"]);
    world.notes(parent, 2, |run| run.passes = 1);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, parent), State::PassedOver);
    assert!(plan.due.is_empty());
    world.redo = true;
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, parent), State::Due);
    let due = &plan.due[0];
    assert!(due.opens);
    assert_eq!(due.windows[0].window.first, 1);
    assert!(due.stored.is_empty());
    let mut script = Script::new().then(answer("r1-2\nThe user asked about a topic."));
    world.derive(&mut script);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, parent), State::Covered);
    assert!(plan.due.is_empty());
}

#[test]
fn a_not_in_value_that_could_mean_several_entries_leaves_them_all_out() {
    let mut world = World::new();
    let all: Vec<Id> = (0..3).map(|_| world.entry("plain", &["a"])).collect();
    world.section.left_out.value = vec!["mixed".to_owned()];
    world
        .table
        .several
        .push(("mixed".to_owned(), vec![all[0], all[1]]));
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, all[0]), State::LeftOut);
    assert_eq!(state_of(&plan, all[1]), State::LeftOut);
    assert_eq!(state_of(&plan, all[2]), State::Due);

    let mut world = World::new();
    let all: Vec<Id> = (0..3).map(|_| world.entry("plain", &["a"])).collect();
    unnamed(&mut world, all[0]);
    unnamed(&mut world, all[1]);
    world.section.named.value = vec!["in".to_owned(), "mixed".to_owned()];
    world
        .table
        .several
        .push(("mixed".to_owned(), vec![all[0], all[1]]));
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, all[0]), State::NotNamed);
    assert_eq!(state_of(&plan, all[1]), State::NotNamed);
    assert_eq!(state_of(&plan, all[2]), State::Due);
}

#[test]
fn a_note_entry_that_ends_in_no_run_record_is_due_from_record_1() {
    let mut world = World::new();
    let parent = world.entry("plain", &["a", "b"]);
    let paragraph = world.paragraph(parent, 1, 2, "A paragraph with no run record after it.");
    world.note_entry(parent, vec![write_paragraph(&paragraph)]);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, parent), State::Due);
    let due = &plan.due[0];
    assert!(due.opens);
    assert_eq!(due.windows[0].window.first, 1);
    assert!(due.stored.is_empty());
    let mut script = Script::new().then(answer("r1-2\nThe user asked about a topic."));
    world.derive(&mut script);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, parent), State::Covered);
}
