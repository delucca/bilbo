---
id: 01KNTMNQVSGEWZWQEZ4JFJ2JZ6
created: 2026-04-09T23:48-03:00
---

# ledger-entries-table: general direction

We settled on a general shape for the ledger-entries-table and this note keeps the reasoning so nobody redoes it. It is a direction, not a spec. Details live in code and migrations.

## Context

Ledgerlark matches processor settlement files against internal ledger entries. The ledger-entries-table is the internal side of that match. Everything the reconciler flags starts from rows here, so the shape of this table drives most of the matching code in Go.

## Decision

Treat the ledger-entries-table as an append-oriented record. Rows are written once and corrected by adding new rows, not by rewriting old ones. Matching state lives next to the entries but is kept separate from the money facts.

## Why append-oriented

Finance ops want to see what the ledger said at the time a mismatch was flagged. Rewriting rows loses that. Append-only also keeps the Kafka consumers simple, since a replayed event just fails to insert a second time.

## Money facts versus match state

Amounts, currency, and the link to the originating event are the money facts. They do not change after write. Match status and review notes are the mutable part and should not sit in the same place as the money facts if we can avoid it.

## Idempotent writes

Every entry carries a stable key taken from the upstream event, and the table enforces uniqueness on it. Consumers rely on the database to reject duplicates rather than checking first in application code.

## Ingestion path

Entries arrive through Kafka consumers written in Go. The consumer writes to PostgreSQL and only then commits its offset. We accept occasional redelivery because of the idempotent key.

## Reads for matching

The matcher reads by processor reference and by time window. Indexing should favor those two access patterns. We do not index for ad hoc reporting; that goes to a separate read path.

## Reporting

Finance teams will want exports and dashboards. Those should not hit the ledger-entries-table directly under load. Use a replica or a derived table.

## Corrections

A wrong entry is fixed with a reversing entry plus a new one. Reviewers can see both. Nobody edits the original, including by hand in production.

## Retention

We keep entries for as long as finance and audit need them. The exact period is a policy matter owned outside engineering, so it is not recorded here. Plan for partitioning by time so old data can be moved or dropped cleanly later.

## Schema changes

Migrations on this table must be additive where possible. Avoid long locks. Backfills run in batches and are separate from the schema step. Terraform manages the database infrastructure, not the table schema.

## gRPC surface

Other services read entries through the gRPC API, not by connecting to the database. This lets us change the table without coordinating with callers. The API exposes the money facts and the match state as distinct parts.

## Multi-currency

Amounts are stored in minor units with an explicit currency on each row. No implicit conversion happens in this table. Conversion, if needed, is a matching concern.

## Rejected options

Updating rows in place was rejected because of audit and replay problems. One wide table holding everything, including review comments, was rejected because it mixes immutable and mutable data.

## Risks

Reversal chains can get long for problem accounts and make reads awkward. Partitioning choices are hard to change later. Match state kept separately means a join on the hot path.

## Open questions

Whether match state should be its own table or a view is still open. The retention policy has no owner yet on our side. Revisit both before the next large schema change to the ledger-entries-table.

## Pointers

Check the migrations directory, the consumer package, and the gRPC service definitions for current detail. If any of them contradicts this note, the code wins and this note needs an update.
