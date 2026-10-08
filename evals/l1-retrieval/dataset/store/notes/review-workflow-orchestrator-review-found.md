---
id: 01KVMNMKW2WRHCN3MHS6HJNC28
created: 2026-06-21T05:44-03:00
sources:
  - "doc: Workflow failure review"
---

# Review of workflow-orchestrator: missing Catch in packaging state

Reviewed the workflow-orchestrator (internal codename: `stepmaster`) and found one real problem that matters for operations: the packaging state in the state machine has no `Catch` clause. When packaging fails for any reason, nothing routes the execution to a failure path, and the job stays in `RUNNING` instead of ending as failed. Media ops people see a job that never finishes and never errors. This note records the finding, the reasoning, and what I would do next. Nothing has been changed yet.

## Naming

The component is `workflow-orchestrator`. People on the team also call it `stepmaster`, which is the internal codename. Both names refer to the same thing: the AWS Step Functions based layer that drives a job from upload through transcoding to packaging. If you grep logs, dashboards or old tickets and only search for one name, you will miss hits. Search for both.

## What was found

The state machine runs the ladder transcode, then a packaging state that builds the HLS output and writes it to S3. The packaging state has Retry-free, Catch-free task definitions. In Step Functions, an error with no matching `Catch` and no handler higher up fails the execution, but our job tracking does not only depend on the execution status. The job record is moved into `RUNNING` when work starts, and only the success path and the explicit failure states move it out. Because the packaging failure never reaches an explicit failure state, the record is never updated.

So there are two things wrong at once:

- The packaging state has no `Catch` clause, so errors are not routed anywhere we control.
- The job status in our own records is only changed by states we wrote, so an unhandled error leaves it in `RUNNING` forever.

The symptoms match what operators reported: jobs that look in progress for a very long time, with partial or missing packaged output in S3.

## Why it matters

Stuck jobs are worse than failed jobs here. A failed job gets looked at and can be resubmitted. A job in `RUNNING` looks healthy, so nobody retries it, and the publisher waits on a video that will never appear. It also hides real packaging bugs, since the error never lands in any place that alerts.

Typical causes of packaging failure that would hit this gap: a bad or truncated rendition from the FFmpeg step, S3 write errors, and malformed input that passes transcoding but breaks playlist generation. None of them are handled today.

## Suggested fix

Add a `Catch` clause to the packaging state that catches all errors and sends the execution to a dedicated failure state. That failure state should do three things: set the job record to a failed status with the error cause attached, clean up or mark any partial packaged output in S3 so it is not served, and emit something that alerting can see.

Also check the other task states in the same machine. If the packaging state was missed, the transcode state may have the same gap. I only looked closely at packaging, so treat the rest as unreviewed. A sensible rule going forward is that every task state has a `Catch` that ends in the shared failure state.

Retries are a separate question. Transient S3 errors probably deserve a bounded retry before the `Catch` fires, but deterministic failures such as bad input should skip retries and go straight to the failure path.

## Open items

- Decide the failed-status shape on the job record and what operators see in the UI.
- Find the jobs currently stuck in `RUNNING` and decide whether to fail them by hand or resubmit them.
- Add a test or a state machine lint check that rejects any task state without a `Catch`.
- Confirm whether the transcode state has the same gap.
- Add an alarm for jobs that stay in `RUNNING` longer than a reasonable limit, as a safety net even after the `Catch` fix, since other kinds of hangs can still happen.
