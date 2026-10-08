---
id: 01KRM14A5QZEHFC9Y7HAPC45K0
created: 2026-05-14T16:58-03:00
---

# sync-outbox-relay design: polling interval

This note replaces the earlier note "sync outbox relay polls"; the new value is that sync-outbox-relay polls the table sync.Outbox every 250 ms, down from the old 500 ms.

## Summary

sync-outbox-relay is the part of LabNotebook Sync that reads pending rows from the sync.Outbox table in SQL Server and hands them to RabbitMQ. It polls the table every 250 ms. The previous interval was 500 ms. Nothing else about the relay's contract changed with this edit.

## Why the interval was shortened

Scientists see a lag between saving a notebook entry and seeing the matching instrument data attached to it. Half of that lag was the wait for the next poll. Cutting the interval to 250 ms halves the worst-case wait before a row is picked up. Compliance officers also like the audit trail to trail the real event by as little as possible, so shorter is better for them too.

## How the relay works

The relay is a long-running C# worker on .NET. On each tick it reads a batch of unsent rows from sync.Outbox, publishes each one to RabbitMQ, and marks the row as sent only after the broker confirms it. Payloads that refer to instrument files point at objects in Azure Blob Storage; the relay does not copy the blob bytes itself.

## Config sketch

```text
sync-outbox-relay
  table:         sync.Outbox
  poll interval: 250 ms   (was 500 ms)
```

## Delivery and ordering

Delivery is at-least-once. If the relay dies between publish and the mark-as-sent step, the row goes out again on restart. Consumers must tolerate duplicates. Row order within one notebook entry is kept; no order is promised across entries. The shorter interval does not change either guarantee.

## Load on SQL Server

Polling twice as often means twice as many empty reads when the outbox is idle. The query is a cheap indexed read, so this was judged acceptable. Watch it if many relay instances run against one database, because the idle cost grows with the instance count.

## Audit trail concerns

An outbox row is evidence that a change happened, so the relay must never delete rows to save space. It only marks them sent. Any retention or archive step belongs to a separate job and must follow the compliance retention rules.

## Open points

- Confirm after a few days in production that database CPU stays flat.
- If load becomes a problem, consider backing off while the outbox is empty instead of raising the fixed interval again.
- The interval is not yet documented in the operator runbook; update it there.
