---
id: 01K1TVKQMY4PNBY3VC1YBE23ZW
created: 2025-08-04T12:36-03:00
---

# deploy-marker-watcher: things to watch when changing it

Notes for anyone about to touch `deploy-marker-watcher`. Nothing here is a spec. It is a list of places where changes have gone wrong, or are likely to, written down so the next person does not rediscover them. Check the code before trusting any detail. The component is small, but everything downstream depends on it. If it records a deploy boundary in the wrong place, every latency regression summary SREs read afterwards is quietly wrong, and nothing errors out.

The short version: the watcher turns "something rolled out in Kubernetes" into "here is a boundary in time for a service", and the summaries compare traces before that boundary with traces after it. Any change that moves the boundary, duplicates it, loses it or labels it with the wrong service changes what the report says. Treat the marker as data that other things are built on, not as a log line.

## What the watcher actually promises

Before changing behavior, be clear about what the rest of TraceQuill assumes the watcher does. Read the consumers first. The summary job, the Grafana panels and the collector side each hold a slightly different picture of it.

- A marker means "the new version started taking traffic for this service", not "someone applied a manifest". Those moments can be far apart. A change that emits earlier, for example on the first sign of a rollout, shifts the boundary and makes the old version look better than it was, because the before window then contains a mix of both versions.
- One rollout should produce one marker per service. Several signals can describe the same rollout (a controller update, a new replica becoming ready, a status condition flipping). Whatever deduplicates them is load-bearing. If you add a new event source, check that the deduplication key still collapses it with the existing sources.
- Markers are append-only in spirit. Later code may refine or annotate a marker, but a marker that was once used in a summary should not silently vanish or move. If you need to correct one, write a correction that is visible as a correction.
- The watcher is not the authority on whether a deploy was good. It records facts. Verdict logic belongs in the summary step. Do not let health judgments leak into the marker path because it is convenient.

If you find yourself adding a field to the marker because a panel needs it, stop and ask whether the panel should join against something else instead. Fields added to markers are hard to remove, since old rows keep them and old queries read them.

## Ordering, duplicates and restarts

The watcher is a long-running process in a cluster, so it restarts, gets rescheduled, loses its connection to the API server and reconnects. Most bugs here come from that.

**Watch streams are not a log.** After a reconnect you may get a replay of current state, or you may miss events that happened during the gap, depending on how the resume is done. Code that assumes it sees every transition will drop markers in exactly the busy periods when deploys cluster. Code that assumes every event is new will duplicate markers after every reconnect. Handle both: reconcile against current state on start and after any resume, and make marker writes idempotent.

**Idempotency needs a stable key.** The key has to be derived from facts about the rollout that do not change between observations of it. Anything that includes the observation time, a local counter or a pod name that churns will produce a new key each time and defeat the point. Anything too coarse will merge two real rollouts of the same service that happen close together. When you change the key, think about both failure directions, and think about what happens to rows written under the old key. A rolling restart of the watcher during a key change is the worst moment, because old and new code can both be live.

**Two instances at once.** During a rollout of the watcher itself, there can be a period where the old and new instances both run. Decide on purpose whether that is safe. If you rely on there being a single writer, say where that is enforced. If you rely on idempotent writes instead, test with two writers racing on the same event.

**Rollbacks are deploys.** A rollback creates a new boundary, and the summaries need it. Code that tries to be clever by comparing the new version string with the previous one and skipping "nothing new" will miss rollbacks to a version that ran earlier. Also watch for redeploys of the same version with changed configuration. These are real boundaries for latency purposes even though the version label did not change.

**Paused, stuck and partial rollouts.** A rollout that never completes has no "done" moment. Decide what the marker means then and keep that consistent. Do not add a timeout that fabricates a completion marker unless the summary side knows the marker may be synthetic.

**Many services in one release.** A single release can touch many services within a short time. Per-service markers should stay per-service. Resist collapsing them into one release-wide marker for convenience, because services reach steady state at different times and the regression window for each one differs.

## Time and windows

The watcher's whole output is a timestamp attached to a service, so anything touching time deserves a slow read.

- Know which clock each timestamp comes from: the cluster's recorded event time, the watcher's own wall clock, or the database's clock. They disagree, sometimes by a lot under load or after a node has been paused. Mixing them within one marker is how you get a boundary that sits after the first traces from the new version.
- Prefer the time the cluster says the change happened over the time the watcher noticed it. The watcher's delay is variable, and it gets worse exactly when the cluster is busy. If you switch sources, expect a visible shift in where historical and new boundaries fall, and tell the people who read the summaries.
- Traces are timestamped by the instrumented services, using OpenTelemetry SDK clocks in their own processes. Spans from the new version can carry timestamps slightly earlier than the marker, and old-version spans can show up after it. The summary has to tolerate that overlap. Do not "fix" the watcher to make the overlap disappear, since that is not possible from this side, and attempts to do it push the marker later and later.
- Late-arriving data matters. Traces reach ClickHouse after buffering and batching in the collector path. A marker written promptly is fine, but anything that reads it immediately and summarizes will see an incomplete after-window. If you add an eager trigger from the watcher into the summary flow, make the delay explicit and configurable, not hidden in a sleep.
- Daylight saving, time zone conversion and truncation to coarse buckets are all places where a boundary can slide. Store instants, convert only at the display edge.
- Be careful with inclusive versus exclusive edges. A trace that straddles the marker needs a rule, and the rule has to match what the summary does. Changing one side without the other produces off-by-a-window discrepancies that look like noise.

## Kubernetes and deployment of the watcher itself

The watcher reads cluster state, so its permissions, selectors and resource shape matter.

