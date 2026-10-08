---
id: 01KK0HKJTSKCEPR27BEKAKTCDS
created: 2026-03-06T00:03-03:00
---

# span-collector Java gateway design notes

These are loose notes on how the span-collector is shaped as the Java gateway in front of ClickHouse. I wrote them from memory of the design discussions and the code layout, not from a fresh read of everything. They are partial. Some of this probably overlaps with an older note on the same component, so treat whatever is there as the more careful record. The one I know of is [[span-ingest-writer-reads]], which covers the read side of the writer more closely than I do here.

The short version: span-collector takes spans from instrumented services, does the cheap work that has to happen before storage, and hands batches to ClickHouse. It is deliberately a thin gateway. The latency regression summaries after each deploy depend on it keeping up, and on it not inventing data.

## What the gateway is for

Services in the cluster emit spans through OpenTelemetry SDKs or agents. They send them to a collector endpoint rather than writing to the database directly. span-collector is that endpoint. It terminates the wire protocol, checks that the payload is something it understands, normalizes a few fields, and queues the result for writing.

The reason for a gateway at all is that we do not want every service to know about ClickHouse, its schema, or its batching preferences. SREs care about the summaries and the Grafana panels, and they should not have to think about how rows get in. The gateway is where the schema knowledge lives on the ingest side.

It is also the place where we can say no. If a sender is misbehaving, the gateway is where we shed load, not the database.

## Process shape

The service is a plain Java process run as a Deployment in Kubernetes. It is stateless in the sense that nothing on local disk is needed to restart it. There is an in-memory queue, and anything in that queue at the moment of a hard kill is lost. We accepted that tradeoff early on, because traces are sampled data anyway and a short gap is tolerable compared with the cost of a durable local buffer.

There are a small number of thread groups: receivers that accept connections and decode, a processing stage that does normalization, and writers that flush batches. They are separated by bounded queues so that a slow stage pushes back on the stage before it instead of growing memory without limit.

Scaling is horizontal. More replicas means more receivers and more writers. There is no coordination between replicas, which keeps things simple but means each replica batches independently.

## Receiving spans

The receiver side speaks the OpenTelemetry protocol over both of the usual transports. Most senders use the binary streaming one, and a few older or constrained clients use the HTTP form. Both paths decode into the same internal span representation so everything after that point is transport agnostic.

Decoding is where we reject bad input. A span without a trace identifier or with a clearly broken time range is dropped and counted rather than stored. We do not try to repair these. The count is exposed as a metric so someone can see when a sender starts producing junk.

Message size limits apply at the receiver. The configured limit is the usual protective one; senders that exceed it get a rejection and are expected to split their batches. We chose not to be generous here, because a single huge request can stall a receiver thread for a long time.

## Normalization and enrichment

After decoding, the processing stage makes a few changes. Resource attributes such as the service name and the deployment marker are lifted into dedicated fields, since the regression summaries group by them constantly. Timestamps are converted to the form the table wants. Attribute maps are trimmed if they go past the configured size, with a counter for how often that happens.

The deployment marker deserves a note. The whole point of TraceQuill is comparing latency before and after a deploy, so the collector needs to carry whatever tells us which release a span belongs to. We read it from resource attributes set by the sender or by the deploy tooling. If it is missing, the span is still stored, but it ends up in a bucket the summaries treat as unknown. We chose not to guess.

We avoid doing anything expensive here. No lookups against external services on the hot path. If enrichment needs outside data, it should be done later, not in the gateway.

## Batching and the ClickHouse writer

Writers take spans off the bounded queue and build batches. A batch is flushed when it reaches the configured size or when the configured time window passes, whichever comes first. ClickHouse prefers fewer, larger inserts, so the size trigger is what we hope to hit under normal load, and the time trigger is what keeps latency of visibility reasonable when traffic is light.

Inserts go over the native client path for the database. Each flush is one insert into the main spans table. We do not do per-span inserts, ever. The shape of the table, its ordering and partitioning, was decided on the storage side and the gateway just follows it.

The read side of the writer, meaning how it checks what was stored and how later queries see it, is covered in the related note. I do not want to restate that here and risk contradicting it.

## Backpressure

When ClickHouse slows down, writers slow down, the queue between processing and writing fills, and then the queue between receiving and processing fills. At that point the receivers stop reading from sockets as fast, and the transport-level flow control pushes back on senders. That chain is intentional.

