---
id: 01K52QCR1JMMJWKDQE4Y76MJ71
created: 2025-09-13T20:43-03:00
---

# entry-store-schema: revision vector migration report

Migration `0042_AddRevisionVector` on entry-store-schema ran in 7 minutes over `3.4 million rows`. That is the headline number from the run, and it is the one to compare against if the migration is ever replayed on a restored copy or on another environment. This note records what the migration did, how the run went, what it means for the sync service and the audit trail, and what to watch for the next time someone touches entry-store-schema. It is written quickly from what was seen during the run, so where something was not measured it says so instead of guessing.

## What the migration did

The point of `0042_AddRevisionVector` was to give every notebook entry row in entry-store-schema a revision vector. Before it, the entry store tracked changes with a single revision counter per entry. That was enough while one writer at a time touched an entry, but LabNotebook Sync has more than one writer. A scientist edits an entry in the notebook client, an instrument gateway appends output to the same entry, and a compliance reviewer may attach signatures or comments. Each of these paths arrives through RabbitMQ and is applied by a consumer written in C# on .NET. With a single counter, two writers that started from the same base revision could both claim to be the next revision, and the loser's change was either dropped or silently reordered. A revision vector records, per writer, the last revision that writer saw, so the consumer can tell a true successor from a concurrent edit.

The schema change itself is small in shape: a new column on the entry table that holds the vector in a compact serialized form, plus a backfill that populates it for every existing row. The backfill is where the time went. For each existing entry the migration derived an initial vector from the history the table already had, so an old entry starts with a vector that reflects the writers who had touched it, not an empty one. Entries with no recorded history from more than one writer got a trivial vector with a single component.

The migration is forward-only in practice. A down script exists, but it drops the column and loses the vectors, so it should be treated as an emergency exit and not as a normal rollback. Anything written after the migration depends on the vectors being there, and dropping them would put the entry store back into the single-counter world while the consumers, if still on the new code, would reject or mishandle entries.

## The run

The run took 7 minutes over `3.4 million rows`. It was run against SQL Server in a maintenance window with the sync consumers paused. The consumers were paused by stopping them from taking new messages off the RabbitMQ queues, not by purging anything. Messages that arrived during the window simply waited in the queues and were consumed afterward. That matters for the audit trail: nothing that a scientist saved or an instrument emitted during the window was lost, and the order within each queue was preserved.

The backfill was done in batches, not as one giant update. A single statement over all the rows would have held locks for the whole duration, grown the transaction log a lot, and made a failure expensive to roll back. Batching kept each transaction short, let the log be reused between batches, and meant that if the process had died partway through, the rows already done would have stayed done. The migration is written so that it can be re-run: it only touches rows whose vector column is still empty, so a second run after a partial first run picks up where the first stopped and does not rewrite finished rows.

Seven minutes is shorter than the first rough estimate people had in mind before the run, which was closer to a full maintenance slot. The estimate had been made by extrapolating from a much smaller test copy and assuming linear growth. The real run did better than linear, most likely because the batch size fit well in the cache and because the indexes touched by the backfill were few. This is a guess about the cause; no profiling was done during the run. What is solid is the elapsed time and the row count. If someone needs to plan for a larger store later, treat the figure as a data point and not as a formula.

There were no errors during the run, no deadlocks that surfaced in the logs, and no manual intervention. Row counts before and after matched, and a sample check of entries chosen from different ages and from different writers showed vectors that agreed with the old history.

## Effect on the sync service and audit trail

After the migration, the consumer that applies incoming changes compares the vector on the stored entry with the vector carried by the incoming message. If the incoming change descends from the stored state, it is applied and the vector advances for that writer. If the stored state descends from the incoming change, the message is a stale duplicate and is acknowledged without effect. If neither descends from the other, the two edits are concurrent, and the service records both, flags the entry as needing a merge decision, and does not overwrite either. Before the migration, the concurrent case was invisible and one edit simply won.

This is the main reason the migration matters to compliance officers. The audit trail has to show who changed what and in what order, and it must not lose an edit because of a race. With vectors, a concurrent edit becomes an explicit event in the trail, not a silent overwrite. The audit records that existed before the migration were not rewritten. They are append-only, and the migration did not touch them. What the migration added is the information needed for new audit records to describe concurrency correctly. For old entries, the initial vectors were derived from history, so the first concurrent edit on an old entry is judged against a sensible baseline.

