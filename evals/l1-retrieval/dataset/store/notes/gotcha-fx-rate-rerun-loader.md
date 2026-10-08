---
id: 01KJN2VC8615MR8J5DZBDW8WPS
created: 2026-03-01T13:14-03:00
---

# fx-rate-loader rerun fails on duplicate key

Rerunning fx-rate-loader for a date that already has rates loaded fails with `duplicate key value violates unique constraint` when the upsert flag is disabled. This is expected behavior of the insert path, not a bug in the loader, but it surprises people because the first run looks clean and the second one dies. Written down so nobody burns an hour on it again.

## Symptom

The loader exits non-zero part way through the write step. The Postgres error text is `duplicate key value violates unique constraint`, followed by the name of the constraint on the rates table. No rows from the second run are kept for the affected batch. The first run's rows stay as they were.

## When it happens

Only when both of these are true: the target date already has rows in the rates table, and the upsert flag is off. With the flag off the loader does plain inserts. Any row whose key already exists hits the unique constraint and the statement fails.

## Why

The rates table has a unique key over the rate date and the currency pair (plus the source, if the table has one in your environment). Plain insert cannot overwrite. That is the point of the constraint: two rate values for the same pair and day would make reconciliation ambiguous, since settlement amounts get converted using whatever row the join finds.

## Why it bites in practice

Typical triggers are a retry after a timeout where the first attempt actually committed, an operator rerunning a job by hand after seeing a warning, a scheduler double fire, and a backfill that overlaps days already loaded. In each case the date is already present and the rerun fails.

## What not to do

Do not delete the day's rows just to make the rerun pass. Reconciliation runs may already have used those rates, and removing them leaves ledger comparisons that cannot be reproduced. Do not drop or loosen the unique constraint either. It is the only thing stopping silent duplicates.

## What to do instead

First decide whether you want the existing rates kept or replaced.

- If the existing rows are correct, do nothing. The failed rerun changed no data for that date.
- If the provider corrected its rates and you need the new values, rerun with the upsert flag enabled so existing keys are updated in place.
- If you only want to fill gaps in a range, narrow the range to the missing dates rather than reloading everything.

## Checking what is loaded

Before a rerun, query the rates table for the date and look at the row count and the loaded-at timestamps. If the count matches what a normal day looks like, the earlier run finished and the rerun is pointless. If it is short, the earlier run was partial and you need to look at why before choosing a mode.

## Partial loads

A partial first run is the awkward case. If the loader writes in batches and one batch committed before a crash, the date has some rows but not all. A plain rerun then fails on the first row that exists. Enabling upsert fixes this, since it fills the missing rows and rewrites the present ones with the same values.

## Upsert caveats

Upsert overwrites. If the provider file for that date differs from what was loaded earlier, the old values are gone after the rerun. Check whether anything downstream has cached the old rates. Reconciliation results already produced are not recomputed automatically, so mismatches flagged under old rates may stay flagged or stay cleared until a review pass is rerun.

## Alerting and logs

The duplicate key error shows up in the loader's logs and in the Postgres server log. If an alert fires on loader failures, this is one of the benign causes; check the date and the flag before paging anyone. Make sure the alert text includes the date so that check is quick.

## Open questions

Whether the loader should fail early with a clearer message when the date is already loaded and upsert is off. Right now the raw database error is all you get, and it does not say which date or pair collided unless you read the detail line. Also unclear whether the scheduler should pass the upsert flag by default for the daily job, which would hide double fires but also hide provider corrections.

## Quick reference

Error string to search for: `duplicate key value violates unique constraint`. Cause: date already loaded, upsert off. Fix: confirm what is loaded, then rerun with upsert only if replacement is wanted.