**Permissions are narrow on purpose.** If you add a new resource type to watch, the role binding has to grow, and that change has to ship before or with the code. A watcher that starts, gets a forbidden response on the new resource, and keeps running with half its inputs is worse than one that crashes. Make a missing permission loud at startup.

**Selectors and labels decide service identity.** The mapping from a workload to a service name is where mistakes hide. Workloads can be renamed, split or merged, and the label that identified the service yesterday may be absent on a new kind of workload. The service name in the marker must match the service name that the OpenTelemetry resource attributes put on spans, or the summary will find no traces for it and report nothing, which looks like a healthy deploy. Before changing how the name is derived, compare it to what the collector and instrumented applications actually emit, not what you remember.

**Namespaces and clusters.** Check whether the watcher is scoped to some namespaces or all of them, and whether more than one cluster feeds the same store. A marker without a cluster or environment qualifier will be applied to traces from the wrong place. If a staging rollout and a production rollout of a service look the same to the watcher, the summaries get mixed.

**Resource limits and backpressure.** Watching many objects can be memory hungry, especially if you cache full objects instead of the fields you need. A watcher that is killed for memory restarts and replays, which brings us back to duplicates. Keep what you hold small, and keep list calls paginated or otherwise bounded.

**Probes and shutdown.** Readiness should mean the watcher has reconciled and can accept new events, not merely that the process is up. On shutdown, flush or abandon in-flight writes deliberately. Half-written state is what makes the next start confusing.

**Configuration changes roll the pods.** If a config change restarts the watcher, that restart overlaps with deploys by definition, since people deploy all day. Prefer reload without a restart for anything that is tuned often, and test the restart path anyway.

The Java side is the usual set of traps: a client library upgrade can change reconnect and resume semantics without any change to your code. Read the release notes for the watch client when bumping it, and rerun the reconnect tests, not just the unit tests.

## ClickHouse writes and schema

Markers land in ClickHouse next to the trace data, and the engine's habits affect correctness.

- Inserts are not updates. If the table engine deduplicates, it does so eventually and by its own rules, so a query run right after a duplicate write can still see both rows. Summaries that count markers, or take the latest one, need to be written to tolerate that, or to read in a way that forces the deduplicated view. Check which one the existing queries do before assuming.
- Ordering key and partitioning choices determine what is cheap to look up. Summaries look up by service and time range. A schema change that makes that lookup scan more will not fail; it will make the post-deploy report slow at the moment people want it.
- Adding a column is easy. Changing the meaning of an existing column is not, because old rows keep the old meaning. If the meaning has to change, add a new column or a new table and migrate readers first. Never reuse a column name for a new meaning.
- Nullable and default behavior differ between old and new rows after a column is added. Queries that filter on the new column will treat old rows as having the default, which may or may not be what you intended.
- Time column types carry a precision and a time zone setting. Check both when you add or alter anything time-related, and match what the trace tables use, so that joins and range comparisons line up without conversion.
- Batching inserts is good for the database but delays visibility. If you tune batch size or flush interval in the watcher, remember that a marker sitting in a buffer is a marker the summary cannot see yet.
- Failed writes need a policy: retry with the same key, give up loudly, or spool locally. Silent drop is the one choice that is never right here. A missing marker means a deploy that never got a report.

A small illustration of the shape to keep in mind when reasoning about a marker. It is a sketch, not the real definition:

```
component: deploy-marker-watcher
sink: ClickHouse
identity: service name as OpenTelemetry reports it
when: the cluster's own rollout time
```

If a change makes any of those four lines untrue, that is the change to slow down on.

## What readers see, and how to check a change

SREs look at this through Grafana panels and the generated summaries. They do not read the watcher's logs. So bugs show up as odd-looking charts, and people tend to blame the service that was deployed rather than the marker.

**Annotations on dashboards.** Panels that draw deploy lines read markers directly. A duplicate marker draws two lines close together, and a missing one draws nothing. Both are easy to mistake for real events. After a change, look at a few services' panels across a period that includes several known deploys and compare the lines to what you know happened.

**Summaries.** Compare a summary generated with the old watcher behavior against one generated with the new behavior for the same past deploy, if the data still exists. A differing boundary means differing numbers in the report, even when nothing about the service changed. Say so in the change description, so nobody files it as a latency regression.

**Backfills.** If you change how boundaries are derived, historical rows stay as they were. Decide whether to leave history alone, which is the safe default, or to rewrite it, which changes past reports. Never do the second casually.

**Relation to the collector.** Whether the after-window has data at all depends on the span path accepting what instrumented services send during and just after a rollout, including revised spans. That is a separate piece but it interacts with the watcher's timing; see [[span-collector-must-accept-revised]]. If the watcher change shortens the delay between marker and summary, re-read that note first.

**Testing that is worth the time.**

- Replay a recorded sequence of cluster events through the watcher, including a reconnect in the middle, and assert on the exact set of markers produced.
- Run the same sequence twice and assert that the second pass adds nothing.
- Include a rollback, a redeploy with only configuration changed, a stuck rollout and a release that touches many services together.
- Test with a service whose workload has no usable label, and see that the failure is visible and not a silent skip.
- Kill the watcher between receiving an event and writing the marker, and check what the restart produces.
- Test against a real ClickHouse instance at least once per change to the write path. Mocks will not show you deduplication timing or type precision.

**Before merging, ask:**

- Does the boundary move for any existing case? If so, who has to be told?
- Can the same rollout now produce a different key than before?
- Does the service name still match what the spans carry?
- What happens on restart, and with two instances running at once?
- Is a failure loud, or does it just produce an empty chart?

If the answer to the last one is "empty chart", add the alarm or the log line before shipping the change. An empty chart is the failure mode that costs the most trust, because people will assume the deploy was fine.
