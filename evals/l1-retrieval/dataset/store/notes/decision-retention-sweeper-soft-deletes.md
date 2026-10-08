---
id: 01KYVBMM131VMZ3BVNX97M9Q2W
created: 2026-07-31T02:51-03:00
---

# retention-sweeper: soft delete first, hard delete later

retention-sweeper does not delete notebook entries in one step. It soft-deletes an entry first and hard-deletes it after 30 days. We chose this because auditors need a recovery window: if an entry is removed by mistake, or a compliance officer later says it should have been kept, there is still something to restore. This note records the decision and the reasoning so nobody collapses it into a single delete pass later.

## Decision

An entry that falls out of its retention period is marked as soft-deleted by retention-sweeper. It is hidden from normal notebook views and from sync to instruments, but the row and its linked blobs stay where they are. The hard delete happens only after 30 days. The hard delete removes the SQL Server rows and the matching objects in Azure Blob Storage.

The window is a property of the sweeper's behaviour, not of the entry. Every entry goes through the same two steps. There is no fast path that skips the soft delete, including for entries a user deletes by hand through the sweeper's code path.

## Why a recovery window

Research scientists sometimes delete or expire things they still need. Compliance officers and auditors then ask for the record back, often well after the event. A hard delete cannot be undone, and the audit trail would show a gap with no way to fill it. A soft delete gives us a period where the answer to "can we get it back" is yes.

The audit requirement is the main driver. LabNotebook Sync exists to enforce audit trails, so destroying data immediately works against the product. Storage cost of keeping soft-deleted data for the window is small next to the cost of an unrecoverable audit finding.

## What soft delete means in practice

Soft delete is a state change, not a removal. The entry keeps its identity, its history and its link to instrument output. The audit trail records who or what triggered the soft delete and when. Restoring an entry clears the soft-delete state and writes a restore event to the trail, so the history shows both the removal and the recovery.

Queries for normal use must filter out soft-deleted entries. This is an easy thing to forget in new queries, so treat it as a checklist item for any change that reads entries.

## What hard delete means in practice

After 30 days the sweeper picks up soft-deleted entries whose soft-delete time is old enough and removes them for good. It should remove the database rows and the blob data together, and it should write an audit event saying the hard delete happened. The event stays even though the data does not, so the trail still shows that the entry existed and when it was destroyed.

Order matters when cleaning up: write the audit event, then delete blobs, then delete rows, so a failure part way leaves something the next run can finish. The sweeper should be safe to run again on the same entries.

## Messaging and sync interactions

Soft delete and hard delete are announced on RabbitMQ so other parts of the system can react, for example the sync side that talks to instruments. Consumers should treat a soft-delete message as "stop showing this" and a hard-delete message as "forget this". A consumer that treats soft delete as final will break restore.

If a restore arrives while the sweeper is running, the restore wins as long as the hard delete has not started for that entry. Keep that check close to the delete itself rather than in the selection query, because the selection can be stale by the time the delete runs.

## What not to change without a new decision

Do not shorten the window to speed up cleanup, and do not add a path that hard-deletes directly. Lengthening the window is also a decision, since it affects storage and what we tell customers about when data is really gone. If a customer needs a different period, that should be a recorded change here, not a quiet edit in the sweeper.

## Open points

Legal holds are not covered by this decision. An entry under a hold should probably never reach the hard delete step, but how holds are modelled is still to be settled. Until then, assume the sweeper has no hold awareness and flag it when someone asks for one.
