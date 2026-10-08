---
id: 01KKK4G72JPAF9GA1PYRPEJ3Y4
created: 2026-03-13T05:20-03:00
---

# retention-sweeper tooling choice

We settled how retention-sweeper gets run and what it leans on. It is a long-running .NET worker, hosted as a normal background service, that wakes up on a RabbitMQ message and also on a slow internal timer as a safety net. It reads policy and hold state from SQL Server and acts on Azure Blob Storage. It does not run as a database job, and it does not run as a cloud timer function. This note records why, what we rejected, and what to watch. It is written quickly, so treat it as working notes and not a spec.

The short version: the sweeper is the one component that deletes things in a system whose whole purpose is proving that nothing was deleted by accident. That pushes every tooling question toward the option that is easiest to reason about, easiest to stop, and easiest to explain to a compliance officer who has never read our code. Cleverness lost every time.

The related survey of how to record audit events is in [[audit-trail-options-survey]]. Read that for the audit side. This note only covers the sweeper's own tooling and how it touches the trail.

## Context

LabNotebook Sync takes electronic notebook entries and the files that instruments produce, and keeps them together with an audit trail. Scientists create entries and attach raw output. Compliance officers define how long things must be kept, and what must be kept longer because of a study, a dispute, or an inspection. Somebody has to remove the things whose time has come, and that is retention-sweeper.

There are a few facts about the environment that shaped the choice.

Instrument output arrives in bulk and unevenly. Some instruments dump a lot at the end of a run, and some trickle. The blobs end up in Azure Blob Storage, and the metadata that says what each blob belongs to sits in SQL Server. The two are not transactionally linked. Anything that deletes has to cope with the fact that a blob and its row can disagree for a while.

RabbitMQ is already the backbone for moving instrument events and notebook changes between services. The team knows how to operate it, has dashboards for it, and has habits around dead-letter handling. A new scheduling mechanism would be a new thing to learn and to get wrong.

Retention rules are not simple. They depend on the kind of record, the study it belongs to, the site it came from, and whether a hold applies. Holds can be placed and lifted at any time by people who are not engineers. So the sweeper cannot work from a precomputed list that goes stale; it has to look at hold state close to the moment of acting.

The audit trail has its own rules. A deletion is itself an auditable event. The sweeper must write the record that it deleted something, and that record must survive even if the deletion fails halfway. The order of operations matters more than speed.

Finally, the people who answer questions during an inspection are not the people who wrote the code. Whatever we pick has to leave behind evidence that a non-engineer can follow: what was considered, what was decided, what was done, and by which identity.

## What we chose

The core choice is a hosted worker in the same .NET family as the other services, deployed the same way as the others, and wired to the same message broker. Concretely:

- The worker listens on a dedicated queue for sweep requests. A request names a scope, such as a study, a site, or a record kind, and never names individual blobs. The worker works out what is eligible by itself.
- A slow timer inside the worker publishes a sweep request for each scope when nothing else has. That means the system keeps cleaning up even if the producers of requests are down. The timer does no deletion itself; it only puts a message on the queue. That keeps one code path for everything.
- Eligibility is decided in SQL Server, in a query that joins the retention policy with the current hold state. The worker does not cache holds. It asks again right before it acts on a batch.
- Deletion in Blob Storage uses soft delete as a first stage. The sweeper marks things for removal and the storage layer keeps them recoverable for a grace window before they are truly gone. Final removal is left to the storage side, not to a second pass written by us.
- Every decision, including a decision to skip because of a hold, is written to the audit trail before the blob action is attempted. If the blob action fails, a follow-up entry says so.

The reason for reusing the broker is plain: it gives us retries, dead-lettering, visibility into backlog, and the ability to pause consumption with a single operational action. A compliance officer asking us to stop all deletions right now gets a clear answer, because stopping the consumer stops the sweeper. With a timer-driven job we would have to find and disable the schedule, and we would never be fully sure nothing else was triggering it.

We also chose to keep the sweeper as its own deployable unit, not a module inside the sync service. Part of the reason is blast radius: a bug in the sweeper should not be able to take down ingestion. The other part is permissions. The sweeper holds the only credentials that can delete blobs, and keeping it separate makes that easy to audit. Other services get read and write but not delete.

