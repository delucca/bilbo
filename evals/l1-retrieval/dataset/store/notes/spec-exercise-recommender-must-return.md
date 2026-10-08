---
id: 01KQCFSGMT2G2WHTH3JVDTNBTE
created: 2026-04-29T08:25-03:00
---

# exercise-recommender spec

This note is the spec for exercise-recommender, the part of ClassroomCompass that suggests the next exercises for a student. The single hard requirement written down so far is about speed: exercise-recommender must return a recommendation for one student within 400 ms. Everything else here is a working description of how the component is expected to behave so that this budget can be met. Where a detail is not settled, the note says so instead of guessing. Details like exact field names, route names and sizes are left out on purpose, because they live in the code and would go stale here.

The people who use the result are secondary school teachers. They open a student or a class view and expect suggestions to appear as the page loads, not after a spinner. That expectation is where the time limit comes from.

## Purpose

ClassroomCompass tracks how each student is doing against curriculum standards. A standard is a statement of what a student should be able to do, and the progress data says how close the student is to meeting it. exercise-recommender takes that progress picture and answers a narrow question: given this student, which exercises should the teacher consider giving next?

The component does not assign work. It suggests. A teacher can ignore, reorder or accept what it returns. This matters for the design because it means a slightly weaker suggestion returned quickly is better than a perfect one returned late. The teacher is the final filter, and a fast, reasonable list lets them stay in flow.

The component also does not own the curriculum or the exercise catalogue. Those are maintained elsewhere in the Django application. exercise-recommender reads them and combines them with progress data. It should never be the place where a standard or an exercise is edited.

A good suggestion has three properties. It targets a standard the student has not yet secured. It is at a difficulty the student can plausibly handle, neither trivial nor out of reach. And it is not a repeat of something the student just did. The rest of this spec is about delivering those three properties inside the time budget.

## Latency requirement

The requirement is simple to state: exercise-recommender must return a recommendation for one student within 400 ms. The figure is `400 ms`. It applies to one student per request. It is not a budget for a whole class, and it is not an average that can be hidden by a few slow cases.

A reader who only remembers one thing from this note should remember that the limit is `400 ms` per student. If someone asks how long a single recommendation may take, the answer is `400 ms`.

A short config-style sketch of how the budget might be written down in settings, using only the value from this spec:

```
RECOMMENDER_LATENCY_BUDGET = "400 ms"
```

The sketch is illustrative. What matters is that the budget lives in one named place, so that the API layer, the fallback logic and the tests all read the same value instead of each carrying their own copy.

### What counts as the request

The clock starts when the Django view that serves the recommendation begins handling the request for a student. It stops when the response body is ready to be sent. Network time between the browser and the server is outside the budget, because the server cannot control it. Rendering in the Vue.js client is also outside the budget.

The budget covers everything the server does in between. That includes reading progress data, querying Elasticsearch for candidate exercises, running the scikit-learn model, applying filters, and building the response. If any of these steps is slow, it eats the same shared allowance. There is no separate allowance per step in this spec, though a team may later choose to split it for monitoring.

## Inputs

exercise-recommender needs a few kinds of input for a single student.

The first is the student's progress against standards. This is the main signal. It says, for each standard that applies to the student's course, how well the student is doing. It comes from the progress tracking part of the Django application and is read from the database or from a cached copy of it.

The second is the student's recent exercise history. This is used to avoid repeats and to see what the student has just worked on. It only needs recent items, not the entire history, and the lookup should be bounded so it cannot grow with the age of the account.

The third is the exercise catalogue, indexed in Elasticsearch. Each exercise is tagged with the standards it addresses and with a difficulty indication. The recommender does not read the catalogue row by row from the main database during a request. It asks Elasticsearch.

The fourth is optional context from the teacher, such as which standards they want to focus on for the class. When present, it narrows the search. When absent, the recommender works from the student's gaps alone.

All inputs must be available without a slow external call. If an input could only be obtained by a slow call, it should be prepared ahead of time by a background job and read from a store at request time.

