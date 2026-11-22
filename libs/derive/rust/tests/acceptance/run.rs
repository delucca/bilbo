use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use bilbo_derive::plan::{self, Asked, State};
use bilbo_derive::record::{self, Run};
use bilbo_derive::run::{Ended, Event};
use bilbo_derive::spool::{self, Block};
use bilbo_derive::view::Paragraph;
use bilbo_llm::model::{Cause, Limits, Outcome, Prompt, Usage, What};
use bilbo_store::entries::{Entries, Generation};
use bilbo_store::id::Id;
use bilbo_store::opaque;

use crate::fakes::{
    EPOCH_AT, Longest, NOW, Script, World, answer, at, busy, cut, failed, instruction_for, no_note,
    read_paragraph, refused, stopped, usage, write_paragraph,
};

fn texts(count: usize) -> Vec<&'static str> {
    vec!["aaaa", "bbbb", "cccc", "dddd", "eeee", "ffff"][..count].to_vec()
}

fn windowed(count: usize) -> (World, Id) {
    let mut world = World::new();
    let parent = world.entry("plain", &texts(count));
    world.section.window.value = 3;
    (world, parent)
}

fn last_run(block: &Block) -> Run {
    record::read(block.records.last().expect("a record")).expect("a run record")
}

fn first_record(prompt: &Prompt) -> u64 {
    let (_, records) = prompt
        .material
        .split_once("<records>\n#r")
        .expect("records");
    let digits: String = records.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().expect("a number")
}

fn field(line: &str, at: usize) -> &str {
    line.split('\t').nth(at).expect("a field")
}

fn seconds(count: u64) -> Duration {
    Duration::from_secs(count)
}

fn para(first: u64, last: u64, text: &str) -> String {
    if first == last {
        format!("r{first}\n{text}")
    } else {
        format!("r{first}-{last}\n{text}")
    }
}

fn each_window(first: u64, count: u64) -> impl FnMut(&Prompt, usize) -> Outcome {
    move |_, called| {
        let at = first + called as u64 - 1;
        assert!(
            at < first + count,
            "called more often than there are windows"
        );
        answer(&para(at, at, "The user asked about a topic."))
    }
}

#[test]
fn a_working_call_spools_its_paragraphs_then_its_run_record() {
    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta", "gamma"]);
    let mut script = Script::new().then(answer("r1-2\nThe user asked.\n\nr3\nDone."));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Done);
    let blocks = world.spooled.blocks();
    assert_eq!(blocks.len(), 1);
    let block = &blocks[0];
    assert_eq!(
        (block.parent, block.generation, block.generation_before),
        (parent, Generation(1), None)
    );
    assert_eq!((block.records_before, block.opens), (0, false));
    let first: Paragraph = world.paragraph(parent, 1, 2, "The user asked.");
    let second: Paragraph = world.paragraph(parent, 3, 3, "Done.");
    let mut run = world.run_record(parent, 1, 3);
    run.made = NOW;
    run.sent = 9;
    run.written = 2;
    run.kept = 2;
    assert_eq!(
        block.records,
        [
            write_paragraph(&first),
            write_paragraph(&second),
            record::write(&run).into_bytes()
        ]
    );
}

#[test]
fn an_answer_of_white_space_spools_a_run_record_alone() {
    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta"]);
    let mut script = Script::new().then(answer("  \n\n"));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    let blocks = world.spooled.blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].records.len(), 1);
    let run = last_run(&blocks[0]);
    assert_eq!((run.first, run.last, run.entry), (1, 2, parent));
    assert_eq!((run.written, run.kept, run.passed), (0, 0, false));
    assert_eq!(report.counts.failed, 0);
    assert_eq!(field(&world.lines.0[0], 2), "empty");
}

#[test]
fn an_answer_with_text_and_no_block_kept_is_a_failed_call() {
    for text in [
        "I could not write notes.",
        "r9\nBeyond the window.",
        "```\n```",
    ] {
        let mut world = World::new();
        let parent = world.entry("plain", &["alpha", "beta"]);
        let mut script = Script::new().then(answer(text));
        let plan = world.plan(Asked::Every);
        let report = world.calls(&mut script, &plan);
        assert!(world.spooled.files.is_empty(), "{text}");
        assert_eq!(report.counts.failed, 1);
        assert_eq!(field(&world.lines.0[0], 2), "failed");
        assert_eq!(field(&world.lines.0[0], 18), "no-note");
        assert_eq!(world.held.of(&world.extent_of(parent, 1, 2)), 1);
    }
}

#[test]
fn a_cut_answer_is_a_failed_call_and_nothing_of_it_is_kept() {
    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta"]);
    let mut script = Script::new().then(cut());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert!(world.spooled.files.is_empty());
    assert_eq!(report.counts.failed, 1);
    assert_eq!(
        (report.counts.tokens_in, report.counts.tokens_out),
        (100, 50)
    );
    assert_eq!(field(&world.lines.0[0], 18), "cut");
    assert_eq!(world.held.of(&world.extent_of(parent, 1, 2)), 0);
}

#[test]
fn a_refusal_is_charged_at_once_and_ends_the_entry_for_the_run() {
    let mut world = World::new();
    world.section.window.value = 3;
    world.section.failures.value = 1;
    let kept = world.entry("plain", &texts(2));
    let refused_entry = world.entry("plain", &texts(2));
    let mut script = Script::new()
        .then(refused(Some("safety")))
        .then(failed(What::Unreadable));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Failures);
    assert_eq!(script.prompts.len(), 2);
    assert_eq!(world.held.of(&world.extent_of(refused_entry, 1, 1)), 1);
    assert_eq!(world.held.of(&world.extent_of(refused_entry, 2, 2)), 0);
    assert_eq!(world.held.of(&world.extent_of(kept, 1, 1)), 0);
    assert_eq!(report.counts.refused, 1);
    assert_eq!(field(&world.lines.0[0], 2), "refused");
    assert_eq!(field(&world.lines.0[0], 18), "safety");
}

#[test]
fn a_refusal_is_no_part_of_the_streak() {
    for (failures, prompts, ended) in [(2, 2, Ended::Failures), (3, 5, Ended::Done)] {
        let mut world = World::new();
        world.section.failures.value = failures;
        for _ in 0..5 {
            world.entry("plain", &["alpha"]);
        }
        let mut script = Script::new()
            .then(failed(What::Unreadable))
            .then(failed(What::Unreadable))
            .then(refused(None))
            .then(failed(What::Unreadable))
            .then(answer("r1\nThe user asked about a topic."));
        let plan = world.plan(Asked::Every);
        let report = world.calls(&mut script, &plan);
        assert_eq!(script.prompts.len(), prompts, "failures {failures}");
        assert_eq!(report.ended, ended, "failures {failures}");
        if prompts > 2 {
            assert_eq!(field(&world.lines.0[2], 18), "unknown");
        }
    }
}

#[test]
fn a_failed_call_ends_its_entry_and_the_run_goes_on_with_the_next() {
    let mut world = World::new();
    world.section.window.value = 3;
    let lower = world.entry("plain", &texts(2));
    let higher = world.entry("plain", &texts(2));
    let mut script = Script::new()
        .then(failed(What::ToolError))
        .then(answer(&para(1, 1, "The user asked about a topic.")))
        .then(answer(&para(2, 2, "The assistant did a thing.")));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Done);
    assert_eq!(script.prompts.len(), 3);
    let blocks = world.spooled.blocks();
    assert_eq!(blocks.len(), 2);
    assert!(blocks.iter().all(|block| block.parent == lower));
    assert_eq!(world.held.of(&world.extent_of(higher, 1, 1)), 1);
}

#[test]
fn a_window_is_never_called_before_the_records_behind_it_are_covered() {
    let (mut world, _) = windowed(3);
    let mut script = Script::new().always(each_window(1, 3));
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    let firsts: Vec<u64> = script.prompts.iter().map(first_record).collect();
    let ends: Vec<u64> = world
        .spooled
        .blocks()
        .iter()
        .map(|block| last_run(block).last)
        .collect();
    assert_eq!(firsts, [1, 2, 3]);
    assert_eq!(ends, [1, 2, 3]);
    for (at, first) in firsts.iter().enumerate().skip(1) {
        assert_eq!(*first, ends[at - 1] + 1);
    }

    let (mut world, parent_failing) = windowed(3);
    let mut script = Script::new()
        .then(answer(&para(1, 1, "The user asked about a topic.")))
        .then(failed(What::ToolError));
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    assert_eq!(script.prompts.len(), 2);
    world.replay();
    let notes = world.shelf.notes_of(parent_failing).expect("notes");
    let stored = notes[0].last().expect("a record");
    assert_eq!(
        record::read(&stored[..stored.len() - 1]).expect("run").last,
        1
    );
    let mut script = Script::new().always(each_window(2, 2));
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    assert_eq!(
        script.prompts.iter().map(first_record).collect::<Vec<_>>(),
        [2, 3]
    );
}