## Alternatives we looked at

Three alternatives came up seriously, and a couple more got a quick no.

### Database-scheduled jobs

The obvious one for a team with SQL Server: have the database agent run a job that finds expired rows and removes them, and let something else clean the blobs. We rejected it because the job lives where the metadata lives but cannot reach the blobs, so you end up with orphans or with a second system to chase them. Worse, deleting the metadata first removes the very thing you need in order to know which blobs to remove. Doing it the other way round needs the database to call out to storage, which we did not want. There was also the concern that holds are checked at query time and a job that runs for a long time may act on a stale view of them. And operationally, schedules in the database are invisible to the people who watch the queues and dashboards.

### Cloud timer functions

A serverless function on a schedule is cheap and easy to start with. We passed because the sweep is not a short, stateless action. It needs to work through large sets in batches, remember where it stopped, back off when storage pushes back, and stop cleanly when told. Those are things a long-lived worker with a queue does naturally and a function with an execution limit does badly. We also did not want deletion rights spread across another hosting model with its own identity and its own logging story. The cold-start and scaling behaviour was not a problem so much as a mismatch: we want one careful consumer, not many eager ones.

### Storage lifecycle rules alone

Blob Storage can expire objects by itself, based on age or tags. This is attractive because it is nearly free to run. We use it only as a backstop for scratch material that has no compliance meaning. We did not use it for anything that is subject to holds, because the rule engine does not know about our holds and cannot ask. Pushing hold state into blob tags to make it work was considered, and we decided against it: tags and database state would drift, and drift is exactly what an inspector would find. A single place that decides, with a trail, beats two places that sort of agree.

### Smaller no's

A general workflow engine was dismissed as too much machinery for a component whose logic is a loop with careful guards. A separate scheduler product was dismissed for the same reason, plus another thing to patch. Running the sweep on demand by hand from an admin screen only was dismissed because people forget, and because the whole point of retention is that it happens without anyone remembering.

## How it interacts with the audit trail

This is where most of the thinking went, so it gets its own section.

The sweeper writes an intent record before it touches storage. The intent says what rule made the item eligible, what the hold check returned, and which worker identity acted. After the storage action, a completion record is written. If there is an intent with no completion for a long time, that is a signal in itself, and a separate check looks for it. We prefer this to a single record written after the fact, because a single record after the fact cannot show that the system tried and failed.

We did not let the sweeper rewrite or remove any audit entry, including its own. Audit entries about deleted items stay, even when the item is gone. The entry carries enough descriptive detail to say what was removed, without holding the content. That was a deliberate choice: the trail proves deletion happened lawfully, not what the data was.

Holds are read at the last moment. The sweeper takes a batch, re-checks holds for the batch, and drops from it anything that has been held in the meantime. A hold placed after the check but before the storage action is still a race, and we have not pretended otherwise. The mitigation is the grace window on soft delete: if a hold arrives late, the item can be brought back, and the trail records both the deletion mark and the recovery.

The survey note on audit options discusses where the trail itself should live and how tamper evidence works. The sweeper does not depend on which of those wins, as long as the trail accepts append-only writes with a stable identity for the writer. If that interface changes, the sweeper's writer is a small adapter and should be the only code that has to move.

## Gotchas and failure modes

A list of things that bit us in design or are likely to bite in practice.

- Message redelivery. The broker may deliver the same sweep request again, and sometimes the same one while the previous attempt is still running. The sweeper must be safe to run twice over the same scope. We made every step idempotent: marking an already-marked blob is a no-op, and intent records carry a stable key so duplicates collapse.
- Orphaned blobs. A blob with no row, or a row with no blob, will show up. The sweeper does not guess. It reports these to a separate review queue and leaves them alone. Auto-deleting orphans was tempting and was rejected, because an orphan may be a new upload whose row has not been committed yet.
- Clock differences. Retention is computed from timestamps that come from different sources: the instrument, the notebook client, and our servers. Which one counts depends on the record kind, and the policy table says which. The sweeper must not substitute its own clock for a record's start time. This sounds obvious and caused the most arguments.
- Large scopes. A scope that touches a lot of items must be processed in bounded batches, with progress saved, so a restart does not begin again from the top. The consumer acknowledges the request only when the scope is done or has been safely re-queued with its position.
- Permission creep. Because the sweeper has delete rights, there is a steady pull to reuse its identity for one-off cleanups. Don't. One-off cleanups go through a reviewed change with their own identity and their own trail entries.
- Silent success on empty. If a policy query returns nothing because of a bug, the sweeper looks healthy while doing nothing. A check on the expected rate of activity per scope catches this, and it is more valuable than most error alerts.
- Soft delete cost. Keeping marked items recoverable costs storage for the grace window. It is the price of being able to undo a mistake, and we accepted it.

