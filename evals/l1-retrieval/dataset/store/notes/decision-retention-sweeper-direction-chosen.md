---
id: 01KVHE810P2ZZEWPK3J96KXC8K
created: 2026-06-19T23:37-03:00
---

# retention-sweeper: general direction

This note records the general direction the team chose for retention-sweeper, the component in LabNotebook Sync that removes or archives data once its retention period has run out. It is written quickly, from what the team agreed. It gives no exact values on purpose. Retention periods, batch sizes, schedules and thresholds live in configuration and in the compliance policy, not here. If you need a value, read the configuration or ask the compliance side. Do not copy a value from this note, because there are none.

The short version: retention-sweeper should be slow, careful and boring. It should prefer leaving data in place over deleting something it is unsure about. It should leave a clear audit record of everything it does. It should be safe to stop, restart and run twice. Most of the sections below follow from those points.

## Why this component needs a stated direction

The sweeper is the one part of the system whose job is to destroy things. Everything else in LabNotebook Sync tries to keep entries, instrument output and audit history intact. A bug here does not show up as a failed request. It shows up later, when a scientist or a compliance officer goes looking for a record that should exist and does not. By then the evidence of what happened may also be gone.

The team had drifted into making small local choices about the sweeper in different pull requests. Some favored speed, some favored caution, and a few quietly changed what counted as eligible for removal. That is why this note exists. The aim is for anyone touching retention-sweeper, human or agent, to start from the same stance instead of rediscovering it from the code.

This is a direction, not a specification. It says which way to lean when a trade-off comes up. Detailed behavior belongs in the design and in tests.

## Caution over throughput

The main choice: when safety and speed conflict, safety wins. The sweeper is not on any user-facing path. Nobody waits on it. A sweep that takes much longer than it could is an acceptable cost. A sweep that removes the wrong thing is not.

In practice this means the sweeper works in small, bounded units. It does not try to clear a large backlog in one pass. It yields to other load on SQL Server and on blob storage. If the system is under pressure from ingestion or from instrument output arriving, the sweeper backs off rather than competing. We would rather have a backlog of expired data for a while than slow down capture of new data.

The team also decided not to tune the sweeper for the benchmark case of the largest possible backlog. The normal case is steady, modest work done regularly. Design for that, and let a large backlog drain over time.

## Eligibility is decided from recorded facts, not inferred

A record becomes eligible for removal only when the stored facts say so: its retention class, the event that starts its retention clock, and the policy attached to it. The sweeper reads those facts. It does not guess from file age, blob timestamps or naming conventions. Blob storage metadata can be changed by tools outside our control, so it is not trusted as the source of truth for when retention started.

If the facts are missing, contradictory or ambiguous, the record is not eligible. It is skipped and reported. Missing data is treated as a reason to keep, never as a reason to delete. This was an explicit choice, since older entries imported from earlier systems sometimes lack complete retention information.

The rule for eligibility lives in one place in the code. Other parts of the system should ask that place instead of re-implementing the check. Two copies of the rule will eventually disagree, and in a deletion component a disagreement is dangerous.

## Holds always win

Legal holds, investigation holds, and any compliance-requested preservation override retention. If a hold applies to a record, directly or through the notebook, project or study it belongs to, the sweeper must not touch it, no matter how overdue it is. The team treats this as the most important invariant of the component.

The sweeper checks holds at the moment of action, not only when it builds its candidate list. A hold may be placed between the time a candidate is chosen and the time it would be removed. A check made early and then trusted is not enough. The check and the removal are tied together so that a hold placed in between is respected.

When the hold status cannot be determined, for example because a dependency is unreachable, the answer is the same as for missing facts: keep, skip, report.

## Two-stage removal instead of immediate deletion

The direction is to remove in stages. A record that becomes eligible is first marked, and its content is made inaccessible to normal users, but it is not physically destroyed straight away. Physical destruction follows only after a further waiting period that gives people a chance to notice a mistake. The length of that period is policy and configuration, not something fixed here.

This costs some extra storage and some extra state to track. The team accepted that. Being able to reverse a mistaken mark is worth far more than the storage. Once physical destruction has happened there is no way back, so the earlier stage is where errors should be caught.

The marking stage must be cleanly reversible and must itself be audited. The destruction stage must refuse to run on anything whose mark has been cleared or whose hold status has changed since marking.

## Audit trail comes first

LabNotebook Sync exists partly to enforce audit trails, so the sweeper cannot be an exception to them. Every action it takes is recorded: what was marked, what was destroyed, what was skipped and why, and under which policy. The audit entry is written before or together with the action, never as an afterthought that could be lost if the process dies.

The audit trail is not subject to the sweeper's own removal. The history of what was removed stays, even when the content is gone. Audit entries should carry enough identifying information to show that a record existed and was removed under policy, without keeping the sensitive content itself.

If the audit write fails, the action does not proceed. We would rather do nothing than delete without a record. This is a hard rule and tests should cover it.

## Idempotent and restartable

The sweeper can be killed at any point: deployment, node restart, crash, operator decision. After that it must be safe to start again and continue. Each step is written so that repeating it has no extra effect. Marking an already marked record does nothing new. Destroying something already destroyed is treated as success, not as an error to escalate.

