---
id: 01KWBKAQNW04GKE3WCA6PNJDZ2
created: 2026-06-30T03:26-03:00
---

# instrument-ingest-worker: general direction

We settled on a general shape for instrument-ingest-worker and this is the short version of why. It is a single-purpose consumer: it takes instrument output off RabbitMQ, stores the raw file, and hands a record to the notebook side. It does not interpret science. Keep it that way.

## Context
Instrument output arrives in many formats and at uneven rates. Compliance officers need to trace every notebook entry back to the original file. Research scientists need entries to show up without babysitting.

## Decision
Raw output is stored first, untouched, in Azure Blob Storage. Parsing and linking happen after that, from the stored copy. The worker never edits raw data.

## Why raw first
If a parser is wrong, we can rerun it. If the raw file was lost or changed, we cannot. The audit trail depends on this.

## Messaging
RabbitMQ is the entry point. Messages are acknowledged only after the raw copy is safely stored and the audit record is written. A crash mid-way means redelivery, not loss.

## Idempotency
Redelivery will happen, so handling a message twice must be harmless. The worker checks whether it already saw the same input before it writes anything new.

## Failures
Bad input goes to a separate holding path for a person to look at. It is not dropped and it is not retried forever. Transient failures get a bounded retry with backoff.

## Audit records
Every step writes an audit row to SQL Server: received, stored, parsed, linked, failed. Rows are append-only. Corrections are new rows.

## Data access
The worker writes through a narrow data layer. No ad hoc SQL scattered through handlers.

## Parsers
One parser per instrument family, behind a common interface. Adding an instrument should not touch the core loop.

## Configuration
Behavior that varies by site or instrument lives in configuration, not in code branches. Values are not recorded in this note.

## Observability
Structured logs carry a correlation key that follows a file from message to notebook entry. Support and compliance both use it.

## Testing
Parsers are tested against sample files. The core loop is tested with a fake queue and a fake blob store. Redelivery cases get their own tests.

## Not doing
- No in-place edits of stored raw files.
- No silent drops.
- No scientific interpretation in the worker.

## Open questions
How long to keep held bad input, and who owns clearing it. Needs compliance input.

## Quick sketch
```text
RabbitMQ -> instrument-ingest-worker -> Azure Blob Storage
                                     -> SQL Server (audit)
```

## Revisit if
Throughput forces splitting storage from parsing into separate workers. The ordering above would still hold.
