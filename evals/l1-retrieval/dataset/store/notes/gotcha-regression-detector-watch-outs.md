---
id: 01JZNZ948982CY2QHZ5HZWEH8Z
created: 2025-07-08T18:32-03:00
---

# regression-detector: general things to watch out for when changing it

Notes for anyone touching regression-detector. Nothing here is a decision or a spec. It is a list of places where changes tend to go wrong, written from the shape of the component and not from one incident. Read it before you start, and add to it when you get bitten by something new.

The component takes traces that were collected through OpenTelemetry and stored in ClickHouse, compares latency after a deploy against some notion of normal, and tells site reliability engineers whether something got slower. Grafana shows the result. It runs in Kubernetes next to the deploy tooling. Every one of those edges can break quietly, and the output is a judgment, so a bug usually looks like a plausible answer and not like a crash. That is the main thing to keep in mind.

## Wrong answers look like right answers

When regression-detector breaks loudly, you notice. The dangerous failures are the ones where it keeps producing a summary that reads fine. A baseline that is too short, a window that is off by one bucket, a filter that drops a whole service: the report still renders, the panel still has numbers, and nobody questions it until an incident review.

So when you change anything, ask what the output would look like if your change were subtly wrong. If the answer is "the same as now, just a bit off," you need a check that can tell the difference. Compare before and after on the same stored traces, not on live traffic, because live traffic moves under you and hides the effect of your edit.

Do not trust a green run as proof. Check that the run actually looked at data. An empty input and a healthy input can produce the same "no regression" verdict, and that is the worst case, because it is a false all clear.

## Empty and thin data

Low-traffic services, new services, and services that were just renamed all have little data in the comparison windows. Any statistic computed on a handful of spans is noise. Make sure a change does not lower the bar for how much data is needed before a verdict is given, and does not raise it so far that quiet services never get a verdict at all.

Decide what the component says when it does not know. "Unknown" and "no regression" must stay different outputs all the way through storage, the Grafana panels, and anything that pages people. Several past edits in components like this one collapsed them into one value by accident, usually through a default in a query or a null that got coalesced to zero.

Watch for division by zero and for percentiles of empty sets. ClickHouse returns special values for some of these cases and does not raise an error, so the bad value flows downstream as if it were data.

## Baseline selection

What counts as the baseline is the most sensitive part of the component. Changing which period, which deploys, or which traffic is used as the reference changes every verdict afterwards. Be careful with anything that reads "the previous version" or "the previous window," because both can be ambiguous when deploys overlap, when a rollback happens, or when two services deploy close together.

A baseline that already contains a regression teaches the detector that slow is normal. If you touch how baselines are refreshed, think about what happens after a bad deploy that is not rolled back for a while. Does the baseline drift toward the slow state? Does the next deploy then look fine by comparison?

Rollbacks are a special case. A rollback to an older version is a deploy, and the detector should still compare sensibly. Do not assume version order matches time order.

## Time windows and clocks

Windows are defined in time, but the time on a span is the time the producing process believed it was. Clock skew between nodes, delayed export, and batching in the collector all move spans across window edges. A change that tightens a window may start dropping late spans that used to count. A change that widens it may pull in the previous deploy's behavior.

Be explicit about which timestamp is used: span start, span end, ingestion time, or deploy time. They are not interchangeable, and mixing them in one query is an easy way to compare things that are not comparable. Time zones and daylight shifts should not matter if everything is in one zone, so keep it that way and do not introduce local time anywhere.

Also remember that a deploy is a process, not a moment. Pods are replaced gradually. The period where old and new versions both serve traffic is mixed, and the detector has to decide what to do with it. Do not change that handling without reading how it is done today.

## Late and out-of-order data

Traces arrive late. A trace is only complete when its slowest span arrives, and the slow spans are often the ones that arrive last, or never. If the detector runs too early, it sees a sample biased toward fast traces and under-reports regressions. If a change moves the run time earlier, or removes a wait, this bias gets worse and the effect looks like an improvement.

Likewise, if you make the detector re-run on data that has since been completed, results may change after they were already shown. Decide whether a past verdict is allowed to change, and check that anyone reading it will not be confused. Idempotence matters here: running the same window twice should give the same answer once the data is final.

## ClickHouse queries

