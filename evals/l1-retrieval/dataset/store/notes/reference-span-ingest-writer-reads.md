---
id: 01KCWNH5E61K8B7CMBKH583S1B
created: 2025-12-19T22:21-03:00
---

# span-ingest-writer reference

span-ingest-writer is the TraceQuill service that takes spans off the collection path and writes them into ClickHouse. Everything downstream depends on it: the latency regression summaries produced after each deploy, and the Grafana panels the site reliability engineers look at. This note is the quick reference for how it is configured, how to check that it is alive, and what it is called in old docs and chat.

The two facts people ask about most: span-ingest-writer reads the ClickHouse connection string from the environment variable `TQ_CLICKHOUSE_URL`, and it serves its health endpoint at `/healthz` on port `8081`. Both are covered in detail below. Its internal codename is `inkwell`, which is the same component as `span-ingest-writer`.

## Configuration and health endpoint

### Connection string

span-ingest-writer gets the ClickHouse connection string from the environment variable `TQ_CLICKHOUSE_URL`. There is no config file entry for it and no command line flag that overrides it. If the variable is unset or empty, the service cannot reach the database and it will not become healthy. If you are debugging a pod that does not come up, the first thing to check is whether `TQ_CLICKHOUSE_URL` is present in the container environment and whether it points at the right ClickHouse cluster for that environment.

The value is a single string that carries the host, the database and the credentials, in whatever form the ClickHouse Java client in use accepts. Treat it as a secret. It belongs in a Kubernetes Secret that is mapped into the container environment, not in a plain config map and not in the deployment manifest in clear text. Do not paste it into tickets, chat or logs. When you share a deployment spec for review, share the reference to the secret, not the value.

Changing the value requires a restart of the pods. The service reads the variable once at startup and builds its connection pool from it. Editing the secret alone does nothing to a running pod. After a rotation, roll the deployment and watch the health endpoint come back.

### Health endpoint

The health endpoint is `/healthz`, served on port `8081`. That port is separate from the one that receives spans, so a health probe never competes with ingest traffic for the same listener. Kubernetes liveness and readiness probes for span-ingest-writer should both target `/healthz` on `8081`.

A successful response means the process is up and has what it needs to take work. A failing response most often means the service has lost its connection to ClickHouse or never made one. It is not a deep check of data quality. A healthy answer does not prove that spans are landing in the right tables, only that the writer believes it can write.

Quick check from inside the cluster or through a port forward:

```
TQ_CLICKHOUSE_URL=<connection string>
GET /healthz on port 8081
```

The first line is what the service reads at startup. The second is the probe target. Nothing else is needed to confirm basic liveness.

## Naming: inkwell and span-ingest-writer

`inkwell` is the internal codename of span-ingest-writer. You will see `inkwell` in older design documents, in some dashboard titles, in a few alert descriptions and in chat history from the early days of the project. It is not a second service. If someone asks whether `inkwell` is deployed, the answer is that it is deployed under the name span-ingest-writer.

Use `span-ingest-writer` in anything new: runbooks, alert names, deployment manifests, tickets. Keep `inkwell` in mind only so that searching old material still finds the right thing. When you update an old document that says `inkwell`, add the real component name next to it rather than silently replacing it, so that people who search for the codename can still land on the page.

A few places where the two names may still be mixed:

- Older Grafana dashboards, where panel or folder titles may use the codename.
- Alert rule descriptions written before the rename to the descriptive name.
- Commit messages and pull request titles from early development.
- Verbal shorthand among people who were on the team early.

None of these change behavior. They are labels only.

## Role in the TraceQuill pipeline

TraceQuill collects distributed traces using OpenTelemetry. Applications and sidecars emit spans, a collection tier receives and batches them, and span-ingest-writer is the stage that persists them. After that stage the data is in ClickHouse, where the regression summarizer and the Grafana dashboards read it.

The service is written in Java and runs on Kubernetes as a regular deployment. It is stateless apart from in-memory batches that are waiting to be flushed. Because of that, it can be scaled out by adding replicas and it can be restarted at any time, with the usual caveat that spans buffered in memory at the moment of a hard kill may be lost.

### What it does with a span

