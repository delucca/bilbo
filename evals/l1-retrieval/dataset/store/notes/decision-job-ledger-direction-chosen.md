---
id: 01KR0P0NBETG2SQKPRXAVNKE1Y
created: 2026-05-07T04:39-03:00
---

# job-ledger: general direction

We settled on a general shape for job-ledger and this note keeps the direction, not the tuning. Details can change; the reasons below should not.

job-ledger is the record of what happened to each transcode job, from upload to packaged output. Step Functions runs the workflow, FFmpeg does the work, and the ledger says what state each job is in.

## Why this note exists

People kept asking where job state should live and who is allowed to change it. We want one answer written down so nobody re-argues it each time a new stage is added.

## Direction in one paragraph

job-ledger is the single source of truth for job state. Workflow executions and workers report into it. They do not keep their own private copy of state that anyone else relies on. If the ledger and an execution history disagree, the ledger wins for operators, and the disagreement gets looked at.

## Append-first records

We record changes as events added to the ledger, not as edits that overwrite earlier state. The current state of a job is derived from its events. This costs a little more on read, but it gives operators a trail when a publisher asks why a rendition is missing.

## Writers

Only a small set of components write to job-ledger: the workflow layer and the workers it starts. Operator tools read. When an operator needs to correct something, the correction is itself a new event with a reason, not a quiet fix.

## Idempotency

Workflow steps get retried, so every write has to be safe to repeat. A repeated report of the same thing must not create a second outcome. We lean on this instead of trying to prevent retries.

## Storage choice

The ledger is kept on durable storage that sits with the rest of the AWS setup, with S3 holding larger artifacts and the ledger holding references to them. We do not put media or large logs inside ledger records.

## Relationship to S3 outputs

A job is not marked finished until the ledger records that its outputs exist where the packaging expects them. The ledger points to outputs; it does not replace checking them. Reconciliation between the two is a periodic job, not a hand task.

## Relationship to HLS packaging

Packaging reads the ledger to know which renditions of the ladder are complete. A partial ladder is a recorded state with a clear meaning, not a failure by default. Publishers differ on whether they accept partial ladders, so that stays a policy choice outside the ledger.

## Failure recording

Failures are recorded with a category and enough context to act on, such as which stage failed and whether a retry is sensible. Raw FFmpeg output is stored by reference, not copied into the record.

## Retention

Old records are kept long enough for operations teams to answer publisher questions, then archived. The exact period is a policy matter for operations and is not fixed here.

## Access and privacy

Records can name publishers and their content, so access follows the same rules as the media itself. Ledger data should not leak into places with wider access, such as general logs.

## Schema changes

Records change by adding, not by reinterpreting. Old events stay readable. If a meaning has to change, add a new event type and leave the old one alone.

## What we ruled out

- Letting each workflow execution own its state with no shared record.
- Overwriting job rows in place.
- Storing media or large payloads in the ledger.

## Open questions

- How operators should browse the ledger without touching storage directly.
- Whether reconciliation needs its own alerting.
- How to present partial ladders to publishers who care.

## Revisit when

Revisit if write volume or read patterns make the derived-state approach painful, or if the workflow layer changes enough that reporting into the ledger no longer fits.
