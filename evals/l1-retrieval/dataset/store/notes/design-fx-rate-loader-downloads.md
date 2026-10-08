---
id: 01M2Y9WMDMCCFZR2TNM5WAKZAD
created: 2026-09-19T23:21-03:00
sources:
  - "code: cmd/fx-rate-loader/main.go"
---

# fx-rate-loader design

fx-rate-loader downloads the daily reference rates from the ECB feed and upserts them into the PostgreSQL table `fx_rates`. That is the whole job. It is a small Go service in Ledgerlark, and it exists so that the reconciliation path never has to call out to a rate provider while it is matching a settlement line to a ledger entry. Everything that needs a conversion reads from `fx_rates` and nothing else. This note records how fx-rate-loader is shaped, why it is shaped that way, and what to check when it misbehaves.

Short version for someone in a hurry: one fetch per day from the ECB feed, parse, validate, upsert into `fx_rates`, log what changed, exit or sleep until the next run. The upsert is idempotent, so running it twice on the same day is harmless. If the feed is late or broken, the table keeps the last good rows and the downstream matcher falls back on those rows with a visible age.

## Purpose and place in the system

Ledgerlark reconciles card-processor settlement files against internal ledger entries and flags mismatches for review. Finance operations teams at online marketplaces use it. Marketplaces take payments in many currencies, and the processors settle in a currency that is often different from the currency the ledger entry was booked in. To compare the two amounts, the matcher has to convert one side into the other, and it has to do that the same way every time. A mismatch that appears only because two services used different rates is a false positive, and false positives are the thing finance teams complain about most.

So the design rule is: one source of rates, one table, one loader. fx-rate-loader is the only writer to `fx_rates`. Every reader treats the table as read-only. If someone needs a different rate source for a special case, that is a new design discussion, not a quiet second writer.

The ECB feed was chosen because it is a published reference rate, it is free, it is stable in format, and finance people recognize it as a legitimate source when they audit a conversion. It is a reference rate, not a tradable rate. That matters. A reconciliation tolerance has to be wide enough to absorb the difference between a reference rate and the rate the processor actually applied. The loader does not try to model that; the tolerance logic lives in the matcher, not here.

Things fx-rate-loader does not do:

- It does not compute cross rates on the fly for the matcher. If a cross rate is needed, the reader derives it from rows in `fx_rates`, or a later change adds derived rows with a clear marker. Today the loader stores what the feed publishes.
- It does not fetch intraday rates.
- It does not back-fill history from any source other than the ECB feed.
- It does not publish to Kafka as part of its main job. See the section on downstream consumers for the open question about that.
- It does not expose a gRPC API. Readers go to the database directly or through the service that owns the read path.

## How the load works

The loader runs as a scheduled job. The schedule is set in the Terraform that defines the deployment, not in the Go code, so changing the cadence is an infrastructure change and goes through the usual review. The Go code assumes it will be invoked about once a day and does not rely on being invoked at an exact moment.

Each run goes through the same steps in order.

First, fetch. The loader makes an HTTP request to the ECB feed with a bounded timeout and a small number of retries with backoff. It reads the whole body into memory; the feed is small. It does not write anything to the database until the whole body has been fetched and parsed. A half-downloaded file never touches `fx_rates`.

Second, parse. The feed is XML. The loader extracts the publication date and the list of currency and rate pairs. The base currency of the feed is the euro, and every rate is expressed as units of the quoted currency per one euro. The loader keeps that convention in storage and does not flip it. Anyone reading `fx_rates` has to know that direction, and the column names and comments in the migration say so. If you ever see a rate that looks inverted, check the reader before you check the loader.

Third, validate. This is where most of the care goes, because bad rates silently corrupt reconciliation results for days before anyone notices. The loader rejects the whole batch, not individual rows, when any of these hold: the publication date is missing or unparseable; the publication date is older than the newest date already stored (a stale or replayed file); the batch has no currency rows at all; a rate is zero, negative, or not a number; or the set of currencies shrinks dramatically against the previous day. The last check is a sanity guard against a truncated file that still parses. The threshold for "dramatically" is a config value and not a magic constant in the code. Rejecting the whole batch is deliberate. A partial day is worse than a missing day, because a partial day looks complete.

There is one softer check: a day-over-day move in a single currency that is larger than a configured band produces a warning in the logs and a metric, but the row is still written. The ECB feed does move sharply on real events, and refusing to store a true rate would be its own bug. The warning exists so a human can look.

Fourth, upsert. The loader writes the validated rows into `fx_rates` inside a single transaction. The key is the combination of the rate date and the currency, so a second run for the same date updates rows in place instead of adding duplicates. The upsert only changes a row when the rate differs from what is stored, and it records an updated timestamp only in that case. That way a repeated run produces no churn and the updated timestamp stays meaningful. If the transaction fails for any reason, nothing is written.

Fifth, report. The loader logs one structured summary line: the publication date, how many rows were inserted, how many were changed, how many were unchanged, and how long the run took. It also sets a gauge for the newest rate date in the table. That gauge is what alerting watches.

## Storage shape and invariants

The table `fx_rates` holds one row per rate date per quoted currency. Besides the date, the currency code and the rate itself, rows carry bookkeeping columns: when the row was first inserted, when it was last changed, and a short string saying which source it came from. The source column is there so that, if a second source is ever added, rows can be told apart without a schema change. For now every row says it came from the ECB feed.

The rate is stored as an exact decimal type, not a float. Floating point rates produce rounding differences that show up as one-unit mismatches in the smallest currency unit, and those are exactly the mismatches the product is meant to flag for real reasons. We do not want to manufacture them. The precision of the column is wider than what the feed publishes, so readers can multiply and divide without losing digits before the final rounding step, which belongs to the reader.

