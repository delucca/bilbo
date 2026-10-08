---
id: 01M2BCDW2A4W8007T2BQ7JEBEV
created: 2026-09-12T15:00-03:00
---

# blob-archive-client upload retry limit

This note replaces the earlier note "blob archive client must". The new value: blob-archive-client must retry a failed upload at most 3 times, which replaces the earlier limit of 5 retries.

The rest of this note says what the limit covers, why it was lowered, and what the code and its callers have to do about it. It is written quickly, so check the code before trusting any detail that is not the retry limit.

## The rule

blob-archive-client must retry a failed upload at most 3 times. The old limit was 5 retries, and that value is no longer valid. If you find 5 in config, in a constant, in a test or in another document, treat it as stale and change it.

The limit counts retries, not attempts. The first try is not a retry. If the first try fails, blob-archive-client may try again, and it may do that at most 3 times. After the last permitted retry fails, it gives up on that upload and reports the failure to its caller. It must not make a further hidden attempt, and it must not restart the count by wrapping the call in another loop.

The limit applies per upload. Each blob that blob-archive-client sends to Azure Blob Storage has its own count. A failure on one blob does not use up retries for the next blob. Equally, a long batch must not share one counter, or the tail of the batch would silently get no retries at all.

## Why it was lowered

The old value let a broken upload sit in the pipeline for too long. A lab entry could look pending to a scientist while the client was still hammering a storage account that was plainly unhealthy. Compliance officers read that delay as a gap in the audit trail, because the entry existed in the notebook but its archived copy did not exist yet, and nothing said why.

A lower limit means an unrecoverable failure shows up sooner and louder. Failures that really are transient, such as a brief network drop or a throttled request, normally clear within a few retries. If they do not clear by then, more retries rarely help and mostly hide the problem.

There is also load. When storage is struggling, many clients retrying many times in a row make it worse. Fewer retries per upload reduces that pressure.

## What counts as a retry

A retry is a repeat of an upload that failed with a condition that might go away by itself. Examples of that kind of condition are a dropped connection, a timeout, a server busy response, and a temporary service error from Azure Blob Storage. These are the only failures that should use the retry budget.

A failure that cannot be fixed by repeating the same request is not retried at all. Examples are a rejected credential, a missing container, a malformed request, a blob that is refused because of a content check, and a local read error on the source data. Those fail on the first try and go straight to the caller. Do not spend any of the retries on them.

If you are unsure whether a given error is transient, treat it as not transient and fail fast. A wrongly retried permanent error wastes the budget and delays the report. A wrongly failed transient error is caught by the caller's own handling, described below.

The delay between retries is a separate matter from the count. Keep a growing wait between attempts with some jitter, so that many clients do not retry in lockstep. This note does not fix the wait values. Whatever they are, the total time spent across the permitted retries should stay short enough that nobody is left wondering whether the upload is stuck.

## What the caller sees

When blob-archive-client runs out of retries, it returns a clear failure to the caller. The failure should carry enough to diagnose the cause: which blob, which step, and the last underlying error. It should also say that the retry budget was used up, so a reader of a log can tell "gave up after retrying" apart from "failed on the first try with a permanent error".

The caller owns what happens next. The sync service that moves notebook entries and instrument output decides whether to requeue the work through RabbitMQ, to park it for a person to look at, or to mark the entry as not archived. blob-archive-client must not requeue by itself, because that would be a second retry loop on top of the first and would defeat the new limit.

Whatever the caller does, the audit trail must show the failure. An upload that gave up is an event worth recording, with the time, the entry it belongs to and the reason. It must never disappear quietly. Entries and records in SQL Server that point to an archived blob must not be marked as archived until the upload really succeeded.

The queue side needs the same care. If a message is redelivered after a failed upload, the new delivery gets a fresh, separate call to blob-archive-client, with its own count of at most 3 times. That is allowed, but the queue should have its own limit on redeliveries and a dead-letter path, so that a poison message cannot loop forever. Keep the two limits separate in your head and in the config.

## Idempotence and partial uploads

Retrying only makes sense if repeating an upload is safe. An upload that is repeated must give the same final blob as one that worked the first time. Use a stable blob name derived from the entry and its content, not from the attempt, so a retry overwrites or reuses the same target rather than creating a duplicate.

A failed attempt may leave a partial result on the storage side. Before or while retrying, the client has to make sure the final blob is complete and matches the source content. Checking a content hash after the upload is the safest way to do it, and it matters for compliance because the archived copy is evidence.

If a retry succeeds, the blob is archived once, and the result looks the same as if no failure had happened, apart from a log line noting that a retry was needed. Do not write a second audit record for the same content just because there was a retry. The audit record should describe the archive event, and may mention how many retries it took.

Large instrument output is the case to watch. Re-sending a very large file several times is expensive. If the client uploads in blocks, a retry should resume at the block level where possible and not start the whole file again. That does not change the count: block-level retries still sit inside the same limit for that upload, so the total for one upload never exceeds the limit in this note.

## Config and code changes

The limit should live in one place, a single named setting or constant in blob-archive-client, and not be repeated in several places. When you change it, search the repository for the old value and for any wording that mentions it, including comments, README text, test fixtures and deployment settings. Lower environments and production should agree.

If the limit is configurable, the default must be the new value, and a configured value above the new limit should be rejected or clamped at startup with a clear message. Silently accepting a larger number would bring back the old behavior without anyone noticing.

Tests to keep or add:

- A transient failure that clears after fewer retries than the limit ends in success.
- A transient failure that never clears stops after at most 3 times and returns the failure with the retry-budget-used-up marker.
- A permanent error fails on the first try, with no retries.
- Two uploads in a row do not share a retry count.
- A retried upload leaves one blob with the right content and one audit record.

Tests that expect the old limit must be changed. Do not loosen them to accept either value.

## Open points

The wait between retries is not settled here, and neither is the exact list of errors counted as transient. Both should be written down in a short follow-up note once they are agreed, and they should be checked against what Azure Blob Storage actually returns.

The handling of an upload that gave up, on the sync service side, needs a clear owner. Right now the behavior depends on the caller, and the compliance side should confirm that the way failures are recorded is enough for their review.

The earlier migration of audit records may touch the same code paths. See [[audit-trail-migrate-prev]] for that history before changing how archive events are recorded.

If anyone proposes raising the limit again, the answer needs a reason that deals with the delay and load problems above. Until then the rule stands: blob-archive-client retries a failed upload at most 3 times.