## Outputs

The output is an ordered list of suggested exercises for the student, best first. Each item carries enough information for the teacher to decide quickly: which exercise it is, which standard it addresses, and a short reason for the suggestion in plain language. The reason is important. Teachers are more willing to trust a suggestion when they can see why it appeared, for example that it targets a standard the student has not yet secured.

The list is meant to be short. A long list is slower to build and harder to read. The exact length is a product choice and is not fixed in this spec.

The response should also say whether it is a full recommendation or a degraded one. If the system fell back to a simpler method to stay inside `400 ms`, the client should be able to tell, so that it can show the result without implying more precision than it has. See the section on fallbacks.

The output is deterministic enough to test. Given the same inputs and the same model version, the same list should come back. Randomness, if used for variety, must be seedable so tests can pin it.

## Candidate generation with Elasticsearch

The recommender works in two stages. The first stage narrows the whole catalogue to a manageable set of candidates. The second stage ranks them. This split exists because the ranking model is too expensive to run over everything within `400 ms`, while Elasticsearch is good at fast filtering.

Candidate generation builds a query from the student's weakest standards. It asks for exercises tagged with those standards, within a difficulty window that fits the student's current level, and excludes exercises the student has done recently. Elasticsearch returns a bounded number of hits ordered by its own relevance, and those hits go to the ranker.

Two things keep this stage fast. The query must use filters on indexed fields rather than free text scoring wherever possible, since filters are cheap and cacheable. And the number of candidates requested must be capped, so the ranker's workload is predictable.

If Elasticsearch is slow or unreachable, candidate generation is the first place the fallback logic has to act. There should be a short timeout on the query, comfortably below the overall budget, so a slow search cannot consume the whole allowance and leave nothing for the rest.

The index itself is kept up to date by a background process when the catalogue changes. The request path never writes to the index.

## Ranking with scikit-learn

The second stage scores the candidates with a model trained with scikit-learn. The model takes features describing the student and the exercise together, such as how far the student is from securing the standard, how the exercise difficulty compares with the student's level, and how recently the student saw something similar. It outputs a score, and the candidates are sorted by it.

The model is loaded once per worker process and kept in memory. Loading it on each request would blow the budget. Prediction is done in a batch over all candidates at once, not one candidate at a time, since batch prediction is much cheaper per item.

The model should be simple enough to score a small candidate set quickly. A heavier model that gives marginally better ordering is not worth it if it threatens the `400 ms` limit. When in doubt, pick the cheaper model and measure.

Model files are versioned. The version in use is recorded with each response or in logs so that a surprising suggestion can be traced back to the model that produced it. Training happens offline, outside the request path, and a new model is only put into service after it has been checked against the latency budget as well as for quality.

Feature preparation counts toward the budget too. Features that need heavy computation should be precomputed and stored, not derived during the request.

## Role of Celery

Celery does the slow work so the request does not have to. It is not on the request path for a single recommendation. Instead, it prepares things the request path then reads quickly.

Typical background jobs are refreshing cached progress summaries when new results arrive, keeping the Elasticsearch index in step with the catalogue, and retraining or reloading the model on a schedule. Jobs that precompute recommendations for students likely to be viewed soon, such as a class about to start a lesson, can also be run here. A precomputed result can be served at once, which is the easiest way to stay well under `400 ms`.

The tradeoff is freshness. A precomputed recommendation may be slightly out of date if the student has just finished an exercise. The design should either invalidate the stored result when new progress arrives or accept a short delay. This spec leans toward invalidating, and recomputing on demand if the stored one is missing, with the live path still held to the same budget.

Celery tasks must be safe to repeat. If a task runs twice, the outcome should be the same as running it once.

## Django API surface

Django exposes the recommender to the Vue.js client through an authenticated endpoint that returns the recommendation for one student. The view checks that the requesting teacher is allowed to see that student, then calls the recommender and returns the result.

