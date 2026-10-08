---
id: 01KE2CMSG019T32WQNB2TQ3CZS
created: 2026-01-03T13:56-03:00
---

# sync-outbox-relay uses publisher confirms, not AMQP transactions

Decision: sync-outbox-relay uses publisher confirms instead of AMQP transactions when it publishes outbox rows to RabbitMQ. We chose this because AMQP transactions cut throughput several-fold, and the relay is the one component that has to keep up with bursts of instrument output. Publisher confirms give the same guarantee we need, which is that the broker has accepted a message before the relay marks the outbox row as sent. They cost far less to get it.

This note records what we decided, why, what we rejected, and what a later session must not undo by accident. It is written for someone who touches the relay code, the RabbitMQ topology, or the audit-trail checks that depend on delivery behavior.

## Context

LabNotebook Sync copies electronic lab notebook entries and instrument output between systems and keeps an audit trail of every change. Scientists use it to see that a run recorded by an instrument shows up in the right notebook entry. Compliance officers use it to prove nothing was lost or silently altered on the way.

The write path uses a transactional outbox. The application writes the business change and an outbox row in one SQL Server transaction. sync-outbox-relay is a separate .NET worker that reads unsent outbox rows, publishes them to RabbitMQ, and then marks them as sent. Large instrument payloads are not put in the message body. They go to Azure Blob Storage, and the message carries a reference to the blob.

The relay is therefore the bridge between a durable database record and a broker message. Everything about its publish step is about one question: when can it safely say a row has been delivered?

## The decision

sync-outbox-relay uses publisher confirms. The relay opens its publishing channel in confirm mode. It publishes a batch of messages, then waits for the broker to acknowledge each one. Only after a positive acknowledgement does it mark the matching outbox row as sent. A negative acknowledgement, a returned message, or a timeout leaves the row unsent so it is picked up again on the next pass.

The relay does not use AMQP transactions. It does not call the select, commit, or rollback transaction operations on the channel. A reader who sees transactional publishing proposed for this component should treat that as a rejected option, not an oversight.

## Why not AMQP transactions

The reason is throughput. AMQP transactions make the channel wait for a commit round trip, and the broker has to do extra work to make the commit atomic. When we measured both approaches against our own workload, transactions cut throughput several-fold compared with publisher confirms. That is too much to pay on the busiest component in the pipeline.

Transactions also give a stronger promise than we need. They group several publishes into an all-or-nothing unit on the broker. Our outbox already gives us the grouping on the database side, and each message is independent once published. Paying for broker-side atomicity of a group we do not need to treat as a group is waste.

A second reason is that transactions serialize the channel. While one commit is pending, the channel does little else. Confirms let the relay keep publishing while earlier acknowledgements are still in flight.

## Why publisher confirms are enough

What the audit trail needs is a no-loss guarantee: an outbox row is never marked sent unless the broker has taken responsibility for the message. Publisher confirms provide that. A confirm means the broker has accepted the message, and for durable queues with persistent messages it means the message has been handled in a way that survives the normal failure cases we care about.

What we do not need is exactly-once delivery from the broker. Confirms give at-least-once behavior in combination with the outbox: if the relay crashes after the broker confirmed but before the row was marked sent, the row will be published again. Consumers are built to handle that, as described below.

## How the relay publishes

The relay works in passes. Each pass reads a batch of unsent outbox rows in order, publishes them, collects confirmations, and updates the rows that were confirmed. Messages are marked persistent and sent with the mandatory flag so that an unroutable message comes back as a return instead of vanishing.

The relay tracks outstanding publishes by the channel's sequence numbers and maps each one to its outbox row. When the broker acknowledges a sequence number, possibly with the multiple flag covering earlier ones, the relay marks every covered row. When the broker rejects a sequence number, the relay records the failure and leaves the row for retry.

Publishing is asynchronous within a batch. The relay does not wait for each confirm before sending the next message; it waits at the end of the batch, or when the number of outstanding messages reaches its configured ceiling. That pipelining is where most of the gain over transactions comes from.

## Failure handling

There are four failure shapes the relay must handle, and the decision to use publisher confirms makes each of them explicit.

The first is a negative acknowledgement. The broker could not take the message. The row stays unsent and is retried with backoff. Repeated negative acknowledgements for the same row raise an alert rather than looping quietly.

The second is a timeout waiting for a confirm. We do not know whether the broker took the message. The row stays unsent and will be published again. This is the case that produces duplicates, and it is accepted on purpose.

The third is a returned message, meaning the exchange accepted it but no queue matched. This is a topology bug, not a transient fault. The relay treats it as a failure, keeps the row unsent, and logs enough to find the exchange and routing key involved.

The fourth is a lost connection. The channel and all outstanding confirms are gone. The relay reconnects, rebuilds its channel in confirm mode, and treats every unconfirmed row as unsent.

## Duplicates and idempotent consumers

Because confirms combined with the outbox mean at-least-once delivery, consumers must tolerate seeing a message more than once. Each message carries a stable identifier taken from the outbox row, and consumers record the identifiers they have applied so a repeated message is a no-op.

