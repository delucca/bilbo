---
id: 01KMENERVGRGFAQX9F8VR1FD7P
created: 2026-03-23T21:55-03:00
sources:
  - "code: src/OutboxRelay/OutboxPoller.cs"
---

# sync-outbox-relay design

sync-outbox-relay is the small worker that moves committed changes out of SQL Server and onto RabbitMQ. It polls the table `sync.Outbox` every `500 ms` and publishes each row to the RabbitMQ exchange `lnb.sync`. Everything else in this note is context for that one behaviour: why it is built as a poller, what the guarantees are, and what to check when it misbehaves. A reader who only has this note should be able to tell how entries and instrument output leave the database and reach the other services.

## What it is for

LabNotebook Sync keeps electronic lab notebook entries in step with instrument output and enforces audit trails. Research scientists write entries and attach instrument files. Compliance officers read the audit trail and need it to be complete and in order. Whenever the main application changes a notebook entry, links an instrument result, or records an audit event, it writes the business change and an outbox row in the same SQL Server transaction. The outbox row is the only thing sync-outbox-relay cares about.

The point of the outbox is that the application never talks to RabbitMQ inside a business transaction. If it did, a broker outage would either block scientists from saving work or, worse, let a message go out for a change that later rolled back. With the outbox, the database commit is the single source of truth. A message exists on the broker only if the change exists in the database.

sync-outbox-relay is the only component that reads the outbox for publishing. Other services consume from RabbitMQ and never read `sync.Outbox` directly. That keeps the table a private contract between the application and the relay, and lets the schema change without coordinating with consumers.

## How the polling works

The relay is a long running .NET worker written in C#. It runs a loop: wait for the interval, read the pending rows from `sync.Outbox`, publish them, mark them as done, repeat. The interval is `500 ms`. That figure is a trade between latency and database load. Scientists do not notice a delay of about that size, and the query against the table is cheap enough to run that often as long as the table stays small and the pending rows are indexed.

A few rules the loop follows:

- Rows are read in the order they were written, so messages for the same notebook entry leave in the order the changes were committed. Compliance depends on this, because an audit trail that shows events out of order is a finding.
- The relay reads a bounded batch each cycle rather than the whole backlog. If a large backlog builds up after an outage, the relay drains it over many cycles instead of holding one huge transaction or one huge read.
- If a cycle finds nothing, it just waits for the next tick. There is no backoff when idle, since the idle query is the cheap case.
- The loop does not overlap itself. A slow cycle delays the next one rather than running in parallel with it. Overlapping cycles would risk publishing the same row twice and would break ordering.

There is no change notification from SQL Server in this design. Polling was chosen on purpose: it has few moving parts, it works the same on every environment we deploy to, and it needs no special database features or permissions beyond reading and updating the one table.

## Publishing to the exchange

Each row becomes one message published to the exchange `lnb.sync`. The relay does not decide who receives it. Routing is done by the exchange using the routing key built from the row's event type, and consumers bind their own queues. Adding a new consumer means declaring a new queue and binding, with no change to the relay.

The message body is the payload stored on the row, passed through unchanged. The relay does not interpret, enrich or reformat it. Large instrument output is not carried in the message. Raw files live in Azure Blob Storage, and the message carries a reference to the blob, so the broker only moves small messages. If a payload ever looks like it contains file content, that is a bug in the producer, not something the relay should work around.

Messages are published as persistent, and the relay uses publisher confirms. A row counts as published only after the broker confirms the message. Fire-and-forget publishing would let a row be marked done while the broker had dropped the message, which would silently lose audit events.

The row's own identity is sent along as the message identifier. Consumers use it to detect duplicates. That matters because of the delivery guarantee described next.

## Delivery guarantee and failure handling

The guarantee is at least once, not exactly once. The relay publishes, waits for the confirm, and only then marks the row as done in SQL Server. If the process dies between the confirm and the database update, the row is still pending after restart and gets published again. Consumers must therefore be idempotent, keyed on the message identifier. This is a deliberate choice. Exactly once across a database and a broker would need a distributed transaction, and we do not want that coupling.

What happens in the common failure cases:

- Broker unreachable or exchange missing: the publish fails, the row stays pending, and the relay retries on later cycles with growing pauses so it does not hammer a broker that is already unhealthy. Nothing is lost; the outbox just grows. An alert on outbox age is the way to notice.
- Negative confirm from the broker: treated like a failed publish. The row stays pending and is retried.
- Database unavailable: the relay cannot read or mark rows. It logs, waits and tries again. Nothing is published that cannot be tracked.
- One bad row: a row that always fails, for example because its payload is malformed, must not block the rows behind it forever. The relay counts failures per row and, past a limit set in configuration, moves the row aside for manual review and carries on. Moving it aside is logged loudly because it means an audit event has not gone out and someone has to look at it.
- Process crash mid batch: safe, for the reason given above. Rows that were published but not marked will be sent again.

Because ordering matters, skipping a bad row has a cost: later messages for the same entry may go out before the skipped one is repaired and sent. Consumers that care about strict order for an entry should check the sequence information in the payload rather than assume arrival order. This is an open weakness and is noted again below.

## Running more than one instance

Running two relays against the same table without care would double publish and scramble order. The current deployment runs a single active relay. If a second instance is started for availability, it must either hold a lock that makes it a standby, or claim rows in a way that cannot overlap, and the claim has to keep per entry order intact. Simply scaling out the worker is not safe today. Anyone changing deployment topology should read this section first.

Restart behaviour is fine for a single instance. A new process picks up whatever is pending and continues. There is no in-memory state that has to survive a restart.

## Audit and compliance considerations

The outbox is part of the audit story, so a few things are handled with care:

- Rows are not deleted the moment they are published. They are marked done and kept for a retention period, then cleaned up by a separate job. This lets a compliance officer or an engineer show when a given change left the database and that it did.
- The relay logs each batch with counts and the age of the oldest pending row, but never the payload. Payloads can contain unpublished research data, and logs are shipped to systems with wider access than the notebook.
- Timestamps on the row come from the database, not from the relay host, so clock drift on a worker machine cannot reorder the trail.
- Any manual action on a row, such as releasing one that was moved aside, has to be recorded as its own audit event by whoever does it. Editing the table by hand without a record is not acceptable.

## Operating notes and what to check first

When messages seem late or missing, go in this order. First, look at the age of the oldest pending row in `sync.Outbox`. If it is small, the relay is healthy and the problem is downstream. If it is growing, the relay is not keeping up or cannot publish. Second, check broker health and whether the exchange `lnb.sync` exists with the expected bindings; a missing binding makes messages vanish quietly because the exchange has nowhere to route them. Third, check the relay log for rows moved aside. Fourth, check that only one relay instance is active.

If the backlog is large and the interval of `500 ms` is the bottleneck, do not shorten the interval first. Look at batch size and at how long each publish confirm takes. Most slowness has come from confirms waiting on a stressed broker, not from the polling gap.

Do not change the exchange name or the table name casually. Both are part of the contract with other services and with deployment scripts, and a rename needs a coordinated release.

## Open questions

- Strict ordering after a row is moved aside is not guaranteed. We may want a per entry hold so later rows wait for the failed one, at the cost of blocking.
- A proper standby or claim mechanism for several instances is not built yet.
- Polling at this rate is wasteful on quiet systems. A slower idle rate that speeds up on activity could cut database load, but it would complicate latency expectations, so it is parked.
- Retention length for done rows should be confirmed with compliance, since the current value was picked by engineering.