Permission checks must be cheap. They count toward the budget, so they should rely on data already loaded for the request and not trigger a chain of database queries.

The view is thin. It validates input, calls the recommender, and shapes the response. The logic for candidates, ranking and fallbacks lives in the recommender code, not in the view, so it can be tested without going through HTTP.

Database access in this path should be reviewed for repeated queries. A common way to lose time in a Django view is to run a query per item in a loop. The recommender should fetch what it needs in a small, fixed number of queries, whatever the size of the result.

Errors return a clear failure and never an empty success. A teacher who sees nothing should be able to tell whether the student truly has no suggestions or whether something broke.

## What the Vue.js client expects

The Vue.js client asks for the recommendation for a student when a student or class view opens. It should request in the background and show the rest of the page without waiting, so that even a slow response does not block the teacher.

The client shows the ordered list with the short reason for each item. If the response is marked as degraded, the client may show a small note, or simply present the list as is, depending on what product decides. It must not treat a degraded response as an error.

For a class view that shows many students, the client should not fire a flood of requests at once. The budget is per student, but many requests in parallel can load the server and push individual requests over the limit. The client should request in a measured way, or the server should offer a batch path whose cost per student still respects the same limit.

The client should handle a timeout gracefully, by showing that suggestions are not available right now and offering to retry, rather than showing a broken panel.

## Degradation and fallbacks

The `400 ms` limit is only meaningful if there is a plan for when things run slow. The recommender should keep track of how much of the budget it has used as it goes, and choose cheaper paths when time is short.

The ordering of fallbacks, from best to cheapest, is roughly as follows. First, serve a precomputed recommendation if one exists and is still valid. Second, run the full two-stage path. Third, if candidate generation was slow, use a smaller or simpler query. Fourth, if the model cannot run in time, order the candidates by a simple rule, such as how far the student is from securing the standard, without the model. Last, if nothing at all can be produced in time, return a clear indication that suggestions are unavailable.

Each fallback is marked in the response, as noted above, and is counted in metrics, so the team can see how often the system is running degraded. A rising count is a sign the normal path is too slow or a dependency is unhealthy.

The fallbacks are meant to protect the teacher's experience. They are not a license to be slow on the normal path. The normal path should meet the budget on its own in the usual case.

## Measuring the budget

A requirement nobody measures is a wish. The team should measure the time exercise-recommender takes for a single student, from the start of handling to the response being ready, and look at the slow end of the distribution, not only the average. The requirement says a recommendation must come back within `400 ms`, so the cases that matter are the slowest ones.

There should be timing for each stage as well: progress read, candidate query, feature preparation, model scoring, and response building. With these, a slowdown can be pinned on a stage instead of guessed at.

Automated tests should include a latency check against the budget on a realistic sample, run in an environment close to production. Such checks are noisy on shared machines, so they are best treated as a guard against large regressions and not as a precise gate. Load tests with many students at once are also worth doing, since contention is where budgets tend to break.

When a change touches the model, the query, or the data it reads, it should be checked against the budget before it goes live, together with the usual quality checks.

## Open questions

Some points are not settled and should be decided before this spec is treated as final.

The first is whether the budget should be split across stages in code, with each stage given a share, or only enforced as a total. A split makes problems easier to locate but is more rigid.

The second is how fresh a precomputed recommendation has to be. If a teacher expects a suggestion to react at once to an exercise just completed, invalidation must be reliable; if a short delay is acceptable, the design can be simpler.

The third is the batch path for class views. A dedicated endpoint could serve many students more efficiently, but it needs its own statement of what latency is acceptable per student, and this spec only covers the single-student case at `400 ms`.

The fourth is how the reason text for each suggestion is produced, and whether teachers want more or less explanation than a short phrase.

The fifth is what to do for a student with very little progress data. With few signals, the ranker has little to work with, and a sensible default, such as starting from the first standards of the course, may serve better than a weak model output.

Until these are answered, treat the single hard fact in this note as the fixed point: one student, one recommendation, within `400 ms`.
