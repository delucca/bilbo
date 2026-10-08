---
id: 01K3HMH33WQQFVY0BW1QB8S6JR
created: 2025-08-25T19:10-03:00
---

# entry-sync-service push acknowledgement spec

This note specifies how entry-sync-service handles a pushed entry revision and, above all, how fast it has to answer. The hard rule: entry-sync-service must acknowledge a pushed entry revision within `2000 ms`, or the client treats the push as failed. Everything else here follows from that rule or sits around it. It is written fast, for whoever touches the push path next, and it stays general on purpose. Check the code for exact names and settings before relying on any detail that is not the deadline.

## Scope

This covers the push path only: a client (the notebook front end or a desktop sync agent) sends a new revision of a lab notebook entry, and entry-sync-service takes it in, records it, and answers. It does not cover pulling entries down, instrument output ingestion in detail, or reporting for compliance officers. Those touch the same data but have their own timing and their own notes.

The service is written in C# on .NET. It uses SQL Server for the system of record, RabbitMQ for work that can happen after the answer, and Azure Blob Storage for large payloads such as instrument files and attachments.

## The deadline

The client waits for an acknowledgement. If none arrives within `2000 ms`, the client marks the push failed. That is a client-side decision and entry-sync-service cannot undo it. The clock starts when the client sends the request, not when the service starts handling it, so network time and queueing in front of the service eat into the same budget.

```
client push of entry revision -> entry-sync-service
ack expected within 2000 ms
no ack within 2000 ms -> client treats push as failed
```

Two consequences matter in practice:

- The service should never do slow work before it answers. The answer should mean "the revision is safely recorded", and nothing more.
- A slow answer is as bad as no answer from the client's point of view, even if the server finished the work. The server has to be ready for the case where the work succeeded and the client still believes it failed.

## What the acknowledgement means

An acknowledgement from entry-sync-service means the revision has been durably recorded in SQL Server together with its audit record, in the same transaction. It does not mean that downstream processing is finished. Indexing, notifications, instrument cross-linking, and copying payloads into long-term storage may all happen later via RabbitMQ.

Keeping that meaning narrow is what makes the deadline reachable. If someone adds a step to the synchronous path, ask whether the client really needs it done before it hears back. Usually it does not.

## Synchronous path

The steps that happen before the acknowledgement, in order:

- Authenticate the caller and check they may write to the entry.
- Validate the shape of the revision and its link to the parent revision.
- Check that the parent revision is the current head, or apply the conflict rule described below.
- Write the revision row and the audit row in a single SQL Server transaction.
- Publish a message to RabbitMQ announcing the new revision, or record an outbox row that a publisher picks up.
- Return the acknowledgement.

Nothing else belongs here. Large payloads must already be in Azure Blob Storage, or be referenced rather than uploaded inline, so that the synchronous path handles only metadata and small text.

## Asynchronous work

After the acknowledgement, consumers reading from RabbitMQ do the heavier work. Examples are validating attachments against their declared checksums, linking instrument output to the entry, updating search data, and fanning out change notices to other clients. A failure in this stage must not retract the acknowledgement. It must instead leave a visible state on the revision, such as pending or needs attention, and write an audit record of the failure.

Consumers have to be idempotent. RabbitMQ can redeliver, and the same revision message can arrive more than once. Each consumer keys its work on the revision identity and skips what is already done.

## Client behaviour on timeout

When the client sees no acknowledgement in time, it treats the push as failed and will normally retry. From the service's side this means a retry can arrive for a revision that is already recorded. That is the central design case, not an edge case.

The retry carries the same revision identity as the original attempt. entry-sync-service must recognise it and answer with an acknowledgement as if the push were new, without creating a duplicate revision and without writing a second misleading audit entry. A repeated push may add an audit note that a duplicate was received, if compliance wants that, but it must not look like a second edit.

## Idempotency and ordering

The revision identity supplied by the client is the idempotency key. The service looks it up first. If it exists with identical content, return the acknowledgement quickly. If it exists with different content, that is a real conflict or a client bug, and the service rejects it with a clear error rather than overwriting. Never silently replace a recorded revision, because the audit trail has to show exactly what was stored and when.