In order, roughly:

- Receives a batch of spans from the upstream collection tier.
- Maps the OpenTelemetry span fields onto the ClickHouse table layout. Attributes that are not part of the fixed columns go into the flexible attribute columns.
- Groups spans into larger insert batches. ClickHouse prefers fewer, larger inserts over many small ones, and this service is built around that preference.
- Flushes a batch when it is big enough or when it has waited long enough, whichever comes first.
- Retries a failed insert a limited number of times with backoff, then drops the batch and records the loss in its own metrics.

The size and timing thresholds are tuning knobs. Their current values live in the deployment configuration, not in this note, because they change with traffic. Do not copy numbers from an old document and assume they still hold.

### Why batching matters for regression summaries

The post-deploy regression summaries compare latency distributions before and after a release. They need spans to arrive in ClickHouse within a short and fairly predictable delay. If span-ingest-writer falls behind, the summary for a fresh deploy can be computed on partial data and look better or worse than it really is. So ingest lag is not just an operational nuisance here. It can produce a wrong answer in the report that engineers use to decide whether to roll back.

When a regression summary looks odd right after a deploy, check ingest lag before blaming the release. The check is simple: compare the newest span timestamp in ClickHouse with the wall clock. A gap larger than usual points at the writer or at the database, not at the code that was deployed.

## Operating notes and troubleshooting

### Starting and restarting

On start, span-ingest-writer reads `TQ_CLICKHOUSE_URL`, opens its pool and then begins answering on `/healthz` at port `8081`. During a rolling update, wait for the new pods to report healthy on that endpoint before the old ones are removed. If readiness is wired to `/healthz`, Kubernetes does this by itself.

Graceful shutdown tries to flush what is in memory before the process exits. Give the pod enough termination grace time for that flush. If the grace period is too short, the tail of the buffer is lost on every rollout, which shows up as a small, repeatable gap in spans right at deploy time. That is exactly when the regression summary looks at the data, so it matters more than it seems.

### Common failure patterns

The service does not become healthy at startup. Check that `TQ_CLICKHOUSE_URL` is set, that the secret it comes from exists in the namespace, and that the network policy allows the pod to reach ClickHouse. A typo in the connection string is the most frequent cause and the service log will show the connection error near the top.

The service was healthy and then turns unhealthy. Usually ClickHouse went away or restarted, or credentials were rotated on the database side without updating the secret. Fix the cause, and if the secret changed, restart the pods, since the variable is only read at startup.

The service is healthy but data is late. This is the more insidious case, because `/healthz` on `8081` looks fine. Suspects, in rough order: ClickHouse is under merge or query pressure and inserts are slow; the writer has too few replicas for current traffic; batch thresholds are too large for a quiet period so that spans wait to be flushed; the upstream collection tier is backed up and the problem is not in the writer at all.

Spans are missing, not late. Look at the writer's drop metrics. If batches are being dropped after retries, ClickHouse was rejecting or timing out on inserts for longer than the retry budget. The fix is on the database side or in capacity. Raising the retry budget only hides it and increases memory use.

Memory grows steadily. Buffers are accumulating because flushes are failing or slow. Treat it as a ClickHouse problem first. Restarting the pod clears the memory but also loses the buffered spans.

### Things to remember

- Configuration of the database target is `TQ_CLICKHOUSE_URL` and nothing else. Do not go looking for a second place to set it.
- Health is `/healthz` on `8081`. Probes, port forwards and service monitors should all use that pair.
- `inkwell` and `span-ingest-writer` are one component. Say `span-ingest-writer` in anything new.
- Healthy does not mean current. Check ingest lag separately when a post-deploy summary looks wrong.
- Rotating the connection string needs a restart.

### Who to ask

The service is owned by the TraceQuill platform team and used mostly by site reliability engineers, who also tend to be the first to notice ingest problems because they live in the Grafana dashboards. If the dashboards go flat after a deploy, start with the health endpoint, then the connection variable, then ClickHouse itself, in that order. Most incidents end at one of those three.

This note should be updated whenever the environment variable, the health path or the health port changes, and whenever the codename stops appearing in old material so that the mapping above can be trimmed.
