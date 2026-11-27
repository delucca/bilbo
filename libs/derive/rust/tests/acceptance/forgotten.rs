use bilbo_derive::plan::{Asked, Plan, State, SummaryState};
use bilbo_derive::spool::{self, Block, Spool};
use bilbo_derive::summary::{
    self,
    asked::About as SummaryAbout,
    block::{Block as SummaryBlock, Form},
};
use bilbo_derive::tags;
use bilbo_store::entries::{Entries, Generation};
use bilbo_store::id::Id;
use bilbo_store::labels::{CaptureLabel, Tag};

use crate::fakes::{World, id};

fn summarising() -> World {
    let mut world = World::new();
    world.summary = Some(summary::keys::defaults());
    world.summary.as_mut().expect("on").least.value = 0;
    world
}

fn state_of(plan: &Plan, entry: Id) -> State {
    plan.states
        .iter()
        .find(|(held, _)| *held == entry)
        .map(|(_, state)| *state)
        .expect("a state")
}

fn summary_of(plan: &Plan, entry: Id) -> SummaryState {
    plan.summaries
        .iter()
        .find(|(held, _)| *held == entry)
        .map(|(_, state)| *state)
        .expect("a summary state")
}

fn derive_label(parent: Id) -> CaptureLabel {
    CaptureLabel::new(format!("derive:{parent}")).expect("label")
}

fn forgotten_ids() -> [Id; 4] {
    [id(201), id(202), id(203), id(204)]
}

#[test]
fn a_forgotten_parent_is_covered_whatever_it_holds() {
    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta"]);
    world.shelf.forget_own(parent);

    let plan = world.plan(Asked::Every);

    assert_eq!(state_of(&plan, parent), State::Covered);
    assert!(plan.due.is_empty());

    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta"]);
    let other = world.entry("plain", &["alpha", "beta"]);
    world.shelf.forget_own(other);
    let stolen = world
        .shelf
        .meta(other)
        .expect("meta")
        .expect("a meta")
        .labels
        .capture
        .expect("a label");
    world.shelf.relabel(parent, &stolen);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, parent), State::Due);

    let mut world = World::new();
    let outside = world.entry("other", &["alpha"]);
    world.shelf.forget_own(outside);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, outside), State::NoInstruction);
}

#[test]
fn notes_forgotten_on_their_own_are_not_written_again() {
    for redo in [false, true] {
        let mut world = summarising();
        world.redo = redo;
        let parent = world.entry("plain", &["alpha", "beta"]);
        let [first, ..] = forgotten_ids();
        world
            .shelf
            .put_forgotten(first, "paragraphs", &derive_label(parent));
        world.settle_summary(parent, None);

        let plan = world.plan(Asked::Every);

        assert_eq!(state_of(&plan, parent), State::Covered, "{redo}");
        assert_eq!(summary_of(&plan, parent), SummaryState::NotComplete);
        assert!(plan.due.is_empty(), "{redo}");
    }

    let mut world = summarising();
    let parent = world.entry("plain", &["alpha", "beta"]);
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, parent), State::Due);

    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta"]);
    let [first, ..] = forgotten_ids();
    world
        .shelf
        .put_forgotten(first, "paragraphs", &derive_label(parent));
    world.note_entry(parent, Vec::new());
    let plan = world.plan(Asked::Every);
    assert_eq!(state_of(&plan, parent), State::Due);

    let emptied = |forgotten: bool| {
        let mut world = summarising();
        let parent = world.entry("plain", &["alpha"]);
        world.settle_summary(parent, None);
        world.shelf.regenerate(parent, &[]);
        if forgotten {
            let [first, ..] = forgotten_ids();
            world
                .shelf
                .put_forgotten(first, "paragraphs", &derive_label(parent));
        }
        let plan = world.plan(Asked::Every);
        (plan, parent)
    };
    let (plan, parent) = emptied(true);
    assert_eq!(summary_of(&plan, parent), SummaryState::NotComplete);
    assert!(plan.due.is_empty(), "{:?}", plan.due);
    let (plan, parent) = emptied(false);
    assert_eq!(summary_of(&plan, parent), SummaryState::NotComplete);
    let retirement = plan
        .due
        .iter()
        .find(|due| due.parent.id == parent)
        .and_then(|due| due.summary.as_ref())
        .expect("a retirement");
    assert!(retirement.retire);
}