If the pressure lasts, the receivers start refusing new requests with a retryable status. Well-behaved SDK exporters retry with backoff. Badly behaved ones drop spans, which is their choice. We would rather refuse cleanly than accept and then lose data silently in our own memory.

There was a discussion about adding a spill-to-disk stage. It was not done. The argument against was operational weight: disks on pods, cleanup, replay ordering. The argument for was fewer lost spans during database maintenance. For now we rely on retries from senders and on keeping database maintenance windows short.

## Failure handling on write

A failed insert is retried a limited number of times with a growing delay. After that the batch goes to a drop path, and the number of spans dropped is counted and logged at a sampled rate so logs do not flood. The goal is that a poisoned batch cannot block the writer forever.

We distinguish between failures that look transient, such as connection resets or timeouts, and failures that look like the data itself is wrong, such as a type mismatch. The second kind is not retried at all, because retrying a bad row does nothing useful. In that case we log enough to identify the sender and move on.

One gotcha worth remembering: a partial failure inside a batch can be confusing, because the database may have accepted some of it. We treat a failed insert as wholly failed from the gateway's point of view and accept that duplicates are possible on retry. The storage side is expected to tolerate that, or the summaries do.

## Configuration

Configuration comes from environment and a mounted config file in the pod. Things that are tunable include queue sizes, batch size and window, receiver limits, retry counts, and the database connection settings. Defaults are conservative. Overrides live in the deployment manifests, not in the code.

We try not to add knobs casually. Every knob is something an SRE might change at three in the morning without knowing the consequence. Before adding one, ask whether a sensible fixed behavior would do.

Secrets for the database come from Kubernetes secrets and are not logged. The config dump on startup redacts them. If you add a new setting that holds a credential, make sure it goes through the same redaction.

## Observability of the collector itself

The gateway exports its own metrics, which Grafana panels read. The ones that matter most are received spans, rejected spans by reason, queue depth for each stage, batch size at flush, flush duration, and dropped spans after retries. Those are enough to tell which stage is the bottleneck.

It would be a little circular to trace the tracer, so we do not send the collector's own spans through itself by default. Logs are structured and kept terse. Health endpoints separate liveness from readiness: readiness reflects whether the writer can reach the database, so a replica that cannot write is taken out of rotation instead of accepting spans it cannot store.

A dashboard for the collector sits next to the regression dashboards, and on-call looks there first when summaries look empty after a deploy.

## Deploys and rollouts of the collector

Rolling out a new collector version uses the normal Kubernetes rolling update. On shutdown the process stops accepting new requests, drains its queues up to a configured grace period, and then exits. If the grace period is too short, spans still in memory are lost. We set the pod termination grace to be longer than the drain, and that relationship is easy to break by accident when editing manifests.

A deploy of the collector itself can show up as a blip in the very summaries it feeds. Worth keeping in mind when reading a regression report that coincides with a collector rollout: a dip in span counts may be ours, not the service's.

## Things deliberately left out

No tail-based sampling in the gateway. That was considered and pushed elsewhere, because it needs holding whole traces in memory and the gateway is stateless on purpose. Head sampling happens at the senders.

No schema evolution logic beyond what the table already supports. New attributes go into the generic attribute map unless there is a strong reason to promote them to a column. Promoting a column is a storage change first and a gateway change second.

No authentication beyond what the cluster network and ingress already provide. If that changes, it belongs at the receiver and should be designed as its own piece of work.

## Open questions

Whether to add a durable buffer remains open. It would help in database outages but costs complexity. Someone should look at how often we actually drop under real incidents before deciding.

Whether batching should be adaptive instead of using a fixed size and window. Under bursty load the fixed settings are either too small or too slow. An adaptive scheme might help, but it is hard to reason about and I would want measurements first.

Whether deployment marker handling should move earlier, to the senders, so the unknown bucket shrinks. That depends on how much the deploy tooling can be trusted to set the attribute consistently.

## Where to look in the code

I am not quoting file names because I have not rechecked them. In general the layout follows the stages above: a receiving package, a processing package, a writer package, and a small configuration package. Start from the receiver wiring and follow the queues. The queue boundaries are the best map of the system.

If something here disagrees with the code, trust the code, and then fix this note or the earlier one rather than leaving two versions of the story.
