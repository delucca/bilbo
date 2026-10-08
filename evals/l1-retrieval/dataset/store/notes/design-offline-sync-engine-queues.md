---
id: 01KJFDZFMWZWM6M4RVVEHATV77
created: 2026-02-27T08:33-03:00
sources:
  - "code: sync/src/main/kotlin/com/parcelpin/sync/DrainWorker.kt"
---

# offline-sync-engine design

offline-sync-engine queues unsent deliveries in the Room table `pending_proof` and drains them with a WorkManager unique work named `sync-drain`. Everything else in this note hangs off those two things: the table is the only place an unsent proof of delivery lives on the device, and `sync-drain` is the only thing that is allowed to push rows out of it. If you are debugging a "my delivery never arrived" report, look at `pending_proof` first, then at the state of `sync-drain`.

## Scope and goal

ParcelPin couriers work in basements, lifts, rural roads and vans with no signal. The app has to let a driver capture proof of delivery (photo, signature, location, timestamp) and move on to the next stop without waiting for the network. offline-sync-engine is the part that makes that safe. It accepts a finished capture locally, keeps it until the backend confirms it, and then lets it go.

What it does not do: it does not decide what a valid proof is (the capture screens do that), and it does not render any sync state beyond a simple count the UI reads.

## Local queue: pending_proof

Each finished capture becomes one row in the Room table `pending_proof`. The row holds the delivery reference, the serialized proof payload, a pointer to the local photo and signature files, a created time, an attempt counter and a status. The payload is a Protocol Buffers message, so the same bytes can be stored, retried and sent without re-encoding.

Rules I want to keep:

- A row is written in one Room transaction together with the status change of the delivery. Either both happen or neither.
- Rows are never edited by the UI after insert, except to mark them cancelled by the driver.
- A row is only deleted after the backend write is confirmed. Failing to delete after a confirmed write is fine; the next run sees the remote copy and drops the row.
- Media files are removed after the row is removed, not before.

## Draining: sync-drain

`sync-drain` is a WorkManager unique work. Unique matters: whatever triggers a drain (a new capture, connectivity coming back, app start, a periodic nudge) enqueues the same name, and the existing run is kept instead of starting a second one. Two concurrent drains would upload the same row twice and fight over the attempt counter.

Constraints on the work: it needs a connected network. It uses exponential backoff on failure. It does not need charging or idle, since drivers want proofs delivered during the shift.

Within one run:

1. Read rows from `pending_proof` oldest first.
2. Upload media to storage, then write the proof document to Cloud Firestore.
3. On confirmed success, delete the row and its files.
4. On a transient error (no network, timeout, server busy), stop the run and return retry so WorkManager applies backoff.
5. On a permanent error for one row (rejected payload), mark that row failed, keep it, and continue with the next row so one bad row cannot block the queue.

Order is oldest first but not strictly guaranteed on the backend; each proof carries its own captured time, so the server does not rely on arrival order.

## Idempotency

Retries will happen, and a run can die after the Firestore write but before the local delete. To make that harmless, the Firestore document id is derived from the delivery reference and a client-generated id stored on the row, so a repeated write overwrites the same document. Uploading media uses a path derived the same way. The engine therefore treats "already exists with the same content" as success.

## Failure handling and visibility

- Process death mid-run: WorkManager restarts `sync-drain`; rows that were in flight are still in `pending_proof` and are retried.
- Failed rows stay in the table and are surfaced to the driver as "needs attention" with a manual retry that simply enqueues `sync-drain` again.
- The UI reads a count of rows in `pending_proof` through Room's observable queries. It does not talk to the worker.
- Logging should include the row's client id and attempt count, never the payload or photo.

## Open questions

- Whether a row that has failed permanently should be uploaded to a separate review collection for support staff.
- How long to keep cancelled rows before cleanup.
- Whether to cap the size of the queue when storage is nearly full, and what to tell the driver.

## Quick reference

```
table:  pending_proof
work:   sync-drain   (WorkManager unique work)
flow:   capture -> pending_proof -> sync-drain -> Cloud Firestore -> delete row
```
