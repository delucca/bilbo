---
id: 01JYNGQ5Z8Z7S43B7XDSNG0T3Z
created: 2025-06-26T04:02-03:00
---

# availability-sync deadlocks when two jobs update the same clinician

availability-sync fails with `Mysql2::Error: Deadlock found when trying to get lock; try restarting transaction` when two jobs update the same clinician at once. This is a trap because the failure is intermittent. It shows up only when two jobs for one clinician overlap in time, so a quiet test environment or a single manual run will not show it. Treat it as a concurrency bug in how availability-sync writes, not as a MySQL fault or a flaky connection.

This note records what the error looks like, why it happens, what to avoid, and what to do about it. It is written from what we know about the design and the failure. Check the current code before relying on any detail here.

## Symptom

A Sidekiq job belonging to availability-sync dies with the MySQL deadlock error. Sidekiq then schedules it for retry. Usually the retry succeeds, because by then the competing job has finished and released its locks. That is why this goes unnoticed for a while: the error appears in the logs and the retry set, but the data ends up correct most of the time.

The worse case is when the retries also collide. Two jobs that fail together and are retried at similar moments can collide again. If the retry timing is close enough, the same pair of jobs can deadlock repeatedly until one of them drops out of the retry window. The result is a clinician whose availability in ClinicSlotter is stale.

## The exact error

The message to search for in logs is `Mysql2::Error: Deadlock found when trying to get lock; try restarting transaction`. The text comes from MySQL through the mysql2 driver and Rails passes it up as an exception from the database layer. Search for the whole string, or at least the first part, when grepping Heroku logs or the error tracker.

MySQL picks one transaction as the victim and rolls it back. The other transaction proceeds. So one job fails and the other succeeds, and the failing job is not necessarily the one that started later.

## Why it happens

Two jobs update the same clinician at once. Each one opens a transaction and touches rows tied to that clinician, such as the clinician's availability windows and anything derived from them. If the two jobs reach those rows in a different order, each can end up holding a lock the other wants. MySQL detects the cycle and aborts one transaction.

The usual shape of this in a Rails app is that each job loads the clinician's rows, changes some, and writes them back inside one transaction. When one job locks the rows in one order and the other job locks them in another order, they deadlock. Gap locks and index range locks under the default isolation level make it easier to hit than a naive reading of the code suggests, because a job can lock more than the rows it visibly touches.

The key point is that the unit of conflict is the clinician. Jobs for different clinicians do not collide. Jobs for the same clinician do.

## When two jobs for one clinician overlap

Overlap is not exotic. Some ways it happens:

- A scheduled sync and an on-demand sync both run for the same clinician around the same moment.
- A front-desk edit to a clinician's availability enqueues a job while a sync from the external source is still running.
- An incoming FHIR update for a clinician arrives while a previous update for that clinician is still being processed.
- A retry of a failed job lands while a fresh job for the same clinician is in progress.
- A burst of changes, such as a clinic changing hours for several clinicians, produces many jobs in a short time. Where several touch the same clinician, they collide.

More Sidekiq concurrency means more chance of overlap. Adding worker threads or dynos to clear a backlog can make the deadlocks more frequent, not less.

## What it looks like for front-desk staff

Staff do not see the exception. They see a clinician whose open slots do not match what they just entered or what the external system says. A slot may be offered that should have been removed, or a slot may be missing that should be open. Since ClinicSlotter exists to avoid double-booking and respect room constraints, stale availability is a real risk and not just cosmetic.

If staff report that a change did not take, check whether a deadlock failure happened for that clinician around that time before looking at anything else.

## How to reproduce

Enqueue two availability-sync jobs for the same clinician so they run at the same time, ideally with enough rows involved that the transactions last long enough to overlap. Use a worker setup with more than one thread so both can run concurrently. With a single thread the jobs run one after another and nothing deadlocks.

