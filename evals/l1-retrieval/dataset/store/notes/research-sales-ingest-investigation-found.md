---
id: 01M0657Z08MEX262TXAJKDF332
created: 2026-08-16T17:47-03:00
sources:
  - "doc: Late POS Arrival Study"
---

# Late POS data in sales-ingest-pipeline

The investigation into late point-of-sale files found that 3% of stores deliver POS files up to 36 hours after close. The sales-ingest-pipeline therefore sees late data as a normal condition, not as a rare fault. This note records what that means for the pipeline, what we think follows from it, and what is still open. Most of what is below is reasoning from that one finding, not measurement, and I mark the places where that is the case.

The short version: a small share of stores is late by a long time. The share is small, so averages look fine. The delay is long, so any logic that assumes "yesterday's sales are complete by the morning run" is wrong for those stores. Because ShelfSense predicts store-level stockouts and generates replenishment orders, a late store is exactly the store where a wrong number does damage: the model thinks the shelf still holds stock that was already sold, or thinks nothing sold when plenty did.

## The finding

3% of stores deliver POS files up to 36 hours after close. Two things are fixed by that sentence and two are not.

Fixed:
- The affected share of stores is 3%.
- The worst observed lateness is up to 36 hours after store close.

Not fixed by the finding:
- Whether the same stores are late every day, or whether the late set rotates. This matters a lot. A stable set can be handled with per-store configuration. A rotating set needs generic handling.
- How the lateness is distributed below the maximum. "Up to 36 hours" says nothing about the typical late store. Some may be a few hours behind and a few may be near the limit.
- Whether lateness is caused by the store, the POS vendor, the network link, or our own collection step.

I treat the 36 hours figure as a ceiling that the design has to survive, not as an average. Any watermark, grace window or reprocessing horizon in the sales-ingest-pipeline should be judged against it.

## Why this matters for ShelfSense

Merchandising analysts use ShelfSense to see which stores are about to run out of which products and to review the replenishment orders it proposes. They read the output in the morning and act on it. If a store's sales for the previous day are missing at that time, the stock position that feeds the prediction is stale.

There are two failure shapes:

1. Missing sales look like zero sales. The model sees a quiet day, estimates on-hand stock too high, and does not flag a stockout that is in fact coming. The order is too small or not generated.
2. Late sales arrive after an order was already generated. The order was based on a wrong position, and nothing corrects it unless the pipeline reprocesses and the downstream step re-evaluates.

The first is silent and the second is visible only if someone compares. Neither shows up as a pipeline failure. All jobs succeed. That is the main reason this is worth writing down.

## How late data reaches the pipeline

The sales-ingest-pipeline is built on Apache Spark with Delta Lake storage, written in Scala, scheduled by Airflow, with Snowflake downstream for serving. I did not re-read the code for this note, so the description below is the general shape, not a line-by-line account.

The flow is: POS files land per store, a Spark job reads and normalizes them, the result is written into Delta tables, and a later step publishes to Snowflake for analysts and for the forecasting side. Airflow decides when each step runs.

Where lateness bites in that flow:

- Airflow schedule. If the ingest run is triggered by a clock, files that arrive after the trigger are not in that run. If it is triggered by file arrival, one late store does not block the others, but the "done" signal for the day becomes ambiguous.
- File discovery in Spark. A job that lists the landing area for a given business date will miss files that show up later unless a later run lists again.
- Delta table writes. Appending late rows is easy. Making sure they land in the right partition, and that nobody reads a half-updated day, takes care.
- Snowflake publish. If the publish step copies a daily snapshot and is not rerun after late data, Snowflake keeps the old picture.

## Event time versus arrival time

The core design question is which clock the pipeline partitions and aggregates on. A sale has an event time (when it happened at the till) and an arrival time (when the file reached us). With 36 hours of lateness possible, those two can be a day and a half apart.

If tables are partitioned by arrival date, late files simply land in a later partition, and a query for "sales on business day X" has to look at more than one partition. Nothing is lost, but daily totals computed from one partition are wrong.

If tables are partitioned by event date, late rows go back into an older partition. Totals for that day change after the fact. That is the correct behavior, but it means older partitions are not immutable, and any consumer that cached them is out of date.

My recommendation is to keep event time as the business key and store arrival time as a separate column on every row. Then both questions can be answered: what happened on day X, and what did we know at the time we ran the model. The second question is what lets us explain a bad order afterwards.

## Watermarks and grace windows

If any part of the sales-ingest-pipeline uses Spark Structured Streaming with a watermark, the watermark delay is a hard cut. Rows older than the watermark are dropped from stateful aggregations. A watermark shorter than 36 hours would drop data from the late stores outright. This is the sharpest risk, because dropping is silent.

If the pipeline is batch only, the equivalent is the lookback on each run: how many past business days does a run re-read? A run that only reads the current day misses everything late. A run that re-reads a window covering the worst case catches it.

The reasoning is simple. The window has to be at least as long as the worst delay we accept, which from this finding is 36 hours, plus some margin for the file to be picked up and processed. I have not chosen the margin. It should be set after we know how the late files are distributed.

There is a cost tradeoff. A longer lookback means more data re-read on every run. With Delta Lake this can be limited by reading only the affected partitions and by merging instead of rewriting. It is not free, but it is bounded, and the 3% share suggests the extra volume of actual late rows is small even if the re-read window is wide.