#[test]
fn a_summary_that_was_forgotten_or_whose_entry_was_is_not_asked() {
    for redo in [false, true] {
        let mut world = summarising();
        world.redo = redo;
        let parent = world.entry("plain", &["alpha"]);
        world.notes(parent, 1, |_| {});
        let [first, ..] = forgotten_ids();
        world
            .shelf
            .put_forgotten(first, "summary", &summary::label(parent));
        let plan = world.plan(Asked::Every);
        assert_eq!(summary_of(&plan, parent), SummaryState::Forgotten, "{redo}");
        assert_eq!(summary_of(&plan, first), SummaryState::NoReader, "{redo}");
        assert!(plan.due.iter().all(|due| due.summary.is_none()));
    }

    let mut world = summarising();
    let parent = world.entry("plain", &["alpha"]);
    world.notes(parent, 1, |_| {});
    world.settle_summary(parent, None);
    world.shelf.forget_own(parent);
    let plan = world.plan(Asked::Every);
    assert_eq!(summary_of(&plan, parent), SummaryState::Forgotten);
    assert!(plan.due.is_empty());

    let held = |former: bool| {
        let mut world = summarising();
        let parent = world.entry("plain", &["alpha"]);
        world.notes(parent, 1, |_| {});
        let [first, ..] = forgotten_ids();
        if former {
            world
                .shelf
                .put_forgotten(first, "summary", &summary::label(parent));
        }
        world.settle_summary(parent, None);
        (world.plan(Asked::Every), parent)
    };
    let (with_former, parent) = held(true);
    let (without, _) = held(false);
    assert_ne!(summary_of(&with_former, parent), SummaryState::Forgotten);
    assert_eq!(
        summary_of(&with_former, parent),
        summary_of(&without, parent)
    );
    assert_eq!(with_former.due, without.due);
}

fn run_block(parent: Id, generation: u32, text: &str) -> Block {
    Block {
        parent,
        generation: Generation(generation),
        generation_before: None,
        records_before: 0,
        opens: false,
        records: vec![text.as_bytes().to_vec()],
    }
}

fn tags_block(parent: Id, world: &World) -> tags::block::Block {
    tags::block::Block {
        entry: parent,
        about: tags::asked::About {
            generation: Generation(1),
            records: 2,
            notes: world.notes_digest(parent),
        },
        picks: vec![Tag::new("garden").expect("a tag")],
        takes_back: false,
        names: std::collections::BTreeSet::from([Tag::new("garden").expect("a tag")]),
        instruction: world.tags_id.clone(),
        provider: "fake".to_owned(),
        model: "model-a".to_owned(),
        asking: 7,
        passed: false,
    }
}

fn summary_block(parent: Id, form: Form) -> SummaryBlock {
    SummaryBlock {
        entry: parent,
        about: SummaryAbout {
            generation: Generation(1),
            records: 2,
            content: 1,
            notes: None,
        },
        asking: 3,
        form,
        instruction: "instruction".to_owned(),
        provider: "fake".to_owned(),
        model: "model-a".to_owned(),
        answered: None,
        made: 1,
        sent: 1,
        tokens_in: None,
        tokens_out: None,
        text: "A summary.".to_owned(),
    }
}

