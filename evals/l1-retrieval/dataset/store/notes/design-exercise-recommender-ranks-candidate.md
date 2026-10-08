---
id: 01K9ADCRC3PQNY5F981ZG65PRW
created: 2025-11-05T13:25-03:00
---

# exercise-recommender design

exercise-recommender is the part of ClassroomCompass that answers one question for a teacher: what should this student work on next? It looks at a student's progress against curriculum standards, builds a pool of candidate exercises, ranks them by predicted mastery gap and returns the top 8. This note records how that is put together and why, so nobody has to rediscover it from the code.

## Purpose

Teachers in secondary schools do not want a long list. They want a short set of exercises they can hand out or assign in a lesson. exercise-recommender gives them the top 8 for a student, ordered so the exercise that targets the biggest gap comes first. Anything past the top 8 is dropped before the response leaves the service, and the Vue.js front end never sees it.

The "gap" is not a raw score. It is the distance between where the student is predicted to be on a standard and where the curriculum expects them to be. A student who has done little on a standard but is predicted to pick it up easily has a smaller gap than one who has done a lot and still struggles.

## Pipeline

The flow is roughly this, in order:

1. Django receives the request for a student and a class context.
2. Candidate exercises are pulled from Elasticsearch, filtered by the standards the student is currently working on.
3. A scikit-learn model predicts mastery for each standard the candidates touch.
4. Each candidate gets a score from the predicted mastery gap on its standards.
5. Candidates are sorted by that score and cut to the top 8.
6. The result goes back to the front end with a short reason per exercise.

Celery is not in the request path. It only runs the background work described below.

## Ranking

An exercise can cover more than one standard. Its score combines the gaps of all the standards it covers, weighted by how much of the exercise is about each one. Exercises that cover only standards where the student is already strong score low and fall out of the top 8.

Ties are broken in favor of exercises the student has not seen before. Repeats are allowed, since revisiting an exercise can be the right call, but they lose to a fresh exercise with the same score.

We kept the ranking as a plain score and sort on purpose. It is easy to explain to a teacher who asks why an exercise was suggested, and easy to debug when a suggestion looks wrong.

## Model and refresh

The mastery model is a scikit-learn model trained on past student results. Training is not done per request. Celery tasks retrain on a schedule and write the new model where the Django service can load it. Predictions at request time use whatever model was loaded last, so a fresh retrain takes effect only after the service reloads it.

Elasticsearch holds the exercise index. Celery also keeps that index in sync when exercises or their standards tags change in the Django database.

## Things to watch

- If the model fails to load, the service should not return an empty list. Fall back to ordering candidates by how recently the student last touched the standards, and log it.
- If fewer candidates exist than the cutoff, return what there is. Do not pad.
- Stale index data shows up as exercises tagged with the wrong standard. Check the Celery sync before suspecting the model.
- Any change to the cutoff touches the front end layout too, because the teacher view is laid out around the current count.

## Sketch

```python
# conceptual, not the real code
scored = [(gap_score(ex, mastery), ex) for ex in candidates]
scored.sort(key=lambda s: s[0], reverse=True)
return [ex for _, ex in scored[:8]]  # top 8
```

## Open questions

- Whether the cutoff should differ by class size or lesson length. Nobody has asked for it yet.
- Whether to show teachers the predicted gap itself, or only the reason text.
