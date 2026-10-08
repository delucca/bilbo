---
id: 01KETBQE9Z0PYHKMPQ1QKCAD3C
created: 2026-01-12T21:22-03:00
---

# results-ledger spec

This note specifies the results-ledger for TownHall Pulse: what it stores, who writes to it, who reads from it, how long rows live, and what has to stay true when moderation changes a result after the fact. It is written quickly from what the team has settled so far. Where something is still open it says so. The one hard number in the spec is retention: results-ledger rows must be retained for 400 days before deletion. Everything else here is described in general terms on purpose, because the sizing and tuning values are not final.

## Purpose and scope

The results-ledger is the durable record of poll and Q&A outcomes for every live event that runs on TownHall Pulse. During an event the live numbers move through Phoenix channels over WebSockets and sit in process memory on the Elixir nodes. That path is fast but it is not a record. The results-ledger is the record. It is what an event producer opens after the event to see what the audience actually answered, what the community manager approved or removed, and what was shown on screen at which moment.

Three things use it. First, the post-event report that producers download. Second, the audit view that community managers use when someone disputes a moderation call. Third, the replay feature in the Next.js dashboard, which rebuilds how a poll looked over time. None of these need the sub-second path. They need correctness, completeness and a stable history.

The results-ledger is not the live tally. It is not the place the audience screen reads from while voting is open. If a change would make the live path wait on a ledger write, that change is wrong. The live path publishes, the ledger consumes, and the ledger catches up.

Out of scope for this spec: the raw submission store for individual audience messages, the billing usage counters, and the analytics warehouse export. They may read from the results-ledger but they have their own specs. Where this note touches them it only says what the results-ledger promises them.

## Retention

Results-ledger rows must be retained for 400 days before deletion. That is the rule, and a reader of this note alone should take it as: nothing in the results-ledger may be deleted, purged, compacted away or overwritten in a destructive way until the row has existed for 400 days. After 400 days a row becomes eligible for deletion. Eligible does not mean deleted immediately; the sweep that removes rows can lag and that is fine.

A few clarifications that came up while settling this.

The clock starts when the row is written to the results-ledger, not when the event starts and not when the event ends. A long-running event that keeps producing rows therefore has rows of different ages, and the oldest ones can become eligible while the event is still running. In practice events are far shorter than the retention period, so this is an edge case, but the sweep must not assume that an event's rows all age together.

Corrections do not reset the clock on the row they correct. A correction is a new row (see the section on writes and corrections). The old row keeps its own age. The new row has its own age. If the old row reaches 400 days first it can be deleted while the correction remains. A reader must never need the old row to interpret the correction, so every correction carries the full corrected value and not a delta.

The retention rule is a floor, not a target. If a customer contract requires a longer hold, the longer hold wins for that customer's events. The mechanism for per-customer holds is not designed yet. Until it exists, the sweep deletes by age alone and legal holds are handled by pausing the sweep, which is blunt and should be treated as a stopgap.

The rule applies to the results-ledger only. Raw audience submissions, session logs and moderation chat have their own retention and are generally shorter. Do not copy the 400 days figure onto those stores by reflex. If you find code that does, check that it was meant.

Deletion must be batched and must not run during a large live event window if it competes for the same CockroachDB ranges as the writers. See the operations section for how the sweep is scheduled.

## Data model

The ledger is append-only in spirit. Rows describe facts that were true at a time. Nothing updates a row in place to change its meaning. The shape is described here in words; the exact column list belongs in the migration files and should be read from there rather than trusted from this note.

Every row belongs to an event and to a subject within that event. A subject is a poll, a poll option, a Q&A question, or a moderation action. The row records the subject's identity, the kind of fact, the value of the fact, who or what caused it, and the time it was recorded. Time is stored as the database's own timestamp plus the ordering value described below.

The kinds of fact currently expected are these. A tally snapshot: the counts for each option of a poll at a moment. A poll state change: opened, paused, closed, reopened. A question state change: submitted, approved, hidden, answered, removed. A moderation action: who did what to which subject and the stated reason if there was one. A correction: a replacement value for an earlier fact, pointing at the row it replaces. A finalization marker: a signal that no more rows are expected for the subject in the normal course.

Each row has a stable identifier that is generated by the writer, not by the database. This is deliberate. The Elixir node that produces the fact makes the identifier, so a retry after a network failure writes the same identifier again and the ledger can recognize the duplicate. The identifier must be unique across the whole ledger, not just within an event.

