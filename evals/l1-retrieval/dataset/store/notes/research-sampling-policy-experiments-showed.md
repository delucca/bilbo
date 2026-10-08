---
id: 01KXVM3HFHMT6RJ5JK1K5RD1Y0
created: 2026-07-18T19:03-03:00
---

# sampling-policy-lib: latency threshold experiments

Short version: in experiments, always keeping traces slower than `2000 ms` in sampling-policy-lib preserved `97 percent` of regressed requests. This note records what that means, how far to trust it, and what is still open. It is research, not a decision; nothing here changes the default policy by itself.

## Question

TraceQuill summarizes latency regressions after each deploy. The summary is only as good as the traces we keep. Storing every trace in ClickHouse is too expensive, so sampling-policy-lib has to drop most of them. The question was whether a simple rule, keep every slow trace and sample the rest, keeps enough of the traces that matter to SREs reading the post-deploy report.

## Finding

Always keeping traces slower than `2000 ms` preserved `97 percent` of regressed requests. A regressed request here means a request whose latency got worse after a deploy and that the regression summary would want as evidence. So a fixed slow-trace cutoff, applied before any probabilistic sampling, covers almost all of them.

The remaining 3 percent are regressions that stayed under the cutoff. They got slower but never crossed the line, so a pure threshold rule never keeps them on purpose.

## Setup

The experiments replayed trace data through the sampling logic in sampling-policy-lib and compared the kept set against the full set. The full set was the reference for what counts as regressed. Traces came from OpenTelemetry instrumented Java services running on Kubernetes, stored in ClickHouse for the comparison.

Details of the exact replay window are not recorded here. Treat the number as coming from our own services, not as a general law.

## How the rule works

The policy checks the trace duration once the trace is complete. If the duration is above the cutoff, the trace is kept without asking the sampler. If not, it goes to the normal sampling path, which keeps a small share at random.

Because the decision needs the whole trace, it belongs at the tail, in the collector stage, not at the head in the SDK. Head sampling cannot know a trace will be slow.

```text
keep if duration > 2000 ms
else  hand to the probabilistic sampler
```

## Why the threshold works

Regressions that matter to SREs tend to show up as requests in the slow tail. A deploy that makes a path worse pushes many of its requests over the line, and once enough cross it, they are all kept. That is likely why coverage is so high even though the rule is crude.

## Limits

- The figure is for the cutoff tested. Other cutoffs were not tuned in this note.
- Services with a normal latency far below the cutoff can regress badly and never trip the rule. Those are the likely home of the missed 3 percent.
- Services whose normal latency is already near or above the cutoff would have most traces kept, which raises storage cost.
- The result depends on how "regressed" was defined in the replay. A stricter or looser definition would move the number.

## Cost side

Keeping every slow trace adds volume in proportion to how many slow requests there are. During an incident or a bad deploy, slow traces spike, and so does what we store. I did not measure this peak. It needs a look at ClickHouse ingest and disk before the rule becomes the default.

## What to try next

1. Per-service cutoffs set from each service's own latency distribution, to catch regressions that stay under a global line.
2. A relative rule: keep traces that are slow compared with that route's baseline before the deploy.
3. A cap on slow-trace keeps per time window, so a bad deploy cannot flood storage.
4. Compare the missed requests across services to see whether they cluster.

## Open questions

- How does coverage change when the cutoff is moved up or down?
- Does the share held depend on deploy size, or on which service was changed?
- Can Grafana panels show the kept versus dropped ratio, so SREs can see when coverage drops?

## Pointers

The sampling logic lives in sampling-policy-lib. Any change to the cutoff should be tested by replay again before rollout, using the same comparison against the full set. Record new results here, or in a new research note if the subject differs.
