---
id: 01KMS2GKSZ9WK9RT9W13T2YFX3
created: 2026-03-27T22:56-03:00
sources:
  - "code: detector/src/main/java/io/tracequill/detector/CompareJob.java"
---

# regression-detector design

regression-detector is the part of TraceQuill that decides whether a deploy made a service slower. It runs as a Kubernetes CronJob with schedule `*/5 * * * *`. Each run looks for deploy markers and compares the p95 latency of the 30 minutes before and after each deploy marker. This note records how it is built and why, so a later session does not have to rebuild the reasoning.

## Purpose

Site reliability engineers want to know, soon after a deploy, whether latency got worse. Reading dashboards by hand after every release does not scale. regression-detector does the comparison automatically and writes a summary that Grafana can show next to the deploy.

## Where it runs

It is a Kubernetes CronJob, not a long-running service. Each run starts a Java process, does its work and exits. The schedule is `*/5 * * * *`, so a run starts every five minutes. There is no state kept in the pod. Anything that must survive between runs lives in ClickHouse.

## Inputs

Two inputs matter. The first is the trace data, which the OpenTelemetry collectors write into ClickHouse. The second is the set of deploy markers, which record when a service version changed. The detector reads both from ClickHouse and never talks to the collectors directly.

## The comparison

For each deploy marker the detector takes two windows. One is the 30 minutes before the marker and the other is the 30 minutes after it. It computes the p95 latency of each window and compares them. A clear rise in the after window is a candidate regression.

## Why p95

The mean hides tail problems, and p99 is too noisy on services with low traffic. p95 was picked as a middle point. It reacts to real slowdowns and stays reasonably stable. If a team needs another percentile, that is a change to the design, not a flag.

## Why 30 minutes

A shorter window gives too few spans on quiet services. A longer window mixes in daily traffic shifts, which look like regressions. Thirty minutes on each side was the compromise. The same length is used before and after so the two samples are comparable.

## Waiting for the after window

A marker cannot be judged until the after window is complete. Because the job runs often, it simply skips markers that are not ready and picks them up in a later run. Nothing is lost by skipping.

## Avoiding duplicate work

Since runs overlap in the markers they can see, the detector must not report the same deploy twice. It records a result per marker in ClickHouse and checks that record before computing. A marker with a result is ignored from then on.

## Output

The result for each marker holds the service, the two p95 values, the difference and a verdict. Grafana reads these rows directly, so no separate API exists. Dashboards show the verdict beside the deploy annotation.

## Example schedule

The CronJob spec carries the schedule like this:

```yaml
kind: CronJob
spec:
  schedule: "*/5 * * * *"
```

## Failure behaviour

If a run fails, the next one starts five minutes later and retries the same markers. Because results are written only after a marker is fully computed, a crash mid-run leaves no half result. Overlapping runs should be prevented in the CronJob concurrency setting so two pods do not race on one marker.

## Known limits

Two deploys close together share a window, so each comparison is polluted by the other. The detector does not try to separate them. Services with very few requests give unreliable p95 values. Traffic mix changes unrelated to the deploy can still trigger a false verdict.

## Things to watch

Watch the age of the oldest unjudged marker. If it keeps growing, the job is failing or ClickHouse queries are too slow for the schedule. Also watch the query time of a run against the five-minute interval.

## Alternatives rejected

A streaming detector was considered. It would react faster, but it needs state, scaling and recovery logic. A CronJob is simpler to run on Kubernetes and the five-minute delay is acceptable to the SREs. Statistical tests on full distributions were also left out for now, to keep verdicts easy to explain.

## Open questions

Should the verdict use a threshold on the difference, or a significance test? How should overlapping deploys be handled? These are not settled and any change should update this note.
