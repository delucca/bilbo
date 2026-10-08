---
id: 01KJX9HKYR49CA61C06CBAJASH
created: 2026-03-04T17:45-03:00
---

# Review themes: ledger-entries-table

Notes from going over recent reviews that touched ledger-entries-table. Nothing here is a decision. It is the pattern of what reviewers keep raising, so the next person does not start cold.

## Why this table gets so much review

ledger-entries-table is what every settlement comparison reads against. A small change in how rows are shaped or indexed shows up in matching, in the review queue and in finance reports. Reviewers are careful with it for that reason, and the comments repeat.

## Schema changes

Most comments are about migrations. Reviewers want each change to be safe on a table that is large and being written to while the migration runs. They ask whether a change rewrites the table, whether it takes locks that block writers, and whether it can be rolled back. Adding columns and constraints in separate steps comes up often.

## Indexes

Index additions get questioned. Reviewers ask which query the index serves and whether an existing one already covers it. Write cost on a busy table is the other worry. Dropping unused indexes is welcomed but needs evidence of non-use.

## Money and currency handling

Amounts and currency fields get close reading. Reviewers push for exact numeric types, never floating point, and for the currency to travel with the amount. Rounding rules should be stated in one place, not scattered across callers.

## Immutability and corrections

The recurring view is that entries should be append-only in spirit. Fixes should come as new entries that reverse or adjust, not edits in place. Reviews flag any update path that overwrites history, since audit depends on it.

## Idempotency of writes

Ingestion can replay. Reviewers ask how duplicate writes are prevented, usually through a natural key or a unique constraint, and what happens when the same event arrives twice from Kafka. They dislike duplicate protection that lives only in application code.

## Time and ordering

Timestamps are a steady source of comments. Reviewers want clarity on which time a column holds: processor time, ingestion time or booking time. They want time zones handled explicitly. Ordering assumptions across Kafka partitions get challenged.

## Matching status and flags

The columns that record match state and review flags are read by several consumers. Reviewers ask that state transitions be clear, that new states be added with all readers in mind, and that a flag never be silently cleared by a later job.

## Query patterns

Reviewers look at how the Go code queries the table. Common points are unbounded scans, missing filters on date ranges, and N+1 patterns in the review path. Batch reads are preferred over row-by-row lookups.

## Retention and growth

Table growth is raised periodically. Reviewers ask about partitioning, archival and how old entries stay reachable for audits. No firm answer is recorded here. It stays an open topic.

## Access and permissions

Reviews mention which roles may write to the table. The preference is narrow write access for the ingestion path and read-only access for reporting and review tools. Terraform-managed grants should match what the code expects.

## Testing expectations

Reviewers want migration tests against realistic data shapes, not only empty tables. They also want cases for duplicates, late arrivals and reversed entries. Tests that depend on wall-clock time get flagged.

## Observability

Requests recur for metrics on write failures, constraint violations and slow queries touching the table. Reviewers prefer alerts on symptoms finance would notice, such as ingestion stalling.

## Open threads

Partitioning and archival remain unsettled. Time column semantics deserve one written reference. A shared description of allowed state transitions would cut repeated comments.
