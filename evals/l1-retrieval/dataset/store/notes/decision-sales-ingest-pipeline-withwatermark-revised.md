---
id: 01KY0T6Y7168X6KAG0X2E6RV7S
created: 2026-07-20T19:26-03:00
---

# sales-ingest-pipeline watermark decision

This note replaces the earlier note about "sales ingest pipeline withwatermark"; the value to use now is `withWatermark("event_time", "48 hours")` in the sales-ingest-pipeline, in place of the earlier watermark.

The short version: the sales-ingest-pipeline now uses `withWatermark("event_time", "48 hours")`, so late store files are no longer dropped. The earlier, tighter watermark was throwing away sales rows from stores that upload late. Those rows fed the stockout predictions, and missing them made some stores look like they had sold less than they really had. Anyone reading this note alone should take it as the current setting for the sales-ingest-pipeline, and treat any other watermark value in old notes, tickets or chat as out of date.

## Decision

The watermark on the event time column of the sales-ingest-pipeline is set with `withWatermark("event_time", "48 hours")`. The column is event_time, the time the sale happened at the store, not the time we received the file. The delay is the one in that call, and it replaces the earlier watermark entirely. There is no second watermark and no fallback path for late data. One setting, one place.

What this changes in practice:

- A store file that arrives late, but still inside the window set by `withWatermark("event_time", "48 hours")`, is now processed and counted. Before, it was silently dropped by the streaming aggregation.
- Windowed state in Spark is kept for longer, because Spark cannot finalize a window until the watermark has passed it. Output for a given window is therefore emitted later than it was.
- Downstream consumers (the feature tables that feed the stockout model and the replenishment order generator) see slightly older "final" data than before, but the data they see is more complete.

We chose completeness over freshness. Merchandising analysts use ShelfSense to decide what to order, and a wrong low sales number for a store is worse for them than a result that lands later. A replenishment order based on sales that are too low under-orders, and under-ordering is how stockouts happen. That is the exact failure the product exists to prevent.

## Why the earlier watermark was wrong

Store files do not arrive on a clean schedule. Some stores upload at close of business, some upload in batches during the night, and some have connectivity problems and send a backlog the next day or later. The old watermark assumed lateness would be small. In practice a real share of files were later than that, and Spark treats rows older than the watermark as too late and discards them from stateful operations.

The symptoms that led to this decision:

- Totals in the Delta tables did not match totals that stores or the chain's own reporting showed for the same days.
- The gaps were not random. They clustered on the stores with the least reliable uploads, which are often the smaller or more remote ones. That biased the model against exactly those stores.
- Nothing failed. There was no error, no failed job and no alert. Rows just went missing, which is why it took a while to notice.

The silent part matters most. A watermark drop is not logged as a problem by default, so a too-tight setting looks like a healthy pipeline. Anyone who touches this setting later should assume that a smaller delay will lose data without any warning.

## What we considered

A few alternatives came up and were not taken. They are listed so nobody has to work them out again.

Keep the old watermark and reprocess late files in a separate backfill job. This works, but it makes two paths that can disagree, and it needs someone to keep the backfill in step with the main logic. It also leaves the main tables wrong until the backfill runs. We did not want two sources of truth for sales.

Remove the watermark altogether. Without a watermark, Spark keeps state for every window forever, so memory and checkpoint size grow without bound. That is not safe for a pipeline that runs continuously across many stores.

Pick a much larger delay. A larger delay catches even more late files, but it holds state longer and delays results more. The value in `withWatermark("event_time", "48 hours")` was picked as a balance: it covers the late arrivals we actually see, without letting state grow too far. If real lateness turns out to be worse than this, revisit it with data, not by guessing.

Fix the stores. Asking stores or the chains' IT teams to upload on time is worth doing where we can, but we do not control it, and the pipeline has to cope with what it receives.

## Consequences and things to watch

State size. Because windows stay open longer, the state store in the streaming job is larger than before. Watch executor memory and the checkpoint location after the change. If the job starts to struggle, look at state size first before blaming anything else.

Latency of final numbers. Anything that waits for a window to close now waits longer. If an analyst asks why today's numbers are still moving, this is the reason: late files are still being folded in until the watermark passes. This is intended behaviour, not a bug. It is worth saying so in any analyst-facing text that shows recent sales.

Airflow scheduling. Downstream Airflow tasks that read the Delta output should not assume that data for a recent day is final. Where a task needs final data, it should read only windows that are older than the watermark delay. If a DAG was written on the assumption of the older, shorter delay, check it.

Snowflake. Whatever is loaded from the Delta tables into Snowflake can now see updated values for a recent day as late files land. Loads should upsert or replace by window, not append blindly, or the same sale could be counted twice. Check the load logic wherever it appends.

History. Data that was dropped under the earlier watermark is not recovered by this change. The change only stops new drops. Past gaps stay unless someone reprocesses the raw store files for the affected period. That has not been decided here, and it would be a separate decision with its own note, since it touches model training data as well.

Model effects. The stockout model was trained, at least in part, on data that had the late-file gaps. Expect small shifts in features for the affected stores once complete data flows in. A change in model quality for those stores after this change is plausible and is probably an improvement, but it should be checked, not assumed.

## How to check it is working

Nothing in the pipeline will announce that the watermark is wrong, so check from the outside.

- Compare daily totals per store in the Delta output against the raw files received, for a recent period that includes known late uploaders. They should now agree once the window has closed.
- Look at the streaming query progress in Spark. The reported watermark should trail the latest event time by the configured delay, which is the one in `withWatermark("event_time", "48 hours")`. If it trails by a different amount, the job is running old code or an old config.
- Confirm that the call in the source of the sales-ingest-pipeline matches exactly, with event_time as the column, and that there is only one watermark call on that stream. A second call further down on a derived stream can quietly override the effective delay, because Spark uses the smallest or the most recent depending on the query shape, and that is easy to miss in review.
- Watch for a rise in state store size over the first days after deploy, then for it to level off. If it keeps climbing, something else is keeping windows open.

## Open points

Whether to reprocess past data that was dropped. Needs an owner and a decision on how far back to go.

Whether to report late-arrival lag per store, so we can see which stores are slow and whether the window is still the right size. This would be cheap to add and would give us the data to revisit the setting later.

Whether analyst-facing screens should mark recent days as provisional. Probably yes, given the longer time before a window is final.

If you change the watermark again, update this note instead of adding another one. It is the single place that records the current value for the sales-ingest-pipeline.
