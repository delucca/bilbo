---
id: 01JS2R949J41VNCS7Y7V6MBF6X
created: 2025-04-17T17:50-03:00
sources:
  - "doc: detector statistics notebook"
---

# regression-detector: choice of statistical test

This note records why regression-detector compares post-deploy latency with a rank-based test and not a t-test. It also records the internal codename, since the codename shows up in dashboards, old threads and branch names. The short version: on replayed history the Mann-Whitney U test raised fewer false alarms than the t-test, so it is the better default.

## Naming

The internal codename of regression-detector is driftfinder. Anyone who searches old tickets, Grafana folders or chat for driftfinder is looking at regression-detector. In this note and in new docs, use regression-detector. The codename is kept only so older material can be matched to the component.

## What the component does

regression-detector runs after each deploy. It takes latency samples from traces collected through OpenTelemetry and stored in ClickHouse. It compares the window before the deploy with the window after it, per service and per operation. When the difference looks real, it reports a latency regression to the site reliability engineers, who see it in Grafana.

## The question

Which test should decide whether a latency shift after a deploy is real? Two candidates were tried: a Welch-style t-test on the means, and the Mann-Whitney U test on ranks. The cost that matters is false positives, because each one pages or annoys an SRE for nothing.

## Why latency data is awkward

Latency is not normally distributed. It has a long right tail, a few huge outliers, and often more than one mode, for example cache hit versus cache miss. A t-test assumes roughly normal means and is pulled around by single outliers. A rank test only cares about ordering, so one very slow request counts as one rank, not as a large number.

## Method of the experiment

We replayed 40 past deploys through both tests. For each deploy we already knew from human review whether a real regression had happened. We fed the same before and after windows to each test with the same significance level, then counted how many alarms fired on deploys that had no real regression.

## Result

Across the 40 past deploys, the t-test gave 3 false positives. The Mann-Whitney U test gave only 1. The same replay set and the same significance level were used for both, so the difference comes from the test, not from tuning.

## Reading the result

Three versus one is a small count. It points in the expected direction, and it fits what the data shape predicts, but it is not strong proof on its own. Treat it as supporting evidence for a choice that was already reasonable on theory.

## Where the t-test went wrong

The false positives from the t-test mostly lined up with deploys where a handful of extreme slow requests landed in the after window. Those moved the mean enough to cross the threshold, while the bulk of requests did not change. The rank test ignored the extremes.

## Where Mann-Whitney U still failed

The one false positive from the rank test came from a window where traffic mix changed, so the after window really did contain a different population of requests. No test on raw samples fixes that. It needs the traffic mix handled before the comparison.

## What the rank test does not tell you

Mann-Whitney U says whether one sample tends to be larger than the other. It does not say by how many milliseconds. For reports we still need an effect size, so the summary shows a shift in a chosen percentile next to the test result. Do not present the test statistic alone as the size of the regression.

## Limits of the replay

The 40 deploys came from a limited period and a limited set of services. Some services had little traffic, so their windows were small. The replay also reused human labels, and those labels could be wrong in borderline cases. Results may shift with other services or other seasons.

## Decision implied

Use Mann-Whitney U as the default test in regression-detector. Keep the t-test code path only for comparison runs and for debugging, not for alerting. If someone proposes switching back, they should bring a replay that beats the numbers above.

## Open questions

- Does the rank test lose power when the regression affects only the far tail, such as the slowest few percent?
- Should small windows use a minimum sample count before any test runs?
- Is a permutation test worth trying on the same replay set?
- How should paired comparison across canary and stable pods be handled?

## Follow-ups

Extend the replay set with newer deploys and rerun both tests. Add a check for traffic mix change so the remaining false positive type is caught before testing. Keep the replay harness reproducible so the comparison can be rerun when the data pipeline changes.

## Where to look

Search for driftfinder in older dashboards and notes to find earlier context. The replay inputs come from the trace store in ClickHouse, and the results were compared by hand against the deploy review labels. Deployment of the component itself is on Kubernetes, which is unrelated to this question.

## Summary of the facts

Replaying 40 past deploys: t-test 3 false positives, Mann-Whitney U 1 false positive. driftfinder is the internal codename of regression-detector.