Progress is kept in durable state, in SQL Server, not in memory. The sweeper does not rely on a long-lived in-memory cursor. On restart it works out where it is from recorded state. Partial failures, such as a database record updated but the blob not yet removed, are expected and have a defined way to be completed or repaired on a later pass.

The ordering between the database and blob storage is chosen so that the failure that leaves things behind is the harmless one: orphaned content that can be cleaned later, rather than a record pointing to content that is gone.

## Single active sweeper

Only one sweeper should be acting on a given set of data at a time. The team chose to enforce this with a lease or lock held in shared durable state, so that scaling out or an overlapping deployment does not produce two sweepers racing. If the lease cannot be confirmed, the sweeper stops acting and waits.

We considered letting several workers split the work through RabbitMQ and decided against it for now. The extra parallelism is not needed, and concurrent deletion makes reasoning about holds, ordering and audit entries harder. If throughput ever becomes a real problem this can be revisited, but it needs its own decision.

RabbitMQ is still used where it fits: to announce what the sweeper did so other components can react, and to receive notice of changes that matter to retention, such as a hold being placed or released. Messages are treated as hints that may be late or duplicated. The authoritative answer always comes from the stored state.

## Dry runs and visibility

The sweeper supports a mode where it works out what it would do and reports that without acting. This mode is the default way to introduce any change to eligibility rules or policy handling. A change to the rules is first run in this mode against real data, and its output is looked at by a person before the real behavior is enabled.

Compliance officers need to see what is coming. The direction is to expose, in a readable form, what is scheduled to be marked and destroyed, so that nothing surprising happens to users' records. Scientists should be able to find out why a record was or was not removed without reading code.

Metrics and logs describe counts and outcomes by category, and never include notebook content or instrument data. Logs help an operator tell whether the sweeper is healthy, stuck, or skipping a lot, and that is all they are for.

## Failure handling

Failures are expected: SQL Server timeouts, blob storage throttling, a broker outage, a bad record. The sweeper handles a failure on one record by isolating it, recording it, and moving on, instead of stopping the whole run. Repeated failure on the same record leads to it being set aside for human attention, not retried forever and not forced.

When the failure suggests something systemic, such as many failures in a row or a dependency being unavailable, the sweeper stops and waits instead of pushing through. An alert goes to operators. The sweeper does not try to be clever about recovering from an unclear situation; it stops acting and keeps the data.

Transient errors use backoff. Permanent errors are not retried. The distinction is made explicitly in code, because treating a permanent error as transient causes endless loops and treating a transient one as permanent causes needless manual work.

## Changes to policy and to eligibility rules

Retention policy will change over time. The direction is that a policy change does not apply backward in surprising ways. Shortening a retention rule must not trigger a sudden mass removal; the effect is introduced gradually and with visibility, and goes through the dry run first. Lengthening a rule takes effect right away, since keeping longer is the safe direction.

The policy that applied at the time of an action is recorded with the audit entry, so later readers can tell why something was removed even after policy moved on. Rules are versioned in the sense that the audit entry says which rules were in force, though the exact version scheme is not part of this note.

Any change to eligibility logic needs review from someone on the compliance side as well as an engineer. This is a process rule, and it is cheap compared to the cost of a bad deletion.

## Testing expectations

The team expects tests to focus on the invariants rather than on happy paths. The important ones: holds always block removal; missing facts mean keep; audit failure blocks action; repeated runs change nothing further; a restart in the middle converges to the right state; and two sweepers do not act at once. Each of these has a test that fails if the rule is broken.

Tests use fakes or local stand-ins for blob storage and the broker where possible, and a real SQL Server for the parts that depend on database behavior, since transactional details matter here. Tests never point at shared or real data stores. Anyone running the sweeper locally must use a non-production setup, and the sweeper refuses to run destructive stages when it cannot tell where it is pointed.

When a bug is found in production behavior, a test for the missed invariant is added with the fix. The point is to build up a list of ways the sweeper has been wrong, so they do not recur.

## What we deliberately did not choose

We did not choose aggressive cleanup for the sake of storage cost. We did not choose to let the sweeper infer retention from blob age. We did not choose immediate hard deletion. We did not choose parallel workers. We did not choose to let other components delete data on the sweeper's behalf or bypass it; all retention-driven removal goes through retention-sweeper so it has one set of rules and one audit story.

We also did not choose to make the sweeper configurable to the point where an operator can switch off the safety checks. Settings may adjust pace and scope, but holds, audit and the staged removal are not options.

## Open questions and next steps

A few things are still undecided and should be settled as separate notes when someone takes them on. How instrument output that arrives late interacts with a retention clock that has already started. How to handle records shared across several notebooks with different retention classes. Whether the waiting period before physical destruction should vary by data class. How the operator-facing view of upcoming removals should be presented and who gets access to it.

Until those are settled, apply the stance in this note: when in doubt, keep the data, write the audit entry, and ask. If you are changing retention-sweeper and your change makes it more eager to delete, stop and check that it is really what the team wants, because the default direction here is the opposite.