Ordering is per entry. Revisions of one entry form a chain; each names its parent. Pushes for different entries are independent and need no coordination. For one entry, the write takes a lock or uses an optimistic check on the head, and the check must be cheap, since lock waits count against the deadline.

## Audit trail rules

Every accepted revision produces an audit record written in the same transaction as the revision itself. There is no state where a revision exists without its audit record, or the reverse. Audit rows are append-only; the service has no update or delete path for them.

Records capture who, what, which revision, and when, using server time rather than client time. Client time may be stored as a claim but never as the authority. Compliance officers rely on this, so do not weaken it to save time on the push path. If the deadline is at risk, cut other work, not the audit write.

## Messaging with RabbitMQ

Publishing happens as part of the push, so it must not block on a slow broker. The preferred shape is an outbox: the revision transaction also writes an outbox row, and a separate publisher moves outbox rows to RabbitMQ. That way a broker outage does not make pushes miss the deadline, and no announcement is lost if the service stops between commit and publish.

If direct publishing is used instead, it needs publisher confirmation with a short bounded wait, and a failure to confirm must fall back to the outbox rather than fail the push. Either way, a broker problem is not a reason to withhold the acknowledgement of a revision that is already committed.

## Storage notes

SQL Server holds revisions, audit rows, and the outbox. Keep the push transaction short: few statements, indexed lookups, no scans. Avoid anything in the transaction that depends on another service.

Azure Blob Storage holds large content. The push carries references to blobs, not blob bytes. If a referenced blob is not yet visible, the service should accept the revision and mark the attachment as pending verification, and let an asynchronous consumer settle it. Calling Blob Storage synchronously from the push path to verify content is a likely way to break the deadline, so avoid it unless a cheap existence check is proven fast enough.

## Failure modes

- Database slow or unavailable: the service cannot record the revision, so it must fail fast with a clear error instead of hanging until the client gives up. A quick error is better than a silent timeout, because the client can then retry sooner and with better information.
- Broker slow or unavailable: the push still succeeds through the outbox; delivery catches up later.
- Blob Storage slow: does not affect the acknowledgement, since blobs are referenced only.
- Service slow under load: the client times out and retries, which adds more load. Protect against this with bounded concurrency and early rejection of work the service cannot finish in time, plus idempotent handling of the retries that arrive anyway.
- Acknowledgement lost on the way back: the server did the work, the client thinks it failed, and the retry hits the idempotency check. This must be a normal, cheap case.

## Observability

Measure time from request arrival to acknowledgement on the server, and compare it with the client deadline. Server-side latency alone understates what the client sees, so also record the arrival lag where it can be known, for example the client-supplied send time as a rough hint only.

Alert on the share of pushes that come close to the deadline, not just on those that exceed it. By the time the service exceeds it, clients are already retrying. Log duplicate-push recognitions separately; a rise in them is an early sign that acknowledgements are arriving late.

## Testing guidance

- A test that pushes a revision and asserts the acknowledgement arrives inside `2000 ms` under normal load, with the broker and Blob Storage deliberately slowed.
- A test that pushes the same revision twice and asserts a single revision and a single edit audit record.
- A test that pushes the same identity with different content and asserts rejection with nothing overwritten.
- A test that kills the publisher between commit and publish and asserts the announcement still goes out afterward.
- A test that makes the database fail and asserts a fast error rather than a hang.

## Open questions

These are not settled and should not be assumed:

- Whether the conflict rule on a stale parent revision should reject the push or accept it as a branch for the user to merge. Today's behaviour should be read from the code before this is changed.
- Whether the deadline should be configurable per client type. At present it is a fixed expectation of the client at `2000 ms`, and the server should be designed to meet it without help from configuration.
- How much of the verification of attachments compliance wants done before, rather than after, the acknowledgement. If any of it must come before, the deadline is the constraint that decides what is feasible, and the answer needs to be agreed with the compliance side.

## Change rule

Anything that adds work between receiving a push and returning the acknowledgement needs a justification against the `2000 ms` limit. If it cannot be shown to fit comfortably, move it behind RabbitMQ and give the revision a visible pending state.
