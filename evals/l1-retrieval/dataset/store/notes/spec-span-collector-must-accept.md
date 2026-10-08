---
id: 01JRJ6RAR3FEN7YDJK7GNPVPAR
created: 2025-04-11T07:36-03:00
---

# span-collector request size spec

This note specs how span-collector handles incoming OTLP export requests in TraceQuill, mainly the size limit on a single batch. The main rule: span-collector must accept OTLP export batches of up to 8 MiB per request. Anything at or under that size is taken in; anything over is a client-side problem to be fixed by splitting.

## Scope

span-collector is the entry point for distributed traces in TraceQuill. Instrumented services running in Kubernetes send spans to it using OpenTelemetry exporters. It hands the spans on toward ClickHouse, where the latency regression summaries after each deploy are computed. This spec covers only the receive side: how big a request may be and what happens at the edge of that limit.

## The limit

The per-request ceiling is 8 MiB, measured on the request body as it arrives. If the exporter compresses the payload, the limit is meant to apply to the size the collector has to deal with, so the reader of this note should check which side of decompression the implementation counts. Right now the intent is that a full 8 MiB batch is never rejected just because of its size. Smaller batches must of course also work, and so must very small ones with a single span.

## Why this size

Exporters in the field batch spans by count or by time, and a busy service during a deploy can produce large batches fast. A limit that is too low makes SRE-facing data go missing exactly when deploys happen, which is when the regression summaries matter most. A limit that is too high lets one client hold a lot of memory in the Java process. 8 MiB was picked as a middle point and should be treated as a contract that clients can rely on.

## Behavior over the limit

A request larger than the limit should be refused with a clear error and not truncated. Partial acceptance of a batch is not allowed, because it would produce traces with holes and hurt the latency numbers. The error should tell the client that the payload was too large so the exporter config can be lowered. Refusals should be counted in a metric so they show up on a Grafana panel.

## Memory and backpressure

Because many requests can arrive at once, the collector should not buffer unbounded data. The size limit per request is only one part of this; the number of requests in flight also needs a bound. When the bound is hit, the collector should push back on clients and not grow its heap. Pod memory requests in Kubernetes must be sized with the maximum batch in mind, times the allowed concurrency.

## Testing

Tests should cover a batch just under the limit, a batch exactly at the limit, and one just over it. Also test a compressed payload that expands past the limit. A load test with several large batches at once should confirm that memory stays inside the pod limits and that nothing is dropped silently.

## Open questions

- Whether the limit counts compressed or decompressed bytes needs to be confirmed in the code.
- Whether the limit should be configurable per deployment or stay fixed.
- What the exact status code and wording of the over-limit error should be, so exporter retry logic does not retry forever.