Most of the cost and many of the surprises live in the queries. Keep these in mind when editing them.

Sampling and approximation. Some aggregate functions are approximate, and their error is not uniform across data sizes. Switching between exact and approximate functions can shift results enough to create or hide a regression. Treat that switch as a behavior change, not an optimization.

Merge behavior. Tables that merge in the background can show duplicates or not-yet-collapsed rows if you read them in the wrong way. If a table relies on a merge to deduplicate, a query that ignores that can double count. Check how the table engine behaves before you assume a row is a row.

Ordering and partitions. The key order and partitioning of the trace tables decide whether a query is cheap or scans everything. A filter added in the wrong column can turn a fast query into a heavy one that competes with ingestion. Test new queries against realistic volume, not against a small dev dataset where everything is fast.

Schema changes. Adding a column to a trace table is usually safe. Renaming or changing the type of one is not, because older rows, older readers, and dashboards may still expect the old shape. Roll changes in an order where both old and new code work during the transition.

## Percentiles, means, and what is being compared

A latency distribution has a long tail, and the choice of statistic decides what the detector can see. A mean hides tail regressions. A high percentile is noisy on small samples. Changing the statistic is changing the definition of a regression. If you do it, say so loudly in the commit and in the dashboard text, because people have learned to read the old numbers.

Do not average percentiles across groups. It looks reasonable and is mathematically wrong. If you need a percentile across several services or instances, compute it from the combined data, or from merged sketches, not from per-group results.

Be careful with relative versus absolute change. A small absolute shift on a fast operation is a large relative change, and a large absolute shift on a slow operation is a small one. Whatever the current rule is, a modification that favors one will flood or starve the other. Check both ends.

## Thresholds and sensitivity

Thresholds are the knobs people want to change first and understand last. Lowering them produces more alerts, and the engineers who receive them will start ignoring the component. Raising them hides real regressions. Neither shows up in tests that only check that the code runs.

Prefer changes that can be evaluated against past traces where the outcome is known. If there is no labeled history, at least replay a set of past deploys and look at what would have been flagged, with a person reading the list. Count false positives and false negatives separately; one number for "accuracy" hides the trade.

Thresholds that differ per service or per operation need an owner. If you add a new override mechanism, think about how stale overrides get found and removed. Old overrides sitting in configuration are a common source of silent blind spots.

## Grouping, cardinality, and attribute handling

The detector groups spans by things like service, operation, and sometimes attributes. Cardinality is the trap. An attribute with unbounded values, such as an identifier or a raw path with parameters, makes each group tiny and the number of groups huge. Memory use goes up, queries slow down, and every group has too little data for a verdict.

If you add a grouping dimension, check how many distinct values it takes in production and whether it is stable over time. If you change how operation names are normalized, you split or merge groups, and history for those groups no longer lines up with the new names. Old baselines then belong to names that no longer exist, and new names have no baseline.

Instrumentation changes upstream also rename things. A team updating its OpenTelemetry library can change span names or attribute keys without telling anyone. The detector then sees a disappearance and a new arrival, not a rename. Handle that gracefully, and do not treat a vanished operation as an improvement.

## Sampling upstream

Traces are often sampled before they reach storage. If the sampling rate differs between the baseline period and the comparison period, counts and even latency distributions can differ for reasons unrelated to the deploy. Tail-based sampling that keeps slow traces more often makes things look slower than they are. Head-based sampling changes volume without biasing latency, in the simple case, but not once rates vary per service.

When you change anything that touches counts, such as throughput comparisons or minimum sample checks, find out whether sampling weights are applied. If they are carried on the data, use them. If they are not, say so in the code where a reader will see it.

Collector configuration is outside this component, but it changes what the component sees. Keep in touch with whoever owns it, and expect that a collector change can look like a regression in this component.

## Deploy events and correlation

The detector has to know when a deploy happened and which version is which. That information comes from outside the trace data, usually from the cluster or the deploy pipeline. If that feed is late, wrong, or missing, the detector compares the wrong periods and produces a confident wrong answer.

Be careful with changes to how deploy markers are read. Check what happens with a deploy that failed halfway, a deploy that was paused, a deploy of a config only, and several services deploying together. In the last case, a regression in one service shows up as slowness in its callers, and the detector may blame the wrong one. Attribution through the call graph is hard, and any simplification there should be stated plainly in the output so readers do not over-trust it.