Ordering inside a subject uses a per-subject sequence value assigned by the single process that owns the subject during the event. Wall-clock time is stored but is not trusted for ordering, because the nodes in a cluster disagree about time by small amounts and moderation actions arrive from several places. When two rows for one subject have the same sequence value, the row is malformed and the writer has a bug.

Personal data is kept out of the ledger where possible. Audience members are referenced by an opaque participant reference, not by name or email. Moderators and producers are referenced by their account reference, since the audit view needs to show who acted. If a deletion request for a person arrives, the participant reference in ledger rows is the thing to break, not the rows themselves, because the rows must stay for the retention period. How that unlinking is done is open; see the open questions.

## Writes and corrections

Writers are the Elixir processes that own polls and Q&A rooms during an event. When a subject changes state or a tally is due, the owning process builds a ledger entry and hands it to the ledger writer. The ledger writer batches entries and writes them to CockroachDB in transactions. The owning process never blocks on the database. If the writer is behind, entries wait in a bounded queue. If the queue is full the system must degrade by writing tally snapshots less often, never by dropping state changes or moderation actions. State changes and moderation actions are the facts that matter most and are the rarest, so they get priority in the queue.

Tally snapshots are taken on a schedule while a poll is open and once more when it closes. The closing snapshot is the authoritative final result. Intermediate snapshots exist for replay and for post-mortems, and it is acceptable to thin them out under load. The closing snapshot is not acceptable to thin out or skip. A poll is not considered finalized in the ledger until its closing snapshot is durably written.

Idempotency is required. Every write is safe to repeat because of the writer-generated identifier. The write path uses an insert that ignores a conflict on that identifier, and then compares the stored row to the intended row in tests. If a stored row with the same identifier has a different value, that is a bug in the producer and should be logged loudly, not silently resolved.

Corrections work like this. Moderation sometimes needs to change a result after the fact: a batch of votes is found to come from a bot, a poll option is found to have been mislabeled, a question was approved and then withdrawn. The process is never to edit or delete the earlier row. Instead a correction row is appended, naming the row it replaces and carrying the full corrected value plus a reason and the acting account. Readers that want the current truth take the latest correction in the chain. Readers that want the history, such as the audit view, show the whole chain.

Because corrections are rows, they are subject to the same retention rule as everything else. The chain for a subject can lose its oldest members first as they age out. Readers must tolerate a chain whose first links are gone and treat the earliest remaining correction as a self-contained statement of the value.

A correction after finalization is allowed but is flagged. The report generator must show that a final result was later corrected and when, so a producer who exported the report earlier can tell it is stale.

## Reads and consumers

The ledger is read in three main ways, and each has a different tolerance for staleness.

The post-event report reads the final state of every poll and question for an event. It tolerates a short delay after the event ends, since the ledger writer may still be draining its queue. The report job should check that every poll it expects has a finalization marker before it claims the report is complete. If some are missing it should say so in the output and offer to regenerate later, rather than presenting partial numbers as final.

The audit view reads the full chain of moderation actions and corrections for a subject or for a moderator. It tolerates no loss of rows inside the retention period. An audit view that shows a gap inside the period is a serious defect. Beyond the period gaps are expected and the view should say that older history is no longer kept.

The replay feature reads tally snapshots and state changes in sequence order and rebuilds the poll over time. It tolerates thinned snapshots but needs state changes to be complete. Replay is the heaviest read, so it goes through a paginated endpoint on the Phoenix side and the Next.js dashboard fetches in pages and assembles locally. The dashboard must not ask for a whole event in one call.

Reads should prefer follower reads from CockroachDB where staleness is acceptable, which is the case for reports on finished events and for replay. The audit view during a live dispute may need current data and should read normally. This split is a performance choice; correctness never depends on it, so if follower reads cause confusion in a test, turn them off before suspecting the data.

Read access is checked against event membership. A producer sees their own events. A community manager sees events they were assigned to. Nobody sees another organization's ledger rows. The check happens in the Phoenix layer and also as a filter in the query, so a bug in one does not expose rows.

## Consistency and failure handling

The guarantees the ledger offers are intentionally modest and should be stated plainly.

Durability: once the ledger acknowledges a write, the row survives the loss of a node, because CockroachDB replicates. The writer must wait for the commit before it tells the owning process the entry is safe. Until then the owning process keeps the entry available for resend.

Ordering: within a subject, rows are ordered by the sequence value. Across subjects there is no promised order. A report that needs a cross-subject timeline sorts by recorded time and accepts small inversions.

At-least-once with deduplication: producers may send the same entry more than once, and the ledger stores it once. This is the basis for retry logic everywhere in the write path.