A reproduction with a tiny dataset may pass many times in a row. The window is narrow. Make the transactions slower, by using a larger set of availability rows or by pausing inside the transaction in a throwaway test, so the overlap is likely. Do this on a development database, never on production data.

## What not to do

- Do not treat the error as a transient network problem and just raise the retry count. It hides the cause and can make colliding retries worse.
- Do not swallow the exception and mark the job done. The write was rolled back, so the data is not updated.
- Do not fix this by lowering concurrency across the whole queue and calling it finished. It reduces the symptom but slows every clinic's sync, and it does not remove the bug.
- Do not widen transactions to cover more work in the hope of making them atomic. Longer transactions hold locks longer and deadlock more.
- Do not restart the database or the dynos. Nothing is stuck. The database already resolved the deadlock by aborting one side.

## Fix options

The error text itself says to restart the transaction, and that is part of the answer, but not all of it. In rough order of preference:

1. Serialize work per clinician. Make sure only one availability-sync job runs for a given clinician at a time. This can be done with a per-clinician lock held outside the row-level transaction, or with a unique-job or queue-per-key mechanism for Sidekiq. This removes the collision at its source. It is the recommended fix.
2. Lock in a consistent order. Where the jobs must run concurrently, make every code path take its row locks in the same order, for example by loading the clinician's rows with an explicit ordering and locking them up front. If every transaction acquires locks in the same order, a cycle cannot form.
3. Keep transactions short. Do the slow work, such as fetching and mapping external data, before opening the transaction, and keep only the writes inside it.
4. Retry the transaction on deadlock, in a bounded way with some random delay. This is a safety net and not the fix. Retrying right away in both jobs can reproduce the collision, so add jitter.
5. Coalesce redundant jobs. If several updates for the same clinician are queued, one run with the latest state is enough, so drop or merge the extras.

Option one combined with a bounded retry is the plan I would take. Ordering locks consistently is worth doing anyway when touching that code.

## Sidekiq retry behavior

Sidekiq retries failed jobs with a growing delay. A deadlock is a failure like any other, so the job goes into the retry set and runs again later. This means the system partly heals itself, which is also why the bug survives. Look at the retry set and the logs for this error, not only the dead set, because most affected jobs will never reach the dead set.

If the job is not idempotent, a retry after a rolled-back transaction is still safe, because the rollback undid the partial work. The risk is elsewhere: side effects outside the database, such as calls to an external system, may already have happened before the failure. Keep those after the commit, not before it.

## Diagnosing on Heroku

On Heroku, look in the application logs for the exact error string given above and note which clinician each failing job was working on. Matching on clinician is the fast way to confirm this is the same problem. If the failures cluster on a few clinicians, or on times when a clinic changed hours, it is this bug.

MySQL can also report the most recent deadlock, including both transactions and the locks involved, through its engine status output. Use that to see which statements collided and in which order. Ask whoever manages the database add-on about access if it is not available to you. Capture the output soon after an occurrence, because it only keeps the latest one.

## FHIR side

Availability arrives from or is exchanged with other systems through HL7 FHIR resources. A single external change can fan out into several updates for one clinician, and that fan-out is a natural source of overlapping jobs. When fixing this, do the per-clinician serialization at the point where the jobs are enqueued or started, not inside the FHIR mapping code, so every entry path is covered.

## Checklist when you see the error

- Find the clinician involved and the jobs that ran for them at that time.
- Check whether the final availability for that clinician is correct now. If not, enqueue a fresh sync for that clinician once nothing else is running for them.
- Check whether a retry already fixed it, so you do not run a redundant sync.
- Look for a pattern: bursts, scheduled runs, or recent concurrency changes.
- Record any new overlap path you find in this note.

## Open questions

I have not confirmed which exact tables and lock orders are involved. The first step for whoever fixes this is to read the latest deadlock report and list the statements on each side. I also have not decided whether the unique-job approach or an explicit per-clinician lock fits better with how the team deploys Sidekiq. Update this note once that is settled, and remove anything above that turns out to be wrong.
