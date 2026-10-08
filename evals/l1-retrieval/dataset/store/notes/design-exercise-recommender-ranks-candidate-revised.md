---
id: 01KCXWEZ0EQYJHP5CZVM7TXF7F
created: 2025-12-20T09:41-03:00
---

# exercise-recommender design (nxtstep)

This note replaces the earlier note "exercise recommender ranks candidate". The new value is that exercise-recommender returns the top 5 exercises, not the top 8 it returned before.

## Naming

The component is called `exercise-recommender` everywhere in the docs and the code layout. Internally the team also calls it `nxtstep`. That is only the codename of `exercise-recommender`. If you see `nxtstep` in a log line, a branch name or a chat thread, it is the same thing.

## What it does

ClassroomCompass tracks each student's progress against curriculum standards. The exercise-recommender takes that progress and suggests what the student should practise next. Secondary school teachers see the suggestions in the Vue.js front end and can use them as they are or pick something else.

## Current output size

The list is now the `top 5`. The earlier version returned eight. Teachers found eight too long to read through in a lesson, and the lower half of the list was rarely used. Five is short enough to scan and still leaves room for a teacher to skip a suggestion.

## Pipeline in brief

Candidate exercises are looked up in Elasticsearch by the standards the student has not yet mastered. A scikit-learn model then scores and ranks the candidates. Celery runs the scoring in the background, and Django serves the stored result to the front end. The cut to the `top 5` happens after ranking, so the ranking logic itself did not change.

## Why cut after ranking

Keeping the cut as the last step means the size can be changed in one place. The scores for the whole candidate set are still computed, which helps when we want to look at why an exercise missed the list. We did not shrink the candidate set earlier, because that would change which exercises compete with each other.

## Things to watch

- Students with few unmastered standards may have fewer than five candidates. The list is then simply shorter, and no padding is added.
- Any front end text or test that assumed eight items needs checking. Old screenshots and help text may still say eight.
- Cached results made before the change may hold longer lists until they are recomputed.

## Related

Input to this component depends on the standards data loading cleanly. Past problems with that are in [[standards-ingest-crashes-files-recap]].

## Open questions

Whether teachers should be able to set the list size themselves is undecided. For now it is fixed at the `top 5` for everyone.
