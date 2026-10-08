---
id: 01KRP1QQ34C0H2VRRX38SQ87X8
created: 2026-05-15T11:48-03:00
---

# Slow review query on recon-results-table: partial index finding

The review query against recon-results-table was too slow for the finance ops screen. EXPLAIN showed a sequential scan over 90 million rows in recon-results-table. A partial index on mismatch rows cut the review query from 8.2 s to 40 ms. This note records what we looked at, why the partial index fits, and what to watch afterwards.

## Symptom

Reviewers open the mismatch queue and wait. The query behind that screen asks for rows flagged as mismatches, newest first, with a few filters (processor, settlement date range, review status). It took 8.2 s in the slow case. That is too long for an interactive page and it also held a connection from the Go service pool for the whole time, so under load other gRPC calls queued behind it.

## What EXPLAIN showed

EXPLAIN on the review query showed a sequential scan over 90 million rows in recon-results-table. The planner had no usable index for the mismatch predicate, so it read the whole table and threw away almost everything. Matched rows are the vast majority of the table. Mismatches are a small fraction, which is the whole point of the product: most settlement lines reconcile cleanly and only a few need a person.

## Why a partial index

A normal index on the status column would work but would be large, because it would carry an entry for every matched row we never query for review. A partial index only covers mismatch rows. It stays small, fits in memory, and is cheap to maintain because matched rows (the bulk of inserts) never touch it. The query predicate has to match the index predicate, otherwise the planner will not use it.

## Result

After adding the partial index on mismatch rows, the same review query dropped from 8.2 s to 40 ms. EXPLAIN no longer showed the sequential scan; it used the partial index. The timing was measured on the same data set before and after, so the comparison is fair, though it was one dataset and not a full benchmark across processors.

## Things to keep in mind

- The review query must keep the same mismatch condition as the index definition. If someone rewrites the filter in a slightly different form, the planner may fall back to the sequential scan.
- When a mismatch is resolved and its status changes, the row leaves the index only if the index predicate depends on that status. Check how the predicate was written before changing the review workflow.
- Terraform does not manage this index; schema changes go through the normal migration path for the Go service.

## Write cost and size

Because matched rows do not enter the index, the extra write cost on the hot ingest path from Kafka consumers is small. Index size is proportional to the mismatch count, not to the table. Still worth watching if a processor starts sending bad files and mismatches spike, since the index then grows quickly and so does the review queue.

## Open questions

- Whether resolved mismatches should stay in the index for an audit view, or whether a second index is better for that.
- Whether the table should be partitioned by settlement date later. The partial index solves the review query, not general growth of recon-results-table.
- Whether other queries (exports, per-processor reports) still scan the table and need their own indexes.

## How to re-check

Run EXPLAIN with actual timing on the review query against a production-sized copy. Look for an index scan on the partial index and not a sequential scan. If the plan regresses, compare the query predicate with the index predicate first, then check that table statistics are fresh, since stale statistics can push the planner back to a full scan.

## Follow-ups

Add a note to the migration that explains why the index is partial, so nobody widens it to all rows by habit. Consider a slow-query alert on the review endpoint so a regression from 40 ms back toward 8.2 s is noticed before reviewers complain.
