---
id: 01K59BRMXJZY4FRD5SQWVTJFS0
created: 2025-09-16T10:34-03:00
---

# span-collector rejects oversized batches

When an exporter sends a batch above the configured limit, span-collector rejects the whole batch and the client logs `RESOURCE_EXHAUSTED: grpc: received message larger than max`. Nothing in the batch is stored. It looks like a collector outage from the client side, but span-collector is up and answering; it is refusing one message.

## Symptom

The exporter side shows the error below, usually repeated on every export attempt for the same batch.

```
RESOURCE_EXHAUSTED: grpc: received message larger than max
```

Traces from the affected service go missing or have holes in them. Other services that send smaller batches look fine, which makes the problem seem random.

## What is actually happening

The limit is on the size of a single message received by span-collector. If the exporter packs more spans into one batch than the limit allows, the receiver drops the message at the gRPC layer before any span is read. The error is returned to the client, and the client logs it.

## Why it is easy to miss

The error is logged by the client, not by span-collector. Looking only at span-collector logs and dashboards shows no failure. Ingest rate for the service simply drops, and that can be read as low traffic.

## Where it bites

Busy services after a deploy are the usual victims. A deploy produces a burst of spans (restarts, warmup, retries), batches fill up, and the batch crosses the limit. That is exactly the window TraceQuill needs for the latency regression summary, so the post-deploy report can look clean when data is missing.

## How to confirm

- Search the exporter or application logs for the error string above.
- Compare the service's span count in ClickHouse before and after the deploy; a sharp drop with no traffic change points here.
- Check Grafana for the service's ingest panel; a flat or falling line while the service is busy is a hint.

## Fix on the exporter side

Lower the maximum batch size in the OpenTelemetry exporter configuration so a batch cannot exceed what span-collector accepts. Also make sure the export queue does not merge several batches into one message. This is the safer fix because it does not change the server.

## Fix on the collector side

Raising the receive limit on span-collector also works, but it raises memory use per connection and can push pods toward their Kubernetes limits. Do it only if the exporter side cannot be changed, and watch memory after the change.

## Keep the two limits aligned

The exporter batch size and the span-collector receive limit have to be chosen together. If someone raises one without the other, the error comes back. Whoever changes either value should check the other.

## What not to do

- Do not treat the error as a transient network fault and add retries. The same batch is retried and rejected every time.
- Do not restart span-collector. It is healthy.
- Do not assume the dropped spans will arrive later. They are lost unless the client keeps and resends smaller pieces.

## Effect on regression summaries

If a service was rejecting batches during a deploy window, its latency summary is built from partial data. Mark that summary as unreliable, or rerun it after the fix when the spans can be recovered from the source. Do not read a missing regression as no regression.

## Detection idea

An alert on a sudden fall in per-service ingest, combined with a deploy event, would catch this earlier than waiting for someone to read client logs. Not built yet.

## Open questions

- Whether the exporters in use can report dropped batches as a metric we can graph.
- Whether span-collector should expose a counter of rejected messages so the server side can see this too.

## Related components

The exporters belong to the instrumented Java services. Storage is ClickHouse. Dashboards are in Grafana. Only the batch size and the receive limit are involved in this problem.

## Short version

See `RESOURCE_EXHAUSTED: grpc: received message larger than max` in a client log: shrink the exporter batch first, then check the span-collector limit, then recheck the post-deploy summary.
