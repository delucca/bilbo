---
id: 01KCZ1J7EDVSHJH8JAJK7W1G08
created: 2025-12-20T20:29-03:00
---

# Schema review of recon-results-table

Review of the schema of recon-results-table, done as a read-through of the table definition, its write paths and how finance operations use it. Outcome: the storage settings are fine as they are. The setting `fillfactor = 90` is sufficient, because rows in recon-results-table are updated only once, on review. No change to the table is needed on this point. The rest of this note records why, what else was looked at, and what is still open.

## Verdict

Keep `fillfactor = 90` on recon-results-table. The reasoning is about update behaviour, not table size. A row is inserted when the matcher finishes comparing a settlement line against the ledger. After that it is touched one more time, when a reviewer marks it. Nothing rewrites it repeatedly, so the free space left in each page only has to absorb one new row version per row, not a stream of them.

## Why the fillfactor is enough

PostgreSQL keeps the old row version in the page until vacuum clears it. With a single update per row, the extra space needed is about one row version per row, and the reserved free space covers that for most pages. That lets the update stay on the same page where possible and keeps the heap compact. Raising the reserved space would waste disk and cache for no gain. Lowering the reserve would push more review updates onto new pages and bloat the indexes sooner. Neither looked worth doing.

## Row lifecycle

The life of a row is short and fixed. The matcher writes it with a status of matched or mismatched. Matched rows are normally never touched again. Mismatched rows sit in the review queue until someone on the finance team resolves them. The review step sets the outcome, the reviewer and the time, and that is the single update. Rows are not reopened in the normal flow. If a reopen feature is ever added, this review has to be redone, since it would break the one-update assumption.

## Write paths checked

There are two writers. The first is the Go matcher service, which consumes settlement events from Apache Kafka and writes results in batches. The second is the review API, exposed over gRPC, which applies the reviewer's decision. I looked for any other code that updates the table, such as backfills or status sweeps, and found none that touch a row more than once. Retries on the matcher side are idempotent inserts, so a replayed Kafka message does not create a second update to an existing row.

## Indexes and hot updates

The review update changes the status and reviewer columns. If any index covers the status column, then the update cannot be a heap-only update, and the free space from the fillfactor will not save an index entry. The queue lookup index is the one to watch here. It is partial, covering unreviewed rows only, so a review update removes the row from it, which is the intended behaviour. Other indexes are on the identifiers used to join to ledger entries and are not touched by review. I did not find a reason to restructure them.

## Vacuum and bloat

Because updates are rare and bounded, dead tuples from review updates are modest. Autovacuum defaults looked adequate for this table. The bigger source of churn is insert volume at settlement time, which autovacuum handles for visibility and statistics. I did not tune per-table vacuum settings and do not think we should until monitoring shows a problem. If bloat shows up, check the review backlog first, since a large backlog of old unreviewed rows delays all of the updates to one burst.

## Retention and partitioning

The table grows with settlement volume and is not trimmed by this review. Partitioning by settlement date was discussed as a way to drop old data cheaply. It was not decided here. It would not change the fillfactor conclusion, since each partition would see the same single update per row. Any retention rule has to come from the finance operations side, because the results are part of the audit trail for mismatches.

## Risks to the conclusion

The conclusion rests on one assumption: one update per row, on review. It breaks if reviewers can edit a decision, if a second automated process starts annotating rows, or if the matcher is changed to re-evaluate old rows when ledger entries arrive late. Late ledger entries are the most likely path. Today a late entry produces a new result row rather than changing the earlier one, and that should stay true. Anyone changing that behaviour should come back to this note.

## Things not covered

This was a schema and storage review. It did not measure query plans under production load, and it did not review the column types or constraints in depth beyond a quick look. Terraform manages the database instance, but the table settings live in the migrations, not in Terraform, so changing the setting means a new migration, not an infrastructure change. I did not check how the setting behaves on the existing replicas beyond reading the migration history.

## Follow-ups

Add a short comment in the migration that sets the fillfactor, stating the one-update assumption, so the next person does not guess. Add a dashboard panel for table bloat and the share of updates that are heap-only, so a drift away from the assumption shows up early. Ask the review API owners to confirm that no edit or reopen flow is planned. If any of these turns up a second update path, revisit the setting.

## Decision record

Decision: leave the table as it is and keep `fillfactor = 90` on recon-results-table. Reason: rows are updated only once, on review. Revisit if the row lifecycle changes, if bloat appears in monitoring, or if the share of heap-only updates drops noticeably.
