---
id: 01KZPKWE0HQ92NATDEKQRTET86
created: 2026-08-10T16:55-03:00
---

# Exercise-recommender: what it must return

Rough notes on what the `exercise-recommender` has to hand back to the rest of ClassroomCompass. Written quickly, not checked against anything else we may already have on this. Treat as partial.

## Basic contract

Given a student and a curriculum standard (or a set of standards), the `exercise-recommender` returns a short ranked list of exercises the teacher can assign next. The list is capped at the configured limit, not unbounded. If fewer exercises qualify, return fewer; do not pad with weak matches just to fill the list.

Each item in the list needs at least the exercise reference, the standard it targets, and a score. The score is only for ordering and for the UI to show a rough confidence. It should not be presented to teachers as a grade or a prediction of a mark.

An empty list is a valid answer. It has to be distinguishable from a failure, so the caller can show "nothing suitable yet" instead of an error banner.

## Ordering and filtering

Ranking comes from the scikit-learn model working on the student's progress history for the standard. Candidates come from Elasticsearch, filtered by standard and by level. Things the result must respect:

- Never return an exercise the student has already completed successfully, unless the teacher explicitly asked for revision material.
- Stay inside the year or band the class is set up for. A slightly easier item is fine for a struggling student, but not something from a different stage entirely.
- Keep some variety. Do not return near-duplicate exercises that differ only in the numbers used.
- Ties are broken in a stable way so the same request gives the same order twice in a row. Teachers notice when the list reshuffles on refresh.

## Cold start and missing data

New students have little or no history. In that case the `exercise-recommender` should fall back to a default ordering by difficulty within the standard, and say in the response that it used the fallback. Same for a standard that has few exercises tagged. Do not fail, and do not quietly return model output built on almost nothing.

If the model artifact is unavailable or stale, fall back the same way and flag it. The Celery side that retrains is not allowed to block a request.

## Open questions

- Whether the response should include a short reason per item (for example "targets a gap in the standard") for teachers. Probably yes, but the wording is not settled.
- How to handle a request that spans several standards: one merged list or one list per standard. Leaning towards one list per standard, with the same limit applied to each.
- Whether the usual limit should be adjustable per teacher or stay a global setting.
- Latency expectations when Elasticsearch is slow. The usual budget needs confirming before we add a timeout and fallback path.
