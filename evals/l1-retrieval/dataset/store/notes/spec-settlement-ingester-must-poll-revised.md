---
id: 01M20JDFGX1ESF4VJFPFKCT4G3
created: 2026-09-08T10:13-03:00
---

# settlement-ingester spec: SFTP polling interval

settlement-ingester must poll the processor SFTP drop every 120 seconds. This note replaces the earlier note "settlement ingester must poll", and the new interval value is 120 seconds; anything that still quotes the old interval is stale and should be corrected to match this note.

This is a spec for how settlement-ingester fetches card-processor settlement files in Ledgerlark. It is written quickly, from what we settled, and it is meant to be read alone. Where a detail is not pinned down here, it is deliberately left general and the code or the Terraform is the source of truth.

## Purpose

Ledgerlark reconciles settlement files from card processors against internal ledger entries and flags mismatches for finance operations review. The settlement-ingester component is the front door of that pipeline. It gets files off the processor SFTP drop, checks them, and hands their contents downstream. If it is late or silently skips a file, the reconciliation side shows false mismatches or no data at all, and finance teams at the marketplaces notice quickly.

Polling is the only way we learn that a new file exists, because the processors do not push notifications to us. So the polling interval is the main knob that sets how fresh the reconciliation data is.

## Polling interval

settlement-ingester must poll the processor SFTP drop every 120 seconds. That is the whole rule. Each cycle lists the drop, compares what it sees against what has already been recorded as ingested, and picks up anything new.

The interval is measured between the start of one poll and the start of the next, not between the end of one and the start of the next. A slow listing should therefore not push the schedule later and later. If a poll runs longer than the interval, the next one starts right after it finishes rather than overlapping with it. There is never more than one poll in flight per drop.

The earlier interval was longer. It was changed because finance operations wanted settlement data to show up in review sooner, and the load that a listing puts on the processor SFTP servers was judged acceptable at this rate.

## Why this value

Shorter polling gains little, since the processors publish files in batches and not continuously. Longer polling made the delay between a file landing and a mismatch being flagged noticeable to the people reviewing. The chosen value sits between those. It is also small enough that a single missed cycle, for example during a transient network error, does not turn into a visible gap for reviewers.

Processors differ in how they treat frequent logins. Some rate-limit or flag clients that reconnect too often. Reusing a connection across cycles where the server allows it keeps us on the polite side, and the implementation should prefer that over reconnecting every time.

## Scheduling behaviour

The poll loop is driven by a timer inside the Go service, not by an external cron. On startup, settlement-ingester runs a poll immediately and then settles into the regular rhythm. That way a restart does not add a full interval of delay on top of the downtime.

If several drops are configured, each has its own loop with the same interval, and a stuck drop must not delay the others. Add a small random offset at startup so that multiple instances or multiple drops do not all hit their servers at the same instant. The offset is only for the first poll and must not change the steady interval.

## Failure handling

A failed poll is logged, counted in metrics, and retried at the next scheduled time. Do not retry in a tight loop; the regular interval is the retry cadence. Authentication failures are treated differently from transient network errors: a rejected credential should raise an alert quickly, because retrying will not fix it and repeated bad logins can lock the account at the processor.

If the drop is unreachable for a long stretch, the service keeps trying at the same interval and does not back off to some longer period on its own. Operators can see the backlog through the lag metric described below. When connectivity returns, the next successful poll picks up everything that accumulated.

## Idempotency and duplicates

Because polls are frequent and files can sit in the drop for a long time, the same file will be seen many times. The ingester must be idempotent. It records each ingested file in PostgreSQL, keyed by the processor, the file name and a content checksum, and skips anything already recorded.

A file that appears with the same name but different content is not silently overwritten. It is stored as a separate version and raised for review, since processors occasionally re-issue a corrected file. Partial files still being uploaded must not be consumed; the ingester waits until the size and modification time are stable across two looks, or until the processor's completion marker exists, depending on the processor.

## Downstream handoff

After a file is stored and validated, settlement-ingester publishes its records to Apache Kafka for the reconciliation side to consume. The publish and the database record of ingestion must agree: a file is marked ingested only once its records are durably in Kafka. If the service dies between the two, the next poll sees an unrecorded file and repeats the publish, and consumers must tolerate duplicates using the stable record keys.

Internal services that need to ask about ingestion state, such as the status of a particular file, use gRPC against the ingester. That API is read-only with respect to polling; it does not change the interval or trigger extra polls.

## Configuration

The interval is a configuration value, not a constant buried in code, so it can be changed without a rebuild. The deployed default must be 120 seconds, and the Terraform that defines the service environment should set it explicitly rather than relying on a default in code. If the code default and the Terraform value ever disagree, the Terraform value wins in deployed environments, and the disagreement is a bug to fix.

Reject nonsense values at startup, such as zero or negative durations, and refuse to run instead of falling back quietly. Very small values should also be refused, to avoid hammering a processor by accident. Changing the interval in any environment should be done through review like any other infrastructure change.

## Observability

Track a few things per drop: time of the last successful poll, count of files discovered, count ingested, count skipped as duplicates, and poll duration. The most useful single signal is lag, meaning the time since the last successful poll. Alert when lag exceeds a few multiples of the polling interval, which means several cycles in a row failed, and not on a single failure.

Logs for each poll should include the drop name and outcome, and never credentials or file contents. Settlement files carry card-related data, so treat them as sensitive in logs and traces.

## Testing and open points

Tests should use a fake clock so the interval can be verified without waiting in real time: assert that polls start on schedule, that a slow poll does not overlap the next, and that a restart polls at once. A separate test should cover the duplicate-skip path using a fake SFTP listing that returns the same files repeatedly.

Open points: whether to add an on-demand poll for operators who know a file just landed, and whether processors with known publishing windows should get a tighter or looser schedule. Neither is decided. Until someone decides otherwise, every drop uses the same rule: poll every 120 seconds.