#[test]
fn a_window_that_always_fails_is_passed_over_after_derive_tries_runs() {
    for (calls, tries, passing_run) in [(20, 3, 4), (1, 3, 4), (20, 4, 5)] {
        let mut world = World::new();
        world.section.calls.value = calls;
        world.section.tries.value = tries;
        let parent = world.entry("plain", &["alpha", "beta"]);
        let extent = world.extent_of(parent, 1, 2);
        let mut script = Script::new().always(|_, _| no_note());
        for run in 1..=passing_run {
            let done = world.derive(&mut script);
            if run < passing_run {
                assert_eq!(
                    world.held.of(&extent) as usize,
                    run,
                    "{calls} {tries} {run}"
                );
                assert_eq!(done.report.counts.failed, 1);
            } else {
                assert_eq!(done.report.counts.passed, 1, "{calls} {tries}");
                assert_eq!(done.report.counts.calls, 0);
                assert_eq!(world.held.of(&extent), 0);
            }
        }
        assert_eq!(script.prompts.len(), passing_run - 1);
        let plan = world.plan(Asked::Every);
        assert_eq!(
            plan.states,
            [
                (parent, State::PassedOver),
                (plan.states[1].0, State::NoInstruction)
            ]
        );
    }
}

#[test]
fn a_streak_of_derive_failures_stops_the_run_and_charges_nothing() {
    for (failures, prompts, ended, charged) in [(3, 3, Ended::Failures, 0), (4, 4, Ended::Done, 1)]
    {
        let mut world = World::new();
        world.section.failures.value = failures;
        let parents: Vec<Id> = (0..4).map(|_| world.entry("plain", &["alpha"])).collect();
        let mut script = Script::new()
            .then(failed(What::Unreadable))
            .then(failed(What::Unreadable))
            .then(failed(What::Unreadable))
            .then(answer("r1\nThe user asked about a topic."));
        let plan = world.plan(Asked::Every);
        let report = world.calls(&mut script, &plan);
        assert_eq!(report.ended, ended, "failures {failures}");
        assert_eq!(script.prompts.len(), prompts, "failures {failures}");
        let held: Vec<u8> = parents
            .iter()
            .map(|parent| world.held.of(&world.extent_of(*parent, 1, 1)))
            .collect();
        assert_eq!(held, [0, charged, charged, charged], "failures {failures}");
    }

    let mut world = World::new();
    world.section.failures.value = 4;
    let parents: Vec<Id> = (0..5).map(|_| world.entry("plain", &["alpha"])).collect();
    let mut script = Script::new()
        .then(failed(What::Unreadable))
        .then(failed(What::Unreadable))
        .then(failed(What::Unreadable))
        .then(failed(What::Unreadable))
        .then(answer("r1\nThe user asked about a topic."));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Failures);
    assert_eq!(script.prompts.len(), 4);
    assert_eq!(script.queue.len(), 1);
    for parent in parents {
        assert_eq!(world.held.of(&world.extent_of(parent, 1, 1)), 0);
    }
    assert!(world.spooled.files.is_empty());

    let mut world = World::new();
    world.section.failures.value = 3;
    let parents: Vec<Id> = (0..5).map(|_| world.entry("plain", &["alpha"])).collect();
    let mut script = Script::new()
        .then(failed(What::Unreadable))
        .then(no_note())
        .then(failed(What::Unreadable))
        .then(failed(What::Unreadable))
        .then(answer("r1\nThe user asked about a topic."));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Done);
    for parent in parents.iter().skip(1) {
        assert_eq!(world.held.of(&world.extent_of(*parent, 1, 1)), 1);
    }
    assert_eq!(world.held.of(&world.extent_of(parents[0], 1, 1)), 0);
}

#[test]
fn a_working_call_ends_a_streak_and_its_failures_are_charged() {
    let mut world = World::new();
    world.section.failures.value = 3;
    let parents: Vec<Id> = (0..3).map(|_| world.entry("plain", &["alpha"])).collect();
    let mut script = Script::new()
        .then(failed(What::Unreadable))
        .then(failed(What::Unreadable))
        .then(answer("r1\nThe user asked about a topic."));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Done);
    assert_eq!(world.held.of(&world.extent_of(parents[2], 1, 1)), 1);
    assert_eq!(world.held.of(&world.extent_of(parents[1], 1, 1)), 1);
    assert_eq!(world.held.of(&world.extent_of(parents[0], 1, 1)), 0);
}

fn alone() -> (World, Id) {
    let mut world = World::new();
    let parent = world.entry("plain", &["alpha"]);
    (world, parent)
}

fn good() -> Outcome {
    answer("r1\nThe user asked about a topic.")
}

#[test]
fn a_busy_provider_is_asked_again_after_the_wait_it_names() {
    let (mut world, _) = alone();
    let mut script = Script::new()
        .then(busy(Some(seconds(7))))
        .then(busy(Some(seconds(9))))
        .then(good());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(*world.watch.slept.borrow(), [seconds(7), seconds(9)]);
    assert_eq!(script.prompts.len(), 3);
    assert_eq!(report.counts.calls, 3);
    assert_eq!(report.ended, Ended::Done);
}

#[test]
fn with_no_wait_named_each_retry_waits_the_factor_times_the_one_before() {
    for (wait, factor, slept) in [
        (30, 2, [30, 60]),
        (31, 2, [31, 62]),
        (30, 3, [30, 90]),
        (30, 1, [30, 30]),
    ] {
        let (mut world, _) = alone();
        world.section.retry_wait.value = seconds(wait);
        world.section.retry_factor.value = factor;
        let mut script = Script::new().then(busy(None)).then(busy(None)).then(good());
        let plan = world.plan(Asked::Every);
        world.calls(&mut script, &plan);
        assert_eq!(
            *world.watch.slept.borrow(),
            [seconds(slept[0]), seconds(slept[1])]
        );
    }
}

#[test]
fn a_wait_over_the_retry_limit_ends_the_run_at_once() {
    let (mut world, _) = alone();
    let mut script = Script::new().then(busy(Some(seconds(300)))).then(good());
    let plan = world.plan(Asked::Every);
    assert_eq!(world.calls(&mut script, &plan).ended, Ended::Done);
    assert_eq!(*world.watch.slept.borrow(), [seconds(300)]);

    let (mut world, _) = alone();
    let mut script = Script::new().then(busy(Some(seconds(301)))).then(good());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert!(matches!(report.ended, Ended::Wait(wait, _) if wait == seconds(301)));
    assert!(world.watch.slept.borrow().is_empty());
    assert_eq!(script.prompts.len(), 1);

    let (mut world, _) = alone();
    world.section.retry_limit.value = seconds(360);
    let mut script = Script::new().then(busy(Some(seconds(301)))).then(good());
    let plan = world.plan(Asked::Every);
    assert_eq!(world.calls(&mut script, &plan).ended, Ended::Done);
    assert_eq!(*world.watch.slept.borrow(), [seconds(301)]);
}

#[test]
fn a_provider_still_busy_after_the_retries_stops_the_run_and_charges_nothing() {
    for (retries, attempts) in [(0, 1), (2, 3), (3, 4)] {
        let (mut world, _) = alone();
        world.section.retries.value = retries;
        let mut script = Script::new().always(|_, _| busy(None));
        let plan = world.plan(Asked::Every);
        let report = world.calls(&mut script, &plan);
        assert_eq!(
            report.ended,
            Ended::Busy(Cause::new(What::RateLimit), attempts as u32),
            "retries {retries}"
        );
        assert_eq!(script.prompts.len(), attempts, "retries {retries}");
        assert_eq!(report.counts.calls as usize, attempts);
        assert!(world.held.charges.is_empty());
        assert!(world.spooled.files.is_empty());
    }
}

#[test]
fn every_attempt_counts_toward_derive_calls() {
    let (mut world, _) = alone();
    world.section.calls.value = 2;
    let mut script = Script::new().then(busy(None)).then(good());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!((report.counts.calls, report.ended), (2, Ended::Done));

    let (mut world, _) = alone();
    world.section.calls.value = 2;
    let mut script = Script::new().then(busy(None)).then(busy(None)).then(good());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Calls);
    assert_eq!(report.counts.failed, 0);
    assert_eq!(script.prompts.len(), 2);

    let mut world = World::new();
    world.section.calls.value = 2;
    world.entry("plain", &["alpha"]);
    world.entry("plain", &["alpha"]);
    let mut script = Script::new().then(busy(None)).then(good()).then(good());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Calls);
    assert_eq!(script.prompts.len(), 2);
}

#[test]
fn a_stop_ends_the_run_at_once_and_charges_nothing() {
    let mut world = World::new();
    world.entry("plain", &["alpha"]);
    world.entry("plain", &["alpha"]);
    let mut script = Script::new().then(stopped()).then(good());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Stopped(Cause::new(What::Unauthorized)));
    assert_eq!(script.prompts.len(), 1);
    assert!(world.held.charges.is_empty());
    assert_eq!(field(&world.lines.0[0], 2), "stopped");
}

