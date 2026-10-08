---
id: 01KQ8CQKHD7JB8C54J1XZ18CS2
created: 2026-04-27T18:15-03:00
---

# scan-scheduler: expected behavior

This is what people expect from `scan-scheduler` in AuditMesh, written in general terms. It is not a contract with values in it. Where a real value matters (an interval, a limit, a table name), look in the code or config, not here. If this note and the code disagree, the code wins, and someone should fix whichever one is wrong.

`scan-scheduler` decides when scans of cloud infrastructure configs happen, and which targets each scan covers. It does not evaluate policy and it does not file tickets. It starts work and keeps track of it. The evaluation belongs to the Open Policy Agent side, and ticket creation belongs to the Jira side. People tend to push logic into the scheduler because it is the first thing that runs. Resist that.

## Role and boundaries

The scheduler sits at the front of the pipeline. Cloud security teams register what they want scanned: accounts, regions, resource families, and the policy bundles that apply. The scheduler turns that registration into units of work and hands them to the scanning workers. When a unit finishes, it records the outcome so the next run knows where things stand.

Things it is expected to do:

- Hold the schedule for each registered target and work out which targets are due.
- Split due targets into units small enough that a single worker invocation can finish them.
- Dispatch those units to workers running on AWS Lambda.
- Record dispatch, progress and completion in DynamoDB so state survives any single invocation.
- Notice work that never came back and deal with it.
- Leave a trail someone can read afterwards to answer "why did this target get scanned then, and not earlier or later?"

Things it is expected not to do:

- Interpret policy results. It may see that a scan succeeded or failed, but it does not decide what a violation means.
- Create, update or close Jira tickets. If it has to talk about tickets at all, it only passes along a flag or a reference the ticket side asked for.
- Hold long-lived in-memory state. Every invocation should be able to start cold and still do the right thing from what is in the store.
- Quietly change a team's policy selection. It runs what was registered.

The rough flow, using only the pieces we already have:

```
scan-scheduler -> AWS Lambda workers -> Open Policy Agent -> DynamoDB -> Jira
```

The scheduler starts the chain and reads from the store at the end of it. It is not a step in the middle of policy evaluation.

## Scheduling behavior

### What "due" means

A target is due when its own cadence says so, measured from the last completed scan, not from the last attempted one. This matters. If a scan was dispatched and died, the target is still overdue, and the scheduler should treat it that way. Basing the next run on the last attempt would hide failures by pushing the target further out each time something broke.

Different targets can have different cadences. A production account with sensitive resources usually wants more frequent scans than a sandbox. The scheduler should not assume one global rhythm. It should also not assume that every target that shares a cadence should fire at the same moment, because that produces a burst of load on the cloud provider APIs and on the workers.

### Spreading load

When many targets become due close together, the scheduler spreads them out rather than firing them all at once. The aim is a steady flow of work. Some jitter or staggering is fine and expected. People reading the logs should not be surprised that two targets with the same cadence ran at slightly different times. What matters is that no target drifts so far that it misses its window without anyone noticing.

There is a real tension here. Spreading load helps with throttling from the cloud provider and with Lambda concurrency, but it also means a scan can start later than the nominal time. The expectation is that the delay is small compared to the cadence, and that it is visible in the record of the run.

### On-demand scans

Teams sometimes ask for a scan outside the normal rhythm: after a config change, after an incident, before an audit. The scheduler accepts those requests and treats them as work like any other. Two rules apply:

- An on-demand request does not reset or cancel the regular cadence unless the request completes and records a completed scan, in which case "last completed" naturally moves forward.
- If a scan for the same target is already in flight, a second request should not start a parallel scan of the same thing. It should either attach to the running one or be rejected with a clear reason. Silent duplication is the thing to avoid.

### Overlap and exclusivity

For a given target and policy selection, at most one scan should be running at a time. The scheduler enforces this with a marker in DynamoDB written before dispatch, using a conditional write so that two scheduler invocations racing each other cannot both win. The loser backs off and does nothing. That is correct behavior, not an error to alarm on.

The marker needs an expiry, because workers die. A marker with no expiry means one crashed Lambda blocks a target forever. A marker that expires too fast means a slow but healthy scan gets a twin. The right length depends on how long scans really take, and that is something to tune from observation, not guess once and forget.

## Interaction with the other parts

### AWS Lambda

The scheduler itself is expected to run as short invocations, triggered by a timer and by requests. It should do its planning quickly and hand off. It should not wait around for workers to finish; waiting wastes paid time and runs into invocation limits. Dispatch is asynchronous, and completion comes back through the state store, not through the scheduler holding a connection open.

Because invocations can be retried by the platform, every action the scheduler takes has to be safe to repeat. Dispatching the same unit of work twice must not produce two scans, and the exclusivity marker is what makes that true. If you add a new action, ask what happens when it runs twice.

Worker payloads should be small. Pass references (which target, which policy selection, which run) and let the worker read details from the store. Large payloads are fragile and make retries expensive.

### DynamoDB

DynamoDB is the source of truth for schedule state. The scheduler reads registrations, last completed times, in-flight markers and per-run records from it. Expectations:

- Reads that decide "is this due" should be consistent enough that a just-finished scan is not immediately seen as overdue. If the access pattern allows eventual consistency, account for that with a small grace rather than assuming reads are fresh.
- Writes that claim a target use conditions, as above.
- Run records are append-style. Do not rewrite history; add a new record or a new status on the run. When someone is trying to understand a bad night, they need to see what actually happened in order.
- Queries by "what is due" should not need a full table walk once the registered set grows. If the schedule data is shaped so that finding due work means scanning everything, that is a design problem to raise, not something to paper over with a bigger Lambda.