## Running it

Operational choices that go with the tooling.

The worker exposes health and a simple status view for the queue depth, the age of the oldest pending request, and the last time each scope completed a sweep. Compliance officers can be pointed at that last item without needing to read logs.

There is a documented pause procedure: stop the consumer, confirm the queue holds its messages, and leave a note in the trail saying who paused and why. Resuming does the reverse. Because requests are scoped and idempotent, a long pause is safe, and the backlog drains in a bounded way afterward. We tested the drain under load in a staging setup and watched storage throttling; the worker backs off, and that is acceptable.

A dry-run mode exists. It does everything except the storage action and writes its intent records into a clearly separate stream so they cannot be confused with real ones. Every change to retention policy should go through a dry run first, with the output reviewed by someone from compliance. This is a process rule more than a code feature, but the code makes it possible.

Configuration of policies lives in the database and is edited through the admin tooling, not through the worker's own settings. The worker's settings only cover technical knobs such as batch sizing and back-off behaviour. We did this on purpose, so that changing how long something is kept is always an audited act by an authorised person, never a deployment.

Secrets and identities: the worker uses a managed identity for storage where possible, and its database login is limited to the reads it needs plus writes to its own tables and the trail. It has no rights to alter policy or holds. If someone wants the sweeper to change a hold, the answer is no.

Logs are structured and carry the scope, the rule, and the correlation key of the request. We avoid logging content. Anything that identifies a person or a sample is left out of operational logs, since logs are not under the same retention rules as the records they describe and would become a second, uncontrolled copy.

## What would change our mind

The choice is not meant to be permanent. Reasons to revisit:

- If the broker stops being the shared backbone, the case for queue-driven sweeps weakens, and a different trigger could make sense. The sweeper's logic would stay the same; only the trigger and the retry story would move.
- If the storage platform offers hold-aware lifecycle rules that can read an external source of truth reliably, then some of the sweeper's work could be handed over. We would want that to be demonstrably equal on audit evidence before moving anything.
- If the number of scopes grows enough that one consumer cannot keep up, we would add consumers per scope group. That is a scaling change, not a redesign, but it needs care so two workers never act on the same scope at once. The current design assumes a single active consumer per scope and relies on the idempotent steps as a second guard.
- If an inspection or an internal review finds that the intent and completion pairs are not enough evidence, we would add more detail to the trail before touching the tooling.
- If the grace window turns out to be a storage cost problem, the answer is a policy discussion with compliance, not a quiet change in code.

We do not plan to move the sweeper into a database job or a function. If someone proposes it later, they should first explain how it handles holds taken at the last moment, how it stops on request, and how its actions appear in the trail.

## Notes for whoever picks this up

Start with the policy query, since everything else follows from what it considers eligible. Read it with someone from compliance and make sure the words they use match the words in the query. Most of the real bugs we expect are mismatches in meaning, not in code.

Before changing anything on the deletion path, run the dry run against a copy of real-shaped data and compare the intent stream with what the compliance side expects. Keep the diff of that comparison with the change.

When something looks wrong in production, pause first and investigate second. The pause is cheap and reversible. A deletion that has passed the grace window is not.

If you find yourself adding a second way to trigger a sweep, stop and put it on the queue instead. One path in means one path to reason about, and one place where a pause works.

Keep the sweeper boring. It should read like a careful checklist: look up the rule, check the hold, write the intent, act, write the completion. Anything more interesting than that belongs somewhere else, with its own review.

Last, keep this note short on specifics on purpose. The actual retention periods, thresholds, and scope definitions live in the policy store and its change history, where they are audited. Copying them here would just create a stale second source.
