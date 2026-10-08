---
id: 01JR931HJEHSZKKPQXG8CPCQ0C
created: 2025-04-07T18:38-03:00
sources:
  - "code: schema/clickhouse/002_latency_rollup_mv.sql"
---

# latency_rollup_mv design

latency_rollup_mv is a materialized view that writes into the AggregatingMergeTree table tq.latency_1m using quantilesTDigestState(0.5, 0.95, 0.99). That is the whole mechanism. Spans arrive in ClickHouse, the view fires on each insert block, computes partial quantile states per group, and stores those states in tq.latency_1m. Nothing else in TraceQuill computes the per-minute latency numbers that the post-deploy regression summaries and the Grafana panels read. This note records what the view is for, why it is shaped this way, and what to watch when changing it. I wrote it quickly, so it is more a working record than a polished spec.

## Purpose and who depends on it

TraceQuill collects distributed traces through OpenTelemetry and tells site reliability engineers whether a deploy made things slower. The question they ask is nearly always the same: for this service and operation, did the median, the tail, and the far tail move after the rollout compared with before it? Answering that from raw spans means scanning a very large table every time someone opens a dashboard or the regression summarizer runs. That is slow and it competes with ingestion for disk and CPU.

latency_rollup_mv exists to avoid that scan. It keeps a compact, mergeable summary of latency at one-minute granularity. The table name, tq.latency_1m, says the bucket size. Readers never touch raw spans for the headline numbers. They read the rollup, merge the partial states over whatever time range they care about, and get quantiles for that range.

The consumers are:

- The regression summarizer, a Java service that runs after each deploy event. It compares a window before the deploy marker with a window after it and writes a short verdict per service and operation.
- Grafana dashboards that SREs use during and after a rollout. Latency panels read the rollup, not the span table.
- Ad hoc queries by people investigating an incident. They are told to start from the rollup and only drop to raw spans when they need a specific trace.

Because the summarizer and the dashboards both rely on the same table, a change in how the view computes its states changes numbers everywhere at once. That is the main reason this note exists: the view looks trivial, but it is a contract.

## The view and its target table

The target is an AggregatingMergeTree table, tq.latency_1m. This engine is the right fit because rows are not final values. Each row holds aggregate function states, and when ClickHouse merges parts in the background it combines states that share the same sorting key into one. Quantile states from t-digest are mergeable, so partial results from different insert blocks, different ingest nodes, and different parts collapse correctly into one state per key per minute.

The state function used is quantilesTDigestState(0.5, 0.95, 0.99). It computes three quantiles at once, the median, the ninety-fifth percentile, and the ninety-ninth percentile, and stores them as a single t-digest based state per row. Choosing one multi-quantile state rather than three separate columns keeps the storage smaller and means the three numbers come from the same sketch, so they are consistent with each other. The consequence is that the set of quantiles is baked into the state shape. A reader has to use the matching merge combinator with the same list of levels. If someone wants a different percentile later, they cannot derive it from the stored states in the general case; they need either a new column or a new table, and a backfill.

The view itself selects from the span ingestion table, groups by the minute bucket plus the dimensions we keep, and applies the state function to the span duration. Dimensions are deliberately few. They identify the service, the operation, and a small number of deployment related attributes so the summarizer can split before and after a rollout. I am not listing the exact columns here because they have changed before and the schema file is the authority. If you need the current list, read the DDL, not this note.

One property worth remembering: a materialized view in ClickHouse is an insert trigger. It sees only the block being inserted, not the whole table. That is why the target must be an aggregating engine and why the view stores states instead of final numbers. Each block produces partial states, and the table merges them over time. Reading before background merges finish still works as long as the query merges states itself, which it must always do.

## Why t-digest and these three quantiles

The decision to use t-digest was about tail accuracy at bounded size. Latency distributions in our traces are heavy tailed and often multimodal, for example a cache hit mode and a cache miss mode. Exact quantiles would need all values, which defeats the purpose of a rollup. Simple averages hide exactly the regressions the SREs care about. A t-digest concentrates its resolution near the extremes, so the ninety-fifth and ninety-ninth percentiles stay reasonably accurate while the state stays small.