Instrument output follows the same path. Instrument gateways publish results to RabbitMQ, and large raw files go to Azure Blob Storage with a reference stored on the entry. The reference is part of what a revision changes, so an instrument appending a result produces a new revision from the gateway's component of the vector. The blob itself is not versioned by the vector; the blob store keeps its own immutability rules. The vector only orders the references. If a blob is uploaded but the entry update loses a concurrent race, the blob stays and the reference shows up in the merge decision, so nothing is orphaned without a trace.

## Things that changed for developers

The entry model in the C# code gained a vector property, and the data access layer reads and writes it with the rest of the row. Any code that builds an entry row by hand, such as test fixtures and seed scripts, must set a vector or let the default produce a single-component one. Fixtures that were written before the migration and still insert rows with no vector will leave the column empty, and the consumer treats an empty vector as the oldest possible state, which can make a fixture look stale against a newer message. When a test fails with an entry being treated as a duplicate or as concurrent unexpectedly, check the fixture's vector first.

Serialization of the vector is part of the contract between the stored row and the messages on RabbitMQ. Messages now carry the vector in a header or in the body, depending on the producer, and both forms are accepted by the consumer. Producers older than the migration do not send a vector at all. The consumer handles that by treating the message as coming from a writer with no known context, which is safe but is more likely to be classed as concurrent. The right fix for noisy concurrency flags from an old producer is to upgrade the producer, not to loosen the comparison.

The comparison logic should stay in one place. There was a temptation during review to add a quick check in a controller to skip the consumer for trivial updates. That would produce two definitions of what a successor is, and the audit trail would then depend on which path an edit took. Keep all vector comparison inside the shared component the consumer uses.

## Operational notes and risks

If this migration or one like it has to run again on a bigger copy, a few points are worth remembering. First, pause the consumers by stopping consumption, and confirm that the queues are building up and not dropping messages before starting. Second, run the backfill in batches and keep it restartable, as this one is. Third, check free space for the transaction log ahead of time; the run here did not hit a limit, but it was not measured how close it came. Fourth, after the run, compare row counts and spot check vectors before resuming the consumers, because resuming brings a burst of queued messages and any wrong baseline vector will turn into a pile of false concurrency flags that are tedious to unwind.

A risk that remains is that the vectors grow with the number of distinct writers over an entry's life. For most entries the writer set is small: the author, an instrument gateway, perhaps a reviewer. Long-lived shared entries with many contributors will carry bigger vectors. This has not been measured and is not a problem today, but if the vector column starts to show up in storage or index costs, the design leaves room to prune components for writers that are retired, as long as pruning is itself recorded in the audit trail.

Another risk is clock-free ordering being misread. The vector does not say when something happened, only what each writer had seen. Reports that sort by vector as if it were a timestamp will be wrong. Timestamps are still stored separately and should be used for display and for time-based queries. The vector is for causality only.

Restore and failover also deserve a thought. If the entry store is restored from a backup taken before the migration, the schema and the code must be brought forward together, and the migration must be re-run on the restored copy. Running new consumers against an old schema fails loudly, which is good. Running old consumers against the new schema works for reads but would write rows without updating the vector, which quietly weakens the guarantee. The deployment order is therefore schema first, then consumers, and old consumers should be retired before any more writes are allowed.

## What was not checked

The run did not include a load test against the new schema with many concurrent writers, so the throughput of the consumer with the vector comparison in place has not been compared properly against the earlier counter-based path. Informally the consumer kept up with the backlog that built during the window, which suggests the overhead is modest, but that is anecdotal. The effect on query plans for the reporting views was also not studied in detail. Those views do not read the new column, so no change is expected, but no one looked at the plans afterward.

The behaviour of the merge decision flow from the notebook client was tested only by hand on a few entries. The flag shows up and both edits are kept. How compliance officers want to record the resolution of a concurrent edit, and whether the resolution should itself carry a signature, is still an open question with the compliance side and is not settled by this migration.

## Follow-ups

Keep this short list near the component. Add an automated check to the migration test suite that runs the backfill on a generated store and verifies that every row ends with a nonempty vector. Add a load test for the consumer with several writers per entry. Decide with compliance how merge resolutions are recorded. Review whether old producers still exist anywhere and retire them. Revisit vector pruning only if sizes prove to be an issue. Finally, whenever someone quotes how long migrations on entry-store-schema take, point them to this one: `0042_AddRevisionVector`, 7 minutes, `3.4 million rows`, batched, restartable, consumers paused, no errors.
