---
id: 01K8P564QQD451MJVEMS17M60F
created: 2025-10-28T16:37-03:00
---

# span-ingest-writer batch size for ClickHouse inserts

span-ingest-writer flushes inserts to ClickHouse in batches of 50000 rows. We chose this to keep the number of new ClickHouse parts per minute low. Small, frequent inserts each create a new part, and the background merges then fall behind. This note records the decision and the reasoning so nobody has to rediscover it when the flush size looks arbitrary.

## Decision

The writer in span-ingest-writer buffers incoming spans and flushes when the buffer reaches 50000 rows. Each flush is one insert, so each flush produces roughly one new part per affected partition. A bigger batch means fewer parts for the same span volume.

## Why

ClickHouse is happiest with a modest number of large inserts. When parts are created faster than merges can combine them, the table gets "too many parts" pressure. Inserts then get slowed down or rejected, and read queries touch more files. TraceQuill depends on fast reads: the latency regression summaries after each deploy and the Grafana dashboards both query the span tables. A part explosion hurts the SRE-facing views first.

The trace volume right after a deploy is the worst case. Traffic is bursty, and that is when people look at the summaries. Batching by row count keeps the part creation rate bounded even while the span rate spikes.

## Trade-offs

- Latency: a span can sit in the buffer until the batch fills, so visibility in queries is delayed. At low traffic this delay grows, which is why the writer should also have a time-based flush as a safety net. Check the current config before relying on it.
- Memory: the Java process holds a full batch in heap before writing. With many partitions or wide span attributes, the heap use per batch is larger than it looks.
- Failure: if the writer dies, anything unflushed in the buffer is lost unless the upstream OpenTelemetry collector retries. Larger batches mean a larger loss window.
- Retries: a failed insert of a big batch is retried as a whole. The insert should be idempotent or deduplicated, otherwise a retry can duplicate spans.

## What to watch

Watch the new-parts-per-minute rate and the active part count per partition in ClickHouse, and the merge backlog. If parts climb while the row count per flush stays at the limit, the cause is probably too many partitions touched per insert, not the batch size. If query freshness complaints show up in Grafana, look at the time-based flush before lowering the row limit.

## Changing it

Do not lower the batch size to fix freshness without checking part counts first. Raising it needs a heap check on the Kubernetes pod limits for the writer. Whoever changes it should update this note with the new value and the measurements that justified it.