This is not a new burden created by the decision. Transactions would not have removed duplicates either, because the crash window between broker commit and database update exists in both designs. The duplicate window lives between the broker and the database, and no broker feature closes it without a distributed transaction we do not want.

Anyone who changes a consumer must keep that idempotence. Anyone who changes the relay must keep the stable identifier on each message.

## Ordering

The relay publishes rows in outbox order on a single channel per ordering scope, and RabbitMQ preserves order within a queue for a single publisher channel. Retries can break strict order: a row that was negatively acknowledged and retried may land after a later row that succeeded.

For most message types this does not matter, because consumers apply changes keyed by entity and version. For audit-trail events, the version and timestamp inside the message decide the order the audit store records, not arrival order. We chose not to hold up the whole stream for one failed row, since that would let a single bad message stall instrument sync for everyone.

If a future feature needs strict ordering for a stream, solve it with a per-entity sequence check in the consumer, not by switching the relay back to transactions.

## Audit and compliance implications

Compliance officers need to show that every recorded change reached the downstream system. The outbox table is the source of truth for what should have been sent. The marking of a row as sent, which only happens after a broker confirm, is the evidence that it was handed to the broker.

That gives a clean statement for audits: a row is marked sent only after the broker acknowledged it, and a row never marked sent is still pending and visible. Nothing is dropped silently because the failure paths all leave the row in place.

The relay writes its own log entries for confirm outcomes, but those logs are operational, not the audit record. The audit record is the outbox state plus the consumer-side applied-message records.

## Alternatives we considered

AMQP transactions were the obvious first choice because they are simple to reason about. We rejected them on throughput, as above.

Fire-and-forget publishing, with no confirms, was rejected immediately. It would let the relay mark rows sent that the broker never received, which breaks the audit guarantee.

A distributed transaction spanning SQL Server and RabbitMQ was rejected as fragile and slow. RabbitMQ does not take part in the usual two-phase commit machinery, and adding a coordinator would add an operational dependency for little gain.

Polling the broker for message receipt after publishing was rejected as redundant. Confirms already deliver that signal on the same channel.

## Blob storage interaction

Large instrument output is written to Azure Blob Storage before the outbox row that references it is committed. The relay does not upload blobs and never needs to; it only publishes the reference. That ordering means a confirmed message never points at a blob that does not exist yet.

The decision here has one consequence for blobs. A republished message after an unconfirmed publish carries the same blob reference, so duplicates do not create duplicate blobs. Cleanup of blobs is a separate concern and must not be triggered by confirm outcomes in the relay.

## Operational notes

Watch for a growing backlog of unsent outbox rows. That is the first sign that confirms are slow or failing. A rising count of rows that were published more than once points to confirm timeouts, which usually means broker pressure or a network issue between the relay and RabbitMQ.

The outstanding-confirm ceiling and the confirm wait timeout are configuration, not code. Raising the ceiling helps throughput until memory or broker flow control pushes back. Lowering the timeout detects trouble sooner but increases duplicates. Change them one at a time and watch the backlog.

If the broker applies flow control, confirms slow down. That is the intended backpressure, and the relay should slow publishing rather than build an unbounded queue of unconfirmed messages in memory.

## Testing

Tests for the relay should cover the confirm paths directly. Cover a positive acknowledgement marking the row sent, a negative acknowledgement leaving it unsent, a timeout leaving it unsent, and a returned message leaving it unsent. Add a test where the process stops after the broker confirmed but before the row update, and check that the next pass republishes the row.

The consumer side needs a matching test that a repeated message with the same identifier is applied once. Integration tests against a real RabbitMQ instance are worth the setup, because mock channels tend to hide ordering and multiple-flag acknowledgement bugs.

Do not add a test that asserts transactional behavior. If one appears, it is a sign someone is reverting this decision.

## What would change this decision

We would revisit it if the workload changed so that grouped, all-or-nothing publishing on the broker became a real requirement, for example if a single logical change had to appear as several messages that must all arrive or none. Even then, first try to carry the group in a single message or add a completion marker the consumer can check.

We would also revisit it if a different broker feature, such as a stronger delivery guarantee in a newer RabbitMQ stream or queue type, gave the same safety as confirms with less complexity. Throughput measurements should be repeated on our own workload before switching, since the several-fold gap came from our measurements here and may differ elsewhere.

## Pitfalls for later sessions

Do not mark an outbox row as sent when the publish call returns. Returning from the publish call says nothing about broker acceptance. Only the confirm does.

Do not enable confirm mode on a channel shared with other code that does not expect it. Keep the relay's publishing channel its own.

Do not treat a duplicate as a bug in the relay. It is an expected result of the at-least-once design, and the fix, if one is needed, belongs in the consumer.

Do not mix transactions and confirms on one channel. The broker does not allow both modes together, and attempting it will fail.

Do not swallow a returned message. An unroutable message is a configuration error that would otherwise lose data quietly.

## Summary of the decision

sync-outbox-relay publishes with publisher confirms and marks outbox rows sent only after a positive broker acknowledgement. AMQP transactions were rejected because they cut throughput several-fold. The accepted tradeoff is at-least-once delivery, handled by stable message identifiers and idempotent consumers. The audit guarantee holds because every failure path leaves the outbox row unsent and visible.