### Open Policy Agent

The scheduler chooses which policy bundles go with a scan, but it does not load or evaluate them. It tells the worker which selection applies and trusts the worker and OPA to do the evaluation. If a policy bundle is missing or cannot be loaded, that is a scan failure for that unit, reported back through the store. The scheduler treats it like any other failure: record it, retry within reason, then surface it.

One point worth stating: a change to policy content is not by itself a reason for the scheduler to rescan everything. Whether a policy change should trigger fresh scans is a product question. Until it is answered and written down, the scheduler follows the cadence and the explicit requests, nothing more.

### Jira

The scheduler has no direct dependency on Jira being up. If Jira is down, scans still run and results still land in the store; the ticket side catches up later. The scheduler should not stall or retry scans because ticket creation is slow. Keep the two failure domains apart.

What the scheduler can reasonably supply to the ticket side is context: which run produced a finding, which target, when. That helps a human reading a ticket trace it back to a scan. It should not try to decide ticket priority or assignment.

## Failure handling and observability

### Retries

A unit of work that fails is retried a limited number of times with a growing pause between tries. The limit and the pause are configuration, and they should be easy to find. Retrying forever hides real problems and burns money. Not retrying at all turns every transient throttle into a missed scan.

Some failures should not be retried at all: a target whose credentials have been revoked, an account that no longer exists, a policy selection that points at nothing. Retrying those only produces noise. The scheduler should classify failures well enough to tell "try again" from "someone has to look at this", and the second kind should be marked on the target so it is not endlessly redispatched.

### Stuck and lost work

Work that was dispatched but never reported back is the common quiet failure. The scheduler is expected to sweep for it: find in-flight markers past their expiry, mark the run as lost, release the target, and let normal due-logic pick it up again. The sweep should be part of the regular pass, not a separate thing someone has to remember to run.

When a run is marked lost, say so in the record. Do not delete it and pretend nothing happened. If a late result does arrive after a run was marked lost, the system needs a sane rule for it. The usual sane rule is to accept the result if it is still the newest information for that target, and to record that it arrived late.

### Backpressure

If workers are saturated or the cloud provider is throttling, the scheduler should slow down rather than keep piling on. Signals can come from failed dispatches, from throttling responses reported by workers, or from a growing backlog of in-flight runs. The reaction is to dispatch less for a while, not to fail the whole pass. Backlog should drain as conditions improve, oldest overdue first, so that the most neglected targets are not starved by newer ones.

### What a reader should be able to learn from the records

For any target, someone should be able to reconstruct, from stored records and logs:

- when it was last scanned to completion,
- when it was next expected,
- whether anything was dispatched in between and what happened to it,
- why a scan was started (cadence, on-demand request, retry),
- and whether the scheduler skipped it, and for what reason.

The "skipped, and why" part is the one most often missing. A target that is quietly not scanned is worse than one that visibly failed. Skips because of an existing in-flight run, a paused registration, a classification of "needs a human", or backpressure should each leave a distinguishable trace.

Logs should carry the run and target references on every line the scheduler writes, so they can be joined with worker logs. Avoid logging credentials, full resource configs or anything that came from the scanned environment beyond what is needed to identify it. These configs can contain secrets, and logs travel further than stores do.

## Behavior people get wrong

A few expectations come up in conversation and are worth correcting early.

- **"The scheduler guarantees a scan happens at the scheduled time."** It does not. It aims for the window and records what happened. Throttling, retries and staggering all shift timing. Teams who need a hard guarantee for a specific target should say so, and that needs its own discussion.
- **"If the scheduler runs twice, we get two scans."** It should not. The exclusivity marker and idempotent dispatch exist for this. If you see doubled scans, that is a bug in the claim logic, not expected behavior.
- **"A failed scan means no violations."** Never. A failed or lost scan means unknown. The scheduler must not record a failure in a way that looks like a clean pass, and downstream reports should be able to tell the difference.
- **"Pausing a target deletes its history."** No. A paused registration stops new dispatch and keeps everything recorded so far. Resuming picks up from the last completed scan, which usually means the target is immediately overdue and should be fit into the normal spreading, not blasted out at once alongside everything else that resumed.
- **"Removing a policy from a registration rescans."** Not by itself. See the policy note above.

## Changing this component

When touching `scan-scheduler`, check these first:

- Is the new action safe to run twice?
- Does it still read its state from the store rather than from memory?
- Does it leave a record that says what it did and why?
- Does it keep ticket concerns and policy concerns out of the scheduler?
- If it adds waiting, does it wait in the store (a marker and a later pass) rather than inside an invocation?

If a change breaks one of those, it probably belongs somewhere else, or the design needs another look before the code does.

## Open questions to settle elsewhere

These are not answered here, and nobody should read this note as answering them:

- Whether policy content changes should trigger rescans, and how broadly.
- How late a late result can be and still count.
- How a team asks for a hard timing guarantee for a sensitive target, and what that costs everyone else.
- Whether the on-demand path should be rate limited per team.
- How the scheduler should treat targets that have never completed a scan: first-run priority versus fairness with the established ones.

When one of these gets settled, put the outcome in its own note and link it from here, rather than growing this one into a list of values.
