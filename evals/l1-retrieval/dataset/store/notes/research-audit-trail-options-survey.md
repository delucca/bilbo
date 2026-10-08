---
id: 01JT0E6VAMJ9ZAGSGYPKADGX5G
created: 2025-04-29T06:31-03:00
---

# audit-trail-writer: survey of general options

This is a working survey of the options we have looked at for audit-trail-writer, the part of LabNotebook Sync that turns notebook edits, instrument imports and sync actions into an append-only record that compliance officers can rely on. It is not a decision log. Nothing here is settled, and none of it carries a target value. It is a map of the choices, what each costs, and what we keep tripping over, so the next person does not start from zero.

The stack is fixed: C#, .NET, SQL Server, RabbitMQ and Azure Blob Storage. Most of the options below are different ways of arranging those pieces, not different technologies.

## What the component has to do

audit-trail-writer receives facts about things that happened and records them so they cannot be quietly changed afterwards. The facts come from a few places: scientists editing entries, the sync process pulling instrument output into entries, and administrative actions such as permission changes or exports. Each fact needs an actor, a subject, a before and after where that makes sense, a time, and a reason when the user supplied one.

Two audiences pull in different directions. Research scientists want the writer to be invisible: no slow saves, no lost edits because the audit store was busy. Compliance officers want the opposite: nothing happens without a record, and the record is complete and ordered. Most of the design tension is that conflict, and each option below is a different compromise between them.

## Where the writer sits in the flow

A rough picture of the path we keep assuming, using only the pieces we already have:

```
ELN entry -> RabbitMQ -> audit-trail-writer -> SQL Server / Azure Blob Storage
```

That picture hides a lot. The first arrow could be a synchronous call inside the save path or a message published after the fact. The last arrow could be one store or both, with different roles. The sections below take those questions in turn.

## Option family one: write inside the same transaction as the change

The simplest way to guarantee that an edit and its audit record agree is to write both in one SQL Server transaction. If the entry changes, the audit row exists. If the audit row fails, the entry change rolls back. This is the strongest consistency story and the easiest to explain to an auditor.

Costs we have noted:

- Every save path in the application must call the writer. A forgotten path means a silent gap, and gaps are exactly what an audit trail must not have.
- The audit write sits on the user's critical path. Contention on the audit table becomes contention on saving a notebook entry.
- Instrument-driven writes do not originate from a user save. They arrive through the sync pipeline and would need the same discipline.
- The audit table and the entry tables must live in the same database, which couples their growth and backup policies.

This option is attractive for the small set of actions where the audit record is the point, such as signing or locking an entry. It is less attractive as the only mechanism.

## Option family two: capture at the database layer

Instead of asking application code to remember, capture changes where they land. Candidates include SQL Server change tracking, change data capture, temporal tables, or triggers that write audit rows.

What is good about it: coverage does not depend on developers. Any path that touches the table, including a hand-run fix, produces a record. That is a real argument for compliance reviewers.

What is awkward about it:

- The database sees row changes, not intent. It knows a column changed but not that a scientist corrected a transcription error, or that the sync process reconciled an instrument value.
- Actor identity is often the application's service account, not the person. Getting the real user into the record needs session context passed on every connection, which is easy to get wrong with pooled connections.
- Retention and shape of the captured data are tied to the feature chosen. Some of these features clean up old data on their own schedule, which conflicts with long retention.
- Triggers add hidden cost to every write and are painful to reason about during bulk imports of instrument output.

A hybrid is plausible: database-level capture as a safety net, application-level events as the meaningful record, and a reconciliation job that compares them.

## Option family three: events through RabbitMQ

Here the application publishes an audit event after (or alongside) the change, and audit-trail-writer consumes it. This decouples saving from auditing, lets the writer batch, and lets the writer be restarted or scaled without touching the main application.

The weakness is the gap between the change and the event. If the app commits the entry change and then crashes before publishing, the record is lost. The usual answer is an outbox: write the event into a table in the same transaction as the change, and have a relay publish it to the broker. That pulls us back toward option one for the outbox row, but the heavy audit processing stays off the critical path.

