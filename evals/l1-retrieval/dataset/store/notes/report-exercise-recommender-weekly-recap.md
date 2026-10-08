---
id: 01KTAB5521Q8KE4PATBPNX16F0
created: 2026-06-04T19:13-03:00
---

# exercise-recommender weekly recap

Quick recap of the week on exercise-recommender. Most of the time went into the candidate retrieval path and the Celery side that refreshes recommendations after progress changes. Not everything landed. Notes below are for whoever picks it up next, including me after a weekend.

## What I worked on

The main thread was making the recommendation flow easier to reason about. Before this week the flow was split across a Django view, a Celery task and a scikit-learn scoring step, and the boundaries between them were fuzzy. A teacher opens a class, the Vue.js page asks for suggested exercises, and depending on whether a cached result exists the answer either comes straight back or triggers a recompute. I spent a good part of the week tracing which of those paths is taken in which situation, and writing down what I found so I stop re-deriving it.

Second thread: the link between curriculum standards and exercises in Elasticsearch. The exercise documents carry tags for the standards they cover, and the recommender uses those tags to build a candidate pool before scoring. I looked at how well the pool matches what a teacher would expect for a student who is behind on one standard but fine on the neighbours. In several cases the pool was too wide and pulled in exercises that were technically tagged but clearly aimed at a different level of difficulty.

Third thread, smaller: cleaning up the Celery task that recomputes suggestions for a student after new progress is recorded. It had grown a few special cases and I removed the ones that were no longer reachable.

## Retrieval and candidate pool

The candidate pool is built from an Elasticsearch query over standard tags plus a few filters on year group and subject. What I confirmed this week:

- The query itself is fine. The problem is mostly in what we feed it, not in how it is written.
- When a student has progress recorded against several standards at once, the query is built as a loose union. That is why the pool gets wide.
- Exercises with missing or partial tags still show up through the subject filter alone. They add noise and should probably be excluded, or at least ranked down, but I have not changed that yet.

I tried narrowing the pool by weighting the standard the student is weakest on, and the suggestions looked more sensible on the handful of classes I checked by eye. That was a spot check, not an evaluation. I do not want to treat it as proven. It needs a proper comparison against the current behaviour before anyone changes the default.

One thing to keep in mind: the index is shared with the search feature teachers use to browse exercises. Any change to analyzers or mappings for tags affects both. I stayed away from mapping changes for that reason and only touched query construction on the recommender side.

## Scoring with scikit-learn

The scoring step takes the candidate pool and ranks it using features derived from the student's progress history and from the exercise metadata. I read through the feature building code again and found two things worth recording.

First, some features are computed per student at scoring time, which makes the step slower than it needs to be when many students in a class are refreshed together. Moving the shared parts out of the per-student loop looks straightforward, but I have not done it.

Second, the model artifact is loaded in a way that makes it easy to end up with a stale copy in a long-running worker. After a retrain, workers that were already running keep using what they loaded at start. This is not new, but it is the kind of thing that makes a suggestion look wrong right after a retrain and then fix itself after a restart. If a teacher reports odd suggestions, check this first.

I did not retrain anything this week and did not change the features. Any change there should go with a retrain and a note about it, so the behaviour change is traceable.

## Celery and refresh behaviour

The refresh task is triggered when progress is recorded. In practice a teacher entering results for a whole class creates a burst of refresh tasks, one per student, and many of them overlap in the work they do. I looked at coalescing these so that a burst results in fewer recomputations, but only sketched it.

What I did change:

- Removed branches in the refresh task that handled an old payload shape no longer sent by anything. I checked the callers before deleting.
- Made the task log a bit more useful: it now says why it skipped a student, for example when there is no progress yet, instead of returning silently.
- Tightened the handling when Elasticsearch is slow or unavailable. The task now leaves the previous suggestions in place instead of overwriting them with an empty list. An empty list was the worst outcome for a teacher, because it looks like there is nothing left to practise.

What I did not change: retry behaviour. The current retry setup is acceptable, but I want to look at it together with the coalescing idea, since both affect how many tasks pile up during a busy period like the end of a lesson.

## Frontend notes

The Vue.js side mostly just displays what the API returns, so there was little to do there. One thing I noticed: while a recompute is in flight, the page shows the old suggestions with no hint that they may be out of date. A small indicator would help teachers understand why the list changes a moment after they entered results. I have not started on it, and it should be agreed with whoever owns the class view first.

## Open questions

- Should exercises with incomplete tags be in the candidate pool at all? My leaning is no, with a fallback only when the pool would otherwise be too small. Needs a look at how many exercises are affected before deciding.
- How do we judge whether a change to the pool or to the ranking is an improvement? Right now it is eyeballing a few classes. Even a small fixed set of example students with expectations written by a teacher would be better. Worth asking a teacher who uses the tool.
- Should the worker reload the model artifact on a signal after a retrain, or is a restart in the deploy routine enough? The second is simpler, the first is more robust.
- Coalescing refresh tasks: do it in the task (skip if a newer one is queued for the same student) or at the point where progress is recorded (debounce)? I lean toward the first because it keeps the Django side simple.

## Next week

In rough order of what I would do first:

1. Write down the three request paths (cached, recompute, degraded) in one place near the code, so the next person does not have to trace them.
2. Build the small fixed set of example students and run current behaviour against it, to have a baseline before touching the pool.
3. Try the weakest-standard weighting against that baseline and compare.
4. Look at coalescing and retries together.
5. Only after that, consider moving shared feature work out of the per-student loop.

## Risks and things to watch

The biggest risk is changing the candidate pool without a baseline. The suggestions are what teachers see and trust, and a quiet drop in quality would not show up as an error anywhere. Second is the shared index: anything that changes tag handling touches the browse search too. Third is the stale model in running workers, which can make a correct deploy look broken.

Nothing in this recap is a decision yet. The observations come from reading code and spot checks, and the proposals above are proposals.
