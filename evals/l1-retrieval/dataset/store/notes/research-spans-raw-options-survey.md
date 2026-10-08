---
id: 01JSNEDJAMZNEQS4Y0QGDTYGX9
created: 2025-04-25T00:03-03:00
---

# spans_raw: options survey

Working notes on the options looked at for `spans_raw`, the ClickHouse table that holds spans as they arrive from the OpenTelemetry collectors before anything is summarized. Nothing here is settled. It is a list of the shapes people usually pick between, what each one costs, and what to check before choosing. The latency-regression summaries after each deploy read from this table (or from things derived from it), so the read patterns matter as much as the write path.

## What spans_raw has to do

It takes a steady stream of spans from many services running on Kubernetes. Writes are append-only and bursty, and bursts tend to line up with deploys, which is also when the SREs look hardest. Reads come in two kinds. One is the summary job, which scans a time window for a set of services and compares it to an earlier window. The other is the interactive lookup: someone in Grafana pastes a trace id, or filters by service and operation and wants a few slow examples.

Those two reads pull in opposite directions on the sort key. A time-and-service ordering suits the summary job. A trace-id lookup wants the id near the front of the key or a good secondary index. Most of the options below are different ways of dealing with that tension.

## Table layout options

### Single wide table, one row per span

The simplest option. Standard span fields become columns (trace id, span id, parent, service, operation, start, duration, status), and everything else goes into attribute maps. Easy to ingest into, easy to explain. The cost is that attribute maps are slow to filter on when the key is not hot, and the table gets large quickly.

Variants worth comparing:
- Promote a short list of frequently filtered attributes to real columns, leave the rest in maps. Needs someone to own that list and a way to add columns without a painful backfill.
- Keep two maps, one for resource attributes and one for span attributes, so that per-service metadata is not repeated in the same shape everywhere. Slightly more awkward to query.
- Store maps as parallel key and value arrays. Sometimes compresses better, but queries are uglier and people will get them wrong.

### Narrow table plus side tables

Keep `spans_raw` limited to what the summary job needs, and put events, links and long attribute payloads in separate tables joined by trace and span id. Scans get cheaper and the hot data stays small. The cost is joins at read time and a more complicated ingest, with a risk of the side tables drifting out of sync with the main one.

### Per-service or per-tenant tables

Isolation is the attraction: one noisy service cannot affect another's retention or ordering. In practice this multiplies tables, migrations and Grafana datasource queries. Probably only worth it if some teams need different retention or access rules. Otherwise a service column in the key gets most of the benefit.

## Sort key and partitioning options

Sort key candidates, roughly in order of how often they come up:
- Service, then time bucket, then trace id. Good for the summary job and for per-service dashboards. Trace-id lookups need help from a skip index or a lookup table.
- Time bucket first, then service. Good for cross-service windows, worse for narrow per-service scans in a large table.
- Trace id first. Fast single-trace fetches but poor locality for time-window aggregation, and inserts scatter across parts.

Partitioning is by time in every variant considered. The open question is the granularity. Coarse partitions mean fewer parts and simpler operations but make dropping old data lumpy. Fine partitions make retention precise but raise part counts and can hurt merges. Worth testing against real ingest volume before committing, not guessing.

A trace-id lookup helper is the usual answer to the sort key tension. Options: a bloom-filter style skip index on the trace id column, or a small separate table that maps trace id to a time range, so the lookup can narrow the scan before touching `spans_raw`. The separate table is more reliable under heavy volume but adds a materialized view to maintain.

## Ingest path options

### Collector exporter writing directly

The OpenTelemetry collector's ClickHouse exporter writes straight into the table. Fewest moving parts. Downsides: batching and backpressure behavior is whatever the exporter does, schema changes are tied to exporter expectations, and a ClickHouse hiccup pushes pressure back to the collectors.

### Queue in front

Collectors publish to a durable queue, and a separate consumer batches into ClickHouse. This absorbs deploy-time bursts and lets ClickHouse be restarted without dropping spans. It costs another system to run and another place for lag to hide. The consumer would probably be written in Java to match the rest of the stack.

### ClickHouse async inserts

Lets many small writers send inserts and have the server batch them. Reduces the need for client-side batching, but trades away some insert acknowledgement guarantees depending on settings. Needs a careful read of what the settings actually promise before anyone relies on it for completeness.

### Custom schema versus exporter default schema

Using the exporter's default table shape saves time and tracks upstream changes. Using our own shape lets us tune for the summary job but means maintaining the mapping ourselves. A middle path: let the exporter write its default table and populate a tuned `spans_raw` through a materialized view. That doubles some storage briefly, so check whether the default table can have a very short retention.

## Compression, retention and tiering

Column codecs matter a lot for span data. Durations and timestamps respond well to delta-style codecs, ids do not compress much, and low-cardinality strings such as service and status should use the dictionary-style type. Attribute maps are the big unknown; their size depends on how chatty the instrumentation is.

Retention options:
- A plain TTL that deletes rows after a fixed period. Simple. Deletion happens during merges, so space comes back late.
- TTL that moves old parts to cheaper storage first, then deletes. Needs a storage policy with a second volume and someone watching it.
- Keep `spans_raw` short-lived and rely on pre-aggregated summary tables for long-term regression history. Cheapest, but old traces cannot be reopened, which SREs may not like during a postmortem on a slow burn.
- Sampling before storage, so `spans_raw` holds a representative subset. Cuts volume a lot but can hide rare slow paths, which is exactly what a regression summary may need to find. Tail-based sampling that keeps slow and errored traces is the less risky form, though it needs a stateful collector tier.

## Derived tables and summaries

The regression summary should probably not scan `spans_raw` directly for every run. Candidates:
- Materialized views that roll spans into per-service, per-operation latency sketches by time bucket. Cheap to query, and quantiles can be merged across buckets if the right aggregate state is stored.
- Scheduled summary jobs that read `spans_raw` after each deploy and write results to a separate table. More flexible, since the comparison logic lives in Java rather than SQL, but slower and easy to run twice by accident.
- A mix: rollups for the always-on dashboards, on-demand scans of `spans_raw` for deep dives into one deploy.

A thing to settle early is how deploy markers join in. Options are a deploy events table keyed by service and time, or a deploy attribute carried on the spans themselves. The second makes before and after comparison trivial but depends on every service setting the attribute correctly. The first is more robust and costs a join.

## Operations and risk notes

- Schema changes on a table this size are slow if they rewrite data. Adding columns is cheap; changing the sort key means a new table and a backfill. This argues for getting the key reasonably right the first time and for keeping a documented way to build a replacement table beside the old one.
- Replication and sharding: a single replicated set may be enough early on, but the sharding key interacts with the trace-id lookup. If shards are split by trace id, lookups hit one shard; if split by service, the summary job is local but lookups fan out.
- Cardinality of attribute keys can blow up if instrumentation puts ids or urls into keys. Worth a guardrail at the collector.
- Grafana queries against `spans_raw` should go through a restricted user with query limits, since an unbounded scan from a dashboard can starve ingest merges.
- Kubernetes side: ClickHouse needs stable storage and careful resource limits. Merge and insert load spike at the same time during deploy bursts, so test under that pattern.

## Things still to check

- Real attribute usage: which keys do SREs actually filter on, so the promoted column list is based on use and not on taste.
- How well the skip index approach performs for trace-id lookups against a realistic table, compared to the helper table.
- Whether the exporter's batching is enough or the queue is justified, by looking at how it behaves through a deploy burst.
- What the postmortem workflow needs in terms of how far back raw traces must stay available.
- Whether sampling is acceptable to the teams who read the regression summaries.