#[test]
fn a_failure_then_a_busy_stop_charges_the_failure() {
    for (first, charged) in [(no_note(), 1), (failed(What::Unreadable), 0)] {
        let mut world = World::new();
        world.section.retries.value = 0;
        let lower = world.entry("plain", &["alpha"]);
        let higher = world.entry("plain", &["alpha"]);
        let mut script = Script::new().then(first).then(busy(None));
        let plan = world.plan(Asked::Every);
        let report = world.calls(&mut script, &plan);
        assert_eq!(report.ended, Ended::Busy(Cause::new(What::RateLimit), 1));
        assert_eq!(world.held.of(&world.extent_of(higher, 1, 1)), charged);
        assert_eq!(world.held.of(&world.extent_of(lower, 1, 1)), 0);
    }
}

#[test]
fn a_failure_then_a_stop_charges_the_failure() {
    for (first, charged) in [(no_note(), 1), (failed(What::Unreadable), 0)] {
        let mut world = World::new();
        let lower = world.entry("plain", &["alpha"]);
        let higher = world.entry("plain", &["alpha"]);
        let mut script = Script::new().then(first).then(stopped());
        let plan = world.plan(Asked::Every);
        let report = world.calls(&mut script, &plan);
        assert!(matches!(report.ended, Ended::Stopped(_)));
        assert_eq!(world.held.of(&world.extent_of(higher, 1, 1)), charged);
        assert_eq!(world.held.of(&world.extent_of(lower, 1, 1)), 0);
    }
}

#[test]
fn a_failed_calls_tokens_reach_the_totals_and_the_log() {
    let (mut world, _) = alone();
    let mut script = Script::new().then(Outcome::Failed {
        cause: Cause::new(What::StopReason),
        usage: usage(700, 90),
    });
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(
        (report.counts.tokens_in, report.counts.tokens_out),
        (700, 90)
    );
    let line = &world.lines.0[0];
    assert_eq!((field(line, 11), field(line, 12)), ("700", "90"));
    assert_eq!(field(line, 18), "stop-reason");
}

#[test]
fn an_earlier_paragraph_over_the_bound_is_counted_a_stretch_at_a_time() {
    let (mut world, parent) = windowed(3);
    world.bound = 64;
    world.section.earlier.value = 200;
    let longest = Arc::new(AtomicUsize::new(0));
    world.tokens = Box::new(Longest(Arc::clone(&longest)));
    let held = "x ".repeat(250);
    let wrote = "y ".repeat(250);
    world.notes_saying(parent, 1, &held, |_| {});
    let mut script = Script::new()
        .then(answer(&para(2, 2, &wrote)))
        .then(answer(&para(3, 3, "The user asked about a topic.")));
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    assert!(longest.load(Ordering::Relaxed) <= 64);
    assert!(script.prompts[0].material.contains(&held));
    assert!(script.prompts[1].material.contains(&wrote));
    assert!(!script.prompts[1].material.contains(&held));
}

#[test]
fn a_pass_is_a_run_record_with_no_call_and_the_entry_goes_on() {
    let (mut world, parent) = windowed(2);
    let first = world.extent_of(parent, 1, 1);
    for _ in 0..3 {
        use bilbo_derive::tries::Tries;
        world.held.charge(&first).expect("charge");
    }
    let mut script = Script::new().then(answer(&para(2, 2, "The user asked about a topic.")));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.counts.passed, 1);
    assert_eq!(script.prompts.len(), 1);
    assert_eq!(first_record(&script.prompts[0]), 2);
    let blocks = world.spooled.blocks();
    assert_eq!(blocks.len(), 2);
    let passed = last_run(&blocks[0]);
    assert!(passed.passed);
    assert_eq!((passed.passes, passed.written, passed.sent), (1, 0, 0));
    assert_eq!((passed.first, passed.last), (1, 1));
    assert_eq!(blocks[0].records.len(), 1);
    assert_eq!(world.held.of(&first), 0);
    let next = last_run(&blocks[1]);
    assert!(!next.passed);
    assert_eq!(next.passes, 1);
    assert_eq!(field(&world.lines.0[0], 2), "passed");
}

#[test]
fn each_call_is_shown_the_paragraphs_before_it_from_the_store_and_from_this_run() {
    let (mut world, parent) = windowed(3);
    world.notes_saying(parent, 1, "Stored words here.", |_| {});
    let mut script = Script::new()
        .then(answer(&para(2, 2, "Written words here.")))
        .then(answer(&para(3, 3, "Final words here.")));
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    let first = &script.prompts[0].material;
    assert!(first.contains("Stored words here."));
    assert!(!first.contains("Written words here."));
    let second = &script.prompts[1].material;
    let stored = second.find("Stored words here.").expect("stored");
    let written = second.find("Written words here.").expect("written");
    assert!(stored < written);
    assert!(!second.contains("Final words here."));
}

#[test]
fn a_call_is_handed_the_limits_the_run_was_given() {
    let (mut world, _) = alone();
    let mut script = Script::new().then(good());
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    assert_eq!(
        script.limits,
        [Limits {
            answer_tokens: 16_000,
            timeout: seconds(300),
            response: 4 << 20,
            poll: Duration::from_millis(5),
        }]
    );
    let (mut world, _) = alone();
    world.limits = Limits {
        answer_tokens: 16_001,
        timeout: seconds(301),
        response: (4 << 20) + 1,
        poll: Duration::from_millis(6),
    };
    let mut script = Script::new().then(good());
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    assert_eq!(
        script.limits,
        [Limits {
            answer_tokens: 16_001,
            timeout: seconds(301),
            response: (4 << 20) + 1,
            poll: Duration::from_millis(6),
        }]
    );
}

#[test]
fn a_calls_line_names_the_provider_the_run_was_handed() {
    let (mut world, _) = alone();
    let mut script = Script::new().then(good());
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    assert_eq!(world.lines.0.len(), 1);
    assert_eq!(field(&world.lines.0[0], 6), "fake");
}

#[test]
fn the_writer_is_prepared_once_before_its_first_call_and_never_when_nothing_is_due() {
    let (mut world, _) = windowed(2);
    let mut script = Script::new().always(each_window(1, 2));
    world.derive(&mut script);
    assert_eq!((script.prepared, script.prompts.len()), (1, 2));

    let mut script = Script::new();
    let done = world.derive(&mut script);
    assert_eq!((script.prepared, script.prompts.len()), (0, 0));
    assert_eq!(done.report.ended, Ended::Done);

    let (mut world, parent) = alone();
    let mut script = Script::new();
    script.refuse_prepare = Some("no key is set".to_owned());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Prepare("no key is set".to_owned()));
    assert!(script.prompts.is_empty());
    assert!(world.spooled.files.is_empty());
    assert_eq!(script.prepared, 1);

    let extent = world.extent_of(parent, 1, 1);
    for _ in 0..3 {
        use bilbo_derive::tries::Tries;
        world.held.charge(&extent).expect("charge");
    }
    let mut script = Script::new();
    script.refuse_prepare = Some("no key is set".to_owned());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(script.prepared, 0);
    assert_eq!(report.counts.passed, 1);
}

#[test]
fn the_run_record_names_what_was_asked_and_what_answered() {
    for (model, answered) in [
        (Some("model-answered"), Some("model-answered")),
        (None, None),
    ] {
        let (mut world, parent) = alone();
        let mut script = Script::new().then(Outcome::Answer {
            text: "r1\nThe user asked about a topic.".to_owned(),
            usage: Usage::default(),
            model: model.map(str::to_owned),
        });
        let plan = world.plan(Asked::Every);
        world.calls(&mut script, &plan);
        let run = last_run(&world.spooled.blocks()[0]);
        assert_eq!(run.kind, "plain");
        assert_eq!(run.instruction, instruction_for("plain").id);
        assert_eq!(
            (run.provider.as_str(), run.model.as_str()),
            ("fake", "model-a")
        );
        assert_eq!(run.answered.as_deref(), answered);
        assert_eq!((run.tokens_in, run.tokens_out), (None, None));
        assert_eq!(run.entry, parent);
        assert_eq!(run.made, NOW);
    }
}

#[test]
fn the_run_record_counts_what_each_check_dropped() {
    let mut world = World::new();
    world.entry("plain", &["alpha", "beta", "gamma"]);
    let text = [
        "r1\nThe user asked about a topic.",
        "r3-2\nBackwards.",
        "r9\nBeyond.",
        "r2\nThe assistant edited src/missing.rs and src/other.rs.",
    ]
    .join("\n\n");
    let mut script = Script::new().then(answer(&text));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    let run = last_run(&world.spooled.blocks()[0]);
    assert_eq!((run.written, run.kept, run.range, run.names), (4, 1, 2, 1));
    assert_eq!((report.counts.notes, report.counts.dropped), (1, 3));
}

