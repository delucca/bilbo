---
id: 01JTF7T53VR7XP0E5JH00YPK0P
created: 2025-05-05T00:28-03:00
---

# sync-outbox-relay weekly recap

Most of the week on sync-outbox-relay went to reading how it behaves under load and tidying the edges that bit us before. Nothing big shipped. This is a rough recap so the next session does not have to rebuild the picture.

## Where things stand

sync-outbox-relay still reads pending rows from the SQL Server outbox and publishes them to RabbitMQ. The audit trail side depends on it, so we treated every change as risky and kept diffs small.

## Polling loop

Went through the polling loop again. It works, but the idle behavior is wasteful and the backoff feels crude. I sketched a gentler approach and did not commit to it. Needs a look with real traffic shapes before anyone touches it.

## Publish confirms

Confirmed again that a row should only be marked as sent after the broker acknowledges it. Read the code path twice. It matches that intent. I want a test that kills the process between publish and mark, since that is where duplicates would come from.

## Duplicates

Consumers have to tolerate repeats. That is the contract, and it is still the contract. I added a reminder in the code comments near the publish call so nobody assumes exactly-once.

## Ordering

Ordering per notebook entry matters for the audit view. The relay keeps rows for one entry in sequence, but I found a spot where a retry could let a later row slip ahead. Not fixed. Logged it for the next pass.

## Retries and poison rows

A row that keeps failing can hold up its neighbors. We looked at parking such rows aside with a flag so compliance can review them. Only discussed, nothing built.

## Instrument output

Instrument-derived payloads are sometimes large, and those go through Azure Blob Storage with a reference in the message. Checked that the relay passes the reference through untouched. It does.

## Blob references

Wondered what happens if the blob is not yet readable when a consumer gets the message. That is on the consumer side, but worth a note to the owners.

## Database access

Reviewed the queries that claim rows. Locking behavior looks reasonable. I would still like to see the execution plans on a production-sized copy before changing indexes.

## Configuration

Config is spread over a few places and some defaults are only in code. Started listing them so they can be moved into one documented spot. List is incomplete.

## Logging

Log lines around failures were vague. Improved a couple so the entry and the failing stage are both visible. Careful not to log notebook content, since that can be sensitive.

## Metrics

We have little visibility into how far behind the relay runs. A backlog age metric would help operators and compliance. Not started.

## Tests

Unit tests cover the mapping from row to message well. Integration coverage against a real broker is thin. That is the biggest gap I saw this week.

## Risks

Silent stalls are the worst case: the relay looks alive while nothing moves. A health signal tied to actual progress would catch that.

## Next

Write the crash-between-publish-and-mark test, fix the retry ordering gap, then add the backlog age metric. After that, revisit the polling backoff.
