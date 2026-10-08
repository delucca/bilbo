---
id: 01KGHPAKF1C2YXWZE6B8K27BBV
created: 2026-02-03T09:06-03:00
---

# Backfill plan for latency_rollup_mv

This is the plan for backfilling latency_rollup_mv so the rollup has history before the regression summaries rely on it. A materialized view only transforms rows inserted after it exists, so everything older than its creation is missing from the rollup target. The fix is to backfill the last 30 days with an INSERT INTO tq.latency_1m SELECT statement, run in daily chunks. This note covers the order of work, how to chunk, how to check the result, and what to do when it goes wrong.

## Goal

After the backfill, the per-minute latency rollup behind latency_rollup_mv covers the last 30 days with no holes and no double counting. SREs read this rollup in Grafana and in the post-deploy regression summary. The summary compares latency after a deploy against a baseline from before it. A baseline with gaps makes every deploy look like a regression, or hides real ones. That is the reason to do this before anyone trusts the summaries for older deploys.

## Why a plain backfill is needed

The view only sees new blocks of inserts into the source table. Rows already stored there never pass through it. Changing the view's query later has the same effect: new data uses the new logic, old data keeps whatever was there, or nothing. So the rollup target has to be filled by hand for the historical window, using the same aggregation the view uses. If the two queries differ, the seam between backfilled and live data shows up as a step in the Grafana panels.

## Scope of the backfill

Window: the last 30 days, counted back from the moment the live view started writing. The end of the window must be the point where live writes already cover, not "now". Picking "now" would count the overlap twice. Picking an earlier cut leaves a gap. Settle the boundary first, write it down in the ticket, and use it as the upper limit of the last chunk.

Out of scope: anything older than 30 days, changes to the rollup schema, and changes to how the OpenTelemetry collectors export spans. If someone asks for a longer history, that is a separate run of the same procedure with a longer window and a fresh look at storage.

## Approach

Use an INSERT INTO tq.latency_1m SELECT statement that reads the raw span data for one day and aggregates it exactly as the view does, writing into tq.latency_1m. Run it once per day of the window, oldest or newest first, it does not matter as long as each day is done once. Daily chunks keep each insert bounded in memory and time, make a failure cheap to redo, and give a natural unit for checking and for resuming.

```sql
-- one chunk: a single day of the 30 days window
INSERT INTO tq.latency_1m SELECT
    -- same grouping and aggregates as latency_rollup_mv
    ...
FROM -- raw span table
WHERE -- timestamp >= chunk start AND timestamp < chunk end
GROUP BY -- same keys as the view
```

Copy the select part from the view definition instead of retyping it. Read it from ClickHouse with a SHOW CREATE on the view, then add only the time filter. Any hand edit of the aggregation is a risk of drift.

## Chunking rules

Each chunk covers a half-open range: start inclusive, end exclusive, aligned to a day boundary in the same timezone the rollup buckets use. Chunks must not overlap and must not leave space between them. Keep a simple list of chunk ranges with a status next to each one, so a restart picks up at the first chunk not marked done. Do not run chunks in parallel at first. Run them in sequence, watch the load, and only then consider running a few together.

## Order of operations

- Confirm latency_rollup_mv is live and receiving new rows, and note the moment it started.
- Take the end boundary of the window from that moment.
- Count rows in the target for the window as it stands now, so there is a before figure.
- Run the chunk insert for the first day, check it, then continue through the window.
- After the last chunk, run the verification below.
- Tell the SREs the history is complete, so summaries for older deploys can be trusted.

## Checks per chunk

After each chunk, compare the number of minute buckets and the total request count in the target against the same figures computed straight from the raw spans for that day. Both should match. Also look at a few services by eye in Grafana: the curve should be continuous across the chunk edges. A mismatch at the edges usually means a boundary was off, or the timezone differed from the bucket alignment.

## Verification at the end

Once all chunks are in, check three things. First, no day in the window is empty unless the source was also empty that day. Second, the seam where backfilled data meets live data has no jump and no doubled counts. Third, percentile columns in the rollup agree with a direct computation on a sample of raw data for a sample of services. Percentile states merge fine, but a wrong merge function in the backfill would show here and nowhere else.

## Load and impact on ClickHouse

The backfill reads a lot of raw data. Run it outside peak hours, and keep an eye on query latency for the dashboards and on merge backlog in the cluster. If dashboards slow down, pause between chunks instead of cancelling a running insert. Settings that cap memory and threads for the session are worth setting for this job, since it is a background task and should lose against interactive queries. Do not change server-wide settings for it.

## Running it from Kubernetes

If the chunks are driven by a script, run that script as a Kubernetes Job, not from a laptop, so a closed terminal does not stop it halfway. The Job should take the chunk range as arguments and exit non-zero on the first failed chunk. Give it its own ClickHouse user with insert rights on the target and read rights on the source, nothing more. Keep the Job's logs until the verification is done.

## Failure and rerun

An insert that fails midway may leave part of a chunk in the target. Do not just rerun it, because that would double count the part that landed. Before rerunning a chunk, remove its range from tq.latency_1m, or check whether the target deduplicates for this table, and then run the chunk again. Treat the range delete as part of the redo, not as an optional step. If the table engine collapses duplicates only during background merges, the counts can look wrong for a while after a rerun, so verify after merges settle or use a query that forces the final view of the data.

## Risks

- Double counting at the live boundary if the end of the window is set too late.
- Gaps if the end of the window is set too early or a chunk is skipped.
- Drift between the backfill query and the view query if the aggregation is retyped.
- Timezone mismatch between chunk edges and bucket alignment.
- Load on the cluster slowing dashboards during the run.
- Late-arriving spans: data from the most recent days may still change, so rechecking the newest chunks a bit later is cheap insurance.

## Rollback

The backfill only inserts into the rollup target for a known time range. If something is badly wrong, delete that range from the target and the live data stays intact. Nothing in the raw span table changes. The view itself is not touched by this plan, so there is nothing to undo there.

## Open questions

- Whether the target table deduplicates on its own, which decides how the redo of a failed chunk works.
- Whether to run chunks newest first, so that the most useful recent history is ready early.
- Who owns the Grafana panels that should be rechecked once the history is in.
- Whether a longer window is wanted later, and what it would cost in storage.

## After it is done

Record the actual window, the boundary used, and the before and after counts in the ticket. Delete the chunk status list or attach it to the ticket. Update this note with anything that went differently from the plan, mainly the dedup behavior and the load seen, since the next backfill of a view like latency_rollup_mv will start from here.