#[test]
fn same_and_passes_carry_from_one_run_record_to_the_next() {
    let (mut world, parent) = windowed(2);
    let mut script = Script::new().always(each_window(1, 2));
    world.derive(&mut script);
    let notes = world.shelf.notes_of(parent).expect("notes");
    let runs: Vec<Run> = notes[0]
        .iter()
        .filter_map(|record| record::read(&record[..record.len() - 1]))
        .collect();
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().all(|run| run.same && run.passes == 0));
    world.shelf.grow(parent, &["cccc"]);
    let mut other = Script::new().always(each_window(3, 1));
    other.model = "model-b".to_owned();
    world.derive(&mut other);
    world.shelf.grow(parent, &["dddd"]);
    let mut again = Script::new().always(each_window(4, 1));
    again.model = "model-b".to_owned();
    world.derive(&mut again);
    let notes = world.shelf.notes_of(parent).expect("notes");
    let runs: Vec<Run> = notes[0]
        .iter()
        .filter_map(|record| record::read(&record[..record.len() - 1]))
        .collect();
    assert_eq!(
        runs.iter().map(|run| run.same).collect::<Vec<_>>(),
        [true, true, false, false]
    );

    let (mut world, parent) = windowed(3);
    let first = world.extent_of(parent, 1, 1);
    for _ in 0..3 {
        use bilbo_derive::tries::Tries;
        world.held.charge(&first).expect("charge");
    }
    let mut script = Script::new().always(each_window(2, 2));
    world.derive(&mut script);
    world.shelf.grow(parent, &["dddd"]);
    let mut script = Script::new().always(each_window(4, 1));
    world.derive(&mut script);
    let notes = world.shelf.notes_of(parent).expect("notes");
    let runs: Vec<Run> = notes[0]
        .iter()
        .filter_map(|record| record::read(&record[..record.len() - 1]))
        .collect();
    assert_eq!(
        runs.iter().map(|run| run.passes).collect::<Vec<_>>(),
        [1, 1, 1, 1]
    );
    assert_eq!(runs.iter().filter(|run| run.passed).count(), 1);
}

#[test]
fn what_is_spooled_is_what_the_stores_filter_gives() {
    let mut world = World::new();
    world.section.names.value = 0.0;
    world.entry("plain", &["alpha"]);
    let blob: String = (0..300)
        .map(|at| {
            char::from(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"[at % 62])
        })
        .collect();
    let mut script = Script::new().then(answer(&para(1, 1, &blob)));
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    let block = &world.spooled.blocks()[0];
    let before = write_paragraph(&Paragraph {
        text: blob.clone(),
        at: Some(at(EPOCH_AT + 1)),
        source: bilbo_derive::tries::Extent {
            parent: block.parent,
            generation: block.generation,
            first: 1,
            last: 1,
        },
    });
    let spooled = &block.records[0];
    assert_ne!(spooled, &before);
    assert_eq!(spooled, &opaque::filter(&before).into_owned());
    assert_eq!(opaque::filter(spooled).into_owned(), *spooled);
    assert!(String::from_utf8_lossy(spooled).contains("[opaque: 300 chars, xxh3 "));
    for record in &block.records {
        assert_eq!(opaque::filter(record).into_owned(), *record);
    }
}

#[test]
fn no_store_port_is_reached_between_the_first_call_and_the_last() {
    let (mut world, _) = windowed(3);
    let touched = Rc::clone(&world.shelf.touched);
    let start = Rc::new(Cell::new(None));
    let watched = Rc::clone(&start);
    let mut script = Script::new().always(move |_, called| {
        let now = touched.get();
        match watched.get() {
            Some(first) => assert_eq!(first, now),
            None => watched.set(Some(now)),
        }
        answer(&para(
            called as u64,
            called as u64,
            "The user asked about a topic.",
        ))
    });
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    assert_eq!(script.prompts.len(), 3);
}

#[test]
fn the_log_gets_one_line_an_attempt() {
    let mut world = World::new();
    for _ in 0..6 {
        world.entry("plain", &["alpha"]);
    }
    let mut script = Script::new()
        .then(good())
        .then(answer("   "))
        .then(refused(Some("safety")))
        .then(failed(What::ToolError))
        .then(busy(None))
        .then(good())
        .then(stopped());
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    let words: Vec<&str> = world.lines.0.iter().map(|line| field(line, 2)).collect();
    assert_eq!(
        words,
        [
            "run", "empty", "refused", "failed", "busy", "run", "stopped"
        ]
    );
    assert_eq!(script.prompts.len(), words.len());
    assert!(world.lines.0.iter().all(|line| line.starts_with("d1\t")));

    let (mut world, parent) = windowed(2);
    let first = world.extent_of(parent, 1, 1);
    for _ in 0..3 {
        use bilbo_derive::tries::Tries;
        world.held.charge(&first).expect("charge");
    }
    let mut script = Script::new().always(each_window(2, 1));
    world.calls(&mut script, &world.plan(Asked::Every));
    assert_eq!(field(&world.lines.0[0], 2), "passed");
}

#[test]
fn two_stores_of_the_same_files_send_byte_equal_prompts() {
    let mut sent = Vec::new();
    for offset in [0, 100] {
        let mut world = World::starting_at(offset);
        world.section.window.value = 3;
        let parent = world.entry("headed", &texts(3));
        world.notes_saying(parent, 1, "Earlier words here.", |_| {});
        let mut script = Script::new().always(each_window(2, 2));
        world.derive(&mut script);
        sent.push(script.prompts);
    }
    assert_eq!(sent[0].len(), 2);
    assert_eq!(sent[0], sent[1]);
    assert!(sent[0][0].material.contains("A header line."));
}

#[test]
fn the_spool_is_kept_in_one_capture_a_parent_and_emptied() {
    let (mut world, parent) = windowed(2);
    let mut script = Script::new().always(each_window(1, 2));
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    assert_eq!(world.spooled.files.len(), 2);
    let expected: Vec<u8> = world
        .spooled
        .blocks()
        .iter()
        .flat_map(|block| block.records.iter())
        .flat_map(|record| record.iter().copied().chain(*b"\n"))
        .collect();
    let replayed = world.replay();
    assert_eq!(replayed.captured, 1);
    assert!(replayed.notices.is_empty());
    assert!(world.spooled.files.is_empty());
    assert_eq!(world.shelf.captures.len(), 1);
    let (label, kind, id_label, bytes) = &world.shelf.captures[0];
    assert_eq!(label, &format!("derive:{parent}"));
    assert_eq!(kind, "summary");
    assert_eq!(*id_label, Some(parent));
    assert_eq!(bytes, &expected);
    assert_eq!(world.shelf.notes_of(parent).expect("notes")[0].len(), 4);
}

#[test]
fn a_block_that_opens_a_generation_is_captured_without_the_stored_records() {
    let mut world = World::new();
    let parent = world.entry("rewrite", &["alpha", "beta", "gamma"]);
    world.notes(parent, 2, |_| {});
    let mut script = Script::new().then(answer("r1-3\nThe user asked about a topic."));
    let done = world.derive(&mut script);
    assert_eq!(done.replayed.captured, 1);
    let notes = world.shelf.notes_of(parent).expect("notes");
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].len(), 2);
    assert_eq!(notes[1].len(), 2);
    let (_, _, _, bytes) = &world.shelf.captures[0];
    assert!(!String::from_utf8_lossy(bytes).contains("Earlier paragraph."));
    let run = record::read(&notes[1][1][..notes[1][1].len() - 1]).expect("run");
    assert_eq!((run.first, run.last, run.generation), (1, 3, Generation(1)));
}

#[test]
fn a_block_the_store_already_holds_is_removed_and_not_kept_twice() {
    for rewrite in [false, true] {
        let mut world = World::new();
        let kind = if rewrite { "rewrite" } else { "plain" };
        let parent = world.entry(kind, &["alpha", "beta", "gamma"]);
        if rewrite {
            world.notes(parent, 2, |_| {});
        }
        let mut script = Script::new().then(answer("r1-3\nThe user asked about a topic."));
        let plan = world.plan(Asked::Every);
        world.calls(&mut script, &plan);
        let files = world.spooled.files.clone();
        assert_eq!(world.replay().captured, 1);
        world.spooled.files = files;
        let again = world.replay();
        assert_eq!(again.captured, 0, "rewrite {rewrite}");
        assert!(again.notices.is_empty(), "rewrite {rewrite}");
        assert!(world.spooled.files.is_empty());
        assert_eq!(world.shelf.captures.len(), 1);
    }

    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta", "gamma"]);
    world.notes(parent, 2, |_| {});
    let mut script = Script::new().then(answer("r3\nThe user asked about a topic."));
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    let files = world.spooled.files.clone();
    world.replay();
    world.spooled.files = files;
    let again = world.replay();
    assert_eq!((again.captured, again.notices.len()), (0, 0));
    assert_eq!(world.shelf.captures.len(), 1);
}

#[test]
fn a_block_for_an_entry_that_moved_on_is_dropped_and_said() {
    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta", "gamma"]);
    let notes = world.notes(parent, 2, |_| {});
    let mut script = Script::new().then(answer("r3\nThe user asked about a topic."));
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    world.shelf.grow(notes, &["{\"other\":1}"]);
    let replayed = world.replay();
    assert_eq!(
        replayed.notices,
        [format!(
            "dropped a run kept for {parent}: the store holds other notes for it now"
        )]
    );
    assert_eq!(replayed.captured, 0);
    assert!(world.spooled.files.is_empty());
    assert!(world.shelf.captures.is_empty());

    world
        .spooled
        .files
        .push(("s9999".to_owned(), b"junk".to_vec()));
    let replayed = world.replay();
    assert_eq!(
        replayed.notices,
        ["dropped a spool file bilbo cannot read: s9999".to_owned()]
    );
    assert!(world.spooled.files.is_empty());
}

