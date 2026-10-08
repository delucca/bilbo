---
id: 01K71X3JH95SBH3ZBKE3P4F980
created: 2025-10-08T09:35-03:00
---

# entry-store-schema index review: IX_NotebookEntry_UpdatedAt

The index review of entry-store-schema is done. The conclusion is that `IX_NotebookEntry_UpdatedAt` is never used by any query plan and can be dropped. This note records what the review looked at, why that conclusion holds, what could still go wrong, and how to drop the index safely. Anyone reading only this note should be able to act on it.

## Verdict

`IX_NotebookEntry_UpdatedAt` is a nonclustered index in entry-store-schema, on the notebook entry table in SQL Server. It is on the entry's last-updated timestamp. No query plan the review examined uses it, for a seek or for a scan. It is safe to drop.

The index is not free. Every insert into the entry table writes to it. Every update that touches the timestamp column rewrites its entry. In this system nearly every edit touches that column, because the sync service stamps it each time an entry changes. So the index adds write cost on the hottest path in the store and gives nothing back on reads. It also takes space in the data files and in backups, and it adds time to index maintenance jobs.

The call was made as a review decision, not a measurement of one bad day. The reasoning is below, so a later session can check it again if the workload changes.

## What the review covered

The review looked at the indexes on the tables in entry-store-schema as a set, not only this one. The question for each index was the same: does a real query in this product need it, and does the optimizer pick it?

Sources of evidence:

- The query shapes the application code issues against the entry store. The C# data access layer is small, and the queries are easy to list by reading it.
- The plans SQL Server actually chose for those queries, looked at in the plan cache and in query store history, not just the plans one would guess from the SQL text.
- The index usage statistics the server keeps. These count seeks, scans, lookups and updates per index. They reset on restart, so they were read with that in mind and compared against the query store history, which survives restarts.
- The reporting queries that compliance officers run for audit trail views, since those are the likeliest place for a query by recent change time.

The finding for `IX_NotebookEntry_UpdatedAt` was consistent across all of these. The usage statistics showed writes to the index and no reads. No cached plan or query store plan referenced it. No reporting query referenced it.

## Why nothing uses it

The obvious reason to have an index on a last-updated column is to find recent changes. The sync service looks like a natural consumer of that, so the review checked it closely.

The finding is that the sync path does not find changes by scanning the entry table for recent timestamps. Changes arrive as messages. Instrument output and notebook edits are published through RabbitMQ, and the consumer handles each message by entry key. The lookup is by key, which the clustered key and the other indexes already serve. Nothing in the hot path asks the question "which entries changed since time T" against SQL Server.

The audit trail is the other place one might expect it. The audit trail is append-only and has its own history records, with its own ordering. Compliance views read the history records, not the current-state entry table, so they never need the entry's last-updated column as an access path. Large attachments and instrument files live in Azure Blob Storage, and the entry table only holds references to them, so blob lifecycle work does not use this index either.

So the index exists, most likely, because it seemed useful when the schema was first laid out, and no feature ever came to depend on it. That is a common history for an index like this, and nothing in the review contradicted it.

## Caveats and risks

Things that could make the verdict wrong, and what was done about each:

- **Rare queries.** A job that runs once a quarter or once a year might use the index and not show up in a short window of statistics. The review leaned on reading the code for query shapes, not only on observed usage, to cover this. No such job was found. If someone knows of a manual or scheduled report that filters or sorts by last-updated time, say so before the drop.
- **Ad hoc queries by people.** Scientists and compliance officers sometimes run their own queries against a reporting copy. If those queries run on the primary store and depend on this index, they would get slower. The review found no sign of it, but ad hoc use is the hardest thing to rule out from statistics alone.
- **Statistics reset.** Usage counters reset on a restart, so a low count can mean a short window and not no use. This is why query store history and code reading were used as well.
- **Plan changes after the drop.** Dropping an index can change plans for queries that never used it, mostly through statistics and maintenance side effects. This is a small risk, but it is why the drop should be watched, not fired and forgotten.
- **Forced or hinted plans.** If any query uses an index hint or a forced plan naming this index, the drop would make it fail instead of just slowing it down. The code review found no hint naming it. A second search of the repository and of any stored procedures for the index name is part of the rollout below.

None of these is a reason to keep the index. They are reasons to do the drop carefully and to be able to reverse it.

## Rollout

Do it in a non-production copy first, then in production in a quiet window. The steps:

1. Search the repository, the deployed stored procedures and any scripts for the index name, to confirm no hint or forced plan refers to it.
2. Script the exact definition of the index from the live database, and save the script with the change. The key columns, included columns, filter and options should come from the live definition, not from memory or from the original migration, in case they drifted.
3. In the non-production copy, drop the index and run the application's normal workloads, including the audit and reporting paths. Compare write latency and plan choice before and after.
4. In production, drop the index through a normal schema migration so the change is tracked in source control and the audit of the schema is complete. This product enforces audit trails for its users, so its own schema changes should leave one too.
5. Watch write latency, lock waits on the entry table and any query regressions for a period after the drop. Expect writes to get a little cheaper. Expect reads to be unchanged.

The statement itself is short:

```sql
-- drop only after the definition has been scripted and saved
DROP INDEX IX_NotebookEntry_UpdatedAt ON <entry table>;
```

The table name is left out on purpose here; take it from the live database when scripting the definition.

## Reversal

If something does need the index after the drop, recreate it from the saved definition. Building it on a large table takes time and some resources, so do it online if the edition allows, and in a quiet window. Because the definition is saved in step two, reversal does not depend on anyone remembering how the index was built.

If a regression shows up, first confirm it really is the missing index. Look at the plan of the slow query and see whether it asks for an index on the last-updated column. If it does, ask whether the query should be changed instead. A query that needs recent changes from the entry table may be better served by the history records or by the message flow, which are the designed paths for that question. Recreating the index should be the last answer, not the first.

## Follow-ups

- Record the drop in the schema migration history, with a pointer back to this note.
- Add a line to the schema conventions that an index needs a named query or feature behind it. Several indexes in this store were reviewed in the same pass, and the lesson from this one is that an index with no named consumer tends to stay forever.
- Revisit the question if a feature ever needs changes since a given time directly from SQL Server. In that case design the index for that query, with the right leading column and included columns, instead of reviving this one by reflex.
- Keep this review in mind when reading usage statistics in future: check for resets, and cross-check with query store and with the code.

## Status

The conclusion stands: `IX_NotebookEntry_UpdatedAt` in entry-store-schema is not used by any query plan and can be dropped. Whether it has actually been dropped in production is not recorded in this note; update this note when the migration runs, including the date and the outcome of the watch period after it.
