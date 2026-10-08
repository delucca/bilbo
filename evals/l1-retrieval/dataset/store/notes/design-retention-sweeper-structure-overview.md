---
id: 01KSJ9E69NY78KWNVADQC40296
created: 2026-05-26T11:01-03:00
---

# retention-sweeper: overall structure

The retention-sweeper is the background part of LabNotebook Sync that decides what stored data is past its retention window and removes or archives it, while keeping the audit trail whole. It is a .NET worker written in C#. It reads policy and record state from SQL Server, looks at blobs in Azure Blob Storage, and talks to the rest of the system over RabbitMQ. This note describes how it is put together, not what any particular policy value is. The open question of which approach to take for the next round of work is in [[retention-sweeper-direction-chosen]].

## Purpose and boundaries

Scientists write notebook entries and instruments push output files. Compliance officers need to show that records were kept as long as required and no longer than allowed, and that every deletion can be explained afterwards. The retention-sweeper owns the "no longer than allowed" half, and it must never break the "kept as long as required" half.

It does not create entries, does not ingest instrument output, and does not decide policy. It applies policy that someone else defined. If a record is in doubt, it leaves the record alone.

## Main pieces

The component splits into a few parts that are easy to find in the code:

- A scheduler that wakes the sweep on a cadence and also accepts on-demand requests.
- A candidate finder that queries SQL Server for records that may be eligible.
- A policy evaluator that takes a candidate and answers keep, archive or delete, plus the reason.
- A hold checker that vetoes anything under legal hold, open audit, or open review.
- An executor that carries out the action against blob storage and the database.
- An audit writer that records what was decided and what was done.
- A messaging layer that publishes outcomes and consumes control messages.

Each piece has an interface so the evaluator and hold checker can be tested without a database.

## Scheduling and triggers

The scheduler runs the sweep on a timer. A sweep can also be triggered by a message, for example when an administrator asks for a run after a policy change. Only one sweep runs at a time per environment; a second trigger while one is active is dropped or queued, not run in parallel. The guard is a lock row in SQL Server, not an in-process flag, so that two instances of the worker do not both sweep.

A sweep is split into batches. Batches keep transactions short and let the worker stop cleanly when told to shut down.

## Candidate discovery

The finder does not scan whole tables. It uses indexed columns for record age and state, and walks them in key order with a stored cursor so a restarted sweep resumes rather than starts over. It returns identifiers and the minimum fields the evaluator needs, not full records.

Candidates come in two groups: notebook entries with their attachments, and instrument output files that are linked to entries. Instrument files are the larger group by volume, so they get their own batch sizing.

## Policy evaluation

Policies are data, held in SQL Server, and are looked up per record by its type and the project or group it belongs to. The evaluator is a pure function over the record facts and the policy. It returns an action and a reason code. It has no side effects, which is what makes it safe to run in a dry mode.

When more than one policy applies, the stricter retention wins. When no policy applies, the answer is keep, and the sweeper raises a warning so that someone fixes the gap.

## Holds and vetoes

The hold checker runs after the evaluator and can only turn a delete into a keep. It looks at legal holds, records referenced by an open audit or investigation, entries not yet signed off or still in review, and records linked from other records that are themselves retained.

Linked records matter. A file may look old but be cited by a newer entry. The checker follows those links before a delete is allowed.

## Execution

The executor handles three actions. Keep does nothing but note it. Archive moves blobs to a cheaper tier and marks the row. Delete removes the blob and then marks the row as purged.

Order matters. The database state change and the blob change cannot share one transaction, so the executor writes an intent first, does the blob operation, then confirms. If it crashes in between, the next sweep sees the intent and finishes or rolls it back. Deletes of blobs tolerate "already gone" as success.

Notebook rows are not hard-deleted by default. A tombstone stays so that references and audit entries still resolve.

## Audit trail

Every decision writes an audit record, including keeps that were caused by a hold. Each record carries the record identity, the policy used, the reason code, the actor (the worker identity or the administrator who triggered it), and the outcome. Audit rows are append-only and the sweeper's database role has no update or delete right on them.

The audit write happens before the destructive step, as the intent. The outcome is appended after. A deletion with no audit row is treated as a bug.

## Messaging

RabbitMQ carries a few message types. The sweeper publishes events when a record is archived or purged, so that other components can drop caches and search entries. It consumes control messages: start a sweep, pause, resume, and policy changed.

Consumers are idempotent, since delivery can repeat. Messages that fail repeatedly go to a dead-letter queue and raise an alert instead of blocking the sweep.

## Dry run mode

The worker can run the whole pipeline without executing. In dry run the evaluator and hold checker run as normal and the audit writer records the proposed action, flagged as not executed. Compliance officers use this to review what a new policy would do before it is turned on. The same code path is used, only the executor is swapped for one that does nothing.

## Configuration

Settings cover the cadence, batch size, concurrency of blob calls, and storage connection details. They come from the normal .NET configuration layers. Secrets for SQL Server, RabbitMQ and Azure storage come from the host secret store, not from files in the repo. Policy itself is not configuration; it lives in the database.

A small shape of the main flow, for orientation:

```
find candidates (SQL Server)
  -> evaluate policy
  -> check holds
  -> write audit intent
  -> act on Azure Blob Storage
  -> confirm in SQL Server
  -> publish event (RabbitMQ)
```

## Failure handling

Transient errors from storage or the database are retried with backoff and a limit. After the limit the record is skipped for this sweep and flagged, and the sweep goes on. A single bad record must not stall the batch.

If the audit write fails, the action is not taken. If the confirm step fails after the blob action, the intent row is what recovers it. The worker logs with the record identity on every line so a compliance question can be traced from logs back to audit rows.

## Observability

The worker exposes counts of candidates seen, kept, archived, purged, vetoed and failed, plus sweep duration and lag between a record becoming eligible and being handled. A rising veto count or a rising skip count is the first sign of a policy or data problem. Logs are structured.

## Testing

Evaluator and hold checker tests are plain unit tests with built records. The executor is tested against a storage emulator and a throwaway SQL Server database, including crash-in-the-middle cases that leave an intent behind. Messaging is tested with a real broker in a container where possible. There is a small set of end-to-end runs in dry mode against sample data.

## Known rough edges

- Link following for the hold check is the slowest step and is easy to make wrong when a new record type is added.
- The two-step blob and database change relies on recovery from intents, which needs care whenever the executor changes.
- Policy gaps produce keeps by default, so storage can grow quietly if warnings are ignored.
- Instrument file volume strains batch sizing more than notebook entries do.
