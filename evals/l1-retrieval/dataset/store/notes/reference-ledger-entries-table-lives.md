---
id: 01M0W94787DEH7SZF5DNMZJ3DY
created: 2026-08-25T07:58-03:00
sources:
  - "code: db/migrations/0042_ledger_entries.sql"
---

# ledger-entries-table reference

Quick reference for ledger-entries-table, the PostgreSQL table in Ledgerlark that holds internal ledger entries. Settlement lines from the card processor are compared against these rows, and mismatches get flagged for finance operations review. The DDL is in db/migrations/0042_ledger_entries.sql. Read that file first; this note only points at it and adds context.

## Where the schema lives

The single source of truth for the shape of ledger-entries-table is db/migrations/0042_ledger_entries.sql. Column names, types, constraints and indexes are defined there. If the table in a running database looks different from that file, the database has drifted or a later migration changed it. Check the migrations directory for newer files before assuming the DDL is wrong.

## What the table is for

ledger-entries-table stores the marketplace's own record of money movements. Reconciliation reads from it and does not treat it as a cache of processor data. Settlement files are parsed elsewhere, and the match happens by comparing parsed settlement lines with rows from this table.

## Who reads it

The Go reconciliation service is the main reader. It queries the table when matching a settlement batch. Other consumers, such as review tooling, should go through the service's gRPC API rather than querying the table directly, where that is practical.

## Who writes it

Entries arrive from the marketplace side, typically via Kafka events consumed by a Go writer. Treat the table as append-oriented. Corrections should be new entries, not edits to old rows, so that a past reconciliation can be reproduced later.

## Changing the schema

Do not edit db/migrations/0042_ledger_entries.sql after it has been applied anywhere. Add a new migration instead. Applied migrations are shared history, and rewriting one makes environments disagree about what the table looks like.

## Infrastructure notes

The database itself is provisioned with Terraform. Table-level changes go through migrations, not Terraform. Keep those two concerns apart: Terraform owns the instance and access, migrations own the tables.

## Gotchas

- Matching depends on the table's keys and indexes. A change that drops or alters them can make reconciliation slow or wrong without any error.
- Backfills into the table can produce false mismatches if run while a settlement batch is being matched.
- Amount and currency handling matters for matching. Check the DDL for the exact column types before writing queries that compare against settlement amounts.

## Open items

Nothing recorded yet about retention or partitioning. If that gets decided, add it here and link the migration that implements it.
