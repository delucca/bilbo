---
id: 01M0ZDAZ4JNWJ3XZ17A80Y5VBR
created: 2026-08-26T13:09-03:00
---

# job-ledger spec

This is the spec for job-ledger, the record ReelForge keeps of each transcode job from upload through packaging. The retention rule is that a row lives for `30 days` and then expires. That replaces the earlier value of 90 days, which was shortened to limit storage cost. The rest of this note says what the ledger holds, what the shorter TTL changes, and what stays the same.

## Replaces an earlier note

This note replaces the earlier note "job ledger rows must". That note gave the retention as 90 days. The value to use now is `30 days`. If you still see the old note or its wording anywhere, treat the old number as wrong and follow this note.

## Naming

The component is called `job-ledger`. Its previous name was `jobtab`. Both names point at the same thing: the table of per-job records that the pipeline writes to and the operations tooling reads from. It is not a new component and was not rebuilt under the new name.

Old code, dashboards, runbooks, alarms and chat history may still say `jobtab`. When you meet it, read it as `job-ledger`. In anything you write now, use `job-ledger` only. Do not invent a third name or a short form. If a search for `job-ledger` finds nothing in some place, search for `jobtab` there before concluding the thing does not exist.

## The TTL rule

The job-ledger TTL is `30 days`. Earlier it was 90 days. The change exists for one reason, which is storage cost. Nothing about correctness or about the packaging output drove it.

```
job-ledger ttl: 30 days   # was 90 days; shortened to limit storage cost
```

The TTL applies to the row, not to the media. Expiring a row does not delete the ladder renditions or the HLS packages that the job produced. Those live in S3 under their own retention and are governed separately. Only the ledger record of the job goes away.

A row is eligible for expiry once its age passes the TTL. Age is counted from when the row was created, not from its last update. A job that is retried or re-packaged keeps its original clock unless it is submitted as a new job, in which case it gets a new row and a new clock.

## Why it was shortened

The ledger grows with every upload, and each job writes several state transitions plus per-rung details for the bitrate ladder. At the volume independent publishers send, most rows are never read again after the first stretch following completion. Keeping them for the old, longer period paid for storage nobody used.

The shorter TTL cuts that cost without touching anything that operators use day to day. Operators look at recent jobs: did the ladder finish, which rung failed, was the package published. Old rows were mostly kept out of caution. If someone needs a longer history for a particular purpose, that should be solved by exporting what they need, not by lengthening the TTL for everyone.

## What a row is for

A job-ledger row is the answer to "what happened to this upload". It should carry enough to answer that without opening logs. In general terms it holds:

- the job identity and the source object it came from
- the current state and the history of states it passed through
- the ladder that was requested and the result for each rung
- the location of the packaged HLS output once it exists
- failure information when a stage did not succeed, including which stage and the error that was reported
- timestamps for creation and for the last change

It is a ledger, so entries describe what occurred. It is not the source of truth for the media itself, and it is not a queue. Nothing should be scheduled off the ledger.

## Expiry behaviour

Once a row expires it is gone. Tools that look a job up by identity will get not found for jobs older than the TTL. That is expected and is not an error in the pipeline.

Expiry is not instant at the boundary. Storage-level TTL removal is lazy, so a row can remain readable for a while after it is due. Do not write code that relies on a row still being present just past the cutoff, and do not write code that relies on it being absent. Treat the TTL as a floor on how soon it disappears from the point of view of cleanup, and as a ceiling on how long we promise to keep it.

Anything that needs to reason about "is this job too old to act on" must compute that from the timestamps in the row or from the source object, not from whether the row exists.

## Interaction with Step Functions

The workflow in AWS Step Functions writes to the ledger as it moves through its states. Each state transition records into the row. The workflow does not read the ledger to decide where to go next; the state machine holds its own execution state. The ledger is a record written alongside it.

Because of that, shortening the TTL does not affect running executions. A job that is in flight is far younger than the TTL, so its row is not at risk. The only case where an execution could outlive its row is a stalled or paused execution held open unusually long. If that ever happens, the workflow should keep working and the lost row should be treated as lost history, not as a reason to fail the job. A write to a row that has expired would simply create a fresh partial record, and that is acceptable.

Retries from the workflow update the same row. They do not create new rows.

## S3 and the media

The uploaded source and the produced renditions sit in AWS S3. FFmpeg works on those objects and the packaging step writes HLS playlists and segments back. The ledger stores references to them, not copies.

When the row expires, the references vanish with it, but the objects stay until their own lifecycle rules remove them. That can leave media in S3 that no ledger row points to. This is accepted. If cleanup of orphaned output matters, it should be done by listing the bucket, not by relying on the ledger, because after the TTL the ledger no longer knows about them.

Do not tie the S3 lifecycle to the ledger TTL by assumption. They are separate settings and either can change without the other.

## Operations notes

For media operations teams the practical effect is a shorter lookback in any view backed by the ledger. If a publisher asks about a job from long ago, the ledger will probably not have it. Tell them up front that history in the ledger is kept for `30 days`.

When reading old runbooks, remember that anything saying 90 days is out of date. Anything calling the table `jobtab` is describing `job-ledger`.

If you need to preserve a specific job's record beyond the TTL, copy what you need out before it expires, such as the failure details or the output location. Do not try to extend one row's life by hand.

When changing the TTL again, update this note in place and keep the old value stated as the earlier one, so a reader can tell what changed.

## Open questions

- Whether a separate, cheaper archive of final outcomes is worth having for the cases where people want longer history. Not decided.
- Whether the row should keep a compact summary after the detailed per-rung data is dropped. Not decided, and not part of this spec.
- Whether lazy removal needs an explicit sweep for cost reporting. Not needed so far.

## Related

See [[upload-gateway-must-reject]] for what the upload side refuses before a job, and so a ledger row, ever exists. Rows only describe jobs that were accepted, so rejected uploads leave nothing in job-ledger.