Do not assume one deploy per service per day, or any fixed rhythm. Frequent deploys leave short windows; rare ones leave long, drifting ones.

## Kubernetes behavior

The component runs in a cluster, so its own lifecycle matters. Restarts, rescheduling, and rolling updates of the detector itself can interrupt a run in the middle. Make sure a half-finished run does not leave partial results that look complete. Prefer writes that become visible all at once, or that are clearly marked as in progress.

If more than one replica can run, think about duplicate work and duplicate output. Two copies computing the same window and both writing results is a classic way to get doubled rows. Leader election, locking, or idempotent writes should be checked after any change to scheduling or scaling.

Resource limits matter. A query result that grows with data volume can push memory past a limit and get the process killed without a clear message. Changes that hold more in memory, such as larger batches or wider groupings, need a look at limits and at how the process behaves under pressure. Also watch timeouts on probes, because a long computation can make the pod look unhealthy and get it restarted in a loop.

## Java code habits

The detector is Java, and some of the usual Java problems are worth naming here. Be careful with floating point when comparing results; do not compare for equality, and watch for accumulated error when summing many small values. Use the right time types and avoid mixing instants with local date-times. Watch integer overflow on counts and on durations expressed in small units, since they can wrap without complaint.

Concurrency: shared mutable state in the statistics code is a risk if anyone parallelizes it. If you introduce parallel processing across services, verify that results do not depend on completion order. Collections that look unordered can leak ordering into output, making results differ between runs and making diffs in tests noisy.

Dependency upgrades, particularly for the ClickHouse driver and the OpenTelemetry libraries, can change defaults: time zone handling, null handling, how large numbers map to types, and batch behavior. Read release notes for behavior changes, not just for new features, and run a comparison of old and new outputs on the same stored data.

## Grafana dashboards and consumers

The people who use this component mostly see it through Grafana. A dashboard encodes assumptions about field names, units, and meaning of values. Renaming a field, changing a unit, or changing a status vocabulary breaks panels without any error in the detector. Panels often just go blank or, worse, keep showing the last value they had.

Keep the output contract stable, and when you must change it, change it in steps: add the new, keep the old for a while, migrate the dashboards, then remove the old. Check alert rules that are defined in Grafana too, since they query the same output and may treat a missing value as fine.

Units deserve a special mention. Mixing milliseconds and microseconds, or seconds and milliseconds, between the detector and the display is a recurring mistake, and the resulting chart often looks plausible because the shapes are the same.

## Testing it properly

Unit tests with tiny hand-made inputs are necessary and not enough. They do not cover window edges, skew, sampling, or real distributions. Keep a set of recorded or synthetic trace sets that include known regressions, known non-regressions, noisy services, quiet services, rollbacks, and overlapping deploys, and run changes against them.

Synthetic data should look like production in the ways that matter: heavy tails, bursts, daily rhythm, and a mix of fast and slow operations. A normal distribution with neat parameters will pass things that production data will not.

Also test the negative paths: missing deploy info, failed database calls, partial results, and timeouts. Check that each ends in an honest "could not tell," not in silence or in a made-up verdict.

## Rollout of changes

Because a wrong verdict is hard to see, roll changes out so that old and new logic can be compared. Running both side by side on the same data for a while, and reviewing where they disagree, catches most problems. Disagreements are the useful signal; read every kind of them, not just the count.

Do not combine a logic change with a refactor, a dependency bump, or a schema change in one release. If the output moves, you will not know which one caused it. Keep each change small enough that a shift in verdicts has one obvious suspect.

Tell the engineers who use it when behavior changes. They have calibrated their trust in this component over time, and a silent shift in sensitivity costs that trust. A short heads-up that says what changed and what they might notice is enough.

## Things to leave a trail for

Leave notes where the next person will look: in code comments next to the surprising line, in the commit message, and in this note. When you make a tradeoff, write the reason down, because the reason is what lets someone later decide whether it still applies. When you find a new trap, add a section here, in plain words, and keep it general enough to stay true after the code moves on.

If something looks odd and you do not understand why it was done that way, find out before removing it. A lot of the odd-looking code in regression-detector exists because of a past false alarm or a past miss, and the history is not always in the file.
