---
id: 01JZWWWTMB143V0NHNSTMBJV84
created: 2025-07-11T11:05-03:00
---

# ledger-entries-table design

The ledger-entries-table holds the internal ledger entries that Ledgerlark matches against card-processor settlement files. It lives in PostgreSQL and is partitioned monthly by the posting timestamp. The partition clause is `PARTITION BY RANGE (posted_at)`, with one partition per calendar month. This note records why it is shaped that way and what to watch for when touching it.

## Why partition by month

Reconciliation works on a settlement period, usually a day or a few days, and almost always inside one month. Queries that join settlement rows to ledger entries filter on posted_at, so PostgreSQL can prune to one or two partitions instead of scanning the whole history. Finance teams at marketplaces generate a lot of entries, and the table only grows, so a single heap would make vacuum and index maintenance slow over time.

Monthly also matches how finance operations close their books. Once a month is closed, its partition rarely changes, which makes it easy to reason about what is still mutable.

## Layout

```sql
CREATE TABLE ledger_entries (
    -- columns omitted
    posted_at timestamptz NOT NULL
) PARTITION BY RANGE (posted_at);
```

The partition key must be part of any primary key or unique constraint on the table. That is a PostgreSQL rule for partitioned tables, and it affects how we enforce uniqueness of an entry. Do not assume a unique index on the entry id alone will work.

## Operating it

- Partitions must exist before entries arrive for that month. If an insert has no matching partition, it fails, so creating the next month's partition ahead of time is part of the routine. Terraform does not manage individual partitions; the Go service or a scheduled job creates them.
- Entries come in through the Kafka consumer path, so a missing partition shows up as a stalled consumer and a growing lag, not as a user-facing error. Check there first.
- Old partitions can be detached and archived without a big delete. Prefer detach over DELETE.
- Late-arriving entries with an old posted_at land in the old partition. That is correct, but it means a closed month can still change. The reconciliation job should not assume closed means frozen.

## Open points

- Retention policy for detached partitions is not settled.
- Whether to use a default partition as a safety net is undecided. It would hide missing-partition problems, which is the main argument against it.
- Any gRPC endpoint that queries by entry id without a time bound will scan every partition. Callers should pass a time range where they can.
