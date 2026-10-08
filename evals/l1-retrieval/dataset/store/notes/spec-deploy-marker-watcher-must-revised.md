---
id: 01KZC7776A0X4SQ765FEJ21NG5
created: 2026-08-06T16:01-03:00
---

# deploy-marker-watcher spec

This note replaces the earlier note about "deploy marker watcher must". The new value is that deploy-marker-watcher must record a marker within 5 seconds of a rollout completing. The old target was 10 seconds and is no longer valid.

## Purpose

deploy-marker-watcher watches Kubernetes for rollouts finishing and writes a deploy marker for each one. TraceQuill uses those markers to split traces into before and after a deploy, so it can summarize latency regressions after each deploy. Site reliability engineers read the result in Grafana, so a late or missing marker shows up directly as a wrong comparison.

## Latency requirement

The requirement is simple: a marker is recorded within 5 seconds of a rollout completing. The clock starts when the cluster reports the rollout as complete. It stops when the marker is stored and can be queried. This replaces the earlier 10 second target. Anything that was sized or tuned against the old target needs a second look.

## Why the target moved

A marker that lands late shifts the boundary between the before and after windows. Traces from the new version then get counted as the old version, and the regression summary looks better than it is. Halving the allowed delay shrinks that blur. It also matters for short rollouts, where a slow marker could swallow most of the post-deploy window.

## Naming

The previous name was tickwatch. The component is called deploy-marker-watcher now. Old dashboards, manifests, log queries, chat history and runbooks may still say tickwatch. Treat them as the same component. Use deploy-marker-watcher in anything new, and rename the old references when you touch them.

## Scope

In scope: detecting rollout completion, building the marker, writing it to storage, and exposing its own health and timing. Out of scope: deciding whether a deploy was bad, computing the latency summary, and rendering panels. Those belong to other parts of TraceQuill.

## Inputs

The watcher reads rollout state from the Kubernetes API. It needs to tell a finished rollout from one still progressing, and it should ignore changes that are not rollouts. It can also use OpenTelemetry resource attributes to match a rollout to the services whose traces it affects, but the exact attribute set is defined elsewhere.

## Outputs

Each marker records which workload was deployed, which version, and when the rollout completed. Markers go into ClickHouse next to the trace data, so queries can join traces to markers without leaving the database. Grafana reads them from there to draw deploy annotations.

## Timing budget

The 5 seconds covers every step: noticing the event, building the marker, and the write. Rough thinking is that detection should be near immediate if the watcher uses a watch stream and not polling. That leaves most of the budget for the write and for retries. If polling is used anywhere, its interval has to be well under the target, or it eats the budget alone.

## Detection approach

Prefer a watch on rollout objects over periodic listing. Keep the watch alive and resume it after a drop without losing events. After a reconnect, do a full list once to catch any rollout that finished while the connection was down, and record markers for those. Late markers from this catch-up path are expected to miss the target, and should be flagged as late and not hidden.

## Writing markers

Writes to ClickHouse should be small and quick. Batching is fine only if the batch wait cannot push a marker past the target. A marker must not be written twice for one rollout, so use a stable key built from the workload, the version and the completion time. A repeated event then becomes a no-op.

## Failure handling

If ClickHouse is slow or down, retry with a short backoff while the budget lasts, then keep retrying in the background and mark the result as late. Never drop a marker silently. If the Kubernetes watch fails, reconnect and run the catch-up listing described above. Failures should be visible in logs and metrics, not only in missed annotations.

## Observability

The watcher should publish the delay between rollout completion and marker stored, as a histogram, so the 5 seconds can be checked against real data. It should also count markers written, markers written late, and errors. Put these on a Grafana panel and alert when the delay goes over the target for a sustained period, not on a single outlier.

## Deployment

It runs as a small Java service on Kubernetes. It needs read access to rollout objects and nothing broader. One active instance at a time is enough. If more than one runs for availability, the stable marker key keeps duplicates out, but the instances should not fight over the same work in a way that delays markers.

## Testing

Test the delay directly: complete a rollout in a test cluster and measure until the marker can be queried. The check passes only if that stays within 5 seconds across repeated runs, not once. Also test the reconnect path, a duplicate event, and a ClickHouse outage, and confirm each gives the right late flag and no duplicate.

## Open questions

Whether the target should be a hard limit or a percentile is not settled here. The wording above says each marker, so treat it as the stated goal for every rollout and watch the late counter. If a percentile is agreed later, update this note and keep the history of the 10 second target and the rename from tickwatch visible.
