---
id: 01KZ9S4YW5Y6H1BXPK4XT1GXV3
created: 2026-08-05T17:17-03:00
---

# span-ingest-writer batch size: 100000 rows per flush

This note replaces the earlier note about "span ingest writer flushes". The new value is `100000 rows` per insert batch, up from the earlier 50000 rows.

span-ingest-writer is the Java service in TraceQuill that takes spans from the OpenTelemetry pipeline and writes them into ClickHouse. In shorthand, logs and chat, it is called `chwriter`. `chwriter` is short for `span-ingest-writer`, and both names refer to the same component. If you see `chwriter` in a dashboard title, an alert or a Slack thread, it is this service.

## Decision

span-ingest-writer flushes inserts to ClickHouse in batches of `100000 rows`. The earlier setting was 50000 rows per batch. We raised it because ClickHouse merges fell behind at the smaller size.

The reasoning is simple. Each insert creates a new data part in ClickHouse. Background merges have to combine those parts. With smaller batches the writer created parts faster than the merge threads could combine them, so the number of unmerged parts kept growing. Larger batches mean fewer parts for the same volume of spans, and merges kept up.

In short, the batch is a single flush of `100000 rows`. Nothing else about the flush path was changed as part of this decision.

```
span-ingest-writer (chwriter) flush batch: 100000 rows
previous batch: 50000 rows
```

## Why the old size was a problem

The symptoms showed up in ClickHouse first, not in the writer. The writer itself looked healthy: it was accepting spans and inserts were succeeding. What degraded was the state of the tables behind it.

- Merges lagged behind inserts, so parts piled up.
- Queries that Grafana panels run against the span tables got slower as the part count grew, because ClickHouse has to read and combine more parts per query.
- The post-deploy latency regression summaries, which are the main product for site reliability engineers, depend on those queries. Slow queries made the summaries late right after a deploy, which is when people want them most.

If ClickHouse keeps accumulating parts, it eventually starts pushing back on inserts. We did not want to find out how that behaves in production, so we changed the batch size before it got there.

The cause was the rate of part creation, not total volume. Total span volume did not change when we saw the problem. That is why fixing it on the writer side, by making each insert carry more rows, was the right lever. Adding ClickHouse capacity would have treated the symptom.

## What this changes and what it does not

Changed:

- Each flush from span-ingest-writer now carries `100000 rows`.
- Fewer inserts happen for the same traffic, so fewer parts are created.

Not changed:

- The component name. It is still `span-ingest-writer`, and `chwriter` is still only a nickname.
- The ClickHouse schema and the tables spans land in.
- The way spans reach the writer from the OpenTelemetry side.
- The deployment shape on Kubernetes.

The trade-off is visible latency. A bigger batch takes longer to fill, so a span can wait in the writer's buffer longer before it becomes queryable. At normal traffic this is small. At low traffic, in a quiet environment or overnight, a batch fills slowly, and fresh spans can look late in Grafana. If the writer also flushes on a time limit, that limit is what protects freshness at low traffic. Check how the time limit is configured before assuming the row count alone controls when data appears.

The other cost is memory. A buffer holding `100000 rows` takes more heap than one holding 50000 rows. The writer runs on the JVM inside a Kubernetes pod, so the pod memory limit and the Java heap settings need to leave room for a full batch, plus whatever is being serialized for the insert at the same time. If you see out of memory kills or long garbage collection pauses after a rollout, look at this first.

## How to check it is working

After a deploy of span-ingest-writer, or any time someone suspects the batch size, look at these things:

- ClickHouse part counts for the span tables. They should stay flat or fall as merges catch up. A steady climb means merges are behind again.
- The merge activity in ClickHouse system tables. Background merges should be running and finishing, not queued up.
- The writer's own flush logs and metrics, to confirm the batches really are `100000 rows` and not the old 50000 rows. A stale config or an old image would silently keep the old value.
- Query latency on the Grafana panels that read from the span tables.

If part counts still climb at `100000 rows`, the batch size is no longer the main problem. Look at merge settings, disk throughput, and how many writer replicas are inserting at once. More replicas means more concurrent inserts, and each replica makes its own batches.

## Things to keep in mind later

- Do not lower the batch back toward 50000 rows to reduce memory or latency without checking merge health first. That was the setting that failed.
- If the value goes up again, record it by updating this note, not by writing a new one. The topic is the span-ingest-writer batch size and there should be one note for it.
- Anyone searching old notes for "span ingest writer flushes" will land on the superseded note. The current value is `100000 rows`.
- When talking to people who use the name `chwriter`, make clear it is `span-ingest-writer`, so nobody goes looking for a separate service.
- Batch size interacts with replica count and with ClickHouse merge capacity. Changing any one of the three means rechecking the other two.

## Open questions

We have not settled whether a still larger batch would help further or only add memory pressure and delay. The current value fixed the merge lag, and that is enough for now. If merges fall behind again as traffic grows, the first step is to look at replica count and merge capacity, and only then revisit the batch size.

We also have not written down how the time-based flush, if any, should be tuned relative to the row count. That is worth doing the next time someone touches the flush logic, since the two settings together decide both part counts and data freshness.
