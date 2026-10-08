---
id: 01KG86ZCVEMKJ50SY8TCQYD825
created: 2026-01-30T16:44-03:00
---

# audit-trail-writer: next steps

This is the rough plan for what to do next with audit-trail-writer. It is not a spec and it fixes no values. Each item should be turned into a proper task before anyone starts it. The order below is the order I would work in, but the first two sections could run side by side.

audit-trail-writer takes events from the notebook side and from instrument ingestion, and records them as an append-only trail that compliance officers can rely on. Most of the plan is about making that guarantee easier to check, and about not losing or duplicating events when things go wrong between RabbitMQ, SQL Server and Azure Blob Storage.

## Pin down what the trail has to guarantee

Before changing code, write down in plain terms what the component promises. Right now this lives in people's heads and in scattered comments. The list should cover:

- Which events must always produce an entry, and which are optional.
- What an entry must contain so a reviewer can reconstruct who did what, to which record, and when.
- What "append-only" means in practice: no updates, no deletes, and what happens on a correction.
- What order guarantees exist, per entry and per notebook record, and which ones we do not give.

Ask a compliance officer to read the list and say what is missing. Do this early, since the rest of the plan depends on it. Anything unclear goes back to the people who use the trail, not into code as an assumption.

## Make message handling safe to repeat

RabbitMQ delivery can repeat messages, and consumers can crash after writing but before acknowledging. The writer should treat every incoming event as possibly seen before.

- Review how the consumer acknowledges messages relative to the database write. Confirm the order and write down why it is the right one.
- Add or verify an idempotency check so a repeated event does not create a second entry.
- Decide what happens to messages that keep failing. They need a dead-letter path that a human can look at, and an alert when something lands there.
- Check behaviour when the broker connection drops and comes back. It should reconnect without dropping in-flight work silently.

Test these with a real broker in a test environment, not only with mocks. Mocks hide most of the ordering problems.

## Storage: SQL Server and blobs

Small structured entries go to SQL Server and larger payloads, such as instrument output attachments, go to blob storage. The seam between the two is where the risk is.

- Make sure an entry never points at a blob that was not fully written. Write the blob first, then the row, or use another clear pattern, and document the choice.
- Think about orphaned blobs left by failed writes, and add a way to find them without deleting anything automatically.
- Consider storing a content hash with each entry so tampering or corruption in the blob store can be detected later.
- Review the table design for append-only use. Permissions should stop the application account from updating or deleting trail rows, and that should be tested.
- Check how migrations are applied, so a schema change cannot rewrite history.

## Integrity checks and verification

A trail is only useful if someone can prove it has not been altered. Plan a verification job or tool that runs on a schedule and on demand.

- Detect gaps in sequence, missing blobs, and hash mismatches.
- Report results somewhere compliance officers can read without help from engineering.
- Decide who gets notified on a failure and what the first response step is.
- Keep the verifier read-only. It must not be able to repair anything by itself.

Consider whether entries should be chained to each other, so that removing one is visible. This needs a design note first, because it affects write concurrency.

## Tests, observability and operations

Add tests that describe the guarantees from the first section, one test per promise where possible. Include failure cases: database unavailable, blob store slow, broker restart in the middle of a batch, and clock differences between machines.

- Structured logging that carries a correlation value from the original event through to the stored entry, without logging sensitive notebook content.
- Metrics for lag between event creation and entry write, retry counts, and dead-letter volume.
- A short runbook for the common failures: stuck queue, failed verification, storage outage.
- A look at how time is recorded. Use one clear source of time and store it in a form that avoids timezone confusion.

## Open questions

- Who owns the decision on retention and on what happens when a record must be legally erased while the trail must stay intact?
- Does any consumer outside this project read the trail directly from the database? If so, that needs to be listed before any schema work.
- How much of the verification tool should be exposed to compliance users versus kept as an engineering tool?
- Is the current handling of corrections to entries acceptable to auditors, or does it need a different shape?

Record answers to these in their own notes when they are settled, and link back from here.
