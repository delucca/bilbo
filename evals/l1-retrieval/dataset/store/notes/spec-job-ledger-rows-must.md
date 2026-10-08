---
id: 01KQ028GFAVVYWM6VPZZEWN39E
created: 2026-04-24T12:38-03:00
---

# job-ledger retention spec

This note specifies how rows in the job-ledger expire. The job-ledger is the record of every transcode job ReelForge has accepted: what was uploaded, which ladder was requested, how far the job got, and where the packaged HLS output ended up. Rows in the job-ledger must expire through the TTL attribute `expires_at`, set `90 days` after creation. That is the whole retention rule. The rest of this note says what it means in practice and what the people and code around the ledger must do about it.

## Scope

This covers the lifetime of ledger rows only. It does not cover the lifetime of the media itself: source uploads and packaged renditions in S3 follow their own bucket rules. A row can expire while its output is still being served, and that is fine, because playback never reads the ledger.

It also does not cover the Step Functions execution history. Executions keep their own history under their own settings. The ledger is the durable summary that operations teams look at, and it is the only thing this spec governs.

## The rule

Every row carries an attribute named `expires_at`. The store's TTL feature is configured to watch that attribute. The value is set once, at row creation, to the creation time plus `90 days`. After that moment the store is free to remove the row.

Two points matter for anyone reading code. First, the value is computed from the creation time, not from the last update. A job that finishes late or gets retried does not get a longer life. Second, the value is written by the ledger code in the Rust service, never by callers. No other component sets or edits `expires_at`.

## Format of the attribute

`expires_at` is stored as a numeric epoch timestamp in seconds, which is what the TTL feature expects. A string, a date-time text value or a millisecond value would be ignored by the expiry process, and the row would then live forever without anyone noticing. The write path should refuse to build a row without a correctly typed `expires_at`.

In Rust, keep the conversion in one small function next to the row type so the unit and the type cannot drift apart. Tests should assert the value is exactly the creation time plus `90 days`, expressed in seconds.

## Who writes it

The job-ledger writer is the only place that creates rows. It is called when a job is accepted, before the first Step Functions execution is started. The creation time is taken from the service clock at that point and reused for both the stored creation field and the expiry calculation, so the two always agree.

Retries of the same job reuse the existing row. They must not insert a fresh row, because that would restart the clock and leave two rows for one job.

## What operators can expect

Operations teams at the publishers can look up a job for `90 days` after it was created. After that the row is gone and the ledger has no memory of it. If a team needs to keep a job's details longer, they should export them within that window; the ledger will not hold them back.

Support answers should say plainly that a missing row older than the window is normal and is not a sign of data loss.

## Deletion is not instant

TTL removal is a background process. A row may remain readable for some time after `expires_at` has passed. Readers must therefore not assume that a row they can fetch is still within its life. Where it matters, the read path compares `expires_at` with the current time and treats a past value as absent.

Do not build features that depend on the exact moment a row disappears. Nothing in ReelForge should wait for it, poll for it, or alert on its timing.

## Reads and listings

Listings shown to operators filter out rows whose `expires_at` is in the past, using the same comparison as single-row reads. This keeps the user-facing view consistent even while the background removal lags behind.

Counts and dashboards built on the ledger should be understood as covering the retention window only. Anything that wants longer history needs its own store fed from job events, not the ledger.

## Updates to existing rows

Status changes, stage progress and output locations are written to the row as the job moves through the pipeline. These updates must leave `expires_at` alone. The update expressions in the Rust code should list the fields they touch explicitly rather than rewriting the whole item, so a stale copy of a row can never overwrite the expiry value.

## Backfill of older rows

Any row that predates this rule and has no `expires_at` will never expire by itself. A one-off backfill should set `expires_at` on those rows, computed from each row's own creation time plus `90 days`. Rows whose computed time has already passed will then be removed by the normal process. Run the backfill outside peak hours and make it safe to repeat, since it only sets a missing attribute.

## Testing

Unit tests cover the expiry calculation and the numeric type. An integration test creates a row through the writer and reads it back, checking that `expires_at` is present and sits `90 days` after the stored creation time. A separate test checks that an update does not change the value.

Do not write tests that wait for real expiry. The background removal is not predictable enough, and the test would be slow and flaky. Test the attribute, not the deletion.

## Monitoring

Watch for rows with no `expires_at`. A scheduled check that samples recent rows and reports any missing or wrongly typed value is enough. A missing value means the writer is broken, and the cost is silent growth of the ledger. Also keep an eye on overall table size; steady growth beyond what job volume explains points at the same problem.

## Changing the window

The window is a product decision, not a tuning knob. If it changes, update this note, the constant in the Rust writer, the tests and the operator-facing documentation together. A change applies to new rows only unless a backfill is run on purpose, so say clearly in the change which rows are affected.

## Open items

- Decide whether the sampling check should run inside the service or as a separate scheduled job.
- Write the backfill as a small standalone Rust binary and keep it with the other maintenance tools.
- Add a line to the operator guide stating the retention window in plain language.
