---
id: 01M30W49X107QDZDC7Z5X5Q4HH
created: 2026-09-20T23:18-03:00
---

# progress-api latency target spec

This note replaces the earlier note "progress api must answer". The new value: the p95 latency target of progress-api is tightened to `250 ms`, which replaces the earlier 300 ms target. Anything that still quotes the older figure is out of date and should be corrected when found.

progress-api is the service in ClassroomCompass that answers questions about how a student is doing against curriculum standards, and that hands back suggested next exercises. Secondary school teachers reach it through the Vue.js front end. This spec covers what the latency target means, where it applies, and what to check when it is missed.

## Naming

The component used to be called `tracker-core`. It is called `progress-api` now. Old branches, dashboards, alert rules, runbooks and chat threads may still say `tracker-core`. Treat them as the same component. In new notes, code and tickets, write `progress-api` only. If you search for history on this service, search for both names, or you will miss older material.

## The target

The p95 latency target is `250 ms`. It is measured at the service boundary, from the moment a request reaches progress-api until the response has been fully written. It is a p95, so one request in twenty may be slower and still be within target. It is not an average, and it is not a promise about the slowest requests.

The earlier target was 300 ms. That value is retired. The new target is stricter, so some endpoints that passed before may now fail and need work.

## Scope of the target

The target applies to the read endpoints that teachers hit while they are looking at a class or a single student: progress summaries per standard, progress history for a student, and the next-exercise suggestions that are served from precomputed results. These are the calls that block a page in the interface, so they are the ones that must feel immediate.

The target does not apply to work that is queued. Anything that goes through Celery, such as recomputing a model or rebuilding a class report, is judged on its own terms and not against `250 ms`.

## What counts as one request

One request is one call from a client to progress-api that returns a response. Retries by the client count as separate requests. Requests rejected early for bad input or missing authentication are left out of the p95, because they say nothing about the real work. Requests that fail with a server error stay in, since a slow failure is still a slow answer for the teacher.

## How suggestions are served

The suggestion path must not train or fit anything while a teacher waits. The scikit-learn models are fitted offline by Celery tasks, and progress-api only reads their stored output or applies an already loaded model. If a change moves model fitting, or any heavy scoring, into the request path, it breaks the target and should be rejected in review.

## Data access

Progress records live in the Django database and are read through the ORM. Searching across standards and exercises goes through Elasticsearch. Each of these can eat the latency budget. The usual causes of a miss are an unindexed query, an N+1 pattern in a serializer, and an Elasticsearch query that asks for more than the page needs. Check these first, before looking at anything exotic.

## Caching

Caching is allowed where it keeps the p95 inside the target, provided a teacher never sees progress that is clearly stale after they have just recorded something. Anything cached must be invalidated or expire soon enough that a newly recorded result shows up on the next view. When in doubt, favour correctness of the student's progress over speed, then fix the speed some other way.

## Measuring

Measure latency in an environment with data of realistic size. A near-empty database hides the problems that matter. Compare the p95, not the mean, and look at it per endpoint, because one slow endpoint can be buried in a healthy overall figure. When reporting a result, say which endpoint, which data size, and whether the cache was warm.

## Alerts and dashboards

Alert thresholds and dashboard reference lines should be set to `250 ms`. Any rule that is still tied to the older 300 ms figure, or that is filed under `tracker-core`, needs updating. When renaming a dashboard, keep a pointer from the old name so people following old links can find it.

## When the target is missed

First find out whether the miss is steady or a spike. A steady miss points at the code or the data shape: queries, serialization, a new field. A spike points at load, a cold cache, a slow Elasticsearch node, or a busy Celery worker sharing resources. Record what was found. Do not loosen the target to make a red graph green; fix the cause or raise the problem with the team.

## Changing the target again

The target is a product decision, not a tuning knob for one pull request. If it needs to move again, write a new note that replaces this one, say what the new value is and why, and update alerts in the same change. Keep one note per subject so nobody has to guess which figure is current.

## Open points

Things still to confirm: whether every read endpoint is covered by a per-endpoint measurement, and whether any old `tracker-core` references remain in deployment config or monitoring. Fix those as they turn up. For now the single figure to remember is `250 ms` for the p95 of progress-api.