#[test]
fn a_note_entry_with_no_record_takes_the_block_kept_for_it() {
    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta"]);
    let mut script = Script::new().then(answer("r1-2\nThe user asked about a topic."));
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    assert!(plan::held(&world.shelf, parent).expect("held").is_none());
    let made = Id::from_bytes([9; 16]);
    world.shelf.put_notes(made, parent, Vec::new());
    assert!(plan::held(&world.shelf, parent).expect("held").is_none());
    let replayed = world.replay();
    let head = plan::held(&world.shelf, parent)
        .expect("held")
        .expect("a head");
    assert_eq!((head.id, head.records), (made, 2));
    assert_eq!(replayed.captured, 1);
    assert!(replayed.notices.is_empty());
    let notes = world.shelf.notes_of(parent).expect("notes");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].len(), 2);
    let owners = world
        .shelf
        .ids()
        .into_iter()
        .filter(|held| {
            world
                .shelf
                .meta(*held)
                .expect("meta")
                .and_then(|meta| meta.labels.capture)
                .is_some_and(|label| label.as_str() == format!("derive:{parent}"))
        })
        .count();
    assert_eq!(owners, 1);
    assert_eq!(spool::bare(b"a\n"), b"a");
}

#[test]
fn the_runs_counts_add_up() {
    let mut world = World::new();
    for _ in 0..4 {
        world.entry("plain", &["alpha", "beta"]);
    }
    let mut script = Script::new()
        .then(answer(
            "r1\nThe user asked.\n\nr2\nThe assistant did it.\n\nr9\nBeyond.",
        ))
        .then(refused(None))
        .then(Outcome::Failed {
            cause: Cause::new(What::ToolError),
            usage: usage(700, 90),
        })
        .then(answer(""));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    let counts = report.counts;
    assert_eq!(counts.entries, 4);
    assert_eq!(counts.calls, 4);
    assert_eq!(counts.notes, 2);
    assert_eq!(counts.dropped, 1);
    assert_eq!(counts.refused, 1);
    assert_eq!(counts.failed, 1);
    assert_eq!(counts.passed, 0);
    assert_eq!((counts.tokens_in, counts.tokens_out), (1000, 115));
    assert_eq!(report.ended, Ended::Done);

    let (mut world, _) = alone();
    let mut script = Script::new().then(busy(None)).then(good());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!((report.counts.entries, report.counts.calls), (1, 2));

    let mut world = World::new();
    world.section.calls.value = 1;
    world.entry("plain", &["alpha"]);
    world.entry("plain", &["alpha"]);
    let mut script = Script::new().then(good());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Calls);
    assert_eq!((report.counts.entries, report.counts.calls), (1, 1));
}

#[test]
fn the_report_lists_each_refusal_failure_and_pass_in_the_order_met() {
    let mut world = World::new();
    world.section.failures.value = 10;
    let made: Vec<Id> = (0..6).map(|_| world.entry("plain", &["alpha"])).collect();
    let first = world.extent_of(made[0], 1, 1);
    for _ in 0..3 {
        use bilbo_derive::tries::Tries;
        world.held.charge(&first).expect("charge");
    }
    let mut script = Script::new()
        .then(refused(Some("cyber")))
        .then(failed(What::Unreadable))
        .then(cut())
        .then(answer("r9\nBeyond the window."))
        .then(answer("r1\nThe user asked about a topic."));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    let at = |index: usize| world.extent_of(made[index], 1, 1);
    assert_eq!(
        report.events,
        [
            Event::Refused {
                extent: at(5),
                category: Some("cyber".to_owned())
            },
            Event::Failed {
                extent: at(4),
                cause: Cause::new(What::Unreadable)
            },
            Event::Failed {
                extent: at(3),
                cause: Cause::new(What::Cut)
            },
            Event::NoNote { extent: at(2) },
            Event::Passed { extent: at(0) },
        ]
    );
    assert_eq!(report.counts.entries, 5);
    assert_eq!(report.counts.passed, 1);
    assert_eq!(world.held.of(&at(3)), 1);
    assert_eq!(world.held.of(&at(2)), 1);
}

#[test]
fn an_unfinished_entry_that_did_not_change_is_finished_over_several_runs() {
    let mut world = World::new();
    let parent = world.entry("rewrite", &texts(2));
    world.section.window.value = 3;
    world.section.calls.value = 1;
    let mut script = Script::new()
        .then(answer(&para(1, 1, "The first window was about alpha.")))
        .then(answer(&para(2, 2, "The second window was about beta.")));
    let first = world.derive(&mut script);
    assert_eq!(first.report.counts.calls, 1);
    assert_eq!(first_record(&script.prompts[0]), 1);
    let second = world.derive(&mut script);
    assert_eq!(second.report.counts.calls, 1);
    assert_eq!(first_record(&script.prompts[1]), 2);
    assert!(
        script.prompts[1]
            .material
            .contains("The first window was about alpha.")
    );
    assert_eq!(world.shelf.notes_of(parent).expect("notes").len(), 1);
    let third = world.plan(Asked::Every);
    assert!(third.due.is_empty());
    assert!(third.states.contains(&(parent, State::Covered)));
    assert_eq!(script.prompts.len(), 2);
}

#[test]
fn an_unfinished_entry_that_changed_starts_again_from_its_first_record() {
    let mut world = World::new();
    let parent = world.entry("rewrite", &texts(2));
    world.section.window.value = 3;
    world.section.calls.value = 1;
    let mut script = Script::new()
        .then(answer(&para(1, 1, "The first window was about alpha.")))
        .then(answer(&para(
            1,
            1,
            "The entry was read again from the start.",
        )));
    world.derive(&mut script);
    world.shelf.grow(parent, &["cccc"]);
    world.derive(&mut script);
    assert_eq!(first_record(&script.prompts[1]), 1);
    assert!(
        !script.prompts[1]
            .material
            .contains("The first window was about alpha.")
    );
    assert_eq!(world.shelf.notes_of(parent).expect("notes").len(), 2);
}

#[test]
fn the_next_prompt_shows_an_earlier_paragraph_as_the_store_holds_it() {
    let blob = "y".repeat(300);
    let answers = || {
        [
            answer(&para(1, 1, &blob)),
            answer(&para(2, 2, "The user asked about a topic.")),
        ]
    };
    let (mut world, _) = windowed(2);
    world.section.names.value = 0.0;
    let [first, second] = answers();
    let mut script = Script::new().then(first).then(second);
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    let within = script.prompts[1].material.clone();

    let (mut world, _) = windowed(2);
    world.section.names.value = 0.0;
    world.section.calls.value = 1;
    let [first, second] = answers();
    let mut script = Script::new().then(first).then(second);
    world.derive(&mut script);
    world.derive(&mut script);
    let after = script.prompts[1].material.clone();
    assert_eq!(within, after);
    assert!(within.contains("[opaque: 300 chars, xxh3 "), "{within}");
    assert!(!within.contains(&blob));
}

#[test]
fn a_paragraph_about_records_before_its_window_takes_their_day() {
    let (mut world, _) = windowed(2);
    let mut script = Script::new()
        .then(answer(&para(1, 1, "The first window was about alpha.")))
        .then(answer(&para(1, 2, "A claim about the first record.")));
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    let blocks = world.spooled.blocks();
    let paragraph = read_paragraph(&blocks[1].records[0]).expect("a paragraph");
    assert_eq!(paragraph.text, "A claim about the first record.");
    assert_eq!(paragraph.at, Some(at(EPOCH_AT + 1)));
}

#[test]
fn a_paragraph_with_a_quote_a_backslash_and_a_line_feed_is_kept_as_it_was_written() {
    let mut world = World::new();
    world.section.names.value = 0.0;
    let parent = world.entry("plain", &["alpha"]);
    let written = "He said \"hi\" in C:\\dir\nand left.";
    let mut script = Script::new().then(answer(&para(1, 1, written)));
    let done = world.derive(&mut script);
    assert_eq!(done.replayed.captured, 1);
    assert!(done.replayed.notices.is_empty());
    let notes = world.shelf.notes_of(parent).expect("notes");
    let stored = &notes[0][0];
    let line = &stored[..stored.len() - 1];
    assert!(!line.contains(&b'\n'));
    let paragraph = read_paragraph(line).expect("a paragraph");
    assert_eq!(paragraph.text, written);
}

fn timeout() -> Outcome {
    Outcome::Busy {
        wait: None,
        cause: Cause::new(What::Timeout),
    }
}