Invariants worth keeping in mind:

- For a given date and currency there is exactly one row. The primary key enforces it.
- Rates are strictly positive. A check constraint enforces it, as a second line of defence behind the loader's validation.
- Rows are never deleted by the loader. If a day turns out to be wrong, it is corrected by a later upsert or by a deliberate manual fix that is written down, not by removing the row.
- The newest date in the table moves forward only. The loader refuses to write a batch whose date is older than the newest stored date. Backfills of older dates are a separate, explicit operation and not part of the daily run.
- Weekends and bank holidays have no published rate. The table simply has no row for those dates. Readers must look up the most recent rate on or before the transaction date and must not assume a row exists for every calendar day. This is the single most common reader bug, and it shows up as a missing-rate error on the Monday after a long weekend.

## Failure handling and operations

The loader fails closed. If anything goes wrong it leaves `fx_rates` as it was and exits with a non-zero status so the scheduler records a failure. It does not try to be clever about guessing a rate.

The common failure modes, and what each one looks like:

- The feed is unreachable or times out. The loader retries a few times, then fails the run. The table keeps its last good rows. The next scheduled run tries again. Nothing else is needed unless the outage lasts for several days.
- The feed is reachable but publishes late. The ECB publishes once a day, at a time that can drift. If the job runs before publication, it sees yesterday's date again. That is not an error: the date check treats a same-date replay as a no-op and the run ends cleanly with all rows unchanged. If this becomes a pattern, move the schedule later in Terraform; do not add polling to the Go code without discussing it.
- The feed format changes. The parser fails loudly on unexpected structure. It is better to page someone than to guess. The fix is a code change plus a test fixture taken from the new format.
- The database is unavailable. The transaction fails, the run fails, and nothing is written. Same recovery as the feed being unreachable.
- A single bad rate in an otherwise good file. The whole batch is rejected, the log line names the offending currency, and a human decides whether to wait for the next file or to load a corrected one by hand.

Alerting. Two alerts matter. One fires when the newest rate date in `fx_rates` is older than the expected publication cadence plus a grace allowance that covers weekends. The other fires when the loader job fails repeatedly. The first is the more important of the two, because it also catches the case where the job is not running at all, which a failure alert cannot see.

What a reader sees when rates are stale. The matcher is expected to use the latest rate on or before the transaction date, and to attach the age of that rate to the mismatch it raises. A reviewer looking at a flagged item can then see that the rate used was several days old and discount the flag accordingly. A stale rate should degrade confidence, not stop reconciliation. If rates are missing for a currency altogether, the matcher raises a distinct kind of flag, not a numeric mismatch, so that it is not confused with a real amount difference.

Running it by hand. For a manual run, point the loader at the target database with the same configuration the scheduled job uses and run it once. Because the upsert is idempotent, a manual run on top of a scheduled run is safe. Check the summary line afterwards: a manual run on an already loaded day should report every row unchanged. If it reports changes on a day you thought was settled, stop and find out why the feed changed its mind before you do anything else.

Configuration. Feed location, timeouts, retry counts, the currency-shrink guard and the daily-move warning band are all configuration. Database credentials come from the deployment environment and are provisioned through Terraform. Nothing sensitive is baked into the image or the repo.

Testing. The parser is tested against saved sample files, including a normal day, a day with a missing currency, a truncated file, and a file with a nonsense rate. The upsert is tested against a real PostgreSQL instance rather than a mock, since the idempotency and the unchanged-row behaviour depend on actual conflict handling in the database. Running the same fixture twice and asserting no changes the second time is the core regression test.

## Downstream consumers and open questions

Today the consumers are the matching code paths that need to convert amounts, and they read `fx_rates` directly. There is no event when new rates land. This is simple and it has worked, but it has two costs. Readers that cache rates in memory have no signal to refresh, so they either re-read on a timer or on every use. And there is no easy way for other parts of the system, such as dashboards or the review UI, to react to a new day of rates.

Open question: should fx-rate-loader publish a small event to Kafka after a successful commit, saying that a new rate date is available? The argument for is that caching readers could invalidate precisely and that the review tooling could show when rates last moved. The argument against is that it adds a second side effect to a job that is deliberately a single transaction, and it raises the usual problem of a commit that succeeds with a publish that does not. If we do it, the event should be published after the commit, carry only the date and not the rates, and be safe to lose: readers must still work correctly without it by reading the table. That keeps the table as the source of truth and the event as a hint. No decision has been made.

Open question: second source. If a marketplace needs a currency the ECB feed does not cover, the loader as designed cannot help. Adding a second source means deciding precedence when both publish the same currency, and the source column in `fx_rates` was left in place for that. The key would then probably need to include the source, which is a migration and a reader change. Defer until a real customer needs it.

Open question: rate at the processor's value date versus the transaction date. Settlement files carry their own dates, and the right date for conversion is a product decision made in the matcher. The loader's only obligation is to make sure the rows for the relevant dates exist and are correct. If the matcher ever needs rates for dates older than what the table holds, that is a backfill job, run deliberately, using the same validation and the same upsert path so there is only one way rates get into the table.

Things that would be easy to get wrong when changing this component:

- Writing rows one at a time outside a transaction. A crash in the middle then leaves a partial day, which is the failure the design is built to avoid.
- Loosening the whole-batch rejection into per-row skipping to make a noisy day pass.
- Turning the rate direction around for convenience in one reader. Keep the stored direction as published and convert in the reader with a clear name.
- Treating a missing weekend row as an error in the loader. It is expected.
- Adding a second writer to `fx_rates` for a one-off need. Use the loader's backfill path or write the fix down as a manual operation.
- Using floats anywhere between the parse step and the database write.

If you change any of the above, update this note in place rather than adding a new one, and say what changed and why. The next person will have only this note and the code.