The tradeoff is that t-digest is approximate and its error is not uniform. Near the median the relative error is small enough that nobody has complained. Near the far tail with few samples in a bucket, the estimate can jump around. For a one-minute bucket on a quiet operation, there may be only a handful of spans, and the ninety-ninth percentile of a handful of spans is mostly noise. That is a property of the data and not a bug in the view, but it does show up as false alarms if the summarizer reads single buckets. The summarizer therefore merges states across a window before reading quantiles, which both smooths the noise and gives the sketch more data to work with.

We considered other state functions. The exact variants were rejected because of memory use and state size. A fixed-bucket histogram was considered because it would allow any percentile later and is trivially mergeable, but the bucket boundaries would need to be chosen up front and would be wrong for some services, whose latencies differ by orders of magnitude. A sampling based quantile state was rejected because results varied between runs on the same data, which made regression verdicts hard to explain. T-digest was the middle path: bounded size, mergeable, and good where we look.

The choice of the three levels came from how people read the output. The median says what a typical request feels like. The ninety-fifth percentile says what a noticeable minority sees. The ninety-ninth percentile says what the worst regular experience is. Every extra level is something that dashboards and the summarizer then have to decide whether to use. Three was enough for every question asked so far.

## How readers must query it

The single most common mistake is reading tq.latency_1m as if it held numbers. It holds states. A plain select of the quantile column returns binary-looking data. Readers must apply the merge combinator that corresponds to the state function, with the same levels, and group by the dimensions and by whatever time bucket they want. Merging over a range of minutes is the intended use: merge the states for the window, then read the three quantiles out of the merged result.

Some practical rules I have settled on:

- Always merge states over the full window and then extract quantiles. Never average the per-minute quantiles. The average of percentiles is not the percentile of the union, and it hides exactly the tail movement we want to see.
- Keep the levels in the read query identical to the ones in the state. A mismatch either errors or silently returns the wrong level, depending on how the query is written.
- When comparing before and after a deploy, use windows of equal length and similar traffic mix, otherwise differences reflect load and not the code change.
- Do not trust a verdict computed from a window with very few spans. The summarizer should say there was not enough data, not give a number.
- Expect small differences between a rollup number and a number computed from raw spans for the same window. That is the approximation. Differences that are large point to a real problem, covered below.

Grafana panels use the same pattern, with the time range macro of the dashboard supplying the window. Panels that show a time series group by an interval larger than one minute so each point merges several buckets. If a panel interval is smaller than one minute it will just show the one-minute buckets, and the lines will look jagged for low traffic services. That is expected.

## Operational behavior and what to watch

The view runs on insert, so its cost is paid at ingestion. The state computation is cheap per row but not free. When ingestion volume rises sharply, for example during an incident when services emit more spans, the extra work in the view shows up as insert latency. If inserts back up, check whether the view is the cause before blaming the network or the collector.

Background merges on the aggregating table do the real compaction. When merges fall behind, the table has many small parts, queries read more data, and merging states at query time costs more. The symptoms are slower dashboards without any change in query shape. The fix is on the ClickHouse side, looking at merge throughput and part counts, and not in the view.

Late arriving spans are a known wrinkle. Spans reach ClickHouse after the collector batches them, and some arrive minutes after the work happened. Because the minute bucket is derived from the span's own timestamp and the view just adds a new partial state, late spans land in the right bucket when they arrive. The state for that minute then changes after the fact. For a deploy comparison run immediately after the rollout, the latest minutes may be incomplete. The summarizer waits a grace period before running for this reason, and anyone adding a new consumer should do the same.

Schema changes need care. Altering the view's select does not reprocess existing data. Old rows keep the old states, new rows get the new shape, and the table mixes both. If the change affects how the duration is measured or which spans are included, the numbers before and after the change are not comparable, and a regression verdict across that boundary would be misleading. For such a change the safer path is to create a new target table and a new view, backfill from raw spans for the retention period, switch the readers, and only then drop the old pair.

