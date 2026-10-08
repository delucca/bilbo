---
id: 01K69XP2PA1NM04TDF1GPE991A
created: 2025-09-29T02:03-03:00
---

# ledger-entries-table size and rebuild cost

After a full VACUUM the ledger-entries-table occupied 412 GB, and rebuilding its indexes took 37 minutes. Those two numbers are the baseline for planning any maintenance on this table. The size is what is left after dead space was reclaimed, so it is not bloat. It is the real footprint of the data plus its indexes.

## Naming

ledger_raw was the previous name of ledger-entries-table. The component is called ledger-entries-table now. Old runbooks, dashboards, Terraform variables and some Go code comments may still say ledger_raw. When you see that name, it is the same table. Do not create a second table under the old name, and do not treat a search hit on ledger_raw as a different component.

## What was measured

The measurement was taken on a full VACUUM, not a plain autovacuum pass. A full VACUUM rewrites the table, holds an exclusive lock for the duration and needs free disk roughly equal to the table's size while it runs. So the 412 GB figure also tells you how much spare space the volume needs for the rewrite.

The 37 minutes covers only the index rebuild. It does not include the VACUUM itself. Add the rewrite time on top when estimating a window. I did not record the rewrite duration separately, so measure it again before promising a window to finance operations.

## Why it matters

Ledgerlark reconciles card-processor settlement files against internal ledger entries. The reconciliation jobs read ledger-entries-table heavily, and the Kafka consumers that write entries need it available. A long exclusive lock blocks both. Settlement matching would stall and mismatch flags would arrive late for the review queues.

- Plan a full VACUUM only in a low-traffic window.
- Pause the Kafka consumers that write entries first, so lag builds on the topic and nothing fails on the database side.
- Check free disk on the database volume before starting. The Terraform that sizes the volume should leave headroom above the current size.
- Expect the index rebuild to take about as long as measured, and a bit more if the table has grown.

## Quick check

Size and index size can be read from PostgreSQL with a query like this:

```sql
SELECT pg_size_pretty(pg_total_relation_size('ledger-entries-table'));
```

Quote the table name as it exists in the schema. If the query errors, the physical relation may still carry the old name ledger_raw in some environments, so check there before assuming the table is missing.

## Open items

- Record the rewrite time of the full VACUUM separately from the index rebuild.
- Find out whether a lighter option than a full VACUUM would give most of the saving, for example rebuilding only the worst indexes.
- Clean up remaining references to ledger_raw in docs and config so only ledger-entries-table is used.
- Re-measure after the next large settlement import, since the table only grows.
