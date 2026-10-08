---
id: 01K2NPANSWSEFE3YMQA970XJJH
created: 2025-08-14T22:43-03:00
---

# settlement-events: general direction

We settled on a general shape for settlement-events and this note keeps the reasoning so nobody has to redo the argument. It is the Kafka-facing part of Ledgerlark that carries parsed settlement records from the processor files toward the reconciler. The direction is: treat it as an append-only stream of facts, keep consumers idempotent, and keep PostgreSQL as the place where truth about matches lives.

## Why this came up

Processor files arrive late, get re-sent, and sometimes get corrected. Early on we had consumers that assumed each record showed up once and in order. That broke whenever a file was replayed. Finance ops then saw phantom mismatches that were really duplicates. We wanted one rule that makes replays boring.

## What we chose

settlement-events is a log of things that were observed, not a command queue. Each event describes one settlement line as the processor reported it. Corrections are new events that refer to the earlier one, never edits of old ones. Consumers must be safe to run twice on the same event and get the same result.

Ordering is only promised where it matters, which is per settlement line grouping, not across the whole stream. We pick the partitioning key so that related events land together and stop there. Anything needing a global order is a sign the design is wrong.

## Where state lives

Kafka is transport and short-term history. PostgreSQL holds the durable record of what was ingested, what was matched, and what was flagged. If the two disagree, PostgreSQL wins and we replay from it or from the source files, not the other way around. Consumers write their results and their progress in the same database transaction where they can, so a crash does not leave half a result.

## Schemas and the gRPC edge

Event shapes are versioned and only change in compatible ways: add optional things, do not repurpose existing ones. The gRPC services that sit next to settlement-events read from the database and do not reach into the stream directly. This keeps the review UI and other callers off the Kafka contract.

## Things we decided against

- Exactly-once as a hard promise from the broker. We prefer idempotent handling in our own code because it also covers file re-sends, which the broker cannot see.
- A single big event that carries a whole file. Per-line events are easier to replay and to correct.
- Letting consumers fix up data in place when a correction arrives. They apply the correction as a new fact.

## Operations and infra

Topic and consumer setup is managed through Terraform like the rest of the infra, so changes to retention or partitioning go through review rather than being tweaked by hand. Failed events go to a separate holding place with enough context to retry, and someone on finance ops tooling looks at it regularly. Dropping an event silently is not acceptable.

## Open points

We still need to agree how long we keep raw history in the stream versus relying on the source files. We also have not settled how to alert on a consumer that is quietly falling behind. Both should be written up as separate notes once someone picks them up.