#[test]
fn a_failure_of_the_windows_own_is_charged_at_once_and_the_entries_behind_it_are_called() {
    let mut world = World::new();
    world.section.failures.value = 1;
    let made: Vec<Id> = (0..4).map(|_| world.entry("plain", &["alpha"])).collect();
    let mut script = Script::new()
        .then(answer("I could not write notes."))
        .then(answer("I could not write notes."))
        .then(no_note())
        .then(answer("r1\nThe user asked about a topic."));
    let first = world.derive(&mut script);
    assert_eq!(first.report.ended, Ended::Done);
    assert_eq!(script.prompts.len(), 4);
    assert_eq!(first.report.counts.failed, 3);
    for parent in &made[1..] {
        assert_eq!(world.held.of(&world.extent_of(*parent, 1, 1)), 1);
    }
    assert_eq!(world.shelf.captures.len(), 1);
    for _ in 0..2 {
        let mut again = Script::new()
            .then(answer("I could not write notes."))
            .then(answer("I could not write notes."))
            .then(no_note());
        world.derive(&mut again);
    }
    for parent in &made[1..] {
        assert_eq!(world.held.of(&world.extent_of(*parent, 1, 1)), 3);
    }
    let plan = world.plan(Asked::Every);
    assert_eq!(plan.due.len(), 3);
    assert!(
        plan.due
            .iter()
            .all(|due| due.windows.iter().all(|w| w.pass))
    );
    let mut none = Script::new();
    let report = world.calls(&mut none, &plan);
    assert!(none.prompts.is_empty());
    assert_eq!(report.counts.passed, 3);
}

#[test]
fn a_failure_that_may_be_the_providers_is_never_charged_in_a_run_with_no_answer() {
    let mut world = World::new();
    let parent = world.entry("plain", &["alpha"]);
    for _ in 0..3 {
        let mut script = Script::new().then(failed(What::ToolError));
        let plan = world.plan(Asked::Every);
        let report = world.calls(&mut script, &plan);
        assert_eq!(report.ended, Ended::Done);
        assert_eq!(report.counts.failed, 1);
        assert!(world.held.charges.is_empty());
    }
    let plan = world.plan(Asked::Every);
    assert!(!plan.due[0].windows[0].pass);
    let mut script = Script::new().then(answer("r1\nThe user asked about a topic."));
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.counts.calls, 1);
    assert_eq!(world.spooled.blocks().len(), 1);
    assert_eq!(world.held.of(&world.extent_of(parent, 1, 1)), 0);
}

#[test]
fn a_call_that_times_out_ends_its_entry_and_the_older_entries_are_called() {
    let mut world = World::new();
    world.section.retries.value = 2;
    let older = world.entry("plain", &["alpha"]);
    let newer = world.entry("plain", &["alpha"]);
    let mut script = Script::new()
        .then(timeout())
        .then(timeout())
        .then(timeout())
        .then(answer("r1\nThe user asked about a topic."));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Done);
    assert_eq!(script.prompts.len(), 4);
    assert_eq!(
        report.events,
        [Event::Failed {
            extent: world.extent_of(newer, 1, 1),
            cause: Cause::new(What::Timeout)
        }]
    );
    assert_eq!(world.held.of(&world.extent_of(newer, 1, 1)), 1);
    assert_eq!(world.spooled.blocks().len(), 1);
    world.section.tries.value = 2;
    let mut again = Script::new()
        .then(timeout())
        .then(timeout())
        .then(timeout())
        .then(answer("r1\nThe user asked about a topic."));
    let plan = world.plan(Asked::Every);
    world.calls(&mut again, &plan);
    assert_eq!(world.held.of(&world.extent_of(newer, 1, 1)), 2);
    let plan = world.plan(Asked::Every);
    let due_newer = plan
        .due
        .iter()
        .find(|due| due.parent.id == newer)
        .expect("the newer entry is due");
    assert!(due_newer.windows[0].pass);
    assert!(plan.due.iter().any(|due| due.parent.id == older));
}

#[test]
fn a_failure_that_may_be_either_is_charged_only_on_proof() {
    let good_answer = || answer("r1\nThe user asked about a topic.");
    let mut world = World::new();
    world.section.retries.value = 0;
    let a: Vec<Id> = (0..3).map(|_| world.entry("plain", &["alpha"])).collect();
    let mut script = Script::new()
        .then(failed(What::Unreadable))
        .then(good_answer())
        .then(busy(None));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert!(matches!(report.ended, Ended::Busy(..)));
    assert_eq!(world.held.of(&world.extent_of(a[2], 1, 1)), 1);

    let mut world = World::new();
    let b: Vec<Id> = (0..2).map(|_| world.entry("plain", &["alpha"])).collect();
    let mut script = Script::new()
        .then(good_answer())
        .then(failed(What::Unreadable));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Done);
    assert_eq!(world.held.of(&world.extent_of(b[0], 1, 1)), 1);

    let mut world = World::new();
    world.section.retries.value = 0;
    let c: Vec<Id> = (0..3).map(|_| world.entry("plain", &["alpha"])).collect();
    let mut script = Script::new()
        .then(good_answer())
        .then(failed(What::Unreadable))
        .then(busy(None));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert!(matches!(report.ended, Ended::Busy(..)));
    assert_eq!(world.held.of(&world.extent_of(c[1], 1, 1)), 0);
}

#[test]
fn a_replay_cut_between_two_removes_removes_the_rest_and_says_nothing() {
    let (mut world, parent) = windowed(2);
    let mut script = Script::new()
        .then(answer(&para(1, 1, "The first window was about alpha.")))
        .then(answer(&para(2, 2, "The second window was about beta.")));
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    let files = world.spooled.files.clone();
    assert_eq!(files.len(), 2);
    assert_eq!(world.replay().captured, 1);
    let made = world.shelf.notes_of(parent).expect("notes").clone();
    world.spooled.files = vec![files[1].clone()];
    let again = world.replay();
    assert_eq!(again.captured, 0);
    assert!(again.notices.is_empty(), "{:?}", again.notices);
    assert!(world.spooled.files.is_empty());
    assert_eq!(world.shelf.notes_of(parent).expect("notes"), &made);
}

#[test]
fn a_spool_file_bilbo_cannot_read_stays_when_the_replay_fails() {
    let (mut world, _) = alone();
    let mut script = Script::new().then(good());
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    world
        .spooled
        .files
        .push(("s9999".to_owned(), b"not a block".to_vec()));
    world.shelf.refuse_capture = true;
    let format = crate::fakes::format();
    let failed = bilbo_derive::run::replay(&mut world.shelf, &mut world.spooled, &format);
    assert!(failed.is_err());
    assert!(world.spooled.files.iter().any(|(name, _)| name == "s9999"));
    world.shelf.refuse_capture = false;
    let done = world.replay();
    assert_eq!(
        done.notices,
        ["dropped a spool file bilbo cannot read: s9999"]
    );
    assert!(world.spooled.files.is_empty());
}

#[test]
fn an_earlier_note_copied_back_is_not_kept_again() {
    let mut world = World::new();
    let parent = world.entry("plain", &texts(3));
    world.notes(parent, 2, |_| {});
    let mut script = Script::new().then(answer(&para(1, 2, "Earlier paragraph.")));
    let plan = world.plan(Asked::Every);
    assert_eq!(plan.due[0].windows[0].window.first, 3);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.counts.failed, 1);
    assert_eq!(report.counts.dropped, 1);
    assert!(world.spooled.files.is_empty());
    assert_eq!(field(&world.lines.0[0], 18), "no-note");
}

#[test]
fn a_made_up_path_of_plain_words_is_dropped() {
    let mut world = World::new();
    world.section.names.value = 1.0;
    world.entry("plain", &["alpha"]);
    let mut script = Script::new().then(answer(&para(
        1,
        1,
        "The assistant edited config/settings for the user.",
    )));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.counts.failed, 1);
    assert_eq!(report.counts.dropped, 1);
    assert_eq!(field(&world.lines.0[0], 18), "no-note");
    assert!(world.spooled.files.is_empty());
}

#[test]
fn a_note_of_plain_prose_is_kept_whatever_its_compounds() {
    let mut world = World::new();
    world.section.names.value = 0.5;
    world.entry("plain", &["The file src/a.rs changed."]);
    let mut script = Script::new().then(answer(&para(
        1,
        1,
        "The assistant fixed src/a.rs, e.g. the follow-up read-only check.",
    )));
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.counts.notes, 1);
    assert_eq!(report.counts.dropped, 0);
}

#[test]
fn a_second_note_under_a_range_the_grammar_cannot_read_is_not_kept_under_the_first() {
    let mut world = World::new();
    world.section.names.value = 0.0;
    let parent = world.entry("plain", &["alpha", "beta"]);
    let answered =
        "r1\nThe user asked about alpha.\n\nr2: The user asked about beta.\nMore about beta.";
    let mut script = Script::new().then(answer(answered));
    let done = world.derive(&mut script);
    assert_eq!(done.report.counts.notes, 1);
    let notes = world.shelf.notes_of(parent).expect("notes");
    let paragraph = read_paragraph(&notes[0][0][..notes[0][0].len() - 1]).expect("a paragraph");
    assert_eq!(paragraph.text, "The user asked about alpha.");
}

fn first_parent(plan: &bilbo_derive::plan::Plan) -> Id {
    plan.due[0].parent.id
}