Things to settle if we go this way:

- Whether the outbox row itself is already considered the authoritative audit record, with the writer only enriching and archiving it.
- How the relay avoids publishing the same event twice after a restart, and how the writer copes when it does anyway.
- What happens to events while the writer is down. The queue holds them, but a queue is not a long-term store, and compliance staff will ask how long it can hold before something is dropped.

## Option family four: hybrid outbox with a thin synchronous core

A middle shape keeps appearing in discussion. A minimal synchronous record, enough to prove that an action happened and who did it, goes in the same transaction. Richer detail, such as full before and after content, diffs, and links to instrument files, is produced asynchronously through RabbitMQ and attached later.

The benefit is that the guarantee compliance cares about is strong, while the expensive part is off the hot path. The cost is two representations of one event, which must be linked and which can disagree if the enrichment step fails or is delayed. We would need a clear state for an event that has its core but not yet its detail, and a way to show that state to a reviewer honestly instead of hiding it.

## Delivery guarantees and duplicates

With a broker in the path, the realistic guarantee is that every event is delivered at least once, so duplicates will happen. Exactly-once delivery is not something we should plan around. The writer must therefore be idempotent: giving it the same event again must not create a second record.

Options for making that work:

- A unique event identity generated by the producer, with the store rejecting repeats through a unique constraint.
- A content hash of the event, which works without producer cooperation but can wrongly merge two genuinely identical actions.
- A deduplication table consulted by the writer, which adds a read to each write.

The producer-generated identity is the cleanest, but it only works if every producer follows the rule, including the sync process handling instrument output. We should also decide what the writer does when a duplicate arrives with different content, because that is a sign of a bug and probably deserves its own record.

## Ordering

An audit trail that shows an entry being signed before it was edited is confusing, and in a dispute it is damaging. RabbitMQ preserves order within a single queue for a single consumer, but ordering degrades with multiple consumers, redelivery after failure, or events split across queues.

Choices:

- Single consumer per partition of the data, where the partition is something like a notebook or entry, so order only matters inside it.
- A per-subject sequence number assigned at the source, with the writer detecting gaps and holding later events until the missing one shows up or is declared lost.
- Accept disorder in arrival and order by a source timestamp at read time. This is simple but depends on clocks, and clock skew between machines is a known problem.

A sequence number per subject is more work at the producer, but it gives us gap detection, which doubles as a completeness check. That second benefit is why it keeps coming back.

## Tamper evidence

Compliance reviewers do not just want a log; they want to know it has not been edited. Approaches range from light to heavy.

- Restrict permissions so the writer's database account can insert but not update or delete. Cheap and necessary, but a database administrator can still bypass it.
- Hash chaining: each record includes a hash of the previous one for the same stream, so any alteration breaks the chain from that point on. Needs care about concurrency, since the chain forces an order on writes.
- Periodic sealing: compute a digest over a batch of records and store it somewhere with different access rules, such as a blob container with immutability policies.
- Signing records with a key held outside the database, which ties into key management and rotation questions we have not answered.

These stack. A realistic plan is restricted permissions plus periodic sealing, with chaining as an upgrade if reviewers push for it. Chaining's concurrency cost is the main reason we have not simply adopted it.

## Storage layout in SQL Server

SQL Server is the natural home for the queryable record. Choices about its shape:

- One wide table for all event types with a flexible payload column, versus a table per event family. The wide table is easier to write to and to keep append-only; per-family tables are easier to query and index.
- Storing the payload as structured text in a column versus normalising fields. Normalising helps reporting, but every schema change in the notebook model then ripples into the audit schema, and old records must stay readable as they were written.
- Partitioning by time, so old data can be moved to cheaper storage and queries over recent activity stay fast.
- Keeping the audit data in its own database or schema with separate permissions and backup rules, which supports the insert-only restriction.

One principle we agree on in discussion: audit records describe what was true when written. We do not rewrite them when the notebook schema evolves. Readers adapt to old shapes instead.

## Storage in Azure Blob Storage

Blob Storage has two plausible roles for audit-trail-writer. One is holding large artefacts that a record refers to, such as instrument output files, full snapshots of an entry at signing, or exported reports. The other is long-term archive of the audit records themselves.

