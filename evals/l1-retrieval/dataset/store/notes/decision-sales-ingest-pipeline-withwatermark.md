---
id: 01KE7VYRP48Q68Q12Z2BCYG3PX
created: 2026-01-05T17:00-03:00
---

# sales-ingest-pipeline: bound streaming state with a watermark

We decided that the sales-ingest-pipeline bounds its streaming state with a watermark on event time, set with `withWatermark("event_time", "6 hours")`. This note records why, what it costs, and what to check if someone wants to change it. It is written for whoever touches the sales-ingest-pipeline next, so they don't have to rebuild the reasoning from the Spark UI.

## Decision

The sales-ingest-pipeline is a Spark Structured Streaming job in Scala. It reads point-of-sale events from the stores and writes them to Delta Lake tables. Downstream, Airflow schedules the batch jobs that feed the stockout model and the replenishment order generation, and some aggregates are pushed on to Snowflake for the merchandising analysts.

The job deduplicates events and aggregates them per store and item over event-time windows. Both operations keep state. Without a bound, that state grows for as long as the job runs. We cap it by declaring a watermark on the `event_time` column:

```scala
val bounded = events
  .withWatermark("event_time", "6 hours")
```

The watermark has to be set before the aggregation or dedup step that uses it, on the same column that appears in the window or the dedup key. If it is set after, Spark does not use it for that state and the state keeps growing.

## Why a watermark

The first versions had no watermark. State store size grew steadily, checkpoints got slower, and the job eventually needed a restart with more executor memory. That is not acceptable for a pipeline whose output decides whether a shelf gets restocked.

Options we looked at:

- No watermark and periodic job restarts. Rejected: restarts hide the problem, and each one risks reprocessing.
- A much shorter watermark. Rejected: store systems reconnect after outages and send backlogs, and a short bound would drop too many legitimate late sales.
- A much longer watermark. Rejected: state stays large, which defeats the point, and results in append mode are held back for longer.
- The 6 hour bound. It covers the normal delay from store systems, including the typical batch uploads from smaller stores, and keeps state at a size the cluster handles without trouble.

## Consequences

Events that arrive later than the watermark allows are dropped from the streaming aggregates. For windowed aggregates in append mode, a window is only emitted once the watermark passes its end, so output arrives at least that much later than the events themselves. Analysts looking at the freshest numbers should know the latest window can be incomplete or missing.

Late data is not lost for good. The raw events land in Delta Lake before the stateful step, so the batch side can recompute any period from the raw table. Corrections for late events are handled by the Airflow backfill, not by the stream. If the stream and the batch recompute disagree for an old period, the batch result is the one to trust.

Dedup also depends on the watermark: a duplicate that arrives after the state for its key has been dropped will not be recognized. This is rare, but it is a reason not to shorten the bound casually.

## Things to check before changing it

- Look at how late the real traffic is. Measure the gap between event time and arrival time per store before picking a new value.
- Check state store size and checkpoint duration after any change. Lowering the bound should shrink them; raising it will grow them.
- Tell whoever owns the downstream Airflow DAGs and the Snowflake loads, because the point at which a window counts as final moves with the bound.
- Do not change the watermark column. `event_time` is the time the sale happened at the store, not the ingest time. Using ingest time would make the bound meaningless for late-arriving backlogs.
- A change to the watermark does not require a new checkpoint by itself, but changing the aggregation keys or windows does. Test on a copy of the checkpoint first.

## Open points

Some stores with unreliable connectivity still send data later than the bound. We accepted that loss in the stream and rely on the batch recompute. If the analysts report stockout predictions that look wrong for those stores, check first whether late events were dropped from the streaming aggregates before looking at the model.

We have not set up an alert on the count of dropped late events. It would be useful and cheap to add, and it would tell us whether the current bound still fits the traffic.