fn noted(world: &crate::fakes::World, ids: &[Id]) -> Vec<Id> {
    ids.iter()
        .copied()
        .filter(|id| {
            world
                .shelf
                .notes_of(*id)
                .is_some_and(|held| !held.is_empty())
        })
        .collect()
}

#[test]
fn at_one_call_a_run_an_entry_that_would_answer_is_called_in_the_second() {
    let mut world = World::new();
    world.section.calls.value = 1;
    let older = world.entry("plain", &["beta"]);
    let newest = world.entry("plain", &["alpha"]);
    let mut script = Script::new().then(failed(What::ToolError));
    let first = world.derive(&mut script);
    assert_eq!(
        (first.report.ended, script.prompts.len()),
        (Ended::Calls, 1)
    );
    assert_eq!(world.held.mark, Some(newest));
    assert!(world.held.charges.is_empty());
    let mut script = Script::new().then(answer("r1\nThe user asked about a topic."));
    world.derive(&mut script);
    assert_eq!(noted(&world, &[older, newest]), [older]);
    assert_eq!(world.held.mark, None);
}

#[test]
fn an_entry_that_takes_every_call_does_not_keep_the_next_from_its_call() {
    let mut world = World::new();
    world.section.calls.value = 3;
    world.section.retries.value = 2;
    let older = world.entry("plain", &["beta"]);
    let newest = world.entry("plain", &["alpha"]);
    let mut script = Script::new()
        .then(timeout())
        .then(timeout())
        .then(timeout());
    world.derive(&mut script);
    assert_eq!(script.prompts.len(), 3);
    assert_eq!(world.held.mark, Some(newest));
    let mut script = Script::new()
        .then(answer("r1\nThe user asked about a topic."))
        .then(timeout())
        .then(timeout());
    let done = world.derive(&mut script);
    assert_eq!(done.report.ended, Ended::Calls);
    assert_eq!(noted(&world, &[older, newest]), [older]);
    assert_eq!(world.held.of(&world.extent_of(newest, 1, 1)), 0);
    assert_eq!(world.held.mark, None);
}

#[test]
fn three_failing_entries_ahead_do_not_keep_the_fourth_from_its_call() {
    let mut world = World::new();
    let oldest = world.entry("plain", &["eeee"]);
    let third = world.entry("plain", &["ffff three"]);
    let second = world.entry("plain", &["ffff two"]);
    let first = world.entry("plain", &["ffff one"]);
    let mut script = Script::new()
        .then(failed(What::ToolError))
        .then(failed(What::ToolError))
        .then(failed(What::ToolError));
    let one = world.derive(&mut script);
    assert_eq!(one.report.ended, Ended::Failures);
    assert!(world.held.charges.is_empty());
    assert_eq!(world.held.mark, Some(first));
    let mut script = Script::new()
        .then(failed(What::ToolError))
        .then(failed(What::ToolError))
        .then(answer("r1\nThe user asked about a topic."))
        .then(failed(What::ToolError));
    let two = world.derive(&mut script);
    assert_eq!(two.report.ended, Ended::Done);
    let said = |at: usize, text: &str| script.prompts[at].material.contains(text);
    assert!(said(0, "ffff two") && said(1, "ffff three") && said(2, "eeee") && said(3, "ffff one"));
    for parent in [first, second, third] {
        assert_eq!(world.held.of(&world.extent_of(parent, 1, 1)), 1);
    }
    assert_eq!(noted(&world, &[oldest, first, second, third]), [oldest]);
    assert_eq!(world.held.mark, None);
    world.section.tries.value = 1;
    let plan = world.plan(Asked::Every);
    assert_eq!(plan.due.len(), 3);
    assert!(
        plan.due
            .iter()
            .all(|due| due.windows.iter().all(|w| w.pass))
    );
}

#[test]
fn a_run_the_provider_ends_begins_the_next_one_entry_further() {
    let mut world = World::new();
    let oldest = world.entry("plain", &["gamma"]);
    let older = world.entry("plain", &["beta"]);
    let newest = world.entry("plain", &["alpha"]);
    let mut script = Script::new().then(stopped());
    let done = world.derive(&mut script);
    assert!(matches!(done.report.ended, Ended::Stopped(_)));
    assert_eq!(world.held.mark, Some(newest));
    let mut script = Script::new().then(stopped());
    world.derive(&mut script);
    assert_eq!(world.held.mark, Some(older));
    let plan = world.plan(Asked::Every);
    assert_eq!(first_parent(&plan), oldest);
}

#[test]
fn a_named_run_and_a_redo_are_turned_by_the_same_mark() {
    let mut world = World::new();
    world.section.calls.value = 1;
    let a = world.entry("plain", &["aaaa"]);
    let b = world.entry("plain", &["bbbb"]);
    let c = world.entry("plain", &["cccc"]);
    let named = [a, b, c];
    let mut script = Script::new().then(failed(What::ToolError));
    let plan = world.plan(Asked::Only(&named));
    assert_eq!(first_parent(&plan), c);
    world.calls(&mut script, &plan);
    assert_eq!(world.held.mark, Some(c));
    let plan = world.plan(Asked::Only(&named));
    assert_eq!(first_parent(&plan), b);
    let mut script = Script::new().then(answer("r1\nThe user asked about a topic."));
    world.calls(&mut script, &plan);
    assert_eq!(world.held.mark, None);

    let mut world = World::new();
    world.redo = true;
    let ids: Vec<Id> = (0..3).map(|_| world.entry("plain", &["alpha"])).collect();
    for id in &ids {
        world.notes(*id, 2, |run| run.passes = 1);
    }
    assert_eq!(first_parent(&world.plan(Asked::Every)), ids[2]);
    world.held.mark = Some(ids[1]);
    assert_eq!(first_parent(&world.plan(Asked::Every)), ids[0]);
}

#[test]
fn the_runs_totals_do_not_overflow() {
    let mut world = World::new();
    world.entry("plain", &["alpha"]);
    world.entry("plain", &["beta"]);
    let huge = || bilbo_llm::model::Outcome::Answer {
        text: "r1\nThe user asked about a topic.".to_owned(),
        usage: crate::fakes::usage(u64::MAX, u64::MAX),
        model: None,
    };
    let mut script = Script::new().then(huge()).then(huge());
    let done = world.derive(&mut script);
    assert_eq!(done.report.counts.calls, 2);
    assert_eq!(done.report.counts.tokens_in, u64::MAX);
    assert_eq!(done.report.counts.tokens_out, u64::MAX);
}

#[test]
fn a_replay_that_removed_a_file_and_then_fails_still_says_so() {
    let mut world = World::new();
    let parent = world.entry("plain", &["alpha", "beta", "gamma"]);
    let notes = world.notes(parent, 2, |_| {});
    let mut script = Script::new().then(answer("r3\nThe user asked about a topic."));
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    world.shelf.grow(notes, &["{\"other\":1}"]);
    world
        .spooled
        .files
        .push(("zzzz".to_owned(), b"not a block".to_vec()));
    world.spooled.fail_removal = Some(2);
    let format = crate::fakes::format();
    let error = bilbo_derive::run::replay(&mut world.shelf, &mut world.spooled, &format)
        .expect_err("a removal that fails");
    assert!(
        error.to_string().contains("a removal that fails"),
        "{error}"
    );
    assert_eq!(
        error.notices,
        [format!(
            "dropped a run kept for {parent}: the store holds other notes for it now"
        )]
    );
}

#[test]
fn a_setting_that_cuts_every_answer_charges_nothing() {
    for outcome in [(|| cut()) as fn() -> Outcome, || failed(What::StopReason)] {
        let mut world = World::new();
        for _ in 0..4 {
            world.entry("plain", &["alpha"]);
        }
        let mut script = Script::new().always(move |_, _| outcome());
        for _ in 0..world.section.tries.value + 1 {
            let done = world.derive(&mut script);
            assert_eq!(done.report.ended, Ended::Failures);
            assert_eq!(done.report.counts.passed, 0);
        }
        assert!(world.held.charges.is_empty());
        let plan = world.plan(Asked::Every);
        assert!(
            plan.due
                .iter()
                .all(|due| due.windows.iter().all(|w| !w.pass))
        );
    }
}

#[test]
fn a_cut_answer_waits_for_another_calls_answer_and_proves_nothing() {
    let mut world = World::new();
    let cut_one = world.entry("plain", &["alpha"]);
    let kept = world.entry("plain", &["alpha"]);
    let mut script = Script::new().then(good()).then(cut());
    world.derive(&mut script);
    assert_eq!(world.held.of(&world.extent_of(cut_one, 1, 1)), 1);
    assert_eq!(world.held.of(&world.extent_of(kept, 1, 1)), 0);

    let mut world = World::new();
    world.entry("plain", &["alpha"]);
    world.entry("plain", &["alpha"]);
    let mut script = Script::new().then(failed(What::Timeout)).then(cut());
    world.derive(&mut script);
    assert!(world.held.charges.is_empty());
}

