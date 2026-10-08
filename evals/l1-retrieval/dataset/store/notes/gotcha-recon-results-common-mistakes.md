---
id: 01JZ325A55AWWDAMTB86RDG8Q2
created: 2025-07-01T10:17-03:00
---

# recon-results-table: common mistakes

Notes on things that keep going wrong around recon-results-table. None of this is exotic. Most of it comes from treating the table as a simple log when it is the thing reviewers actually read from.

## Treating it as append-only when it isn't

People assume every row is written once. Reruns of a settlement file touch the same logical rows again. If you insert without thinking about how a rerun matches an earlier row, you get duplicates, and reviewers see the same mismatch twice.

## Writing rows outside the consumer's idempotency path

The Go consumer reading from Kafka can see the same message more than once. Any write to recon-results-table that skips the idempotent path will double-count after a redelivery. Test with a replayed message, not only a clean run.

## Mixing up match status and review status

These are different things. Match status says what the reconciler found. Review status says what a human did about it. Updating one to mean the other hides mismatches or reopens closed ones. Keep the writers for each separate.

## Overwriting a human decision on rerun

A reconcile rerun that recomputes a row can clobber a reviewer's resolution. Reruns should leave reviewer fields alone and only change the computed fields. Check this before shipping any change to the upsert logic.

## Money handled as floats

Amounts in the table must be exact decimals or integer minor units. Converting through floating point anywhere in the Go code, or in a gRPC message field choice, causes tiny mismatches that look like real ones. Also keep the currency next to every amount; comparing across currencies without it is a classic false mismatch.

## Ignoring timezones and settlement dates

Processors report dates in their own convention. Ledger entries use another. Rows that straddle a day boundary get flagged as unmatched when they are fine. Be explicit about which date a column holds.

## Adding indexes or columns casually

The table is queried heavily by the review UI and by batch jobs. A new index on a hot table can slow ingestion, and a new non-null column without a default can lock things during migration. Plan schema changes so they are safe to apply while writes continue.

## Long-running queries against the live table

Ad hoc analysis on the primary blocks vacuum and holds back cleanup. Use a replica or a bounded query. Large deletes done in one statement cause the same kind of trouble.

## Schema drift between Terraform and migrations

Database settings and roles are in Terraform, table definitions are in migrations. Changing one and forgetting the other leaves environments different. Staging passing does not prove production has the same grants or settings.

## Assuming ordering

Rows do not arrive in settlement-file order, and the table gives no order unless you ask for one. Do not rely on insertion order for pagination or for pairing; sort on a stable key.

## Before changing anything

Replay a mixed batch with duplicates, reruns, and a reviewer-resolved row, then compare the table before and after. If a resolved row changed, something is wrong.
