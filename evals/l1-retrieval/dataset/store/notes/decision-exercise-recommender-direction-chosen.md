---
id: 01JV53K64E5C9R8363AVE7WD8Q
created: 2025-05-13T12:18-03:00
---

# exercise-recommender: general direction

We settled on a plain, explainable approach for exercise-recommender. Teachers must be able to see why an exercise was suggested, so we favour simple models and candidate filtering over anything opaque. This note keeps the direction only, not tuning values.

## Context

exercise-recommender takes a student's progress against curriculum standards and proposes next exercises. Teachers read the output and often override it. Trust matters more than squeezing out accuracy.

## Direction chosen

Two stages. First, narrow the pool of exercises by standard, difficulty band and what the student has already done. Second, rank what is left with a small scikit-learn model. Ranking stays a thin layer on top of the filters.

## Why two stages

Filtering is easy to explain and easy to test. If a suggestion looks wrong, we can usually tell which filter let it through. A single large model would hide that.

## Where Elasticsearch fits

Elasticsearch handles candidate retrieval: matching exercises to standards and metadata. It does not do the ranking. We keep the index as the source for search, and the database as the source of truth.

## Where Celery fits

Recommendations are computed in background jobs, not during a page request. Django serves the stored results. Teachers see the last computed set, and it refreshes after new progress data arrives.

## Model choice

Prefer linear or tree-based models from scikit-learn. They give feature weights we can show. Deep models were left out for now. Revisit only if the simple ones clearly fall short on real classroom data.

## Explanations in the UI

Each suggestion carries a short reason, such as the standard it targets and the gap it addresses. The Vue.js front end shows that reason next to the exercise. A suggestion without a reason should not ship.

## Teacher control

Teachers can dismiss or pin suggestions. Those actions feed back as signals, but they also act as hard rules until the teacher clears them. A manual override beats the model.

## Cold start

New students and new classes have little history. In that case fall back to standard-based defaults from the curriculum order instead of guessing from sparse data.

## Evaluation

Check changes offline against past progress data first, then compare with what teachers actually picked. Teacher acceptance is the main signal. Keep the evaluation set separate from training data.

## Things we avoided

No per-student models. No opaque scores shown as numbers without meaning. No student-identifying data in the search index beyond what retrieval needs.

## Open questions

How to weigh recent progress against older progress. How to handle standards that overlap. Whether teacher overrides should carry over between terms. None of these is decided.

## Next steps

Keep the two-stage layout when adding features. Write down any change to the explanation text, since teachers rely on it. Record new decisions in separate notes rather than editing this one in place.
