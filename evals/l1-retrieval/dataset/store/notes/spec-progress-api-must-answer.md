---
id: 01KCVK6YESEPEFME9R1RPEJDMR
created: 2025-12-19T12:21-03:00
---

# progress-api spec

This note specifies what progress-api has to do, what it has to be fast at, and where the limits of the component sit. It is written for whoever changes the service next, whether an engineer or a coding agent. It is a spec, so it says what must hold. It does not describe every line of the current code. Where something is a guess or a preference, it says so.

progress-api is the Django service that the Vue.js front end calls to show a teacher how a class and each student are doing against curriculum standards. It also hands back the suggested next exercises. It reads from the relational store, asks Elasticsearch for exercise matches, and takes model output that Celery workers have already computed with scikit-learn. It should not compute models on the request path.

## Latency requirement

The one hard number in this spec is the latency target. progress-api must answer the progress endpoint with a p95 latency below 300 ms. That is the figure to hold the service to. If a change pushes the p95 of the progress endpoint to 300 ms or above, the change is a regression, even when every functional test passes.

A few points about how to read that requirement:

- It is a percentile target. Slow outliers are tolerated, but only a small share of requests may be slow. A good median does not meet the target if the tail is bad.
- It applies to the progress endpoint, which is the one teachers open at the start of a lesson. Other endpoints in progress-api have no number written down here. Keep them reasonable, and do not assume they inherit the 300 ms target unless someone says so.
- It is measured on the server side, from when the request reaches the Django view to when the response is written. Network time to the browser and the Vue.js render time are not counted. If someone wants an end-to-end budget, that is a separate decision and belongs in its own note.
- It should hold under normal classroom load, which means many teachers opening their class views in the same few minutes at the start of a period. A quiet test environment can look fine and still miss the target at that peak. Test with concurrent requests, not one at a time.

The reason for the target is plain. Teachers open the view with a class waiting. If the page takes noticeably longer than a moment, they stop using it and go back to a paper mark book. The number was chosen to keep the page feeling immediate once the front end adds its own time on top.

## What the progress endpoint returns

The progress endpoint returns, for a class or a single student, the current standing against each curriculum standard in scope, plus a short list of suggested next exercises per standard that is not yet secure. The shape should stay stable, because the Vue.js components depend on it. Add fields rather than renaming or removing them, and tell the front end owners before any removal.

For each standard the response carries:

- the standard identifier and its human readable label, as stored in the curriculum data;
- a standing, which is a small fixed set of levels rather than a raw score, so the front end can colour it without knowing the scoring rules;
- a measure of how recent the evidence is, so a teacher can tell a standard that was assessed last week from one that was assessed a term ago;
- the suggested exercises, each with an identifier, a title and the reason it was suggested.

The response must be deterministic for the same stored data. Two calls with no new evidence in between return the same answer. This matters for trust and for testing. If suggestions are randomised to add variety, the randomness has to be seeded from stored data so it does not change between calls.

A teacher may only see their own classes. The permission check happens before any expensive work. A request for a class the teacher does not teach is refused early, so it does not load progress data that will be thrown away.

## How the target is met

The latency target shapes the design more than anything else in the service. The main rules are below.

### Precompute, do not compute on request

Standing per student per standard, and the ranked suggestion candidates, are computed by Celery tasks and stored. The view reads stored results. Celery runs the scikit-learn work when new evidence arrives and on a periodic refresh. If a request needs a value that has not been computed yet, the endpoint returns what it has and marks the missing part as pending. It does not start the computation inline and wait for it. A slow model fit inside a web request is the quickest way to miss the target.

### Keep database access small and predictable

The view should issue a bounded number of queries that does not grow with class size. In Django terms, use select_related and prefetch_related or explicit aggregate queries, and avoid per-student queries inside a loop. Add a test that counts queries for a small class and a large class and checks they match. A rise in query count with class size is a defect, even if the page still loads quickly on a developer machine with little data.

Anything returned as a list should be paginated or capped where a teacher could not meaningfully read the whole thing anyway. Do not return history that the page does not show.

### Elasticsearch use

Elasticsearch is used to find exercises that match a standard and a level. Calls to it from the request path need a short timeout. If Elasticsearch is slow or down, progress-api should still return the standings, with the suggestions section empty and flagged as unavailable, rather than letting the whole endpoint wait. Suggestion candidates that were already stored by a Celery task can be served as a fallback. A degraded answer that arrives in time is better than a complete one that arrives late.

Queries should be built from stored identifiers and filters. Free text search over exercise bodies is not something the progress endpoint should do on demand.

### Caching

Short lived caching of the assembled response per class is allowed and probably useful, since the same class view is opened many times in a period. The rules are: the cache key includes everything that changes the answer (class, the teacher's view options, and a version that moves when new evidence is stored), and a cached answer must never be shown to a user who is not allowed to see it. Invalidation on new evidence matters more than the cache lifetime. If you cannot invalidate reliably, use a very short lifetime instead of a long one that shows stale standings after a teacher has just recorded a result.

## Measuring and guarding the target

The target means nothing without a measurement. What is needed:

- The service records request duration for the progress endpoint as a histogram, so a percentile can be read from it rather than guessed from an average. Averages hide exactly the problem the target is about.
- A load test, run before a release that touches the progress endpoint, drives concurrent requests against a realistic data set, with several classes of different sizes, and reports the p95. The release is blocked if it is not below 300 ms.
- Alerting in production fires when the observed p95 for the progress endpoint stays at or above the target for a sustained period, not on a single slow minute.

When the target is missed, look in this order: query count and slow queries, then Elasticsearch timeouts or slow calls, then serialisation cost of a large response, then cache misses after an invalidation. Model computation should not be on the list at all, because it is not supposed to be on the request path. If it shows up in a profile, that is the bug.

## Out of scope and open questions

This spec does not cover how the Celery tasks decide a standing, how scikit-learn models are trained or validated, or how exercises are indexed. Those belong to other components and should get their own notes. It also does not cover the front end behaviour while a response is pending.

Open questions, none of them settled:

- Whether the other endpoints in progress-api should get written latency targets, and if so what they are.
- Whether the target should be restated end to end once real browser timings are collected.
- How stale a stored standing may become before the response should flag it to the teacher more loudly than a recency measure does.
- Whether very large classes, such as a whole year group viewed at once, are supported on the progress endpoint or must be split by the front end.

Until someone settles these, treat the progress endpoint latency target as the only fixed performance requirement, and do not trade it away for a feature without writing the trade down here.