## Idempotent reprocessing with Delta Lake

Re-reading a window only works if rerunning is safe. Duplicates are the obvious risk: a file processed once on time and again on a later run must not double the sales.

The approach I would use, and which fits Delta Lake well:

- Give every POS record or file a stable identity, from store, source file and record position, or from a natural transaction key if the POS provides one.
- Write with a merge keyed on that identity, so a second pass updates or ignores rather than appends.
- Keep the merge condition narrow, limited to the partitions in the lookback window, so the merge does not scan the whole table.
- Record which files have been seen, so that discovery can tell new files from known ones cheaply.

Delta transactions give us atomic commits, so a reader sees either the table before the merge or after it. That removes the half-updated day problem at the table level. It does not remove it at the publish level, where Snowflake is loaded as a separate step.

I would not use delete-and-reinsert of a whole day for this. It works, but it widens the window in which a day looks empty or short, and a reader in that window sees the wrong thing.

## Detecting late stores

Handling late data is one part. Knowing which stores are late right now is the other, and it is what analysts need. A store with no data for the day could be closed, could be late, or could have had a real zero.

A small completeness check per store and business day would help:

- Expected: the store was open that day, from the store calendar.
- Received: at least one POS file with that business day arrived.
- Status: complete, pending, or overdue.

Pending is the important state. A store is pending until the maximum accepted delay has passed, which for the late group is up to 36 hours after close. After that it becomes overdue and someone should look at it. Without this state, the model cannot tell "no sales" from "no data yet".

The check itself is cheap, a set comparison between expected and received stores. It can live as an Airflow task after ingest, and its result should be written to a table, not only logged, so the forecasting side and the analysts can read it.

## What the forecasting side should do

This note is about the sales-ingest-pipeline, but the consequence reaches past it. If a store is pending, the stockout prediction for it should be marked lower confidence, or should be computed with an explicit estimate for the missing sales, not with zero.

Options, from simplest to most work:

1. Flag the store's output as based on incomplete data and let the analyst decide. Cheap, honest, and it puts the judgment where the domain knowledge is.
2. Hold back replenishment orders for pending stores until their data is in or the delay limit passes. Safe but delays orders for the 3% that are already slowest.
3. Fill the gap with an expected-sales estimate from recent history, and mark it as estimated. Better orders, more risk of hiding a real problem.
4. Regenerate orders when late data arrives and the position changed enough to matter. Most complete, and needs a rule for what "enough" is.

My recommendation is option 1 now and option 4 later. Option 1 needs only the completeness table. Option 4 needs a decision from the merchandising side about how often orders may change after being issued, which is not a technical call.

## Open questions

- Is the late set of stores stable or rotating? Look at several weeks of arrival times per store.
- What is the distribution of lateness for the late stores, not only the maximum? A histogram would settle the margin on the lookback window.
- Is the cause on the store side, the vendor side or ours? If the cause is a nightly upload job at the store that runs late, a conversation with the chain could remove the problem at the source.
- Does anything in the current sales-ingest-pipeline already use a watermark or a fixed lookback, and what are the values? I did not confirm this when writing the note.
- How does the Snowflake publish react to a changed past day? If it only appends, corrected days never reach analysts.
- What do analysts currently do when a store looks wrong in the morning? Their workaround may show how much this already costs.
- Is 36 hours really the ceiling, or just the largest delay seen in the investigation window? A longer tail may exist.

## A compact statement of the finding

For quick reference, the facts to carry forward:

```
late_share_of_stores: 3%
max_delivery_delay: 36 hours after close
```

Everything else in this note is inference or recommendation built on those two values.

## Next steps

In rough order of value for effort:

1. Pull per-store arrival times for a recent period and answer the stable-versus-rotating question and the distribution question. This costs little and decides most of the later choices.
2. Read the current job code and the Airflow definitions for the sales-ingest-pipeline and write down the actual lookback, watermark and trigger logic. Compare each against the 36 hours ceiling.
3. Add arrival time as a column if it is missing, and make the write path an idempotent merge on a stable record identity.
4. Add the per-store completeness table with pending and overdue states, produced by an Airflow task after ingest.
5. Make the Snowflake publish rerunnable for past days, and check that corrected days propagate.
6. Talk to the merchandising side about how late-arriving sales should change already issued orders, and what the analyst screen should show for pending stores.

Until step 2 is done, treat any claim here about what the pipeline currently does as unverified. The finding itself, 3% of stores up to 36 hours late, is what the investigation established, and the rest follows from taking it seriously.

## Risks if we do nothing

If the pipeline stays as it is and the 3% of late stores continue, the likely effects are these. Late stores get systematically worse predictions than the rest, because their position is always a day or more behind. The error is not random; it is biased toward overstating stock, since sales that already happened are missing. Analysts who see repeated misses on the same stores will stop trusting the tool for those stores, and then possibly for others.

Because the late share is small, a chain-level accuracy metric will barely move. The problem hides in the average. Any monitoring we add should therefore be sliced by store and by data completeness, not only reported as a single number. If we only track overall accuracy, we would not see this get better or worse.

There is also a quieter cost in rework. Without a clean reprocessing path, each case of missing data turns into a manual backfill by whoever notices. A designed path that tolerates the full delay is cheaper than repeated manual fixes, even if building it takes some effort.
