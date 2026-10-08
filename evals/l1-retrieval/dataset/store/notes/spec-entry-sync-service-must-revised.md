---
id: 01M1YX318GS4XSY3ZRERKQEBDM
created: 2026-09-07T18:41-03:00
---

# entry-sync-service acknowledgement time limit

This note replaces the earlier note "entry sync service must" and gives the new value: entry-sync-service must acknowledge a pushed entry revision within 1500 ms, where the earlier limit was 2000 ms. Treat the earlier note as out of date wherever the two disagree.

The rest of this note says what the limit covers, what counts as an acknowledgement, and what the service may and may not do to meet it. It is written for people changing the service and for people checking it against the audit requirements.

## The requirement

When a client pushes an entry revision to entry-sync-service, the service must send back an acknowledgement within 1500 ms. The clock starts when the service has received the whole push request. It stops when the acknowledgement leaves the service. Time the client spends uploading a large payload does not count against the limit, but all server-side work before the acknowledgement does.

The limit applies to every pushed revision, including revisions that arrive in a burst from an instrument import. It is a hard ceiling per revision, not an average. A service that is fast on average but occasionally slower than 1500 ms does not meet it.

## What changed from the earlier limit

The earlier limit was 2000 ms. The new value is 1500 ms, so the budget is smaller by a quarter. Anything that was tuned to sit just under the old figure, such as client timeouts, retry delays, health probe thresholds and alert rules, needs another look. A client timeout that is shorter than the service limit would make clients give up on pushes the service still counts as on time.

The old note should not be used for any new design work. If a document or a config comment still quotes the old figure, update it to 1500 ms or point it at this note.

## What counts as an acknowledgement

An acknowledgement means the service has accepted responsibility for the revision. At that point the revision must be durably recorded in the service's own store, so a crash right after the acknowledgement does not lose it. It does not mean that every downstream step has finished.

The following are not required before the acknowledgement:

- Copying large attachments or raw instrument files to blob storage.
- Publishing follow-up messages to other consumers.
- Reconciling the revision with the instrument output it refers to.
- Any report or export generation.

The following are required before it:

- Basic validation of the request and of the author identity.
- Checking that the revision follows on from the revision the client says it is based on.
- Writing the revision and its audit record to the database in one transaction.

## Why the budget is tight

Scientists work in the notebook while an instrument is running, and a slow acknowledgement makes the editor feel stuck or makes the client retry. Retries cause duplicate pushes, which are harmless to the data when handled correctly but noisy in the audit log and wasteful under load. A shorter limit keeps the editing experience responsive and keeps retry traffic low.

Compliance officers care about a related point: a revision the user believes was saved must really be saved. The limit is therefore never a reason to acknowledge early. The service must not reply before the durable write is done just to hit the number.

## Work that stays on the fast path

The fast path is kept as short as possible. It receives the request, validates it, writes the revision and the audit record to SQL Server in a single transaction, and replies. Everything else is moved off the path.

In practice that means the service hands later work to RabbitMQ after the write, and workers pick it up. Large binary content goes to Azure Blob Storage from a background step, with the entry holding a pending reference until the upload completes. The fast path should not wait for a blob upload, for a broker confirmation from a slow queue, or for any call to another service that is not essential to accepting the revision.

If a change adds a synchronous call to the fast path, it needs a measured cost and a reason, and it should be reviewed against the 1500 ms limit.

## Failure and retry behaviour

If the durable write fails or cannot finish in time, the service must return a clear failure instead of a late success. The client then knows the revision is not saved and can retry. A late success is worse than a prompt failure because the user may already have moved on.

Pushes must be safe to repeat. A retried push of the same revision should be recognised and acknowledged again without creating a second revision or a second audit entry that looks like a new edit. This idempotence is what lets clients retry quickly after a timeout.

When the message broker is unavailable after a successful write, the acknowledgement is still sent. The pending follow-up work must be recorded so it can be published later. Losing the follow-up silently is not acceptable.

## Audit trail implications

The audit trail must stay complete under the shorter limit. Every accepted revision has its audit record written in the same transaction as the revision. There is no mode in which the audit record is written later to save time. If the transaction cannot include the audit record, the push fails.

Rejected pushes, including those rejected for being too slow to process, should still leave a trace that compliance officers can find, but that trace is not allowed to extend the time to respond. Write it after the reply or from a separate path.

## Measuring and alerting

Latency should be measured inside the service from receipt of the complete request to sending the acknowledgement, and reported as a distribution, not only a mean. The figures that matter are the high percentiles and the maximum, since the requirement is a ceiling on every revision.

Alerts should fire before the limit is breached in a sustained way, for example when a high percentile approaches 1500 ms. Load tests should include bursts from instrument imports and large revisions, because those are the cases most likely to push the service over.

## Open points

- Client timeouts and retry intervals need to be checked against the new limit in each client that talks to entry-sync-service.
- Dashboards and alert rules that still use the earlier 2000 ms figure should be updated.
- If a database or broker slowdown makes the limit hard to meet under normal load, raise it with the compliance owners instead of weakening the durability rules above.