#[test]
fn the_replay_drops_what_was_kept_for_a_forgotten_parent_and_replays_the_rest() {
    let mut world = World::new();
    let first = world.entry("plain", &["alpha", "beta"]);
    let second = world.entry("plain", &["alpha", "beta"]);
    world.notes(first, 2, |_| {});
    world
        .spooled
        .keep(&spool::write(&run_block(first, 1, "one")))
        .expect("kept");
    world
        .spooled
        .keep(&tags::block::write(&tags_block(first, &world)))
        .expect("kept");
    world
        .summary_spool
        .keep(&summary::block::write(&summary_block(first, Form::Answer)))
        .expect("kept");
    world
        .spooled
        .keep(&spool::write(&run_block(second, 1, "two")))
        .expect("kept");
    world.shelf.forget_own(first);

    let replayed = world.replay();

    assert_eq!(replayed.captured, 1);
    assert_eq!(
        replayed.notices,
        [
            format!("dropped a run kept for {first}: it was forgotten"),
            format!("dropped the tags kept for {first}: it was forgotten"),
            format!("dropped the summary kept for {first}: it was forgotten"),
        ]
    );
    assert_eq!(world.shelf.captures.len(), 1);
    assert!(world.shelf.captures[0].0.ends_with(&second.to_string()));
    assert!(world.spooled.files.is_empty() && world.summary_spool.files.is_empty());
    assert!(world.memo.lines.is_empty() && world.noted.lines.is_empty());
    assert_eq!((world.memo.answers, world.noted.writes), (0, 0));
    assert!(world.shelf.set_metas.is_empty());
}

#[test]
fn the_replay_drops_a_run_and_a_summary_whose_entry_was_forgotten_alone() {
    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta"]);
    world.notes(parent, 2, |_| {});
    let [first, second, ..] = forgotten_ids();
    let tagged = tags_block(parent, &world);
    world.shelf.remove(world.shelf.notes_item(parent));
    world
        .spooled
        .keep(&tags::block::write(&tagged))
        .expect("kept");
    world
        .shelf
        .put_forgotten(first, "paragraphs", &derive_label(parent));
    world
        .shelf
        .put_forgotten(second, "summary", &summary::label(parent));
    world
        .spooled
        .keep(&spool::write(&run_block(parent, 1, "one")))
        .expect("kept");
    for form in [Form::Answer, Form::Pass, Form::Retirement] {
        world
            .summary_spool
            .keep(&summary::block::write(&summary_block(parent, form)))
            .expect("kept");
    }

    let replayed = world.replay();

    let mut expected = vec![
        format!("dropped a run kept for {parent}: its notes were forgotten"),
        format!("dropped the tags kept for {parent}: its notes changed since they were asked"),
    ];
    expected.extend(std::iter::repeat_n(
        format!("dropped the summary kept for {parent}: its summary was forgotten"),
        3,
    ));
    assert_eq!(replayed.notices, expected);
    assert!(world.shelf.captures.is_empty());
    assert!(world.memo.lines.is_empty() && world.noted.lines.is_empty());
    assert_eq!((world.memo.answers, world.noted.writes), (0, 0));
    assert!(world.shelf.set_metas.is_empty());
    assert!(world.spooled.files.is_empty() && world.summary_spool.files.is_empty());
    assert_eq!(
        world
            .shelf
            .asked
            .borrow()
            .iter()
            .filter(|text| *text == "forgotten:")
            .count(),
        1
    );
}

#[test]
fn the_forgotten_entries_are_asked_for_once_a_plan() {
    let mut world = World::new();
    for _ in 0..5 {
        world.entry("plain", &["alpha"]);
    }

    world.plan(Asked::Every);

    assert_eq!(
        world
            .shelf
            .asked
            .borrow()
            .iter()
            .filter(|text| *text == "forgotten:")
            .count(),
        1
    );
}

#[test]
fn the_replay_drops_a_run_made_from_another_generation_of_its_parent() {
    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta"]);
    world
        .spooled
        .keep(&spool::write(&run_block(parent, 1, "one")))
        .expect("kept");
    world.shelf.regenerate(parent, &["gamma", "delta"]);

    let replayed = world.replay();

    assert_eq!(
        replayed.notices,
        [format!(
            "dropped a run kept for {parent}: the entry changed since it was asked"
        )]
    );
    assert!(world.shelf.captures.is_empty());
    assert!(world.spooled.files.is_empty());

    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta"]);
    world
        .spooled
        .keep(&spool::write(&run_block(parent, 1, "one")))
        .expect("kept");
    let replayed = world.replay();
    assert_eq!(replayed.captured, 1);
    assert!(replayed.notices.is_empty());
}

