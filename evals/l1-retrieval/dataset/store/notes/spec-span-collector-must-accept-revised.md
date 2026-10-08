---
id: 01JYC7CZHCJ75BACB45CXGQQKK
created: 2025-06-22T13:26-03:00
---

# span-collector: OTLP batch size limit

span-collector must accept OTLP export batches of up to 16 MiB per request. This note replaces the earlier note "span collector must accept", which gave the old limit of 8 MiB; the new value is 16 MiB and nothing else from that note changes the intent.

## Why the limit moved

Some services in the fleet export large batches right after a deploy, when many spans finish close together. With the old limit, span-collector rejected those requests and the exporters either retried or dropped data. The drops landed exactly in the window SREs care about most, the period just after a rollout, so the latency regression summaries were built on thin data. Raising the ceiling to 16 MiB removes the most common cause of those rejections.

## The requirement

A single OTLP export request, whether it arrives over gRPC or over HTTP, may carry a payload of up to 16 MiB. span-collector must read it, decode it and hand the spans on without rejecting it for size. A request above 16 MiB may be refused. The limit applies to one request, not to a connection, a client or a time window.

## What counts toward the size

The limit is about the request payload as the collector receives it. When the client compresses the body, the question is which size is checked. The intent is that the limit protects memory, so the decoded size is what matters in the end, and the check must not let a small compressed body expand into something far larger than 16 MiB. Whoever implements this should decide where the check sits and write it down here. Until then, treat 16 MiB as the ceiling on what the collector will hold in memory for one request.

## Scope of the change

This spec covers span-collector only. Exporters, agents and any gateway in front of it have their own settings. If one of them has a lower cap than 16 MiB, large batches will still fail there and the failure will look like a collector problem. Check every hop between the application and span-collector before declaring the new limit live.

## Configuration

The limit should be a setting, not a constant buried in code. The default is 16 MiB. Operators may lower it for small clusters. Raising it above the default is not part of this spec and needs its own review, because memory use grows with it. Keep the setting name in line with the other collector settings and document it next to them.

## Memory and resource impact

Doubling the ceiling doubles the worst case for one in-flight request. If many large requests arrive at once, the collector pods can run out of memory under Kubernetes limits and get killed. Review the pod memory requests and limits before rollout. Consider a cap on concurrent large requests, or a bound on total bytes in flight, so that one noisy service cannot take the collector down. A restart loop would lose more spans than the old limit ever did.

## Interaction with the pipeline

After decoding, spans go on toward storage in ClickHouse. A larger request means a larger batch to write. The writer should split big batches into smaller inserts rather than send one huge insert, so that ClickHouse is not hit with a single heavy write. Back pressure should flow upstream: when the writer is slow, the collector should slow its intake or return a retryable status, not buffer without bound.

## Error behavior

When a request is over the limit, span-collector must answer with a clear, non-retryable status for the transport in use, so exporters do not retry a request that can never succeed. The message should say that the payload is too large and state the configured limit. Do not close the connection silently. Requests under the limit that fail for other reasons keep their existing behavior.

## Client guidance

Exporters should batch below the limit with some margin. The OpenTelemetry SDKs have batch settings for span count and for timing; teams should tune them so a typical batch is well under 16 MiB. The higher ceiling is a safety margin for bursts, not an invitation to send huge batches as the normal case. Large batches also raise the cost of a single failed attempt.

## Observability

Add metrics so the limit can be watched in Grafana. Useful ones: the size distribution of accepted requests, a count of rejected requests with the reason, and in-flight bytes. A panel showing how close real traffic comes to the ceiling tells us whether it needs another look. An alert on a rise in size rejections would catch a misconfigured exporter early.

## Testing

Tests should cover a request just under the limit, a request exactly at it, and one just over it, on both transports. Add a compressed case that expands past the limit. Run a load test with several large requests at once against pod limits that match production, and watch memory. The old tests that asserted the previous limit must be updated to the new value, not deleted.

## Rollout

Ship the collector change first, then raise caps on any gateway or ingress in front of it, then tell service owners they may tune their exporters. Roll out one environment at a time and watch memory and the rejection counters after each step. Keep the ability to set the limit back to the earlier value through configuration if memory pressure shows up.

## Open questions

Where exactly the size check sits relative to decompression is still to be fixed. Whether to cap total bytes in flight, and at what level, is not decided. Whether a lower limit per tenant is wanted is also open. Record the answers here when they are settled, and keep this note as the single place for the batch size requirement of span-collector.
