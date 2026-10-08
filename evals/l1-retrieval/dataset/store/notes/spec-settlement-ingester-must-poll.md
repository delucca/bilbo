---
id: 01KVJ6ZFE4QMSX3X7NRE2VYRY9
created: 2026-06-20T06:49-03:00
---

# settlement-ingester spec

This note specifies what settlement-ingester does: it pulls card-processor settlement files from the processor's SFTP drop, parses them, and hands the rows on to the rest of Ledgerlark for matching against internal ledger entries. It is the entry point for all processor data. If it is late or wrong, everything downstream is late or wrong.

## Polling requirement

settlement-ingester must poll the processor SFTP drop every 300 seconds for new settlement files. The interval is fixed by this spec, not left to each deployment's taste. Finance operations teams expect a file to show up in review within a short, predictable window after the processor publishes it, and 300 seconds is the agreed upper bound on the discovery delay. A poll that finds nothing is normal and should not log at warning level or above.

## Poll behavior

Each poll lists the drop, compares file names and sizes against what has already been ingested, and picks up only files not seen before. A file still being written by the processor must not be read half-finished. Treat a file as ready only when its size has stopped changing between two listings, or when the processor's own completion marker exists, if one is provided for that processor.

If a poll overruns, the next one waits for it to finish. Polls never overlap, and the timer restarts from the end of the previous one only if the previous run took longer than the interval. Otherwise the schedule stays on its original cadence.

## Ingestion flow

Once a file is ready, settlement-ingester downloads it to local scratch space, checks that it is complete and parseable, and reads it row by row. Parsed rows are normalized into one internal settlement record shape, whatever the processor's format. Records are published to Apache Kafka for the matching side to consume. The ingestion itself is recorded in PostgreSQL so a restart knows what has been done.

A file counts as ingested only after its records have all been published and the PostgreSQL bookkeeping row is committed. If the process dies halfway, the file is picked up again on the next poll, and consumers must tolerate duplicates.

## Idempotency

Re-reading the same file must not create duplicate settlement records downstream. Each record carries a stable key derived from the processor, the file identity and the row position, so a repeat publish is recognizable. The bookkeeping row in PostgreSQL has a uniqueness constraint on file identity as the second line of defense.

## Failure handling

Connection failures to the SFTP drop are retried with backoff inside a poll. If the drop stays unreachable, the poll fails, a metric is raised, and the next scheduled poll tries again. Do not alarm on a single failed poll; alarm on several in a row.

A malformed file is not retried forever. It is marked failed in PostgreSQL with the reason, left in place on the drop, and surfaced to a human. Other files in the same poll continue to be processed. One bad file must not block the queue.

## Interfaces

Outbound, settlement-ingester publishes normalized records to Kafka. It exposes a gRPC interface for status, so operators and other services can ask which files were seen, which succeeded, and which failed. It does not accept files over gRPC; the SFTP drop is the only input path. Credentials for the drop come from the deployment secrets, never from config files in the repo.

## Deployment notes

Infrastructure for the service, including its network access to the processor drop and its database, is defined in Terraform. Run a single active instance per processor drop, or make sure instances coordinate through the PostgreSQL bookkeeping, so two pollers do not race on the same file.

## Open points

- Whether to add a manual trigger for an immediate poll over gRPC, for operators waiting on a late file. Not decided.
- Per-processor overrides of the interval are not allowed for now. The 300 seconds rule applies to every processor until this spec changes.
- How long failed files stay on the drop before cleanup is still to be agreed with finance operations.
