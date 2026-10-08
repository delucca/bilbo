---
id: 01KXJNZWAKJ4KS5CFASP3ARKNQ
created: 2026-07-15T07:43-03:00
---

# recon-results-table write idempotency

Writers to recon-results-table use INSERT with `ON CONFLICT DO NOTHING` on the unique pair of settlement and entry. This keeps matcher reruns idempotent: running the matcher again over the same settlement file does not create duplicate result rows and does not fail on rows that already exist. Decided, and every writer should follow it.

## Context

The matcher in Ledgerlark compares card-processor settlement lines against internal ledger entries and writes one result row per settlement/entry pair into recon-results-table. Those rows are what finance operations reviewers look at when they work through flagged mismatches.

Reruns happen a lot in practice. The matcher gets rerun after a crash, after a Kafka consumer rebalance replays messages, after a processor re-delivers a corrected settlement file, and when someone backfills a period by hand. Kafka gives at-least-once delivery for us, so the same work arriving twice is normal and not an incident. The write path has to tolerate it without any coordination between instances.

Without a guard, a rerun either duplicates rows (reviewers see the same mismatch twice, counts are inflated) or hits a unique violation and aborts the batch halfway, leaving a partial state that is hard to reason about.

## Decision

The table carries a unique constraint over the settlement and entry pair. Every writer does a plain INSERT and adds `ON CONFLICT DO NOTHING`. If the pair already has a row, the insert is silently skipped and the existing row stays exactly as it was.

Why DO NOTHING and not an upsert that overwrites:

- A row may already have been touched by a reviewer (status, notes, assignment). Overwriting on rerun would wipe that work.
- The first result for a pair is the one that was surfaced, and keeping it makes the history stable and easy to explain.
- It is the cheapest option: no read before write, no locking beyond the unique index check, and safe under concurrent matcher instances because the database arbitrates.

Why not check-then-insert in Go: two instances can both see no row and both insert. The constraint is the only thing that is actually race-free, so the application logic should not try to duplicate it.

## Consequences and gotchas

- A skipped insert is not an error. Code must not treat zero rows affected as a failure. Count it as "already present" in metrics if you want visibility, but do not retry and do not log it at error level.
- Because existing rows are never updated, a rerun with a changed matching rule will not change old results. If a result genuinely needs to be recomputed, that has to be an explicit, separate operation (a deliberate delete or a dedicated update path, reviewed on its own), not a side effect of the normal write.
- The idempotency depends on the unique constraint existing. Anyone changing the schema, for example in a migration or in the Terraform-managed database setup, must keep the unique pair intact. Dropping or loosening it silently turns reruns back into duplicate generators.
- If the definition of the key ever changes (say a settlement can legitimately match the same entry in two roles), the conflict target and the constraint must change together. Do not widen one without the other.
- Bulk writes should still go through the same statement shape, batched. Do not switch to a copy-style load that bypasses conflict handling unless it loads into a staging table first and then inserts with the same clause.

## Rejected alternatives

- Upsert with DO UPDATE: rejected because it overwrites reviewer-visible state and makes reruns change data.
- Application-level dedupe with a cache or a Kafka-side exactly-once setup: rejected as more moving parts, and still not safe across restarts and replays.
- Delete and reinsert per settlement before each run: rejected because it loses reviewer state and leaves a window where results are missing.

## Where to look

The writer code is in the Go matcher service, in the repository code that handles result persistence. Look for the insert statement there when checking that the clause is present. If you add a new writer, copy that statement shape and add a test that runs the same input twice and asserts the row count stays the same.