#[test]
fn a_stale_run_among_current_ones_is_dropped_and_the_rest_are_captured_together() {
    let stale = "the entry changed since it was asked";
    let orders: [([u32; 3], &str); 4] = [
        ([2, 1, 2], "one\nthree\n"),
        ([1, 2, 0], "two\n"),
        ([2, 1, 0], "one\n"),
        ([1, 1, 2], "three\n"),
    ];
    for (generations, bytes) in orders {
        let mut world = World::new();
        let parent = world.entry("plain", &["alpha", "beta"]);
        world.shelf.regenerate(parent, &["gamma", "delta"]);
        for (number, text) in generations.iter().zip(["one", "two", "three"]) {
            if *number > 0 {
                world
                    .spooled
                    .keep(&spool::write(&run_block(parent, *number, text)))
                    .expect("kept");
            }
        }

        let replayed = world.replay();

        let stale_blocks = generations.iter().filter(|number| **number == 1).count();
        assert_eq!(
            replayed.notices,
            vec![format!("dropped a run kept for {parent}: {stale}"); stale_blocks],
            "{generations:?}"
        );
        assert_eq!(replayed.captured, 1);
        assert_eq!(
            world.shelf.captures.last().expect("a capture").3,
            bytes.as_bytes(),
            "{generations:?}"
        );
        assert!(world.spooled.files.is_empty());
    }
}

#[test]
fn a_run_for_a_parent_that_is_gone_keeps_todays_handlers() {
    let mut world = World::new();
    let gone = id(99);
    let [first, second, ..] = forgotten_ids();
    world
        .shelf
        .put_forgotten(first, "paragraphs", &derive_label(gone));
    world
        .shelf
        .put_forgotten(second, "summary", &summary::label(gone));
    world
        .spooled
        .keep(&spool::write(&run_block(gone, 1, "one")))
        .expect("kept");
    world
        .summary_spool
        .keep(&summary::block::write(&summary_block(gone, Form::Answer)))
        .expect("kept");

    let replayed = world.replay();

    assert_eq!(replayed.captured, 1);
    assert!(
        replayed
            .notices
            .iter()
            .all(|notice| !notice.contains("were forgotten") && !notice.contains("was forgotten")),
        "{:?}",
        replayed.notices
    );
    assert!(
        replayed.notices.contains(&format!(
            "dropped the summary kept for {gone}: the store no longer holds it"
        )),
        "{:?}",
        replayed.notices
    );
}

#[test]
fn a_stale_block_meets_the_forgotten_parent_and_the_forgotten_notes_before_its_generation() {
    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta"]);
    world.shelf.regenerate(parent, &["alpha", "beta"]);
    world.shelf.forget_own(parent);
    let [former, ..] = forgotten_ids();
    world
        .shelf
        .put_forgotten(former, "paragraphs", &derive_label(parent));
    world
        .spooled
        .keep(&spool::write(&run_block(parent, 1, "one")))
        .expect("kept");
    let replayed = world.replay();
    assert_eq!(
        replayed.notices,
        [format!("dropped a run kept for {parent}: it was forgotten")]
    );

    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta"]);
    world.shelf.regenerate(parent, &["alpha", "beta"]);
    let [first, ..] = forgotten_ids();
    world
        .shelf
        .put_forgotten(first, "paragraphs", &derive_label(parent));
    for text in ["one", "two"] {
        world
            .spooled
            .keep(&spool::write(&run_block(parent, 1, text)))
            .expect("kept");
    }
    let replayed = world.replay();
    assert_eq!(
        replayed.notices,
        [format!(
            "dropped a run kept for {parent}: its notes were forgotten"
        )]
    );
}