Retention follows the table's time to live settings, which are shorter than the raw span retention only if someone configures it that way. The point of the rollup is that it is cheap to keep for much longer than raw spans, which lets SREs compare against last month's behavior. Check the table settings before assuming how far back the data goes.

## Known problems and gotchas

A few things have bitten people or are likely to.

First, the view only sees data inserted after it was created. Creating latency_rollup_mv on a database that already holds spans leaves the history empty in the rollup. Backfilling needs an explicit insert from the span table into the target using the same state function and grouping. Do the backfill in time slices so it does not starve live ingestion, and make sure no slice overlaps the range the live view is already writing, or the same spans will be counted twice. Double counting does not change quantiles much but inflates the counts if a count is ever stored beside them.

Second, dropping or detaching the target table while the view exists makes inserts into the source fail or the view stop writing, depending on how it was defined. Always treat the view and its target as a pair when doing maintenance.

Third, duplicates from retries. If the collector retries a batch after a timeout and the first attempt actually succeeded, spans are inserted twice and both the span table and the rollup see them. The effect on quantiles is mild because the duplicated values come from the same distribution, but it can matter on low traffic operations. Deduplication, where we have it, protects the raw table through insert settings; the rollup inherits that protection only if the view is fed by the same deduplicated blocks. This is worth re-checking when the ingestion path changes.

Fourth, time zones and bucket boundaries. The minute bucket is computed from the span's start timestamp in UTC. A span that crosses a minute boundary counts in the bucket where it started. Readers who think in terms of end time will see a small shift. Not a problem for regression detection, but people notice when they compare against another tool.

Fifth, cardinality. Every extra dimension multiplies the number of rows per minute. Adding a high cardinality attribute, such as a request or user identifier, would turn the rollup into something close to the raw table and destroy the savings. Proposals to add dimensions should be reviewed for cardinality first. Pod name is borderline: it is useful during a Kubernetes rollout to see whether only new pods are slow, but it multiplies rows by the number of replicas. We keep deployment level attributes and not per pod ones for that reason, and per pod questions go to raw spans.

Sixth, empty buckets. A minute with no spans has no row. Readers that expect a continuous series need to fill gaps themselves, and the summarizer must not treat a missing minute as zero latency.

## Testing, validation, and open questions

To validate a change to latency_rollup_mv, compare the rollup against raw spans on a recent window for a few services with different traffic levels and latency profiles. Compute the three quantiles both ways. Expect agreement within the sketch's tolerance on busy operations, looser agreement on quiet ones. If the rollup is consistently higher or lower than raw by a visible margin, check the filters in the view, the duration unit, and whether the view is excluding span kinds that the raw query includes. Most past discrepancies came from a filter difference, not from the sketch.

A second check is merge consistency: merge states for two adjacent windows separately and then together, and confirm that the combined result sits between or near the separate ones in a way that makes sense. If that fails, something is wrong with how the states are being stored or combined, and the likely culprit is a mixed shape table after a schema change.

A third check is the deploy scenario itself. In a staging cluster, roll out a build with a deliberately injected delay and confirm that the summarizer flags the right service and operation using only the rollup. We should keep that as a repeatable test, since it covers the whole chain from collector to verdict.

Open questions I have not resolved:

- Whether to store a count of spans next to the state so the summarizer can say plainly when a window had too little data, without a second query. It would cost a little space and remove a class of misleading verdicts.
- Whether to add a second, coarser rollup for long range comparisons, built from this one instead of from raw spans, so month over month views stay fast. Merging states from the one-minute table into a coarser one is possible because the states are mergeable.
- Whether the far tail deserves a different structure. The ninety-ninth percentile from small buckets is noisy, and a dedicated slow request counter against a threshold per service might be a more stable signal than the quantile.
- Whether to expose per pod splits for rollout debugging through a separate short retention table rather than widening this one.

Until those are decided, treat the view as stable: do not change the state function, the list of levels, or the grouping without a plan for backfill and for the boundary in the data, and tell the people who own the summarizer and the dashboards before it ships.
