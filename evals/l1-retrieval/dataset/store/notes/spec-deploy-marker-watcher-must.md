---
id: 01KHT6D3CJ53P5BBB0H6888PDJ
created: 2026-02-19T02:36-03:00
---

# deploy-marker-watcher spec

deploy-marker-watcher is the piece of TraceQuill that notices a Kubernetes rollout finishing and writes a deploy marker. The marker is what the latency regression summaries pivot on: everything before the marker is "before deploy", everything after is "after deploy". If the marker is late or wrong, the summary compares the wrong windows and SREs lose trust in it. This note is the spec for what the watcher has to do. It is not a design doc, so implementation choices are left open unless stated.

## Core requirement

deploy-marker-watcher must record a marker within 10 seconds of a rollout completing. That is the one hard timing rule. "Rollout completing" means Kubernetes reports the rollout of a workload as finished, meaning the new replicas are available and the old ones are gone or draining as the rollout strategy defines. The clock starts at the moment the cluster reports completion, not when the deploy was kicked off and not when the first new pod came up. The clock stops when the marker is durably stored and queryable by the summarizer.

A reader asking "how fast does the marker have to show up" should get this answer: within 10 seconds of the rollout completing, measured end to end, including any batching or retry delay inside the watcher.

## What a marker contains

Keep this general; the exact schema lives in code, not here.

- The workload identity: namespace, workload name, and the cluster it ran in.
- The rollout completion time as seen from the cluster, plus the time the watcher recorded it. Both are stored so lag can be measured.
- The new version identifier of the workload, such as the image tag or revision, and the previous one when known.
- Enough to deduplicate: the same completion event must not create two markers.

Markers go into ClickHouse next to the trace data so the summarizer can join on time without calling another service. Grafana panels read the same table to draw deploy annotations on latency graphs.

## Behavior

- Watch rollout status through the Kubernetes API using a watch, not slow polling. Polling is only a fallback when the watch breaks, and it must still meet the timing rule or the watcher must report that it is behind.
- Handle restarts of the watcher itself. After a restart, reconcile with current cluster state so a rollout that completed during downtime still gets a marker. A late marker like this carries its real completion time, and it is flagged as late rather than pretending it was on time.
- Handle watch disconnects and API errors with bounded retries and backoff. Backoff must not push a healthy case past the timing rule.
- Writes to ClickHouse should be small and prompt. Do not hold a marker in a large batch waiting for more rows. If ClickHouse is unavailable, buffer in memory with a limit, retry, and surface the failure through metrics instead of silently dropping.
- Rollouts that fail or are rolled back do not get a normal "completed" marker. If we record them at all, they use a distinct type so the summarizer does not treat them as a good deploy.
- Rapid back to back rollouts of the same workload each get their own marker. Do not collapse them.

## Observability of the watcher

The 10 second rule is only useful if we can see it holding. The watcher should expose the lag between rollout completion and marker recorded as a metric, and an alert should fire when that lag goes over the limit for real rollouts. Also expose a count of markers written, a count of write failures, and the age of the oldest buffered marker. Put these on a Grafana dashboard for the watcher so an SRE can tell quickly whether a missing regression summary is caused by a missing marker.

The watcher is itself instrumented with OpenTelemetry, so its own slow paths show up as traces in TraceQuill.

## Out of scope and open questions

- Deciding whether a deploy caused a regression. That is the summarizer's job; the watcher only supplies the marker.
- Non-Kubernetes deploy sources. Not supported now.
- Open: how to treat rollouts that complete in a partial state, such as a paused or progress-deadline-exceeded workload. Needs a decision from the SRE users before implementation.
- Open: whether the lag metric should be per workload or per cluster. Per cluster is cheaper; per workload is easier to debug with.

## Testing notes

Test the timing rule against a real or kind-style cluster, not only with mocks, since the watch behavior is what can go wrong. Include a case with ClickHouse slow to respond, a case with a watcher restart mid-rollout, and a case with a dropped watch connection right as the rollout finishes. Each should either meet the 10 seconds rule or visibly report the miss.