No silent loss: if the ledger writer cannot persist an entry after its retries, it must surface the failure to operators and keep the entry in a holding area, not discard it. A failed write of a closing snapshot or a moderation action is an incident. A failed write of an intermediate snapshot is a warning.

Owner crash: if the process that owns a subject dies mid-event, its replacement resumes from the last durable sequence value found in the ledger, not from memory. This means the ledger is read at recovery time, and the recovery path must tolerate an empty history for a subject that never wrote anything. The replacement must not reuse a sequence value that was already written. When uncertain, skip ahead; gaps in sequence values are harmless, duplicates are not.

Transaction retries are normal with CockroachDB under contention. The writer must treat serialization retry errors as routine and retry with backoff, and only escalate after repeated failure. Keep transactions small. A transaction that spans many subjects invites contention and is hard to reason about; prefer one transaction per batch of independent entries and accept partial batch success when entries do not depend on each other.

Clock skew: nothing in correctness may depend on comparing recorded times from different nodes. If you find a comparison like that, replace it with the sequence value or a causal reference.

## Operations and the deletion sweep

The deletion sweep is the job that removes rows older than the retention period. It is part of the spec because getting it wrong in either direction is costly: deleting early loses audit history that customers are owed, and never deleting grows storage without bound and may break promises about data minimization.

The sweep selects rows whose age is beyond 400 days, deletes them in small batches, pauses between batches, and records how many it removed. It must be safe to stop and start at any time. It must never delete a row younger than the retention period, and the age test must use the row's recorded write time with a margin that errs toward keeping. If the sweep cannot determine a row's age it keeps the row.

The sweep runs at quiet times for the cluster and yields to live events. A large event in progress is a reason to skip a run, not to push it through. Missed runs are made up later; there is no deadline inside a day.

The sweep needs its own alerting. Two conditions matter: it has not completed a run for a long time, and it removed an unusually large number of rows in one run. The second one is a possible sign of a wrong age calculation and should page someone. A dry-run mode that reports what would be deleted without deleting is required and should be used after any change to the age logic.

Backups complicate the retention story. Rows deleted from the live ledger may persist in backups for as long as backups are kept. Backup retention is set separately and should not be shorter than the ledger's own safe recovery needs, but it also should not be treated as an extension of the retention promise. If a customer asks what is kept, the answer is the live ledger rule plus a separate statement about backups.

Migrations on the ledger must be additive while events are live. Adding a column or an index is fine. Changing the meaning of an existing column is not; add a new column and migrate readers. Because rows live for a long time, readers must handle old rows written under earlier shapes for the full retention period. That means a reader written today may meet rows written more than a year ago, and tests should include old-shape fixtures.

Capacity planning waits on real numbers from the first large events. Until then, treat sizing as unknown and watch growth after each big event.

## Testing and open questions

Tests that matter most, in rough priority order. Idempotent replay of the same entry yields exactly one row. Closing snapshots are never dropped under queue pressure. Corrections never alter earlier rows and the latest correction wins for current-value reads. Recovery of an owning process continues the sequence without reuse. The sweep never touches a row younger than the retention period, including rows near the boundary, and keeps rows whose age cannot be determined. Readers cope with a correction chain whose oldest links were swept.

Property-style tests are worth the effort for the write path: generate sequences of state changes, tallies, and corrections, inject duplicates and reorderings of delivery, and check that the stored result equals the intended one.

Load tests should simulate a very large audience on a few polls with heavy moderation, since the combination of many tally snapshots and bursts of moderation actions is the realistic worst case for the ledger writer.

Open questions, to be settled before this spec is called done.

- Per-customer holds longer than the default. Needed for some contracts. No design yet; the stopgap is pausing the sweep, which affects everyone.
- Unlinking participant references after a deletion request while keeping the rows for the retention period. Options are a mapping table that can be destroyed, or per-participant keys that can be discarded. Not decided.
- Whether intermediate tally snapshots deserve a shorter life than state changes and moderation actions. The current rule keeps all rows for the same period because one rule is easier to explain and audit. Storage cost may force a revisit, in which case the spec should split the rule by kind of fact and say so clearly.
- Whether the finalization marker should be mandatory for Q&A rooms as it is for polls, since a room can stay open indefinitely.
- Where the export to the analytics warehouse draws its copy, and whether it has to finish before a row is eligible for deletion. For now the answer is that exports must be done well before the retention period ends, and nobody should rely on the ledger as the only copy after that.

If you change anything about retention, update this note first and say why. The rule that rows are kept for 400 days before deletion is the one thing here other teams may be quoting to customers.