#[test]
fn a_run_of_passes_only_asks_the_writer_nothing_and_writes_no_model() {
    let mut world = World::new();
    let parent = world.entry("plain", &["alpha"]);
    let extent = world.extent_of(parent, 1, 1);
    for _ in 0..world.section.tries.value {
        use bilbo_derive::tries::Tries;
        world.held.charge(&extent).expect("charge");
    }
    let key = "sk-secret-key-text";
    let mut script = Script::new();
    script.model = key.to_owned();
    let done = world.derive(&mut script);
    assert_eq!((done.report.counts.passed, script.prepared), (1, 0));
    assert_eq!(world.shelf.captures.len(), 1);
    let held = world
        .shelf
        .notes_of(parent)
        .expect("notes")
        .iter()
        .flatten()
        .fold(Vec::new(), |mut all, record: &Vec<u8>| {
            all.extend_from_slice(record);
            all
        });
    let held = String::from_utf8_lossy(&held).into_owned();
    assert!(held.contains("\"model\":\"\""), "{held}");
    assert!(!held.contains(key));
    assert_eq!(field(&world.lines.0[0], 2), "passed");
    assert_eq!(field(&world.lines.0[0], 7), "-");
    assert!(world.lines.0.iter().all(|line| !line.contains(key)));
}

#[test]
fn a_window_only_the_provider_refuses_does_not_keep_the_others_from_their_call() {
    let mut world = World::new();
    world.section.retries.value = 0;
    world.section.calls.value = 1;
    let older = world.entry("plain", &["beta"]);
    let newest = world.entry("plain", &["alpha"]);
    let mut script = Script::new().then(busy(None));
    let first = world.derive(&mut script);
    assert!(matches!(first.report.ended, Ended::Busy(..)));
    assert_eq!(world.held.mark, Some(newest));
    let mut script = Script::new().then(good());
    world.derive(&mut script);
    assert_eq!(noted(&world, &[older, newest]), [older]);
}

#[test]
fn a_request_the_provider_rejects_ends_its_entry_and_waits_for_its_proof() {
    let mut world = World::new();
    let lower = world.entry("plain", &["alpha"]);
    let higher = world.entry("plain", &["alpha"]);
    let rejected = || Outcome::Stopped(Cause::new(What::Rejected));
    let mut script = Script::new().then(rejected()).then(good());
    let done = world.derive(&mut script);
    assert_eq!(done.report.ended, Ended::Done);
    assert_eq!(noted(&world, &[lower, higher]), [lower]);
    assert_eq!(world.held.of(&world.extent_of(higher, 1, 1)), 1);

    let mut world = World::new();
    for _ in 0..3 {
        world.entry("plain", &["alpha"]);
    }
    let mut script = Script::new().always(move |_, _| rejected());
    let done = world.derive(&mut script);
    assert_eq!(done.report.ended, Ended::Failures);
    assert!(world.held.charges.is_empty());
}

#[test]
fn a_failure_that_waits_is_charged_at_a_run_that_ends_by_its_calls() {
    let mut world = World::new();
    world.section.calls.value = 2;
    world.entry("plain", &["alpha"]);
    let waiting = world.entry("plain", &["alpha"]);
    world.entry("plain", &["alpha"]);
    let mut script = Script::new().then(good()).then(failed(What::ToolError));
    let done = world.derive(&mut script);
    assert_eq!(done.report.ended, Ended::Calls);
    assert_eq!(world.held.charges.len(), 1);
    assert_eq!(world.held.of(&world.extent_of(waiting, 1, 1)), 1);

    let mut world = World::new();
    world.section.calls.value = 1;
    world.entry("plain", &["alpha"]);
    world.entry("plain", &["alpha"]);
    let mut script = Script::new().then(failed(What::ToolError));
    world.derive(&mut script);
    assert!(world.held.charges.is_empty());
}

#[test]
fn the_calls_end_the_run_before_a_busy_windows_retries_are_used_up() {
    let mut world = World::new();
    world.section.calls.value = 1;
    world.section.retries.value = 3;
    world.entry("plain", &["alpha"]);
    let newest = world.entry("plain", &["alpha"]);
    let mut script = Script::new().then(timeout());
    let done = world.derive(&mut script);
    assert_eq!(done.report.ended, Ended::Calls);
    assert!(done.report.events.is_empty());
    assert!(world.held.charges.is_empty());
    assert_eq!(world.held.mark, Some(newest));
}

#[test]
fn a_name_the_record_wrote_with_a_tag_of_the_frame_is_found() {
    use crate::fakes::Plain;
    let named = "Vec<records::Entry>";
    let answer_for = |first: u64| {
        answer(&format!(
            "r{first}\nThe user asked about the type {named} in the module."
        ))
    };
    let setup = |preface: Option<&'static str>, earlier: &str, record: &str| {
        let mut world = World::new();
        world.section.names.value = 1.0;
        world.readers[3] = Box::new(Plain {
            kind: "headed",
            appends_only: true,
            silent: "",
            preface,
        });
        let parent = world.entry("headed", &["start", record, "end"]);
        world.notes_saying(parent, 1, earlier, |_| {});
        (world, parent)
    };

    let (mut world, parent) = setup(
        None,
        "Earlier words here.",
        "the field holds Vec<records::Entry>",
    );
    let mut script = Script::new().then(answer_for(2));
    let done = world.derive(&mut script);
    assert!(
        script.prompts[0]
            .material
            .contains("Vec&lt;records::Entry>")
    );
    assert_eq!(
        (done.report.counts.notes, done.report.counts.dropped),
        (1, 0)
    );
    assert!(world.shelf.notes_of(parent).is_some());

    let (mut world, _) = setup(
        None,
        "Earlier note names Vec<records::Entry>.",
        "plain words",
    );
    let mut script = Script::new().then(answer_for(2));
    let done = world.derive(&mut script);
    assert!(
        script.prompts[0]
            .material
            .contains("Vec&lt;records::Entry>")
    );
    assert_eq!(
        (done.report.counts.notes, done.report.counts.dropped),
        (1, 0)
    );

    let (mut world, _) = setup(
        Some("The header names Vec<records::Entry>."),
        "Earlier words here.",
        "plain words",
    );
    let mut script = Script::new().then(answer_for(2));
    let done = world.derive(&mut script);
    assert!(
        script.prompts[0]
            .material
            .contains("Vec&lt;records::Entry>")
    );
    assert_eq!(
        (done.report.counts.notes, done.report.counts.dropped),
        (1, 0)
    );

    let (mut world, _) = setup(None, "Earlier words here.", "plain words");
    let mut script = Script::new().then(answer_for(2));
    let done = world.derive(&mut script);
    assert_eq!(
        (done.report.counts.notes, done.report.counts.dropped),
        (0, 1)
    );
}

#[test]
fn every_attempt_of_a_window_is_handed_the_same_limits_a_retry_included() {
    let (mut world, _) = alone();
    world.limits = Limits {
        answer_tokens: 16_001,
        timeout: seconds(301),
        response: (4 << 20) + 1,
        poll: Duration::from_millis(6),
    };
    let mut script = Script::new()
        .then(busy(Some(seconds(1))))
        .then(busy(Some(seconds(1))))
        .then(good());
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    assert_eq!(script.limits, [world.limits; 3]);
}

#[test]
fn a_retry_sends_the_prompt_the_first_attempt_sent() {
    let (mut world, parent) = windowed(3);
    world.notes_saying(parent, 1, "Stored words here.", |_| {});
    let mut script = Script::new()
        .then(busy(Some(seconds(1))))
        .then(busy(Some(seconds(1))))
        .then(answer(&para(2, 2, "Written words here.")))
        .then(answer(&para(3, 3, "Final words here.")));
    let plan = world.plan(Asked::Every);
    world.calls(&mut script, &plan);
    assert_eq!(script.prompts.len(), 4);
    assert_eq!(script.prompts[0], script.prompts[1]);
    assert_eq!(script.prompts[0], script.prompts[2]);
    assert!(script.prompts[0].material.contains("Stored words here."));
}

#[test]
fn a_run_out_of_calls_does_not_wait_for_a_retry_it_cannot_make() {
    let (mut world, _) = alone();
    world.section.calls.value = 1;
    let mut script = Script::new().then(busy(None)).then(good());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Calls);
    assert_eq!(script.prompts.len(), 1);
    assert!(world.watch.slept.borrow().is_empty());
}

#[test]
fn a_failure_that_waits_is_not_charged_when_the_provider_stops_the_run() {
    let mut world = World::new();
    world.section.tries.value = 1;
    world.entry("plain", &["alpha"]);
    let waiting = world.entry("plain", &["alpha"]);
    world.entry("plain", &["alpha"]);
    let mut script = Script::new()
        .then(good())
        .then(failed(What::ToolError))
        .then(stopped());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Stopped(Cause::new(What::Unauthorized)));
    assert_eq!(script.prompts.len(), 3);
    assert!(world.held.charges.is_empty());
    assert_eq!(world.held.of(&world.extent_of(waiting, 1, 1)), 0);

    let mut script = Script::new().always(|_, _| good());
    let plan = world.plan(Asked::Every);
    let report = world.calls(&mut script, &plan);
    assert_eq!(report.ended, Ended::Done);
    assert_eq!(script.prompts.len(), 3);
}
