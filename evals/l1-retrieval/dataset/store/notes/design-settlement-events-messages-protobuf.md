---
id: 01KXVSX7E26REVZ83VF75Q687W
created: 2026-07-18T20:45-03:00
sources:
  - "code: proto/settlement/v1/event.proto"
---

# settlement-events design

settlement-events is the Kafka stream that carries settlement rows from the card-processor ingest side to the reconciliation side of Ledgerlark. Messages are Protobuf-encoded and keyed by `merchant_id`, so all rows of one merchant stay ordered within a partition. The rest of the design follows from that choice. This note is for whoever touches producers, consumers or topic config next.

## Purpose

The stream decouples file parsing from matching. Parsers read the processor settlement files and publish one event per settlement row. Matchers consume those events and compare them with internal ledger entries in PostgreSQL. Mismatches go to the review queue for finance operations.

## Message format

Every message on settlement-events is Protobuf. The schema is shared by the Go producers and consumers through generated code. Fields are only added, never reused or renumbered. Consumers must ignore fields they do not know, so a producer can ship a new field before consumers are updated.

## Message key

The key is `merchant_id`, always, with no exceptions. Do not key by file, batch, processor or transaction. A producer that publishes without the key breaks the ordering guarantee, and a consumer has no way to detect it afterwards. Treat a missing or empty `merchant_id` as a bug in the producer and reject the row before it reaches Kafka.

## Ordering guarantee

Kafka orders messages only within a partition. Because the key is `merchant_id`, the default partitioner sends every row of one merchant to the same partition, so a consumer sees that merchant's rows in publish order. There is no ordering between merchants, and nothing downstream may assume any.

## Why ordering per merchant matters

Settlement rows for one merchant can depend on each other: a refund or a chargeback adjusts an earlier settlement, and a fee line follows the batch it belongs to. Matching these out of order produces false mismatches that waste reviewer time. Ordering per merchant is enough for this, and global ordering is not needed.

## Producers

Producers are Go services in the ingest path. They set the key explicitly on every record and do not rely on defaults. Idempotent producing is on, so retries do not create duplicates inside a partition. Each producer logs the merchant and the source file reference when it publishes, which helps when tracing a missing row.

## Consumers

Consumers run in one consumer group for the matcher. Each partition is handled by one consumer at a time, so a merchant's rows are processed serially. Handlers must be safe to run twice for the same message, because delivery is at least once. Offsets are committed only after the database write succeeds.

## Partitioning and hot merchants

A very large merchant lands entirely on one partition, which can make that partition lag behind the others. This is the accepted cost of the key choice. If lag shows up, look at consumer throughput for that partition first. Adding partitions later changes which partition a merchant maps to, so do it only with a plan for rows in flight.

## Changing partition count

Increasing the partition count remaps keys. During the change, older rows of a merchant can sit on the old partition while new rows go to another, and the per-merchant ordering is briefly lost. Drain the topic before changing the count, or pause the matcher while it happens.

## Schema evolution

Keep the Protobuf definitions backward compatible. Removed fields are reserved, not deleted. The key format is not part of the schema, so changing what goes into the key is a migration of its own and not a normal schema change.

## Idempotency in PostgreSQL

The matcher writes results to PostgreSQL with a uniqueness rule on the settlement row identity, so a redelivered message updates the same row rather than adding a new one. This is what makes at-least-once delivery acceptable.

## Retries and poison messages

A message that cannot be decoded or fails validation is moved to a separate dead-letter topic with the failure reason attached. It is not retried in place, because blocking the partition would stall every other row of that merchant. Someone should look at the dead-letter topic regularly.

## Infrastructure

The topic, its retention and its access rules are defined in Terraform. Do not change them by hand in the cluster, since the next apply will undo it. Retention should be long enough to replay a full reconciliation period after an incident.

## Observability

Watch consumer lag per partition, not only the total, because the key choice makes skew possible. Also watch the rate of dead-lettered messages and the rate of rows rejected for a missing `merchant_id`. A rise in either usually points at a producer change.

## Open points

No per-merchant rate limiting exists yet for very large merchants. Replay tooling for a single merchant is still manual. If either gets built, keep the key and the ordering guarantee unchanged.
