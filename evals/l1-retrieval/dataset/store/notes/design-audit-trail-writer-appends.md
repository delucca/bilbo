---
id: 01K02E7MHP77MGW8BT3H3T2QER
created: 2025-07-13T14:44-03:00
---

# audit-trail-writer design

audit-trail-writer is the service that records every change to a lab notebook entry and every instrument import as an audit event. The core rule is that it only appends. It writes to the table audit.TrailEvent, and the database role of the service is granted DENY UPDATE, DELETE on it. The point is that a bug or a compromised service account cannot rewrite history. Compliance officers rely on this, so do not weaken it for convenience.

## Why append-only is enforced in the database

An application-level promise ("we never call update") is not enough for an auditor. The restriction sits in SQL Server permissions, so even a bad migration script run under the service role, or a code path nobody reviewed, fails with a permission error instead of altering a row. DENY wins over any GRANT the role might inherit through another role membership, which is why DENY is used and not just the absence of a grant. If someone adds the service role to a broader role later, the DENY on audit.TrailEvent still holds.

## What the writer does

The writer consumes messages from RabbitMQ. Producers are the notebook sync worker and the instrument ingest worker. Each message describes one thing that happened: who, which entry or instrument run, what kind of change, and when. The writer validates the message shape, adds its own server-side timestamp, and inserts one row into audit.TrailEvent. Only after the insert commits does it acknowledge the message. If the insert fails, the message is not acknowledged and goes back to the queue.

Large payloads, such as raw instrument output files, are not stored in the table. They live in Azure Blob Storage and the event row holds a reference to the blob plus a content hash. That keeps the table narrow and makes it possible to check later that a blob was not swapped.

## Corrections and mistakes

Since nothing can be updated or deleted, a wrong event is fixed by appending a new event that points at the earlier one and says what it corrects. Readers of the trail must follow those links. Do not add a "soft delete" flag column that someone would then want to update; that would break the model.

## Failure handling

Redelivery from RabbitMQ can produce duplicates. The writer carries a producer-supplied event key and the table has a uniqueness rule on it, so a duplicate insert is rejected and treated as already done, then acknowledged. Poison messages that fail validation repeatedly go to a dead-letter queue and raise an alert; they are never silently dropped, because a missing audit event is itself a compliance problem.

If SQL Server is unreachable, the writer stops acknowledging and lets messages accumulate in the queue. Back-pressure on producers is acceptable. Losing events is not.

## Things to watch when changing it

- Schema changes to the table must be done by a separate deployment identity, never by the service role. Adding columns is fine; anything that rewrites existing rows is not.
- Do not give the service role any permission that would let it alter the DENY. Permission changes should go through review.
- Tests that need to clean up should use a throwaway database, not the real table with a privileged login.
- Retention and archiving are an open question. Without delete rights, old rows can only be moved by a controlled process outside the service, and that process needs compliance sign-off.

## Open points

Partitioning of the table by time is likely needed as volume grows. Whether the hash chain across rows should be added for tamper evidence has been discussed but not decided.
