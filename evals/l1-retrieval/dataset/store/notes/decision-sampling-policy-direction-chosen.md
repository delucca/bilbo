---
id: 01KQPEX7FP8RBW7KZZ5YCZQRTK
created: 2026-05-03T05:22-03:00
---

# sampling-policy-lib: general direction

We settled on a general direction for sampling-policy-lib and I'm writing it down before it gets lost in chat. This note is deliberately loose. It records the shape of the choice and the reasons, not tuned values. Anything numeric lives in config and in the deploy history, and it will change more often than this note should.

The short version: sampling-policy-lib stays a small Java library that decides what to keep, and it keeps decisions tied to what the deploy-regression summary needs. It is not a general trace-shaping toolkit. When a request for a new feature doesn't help us compare latency before and after a deploy, the default answer is no.

## Context

TraceQuill collects distributed traces and summarizes latency regressions after each deploy. The readers are site reliability engineers. They open a Grafana view after a rollout on Kubernetes and want to know whether something got slower, where, and for whom. Traces arrive through OpenTelemetry and land in ClickHouse, where the summaries are computed.

Sampling sits in the middle of that. If we keep too little, the summaries are noisy and a real regression hides in the gaps. If we keep too much, storage and query cost grow and the summaries get slower for the same people who need them fast right after a deploy. Before this decision, sampling behavior was spread across a few services, each with its own small rules, and nobody could say with confidence what share of a given route was being kept. That was the real trigger: not cost, but the fact that we couldn't explain our own data.

The flow, as a reminder of where the library sits:

```text
OpenTelemetry -> sampling-policy-lib -> ClickHouse -> Grafana
```

The library is linked into the Java services and into the collection path. It does not run as its own service. That was discussed and left alone for now.

## What we chose

The direction has a few parts, all general.

First, one policy surface. Services ask sampling-policy-lib for a decision and do not keep private sampling rules. If a service needs special behavior, it expresses that as input to the library (a hint, a category, a route class), not as a local override. This is the main change. It means that when someone asks what is being kept for a route, there is one place to read.

Second, decisions favor keeping the interesting traces over keeping a uniform slice. Slow and failing requests are kept at a higher rate than ordinary fast successes. Ordinary traffic is still kept at some baseline, because the regression summary needs a fair comparison population, and a sample made only of slow traces would make everything look like a regression.

Third, the policy is stable across a deploy window. We do not want the sampling rate to shift in the middle of a before-and-after comparison, because that would be confused with a latency change. Policy changes are therefore treated like code changes: reviewed, rolled out deliberately, and not tuned live during a rollout.

Fourth, the library records enough about each decision to correct for it later. Whatever rate applied to a trace travels with the trace, so the summaries in ClickHouse can weight results properly instead of treating kept traces as the whole population.

## Why this and not the alternatives

We looked at three other directions and set them aside.

### Keep per-service rules

This is the status quo. It is easy for each team and bad for everyone else. The cost shows up as inconsistent summaries: two services on the same request path could keep different parts of the same trace, and then the trace is incomplete exactly where the regression is. We decided the convenience wasn't worth that.

### Push everything into the collector

Tail-based decisions in the collector are attractive because the full trace is visible there. We did not reject this outright. We rejected making it the only mechanism. The collector is a shared piece of infrastructure, and putting all policy logic there makes policy changes depend on collector rollouts and makes it hard to test policy in isolation. The library can be tested as plain Java code. We keep the option of calling the same library from the collection path, which is why it is built to be linkable there.

### Keep everything and rely on ClickHouse

ClickHouse is good at this volume, and for a while keeping everything looked simple. But it moves cost and query time onto the people waiting on a post-deploy summary, and it makes the retention conversation harder. We prefer to make a deliberate choice about what to keep rather than defer it to storage.

## Boundaries of the library

Writing these down so scope does not creep.

sampling-policy-lib decides whether to keep a trace or span and reports the reason and the applied rate. It does not export, batch, buffer, or retry. Those belong to the OpenTelemetry pipeline. It does not know about Grafana dashboards or about how summaries are built. It also does not own storage schema in ClickHouse, though it has to agree with that schema on how the applied rate is recorded.

It should stay free of heavy dependencies. A policy library that drags in half the world becomes hard to link into every service, and that defeats the point of having one surface. Prefer the OpenTelemetry API types for the interface and keep implementation details private.

Configuration comes from the outside. The library reads policy from whatever the host service gives it and does not fetch policy over the network on its own. In Kubernetes the usual route is configuration delivered with the deployment. How exactly that is delivered is the host's concern. If we later want live policy updates, that needs its own decision, and this note does not grant it.

Behavior on failure matters. If the library cannot make a decision, for example because policy is missing or malformed, it falls back to a conservative default and says so in what it reports, rather than throwing into the request path. Tracing must never be the reason a user request fails.

## Consequences and trade-offs

What we get: one place to read and reason about sampling, summaries that can be corrected for the applied rate, and a library that can be unit tested without a cluster.

What we pay: every service has to adopt the library and drop its local rules, which is tedious work and will land unevenly. During the transition some services will still behave the old way, and the summaries need to tolerate that. Be careful when reading a regression for a route that has a mix of migrated and unmigrated services in its path. The numbers can look off for reasons unrelated to the deploy.

There is also a risk that the library becomes a bottleneck for change, since everyone depends on it. The mitigation is to keep the interface small and to treat additions to it as something that needs a reason tied to regression summaries.

Biasing toward slow and failing traces means raw counts in ClickHouse are not representative. Anyone writing a new query against the trace tables must use the recorded rate, or they will overstate error and slowness. This is the most likely way for someone to get a wrong answer, so it is worth repeating in query reviews.

## Open items

These are not decided and should not be treated as decided.

- How to handle very low-traffic routes, where any baseline may keep too few traces to compare. Probably a different treatment, but we have not chosen one.
- Whether policy should be adjustable per environment in a structured way, or stay the same everywhere to keep comparisons honest.
- Whether the collector-side use of the library becomes the main path later. The library is built so that this is possible. The choice waits until we see how the first rollout goes.
- How to expose the library's reasons to engineers in Grafana so they can see why a trace was kept. Useful, not urgent.
- A migration order for services. Start with the ones on the paths that matter most for deploy summaries, and expand from there.

If you are an agent picking this up: do not add per-service escape hatches to sampling-policy-lib, do not change the shape of the reported rate without checking the ClickHouse side, and do not tune policy during a rollout. If a change seems to need any of those, stop and ask first, and update this note if the direction itself changes.
