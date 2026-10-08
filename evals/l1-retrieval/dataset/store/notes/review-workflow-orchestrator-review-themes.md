---
id: 01KR2V5YFYKX8894XRDQ8PZPGK
created: 2026-05-08T00:47-03:00
---

# workflow-orchestrator review themes

Notes on what keeps coming up when people review `workflow-orchestrator`. This is a general read of the pattern, not a log of any one review. Details live in the individual review threads and the code. The component drives the transcode and packaging pipeline on AWS Step Functions, hands work to FFmpeg-based workers written in Rust, and moves artifacts through S3 toward HLS output.

## Why this note exists

Reviewers repeat themselves. The same five or six concerns show up on most changes to `workflow-orchestrator`. Writing them down once saves the next reviewer, human or agent, from rediscovering them. Read it before opening a diff, then skim the diff with these in mind.

## State machine shape

Reviews often start with the shape of the state machine. People ask whether a new state is really needed or whether an existing one can carry the work. Large flat definitions get pushback because they are hard to read and hard to diff. Nested or split definitions get pushback too when the boundary between them is arbitrary. The usual outcome is a smaller change that reuses what is there.

## Retries and backoff

The most common comment is about retry policy. Reviewers want to know which failures are retried, which are not, and why. Blanket retries on every error are disliked because they hide real bugs and burn compute on a transcode that will never succeed. The reverse also happens: a transient storage or throttling error that fails the whole run with no retry. Expect to be asked to justify each retry block in plain words.

## Idempotency

Steps can run twice. Step Functions can redeliver, workers can be restarted, and people re-run executions by hand. Reviewers look for steps that would double-write, double-publish, or leave a half-built ladder behind. The question is always the same: what happens if this runs again with the same input?

## Failure classification

Reviewers like a clear split between input problems (bad or odd source media), infrastructure problems (storage, throttling, timeouts), and bugs in our own code. Catch blocks that treat all of these the same get flagged. Error names passed between states should be stable, since downstream states and alerts depend on them.

## Timeouts and heartbeats

Long encodes make timeouts a real concern. Reviewers check that every task state has a timeout and that long-running work reports liveness somehow, so a stuck worker is noticed before the overall limit is hit. A missing timeout is treated as a defect, not a style point.

## Fan-out and concurrency

Ladder renditions are encoded in parallel, so fan-out limits come up. Reviewers ask what happens when a large upload produces many parallel branches, and whether one slow rendition blocks the rest. Partial success handling is a recurring topic: should a ladder with a missing rung still be packaged, or should the run fail? The team has not always agreed, so check current practice before assuming.

## S3 layout and handoff

Artifacts move between steps through S3 rather than through state payloads. Reviewers watch for payloads growing toward service limits and for steps that pass large blobs inline. They also check that key naming is consistent and that intermediate objects are cleaned up or covered by lifecycle rules. Cleanup after failure is a frequent gap.

## Packaging and HLS output

Changes that touch playlist or segment generation get extra scrutiny because broken output is visible to viewers. Reviewers look at ordering of the publish step, so a manifest never points at segments that are not yet there. They also ask whether a change alters output for already-published content.

## Observability

Reviewers ask for enough logging and metrics to tell which execution, which asset and which step something belongs to. Correlation across the orchestrator and the workers matters more than volume. Noisy logs are as unwelcome as silent failures.

## Cost awareness

Because the component starts many billable tasks, reviewers watch for changes that multiply work: extra state transitions in hot paths, retries that re-encode everything, or polling where a callback would do. Media operations teams at small publishers notice cost, so this gets raised often.

## Testing expectations

Reviewers want some way to exercise the workflow definition without running a full encode: unit tests on the Rust side, and some check of the definition itself. Gaps here are the usual reason a change is sent back. Tests that only cover the happy path are called out.

## Rollout and compatibility

Executions can be in flight while a new definition is deployed. Reviewers ask whether old and new shapes can coexist, and whether a change to input or output format breaks running work. Breaking changes are expected to come with a plan, not a surprise.

## Naming and readability

Small but constant: state names, error names and variable names should say what they do. Reviewers push back on clever abbreviations. Comments are wanted where the reason for an odd choice is not obvious from the code.

## Config and permissions

Role and policy changes get read closely. The standing preference is the narrowest access that works, and reviewers question wildcards. Configuration that differs between environments should be explicit, not inferred.

## A typical review checklist

A shorthand that fits what reviewers actually do:

```text
workflow-orchestrator review:
  retries justified
  rerun is safe
  timeouts present
  errors classified
  cleanup on failure
  in-flight runs unaffected
```

## Open questions

- Whether partial ladders should ever be published is not settled in a way everyone accepts.
- Nobody owns a shared standard for error names yet; reviews fill the gap case by case.
- Testing the state machine definition is still more manual than people want.

## How to use this

Treat these as prompts, not rules. When a diff touches one of these areas, check the matching section and look at current code before commenting. If a theme changes, edit this note instead of adding another.
