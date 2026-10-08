---
id: 01K2KN0MD18AAYBYKQXQDKWXMT
created: 2025-08-14T03:41-03:00
---

# blob-archive-client spec

This note specifies how blob-archive-client behaves when it pushes instrument output and notebook attachments to Azure Blob Storage. It is a spec, not a design. It says what the component must do and leaves the internal structure to the code. The main rule: blob-archive-client must retry a failed upload at most 5 times with exponential backoff. Everything below follows from that rule or sits next to it.

## Purpose

LabNotebook Sync ties electronic lab notebook entries to the raw output of instruments. The raw files (spectra, plate reader exports, chromatography runs and so on) are too large for SQL Server, so they go to Azure Blob Storage. blob-archive-client is the one component that talks to blob storage for this purpose. Other services hand it a file and metadata and get back a reference they can store next to the notebook entry.

Compliance officers rely on the archived copy being the same as what the instrument produced. Scientists rely on uploads not blocking their work. The retry rules exist to serve both.

## Retry rules

blob-archive-client must retry a failed upload at most 5 times with exponential backoff. That limit counts retries, not the first attempt. So a single upload can be tried once plus the retries, and then it is declared failed.

- The delay between attempts grows exponentially. Add some random jitter so many clients do not retry in lockstep after a storage hiccup.
- Cap the delay at a sane ceiling so a late retry does not sit for minutes. The exact ceiling is a config value, not fixed here.
- Retry only errors that can go away: timeouts, throttling responses, transient server errors, dropped connections.
- Do not retry errors that will not change: bad credentials, a missing container, a request that is malformed, a blob that is rejected by policy. Fail these at once.
- Nobody may raise the retry limit per call to get around a flaky network. If uploads fail often, fix the cause or alert, do not retry more.
- When the retries run out, the upload is marked failed and handed to the failure path below. It must not be dropped silently.

## Failure handling

A failed upload is a real event for the audit trail. When retries are exhausted, blob-archive-client records the failure with the entry it belonged to, the number of attempts made and the last error category. It then publishes a failure message on RabbitMQ so the sync side can show the scientist that the file is not archived yet and can schedule a later attempt.

The notebook entry must not be marked as having an archived attachment until the upload has succeeded. A half-done state is worse than a visible failure, because an auditor would read it as complete.

Each retry attempt should be logged with the attempt number and the delay chosen, without file contents or secrets in the log line.

## Integrity and idempotency

Retries must be safe to repeat. The blob name is derived from the entry and the file, not from the attempt, so a second attempt writes to the same target and does not create duplicates. Before reporting success, the client compares a content hash of what was sent with what storage reports. A mismatch counts as a failed attempt and goes through the same retry rules.

Blobs are never overwritten once archived with a different hash. If the same name is presented again with different content, that is an error for the caller, not something to retry.

## Open points

- Whether the failure message should carry a suggested time for the next scheduled attempt, or leave that to the consumer.
- Whether large files should use block-level resume so a retry does not re-send everything. Not decided; today a retry re-sends the whole file.
- Where the backoff base and ceiling are configured, and whether operators may change them without a deploy. The retry count itself stays fixed by this spec.
- How the compliance report should present uploads that succeeded only after one or more retries.
