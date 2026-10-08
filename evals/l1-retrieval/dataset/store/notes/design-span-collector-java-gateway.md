---
id: 01K2H65AR2QKDDBC1FPRZV2AZ5
created: 2025-08-13T04:43-03:00
---

# span-collector design

span-collector is the ingest gateway of TraceQuill. It is a Java 21 service that exposes an OTLP gRPC receiver on port 4317 and forwards accepted spans to the Kafka topic tq.spans.v1. Everything downstream (the ClickHouse writers, the latency summaries after each deploy, the Grafana panels the SREs look at) starts from that topic. This note records how the component is shaped and why, so nobody has to rebuild the reasoning from the code.

## Names

quillgate is the internal codename of span-collector. You will see quillgate in older dashboards, some Kubernetes labels, chat history and a few log fields. It is the same component as span-collector, not a separate service and not a fork. In new docs, alerts and code comments, use span-collector. When searching old material, search for both names, because half the history uses the codename.

## What it does

The job is narrow on purpose. It accepts OTLP trace exports over gRPC, checks that the payload is usable, and hands the spans to Kafka. It does no aggregation, no sampling decisions beyond what the sender already made, and no enrichment that needs a lookup against another system. Anything that needs state or a join belongs further down the pipeline, where it can be replayed from the topic.

Receiving side:
- One gRPC listener on port 4317, the standard OTLP gRPC port, so stock OpenTelemetry SDKs and agents work with default settings.
- Requests are decoded, basic validation is applied, and spans that pass are batched.
- Bad requests get a gRPC error back so the sender can see the problem. We do not silently drop them.

Sending side:
- Accepted spans go to the Kafka topic tq.spans.v1.
- The suffix on the topic name is the schema version. A breaking change to the record layout means a new topic with a new suffix, with both running side by side for a while, rather than an in-place change.
- Records are produced in batches. The collector acknowledges the gRPC request only after the batch has been handed to the Kafka producer under the configured delivery guarantee, so a client that got a success response can assume the spans reached the broker.

## Why a gateway in front of Kafka

We did not want application services to know about Kafka. Senders speak plain OTLP and only need an endpoint. That keeps credentials, topic names and partitioning out of every service's configuration, and lets us change the transport behind the gateway without touching the fleet.

It also gives one place to apply protection. If Kafka is slow or unavailable, the gateway is where backpressure shows up, and we can choose how to answer clients instead of having every service invent its own behavior.

## Backpressure and failure behavior

The rule is that the collector must never grow memory without bound. Incoming work waits in a bounded queue. When the queue is full, new requests are refused with a retryable gRPC status, and standard OpenTelemetry exporters back off and retry. Losing some spans during a long Kafka outage is acceptable; taking the gateway down with an out-of-memory kill is not, because that loses everything from every service at once.

Things to keep in mind:
- A refused request is better than a delayed one that times out on the client and is then retried, producing duplicates.
- Duplicates can still happen on retry, so downstream consumers must be tolerant. De-duplication is their job, not the gateway's.
- Shutdown should drain the queue and flush the producer before the process exits. Kubernetes needs a termination grace period long enough for that, or deploys will quietly lose the last batch from each pod.

## Deployment on Kubernetes

span-collector runs as a Deployment behind a Service. gRPC is long-lived HTTP/2, so plain connection-level load balancing pins a client to one pod and leaves others idle. Either use a load balancer that understands gRPC streams, or have the collector close connections after a maximum age so clients reconnect and spread out. Without one of these, a rolling deploy produces uneven load and noisy latency numbers, which is exactly the signal TraceQuill is supposed to report on.

Scaling is horizontal. Pods hold no state beyond the in-flight queue, so adding or removing replicas is safe. Autoscaling on CPU works reasonably, since most of the cost is decoding and compression. Memory limits should be set with the queue bound in mind so the two agree.

## Observability of the collector itself

The gateway needs to be watched like any production service, and the numbers are easy to misread.
- Count accepted and refused requests separately. A rise in refusals is the first sign that Kafka or a downstream stage is struggling.
- Track queue depth and time spent in the queue.
- Track producer errors and batch send latency.
- Expose these through OpenTelemetry metrics and chart them in Grafana next to the pipeline panels.

Do not alert on a single pod. Alert on the fleet-level refusal rate, since one pod being restarted during a deploy is normal.

## Open questions and caveats

- Whether to add per-tenant rate limits at the gateway is undecided. It would protect shared capacity but adds configuration that someone has to own.
- Payload size limits exist but have not been tuned against the largest real traces. Very wide traces may be rejected until that is done.
- The interaction between deploy-time connection churn and latency regression detection is a known source of false positives. See [[regression-detector-watch-outs]] for how the detector side handles it.
- Old material that says quillgate is talking about span-collector, so do not treat it as a missing component.