For the archive role, immutability features (write once, read many style policies) are attractive because they give a storage-level guarantee independent of our own code and database admins. For the artefact role, the main question is how the record points at the blob in a way that survives renames or lifecycle moves, and whether a content hash is stored in the record so a changed blob is detectable.

Risks: blob and database writes cannot share a transaction. The order matters. Writing the blob first and the record second leaves orphan blobs after a failure, which are harmless but need cleanup. Writing the record first leaves a dangling reference, which is worse for an audit trail.

## Capturing instrument output

Instrument data is where the audit story gets subtle. The audit record should show that a specific piece of instrument output was brought into a specific entry, by what process, and unchanged. It should not duplicate the data itself.

Options: store a hash and a reference to the original file in Blob Storage; store a copy of the file under a naming scheme owned by the writer; or record only the metadata the instrument provided. The hash-plus-reference approach is lightest, and it depends on the original staying put. A copy is safer but multiplies storage and raises a question of which one is the original.

We also need to decide how automated actions are attributed. A sync job is not a person. The record should say it was the sync process, on whose behalf if anyone, and not borrow the name of whoever last touched the entry.

## Failure behaviour: fail closed or fail open

What should happen when the writer cannot record something? Fail closed means the user action is refused. That is the strict compliance answer and it is miserable if the audit store has a bad afternoon. Fail open means the action goes ahead and the gap is repaired later, which is friendlier but needs a trustworthy repair path and an honest marker for the gap.

A split policy is likely: actions with regulatory weight fail closed, routine edits go through the queue and are allowed to lag. Whichever way it goes, the writer needs a dead-letter path for events it cannot process, with alerts, because a silently parked event is a missing audit record.

## Throughput and batching

Bulk instrument imports can generate many events in a burst, far above normal editing. Row-by-row inserts will be slow and will lock more than we want. Batching in the writer, using bulk insert paths and one transaction per batch, helps, but a failed batch then needs a clear rule: retry the whole batch, split it, or fall back to single rows to find the bad event.

Backpressure matters too. If the writer falls behind, queue depth grows. We should decide in advance what the system does when the queue is deep: slow producers, shed nothing and keep waiting, or raise an operational alarm. Dropping events is not on the list.

## Reading and reporting

A trail nobody can read is not much use. Compliance officers will want to filter by person, entry, time range and action type, and export the result. Scientists will want a simple history view on an entry.

Options are to read straight from the primary audit tables, to maintain read-optimised projections fed from the same events, or to export periodically to files for external tools. Projections are faster but can drift from the source, so they need a rebuild path and must never be treated as the source of truth. Whatever we pick, reading the audit trail should itself be auditable for sensitive subjects, which is a small extra event type that is easy to forget.

## Retention, privacy and deletion

Audit data is kept for a long time, while personal data rules may require removal of some information. These collide. Approaches include storing identifiers rather than names so a person's details can be removed elsewhere while the trail stays intact, keeping sensitive payloads in a separate store with its own lifecycle, and recording a redaction event when content is removed instead of altering the original record.

The legal and policy answers belong to the compliance side, not to us. What we can do is keep the design flexible enough that either policy is implementable later: separable payloads, stable identifiers, and a deletion that leaves a visible mark.

## Testing and verification

This component is hard to test by looking at happy paths. Worth having in some form: tests that replay duplicates and out-of-order events, tests that kill the writer midway through a batch, a completeness check that compares what the application says it did against what the trail holds, and a periodic integrity verification job that recomputes seals or chains. The completeness check is the one that catches a forgotten call site, so it deserves the most attention.

## Open questions

- Is the outbox row the audit record, or a message that leads to one?
- Do we need hash chaining, or is sealing of batches enough for reviewers?
- Which actions fail closed?
- Where does the authoritative copy of instrument output live, and who owns its retention?
- How are automated actors named and attributed?
- How do we prove completeness, not just integrity?

Until those are answered, keep new code behind a narrow interface so the storage arrangement behind audit-trail-writer can change without rewriting every producer.
