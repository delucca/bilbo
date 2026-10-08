---
id: 01KZ8DPV422H14CDDXB71RPV5V
created: 2026-08-05T04:37-03:00
---

# mastery-model: things to watch out for

Notes for anyone changing mastery-model. Nothing here is a decision, just traps that are easy to hit when you are rushing.

## Scope of the component

mastery-model sits between recorded student work and the exercise suggestions teachers see. A small change in how it scores can shift what every teacher sees, so treat edits as user-facing even when they look internal.

## Training and serving drift

The scikit-learn model is trained in one place and used in another. If feature code changes on one side only, predictions go quietly wrong with no error. Keep feature building shared, not copied.

## Saved model artifacts

Old pickled or serialized models may not load after a library upgrade or a feature change. Check that stored artifacts still load before shipping. Keep a way back to the previous artifact.

## Celery tasks

Retraining and rescoring run as Celery tasks. Make them safe to run twice, since retries and duplicate delivery happen. Do not assume order between tasks for the same student.

## Long-running rescoring

Rescoring a whole school in one task can time out or hog workers. Batch it, and don't block the queue that interactive work uses.

## Curriculum standards changes

Standards get renamed, merged or split. Mastery data keyed to old standards can orphan. Think about migration of existing records before changing how standards are mapped.

## Sparse data per student

Many students have little recorded work on a given standard. Don't let small samples produce confident-looking mastery. Check behaviour for new students and for standards with almost no evidence.

## Elasticsearch index sync

Exercise suggestions rely on search data. If mastery output changes shape, the index mapping and the documents may disagree. Reindexing may be needed, and the order of deploy matters.

## Django migrations

Schema changes on mastery tables touch large data. Write migrations that are reversible and don't lock tables for long. Test against realistic volumes, not an empty dev database.

## Vue.js display

The front end shows mastery levels and suggestions. If you change the scale, labels or null handling, update the Vue components too. Teachers read these numbers literally.

## Fairness and explainability

Teachers need to understand why an exercise was suggested. Don't swap in something opaque without keeping some explanation. Watch for groups of students who get systematically lower estimates.

## Tests

Unit tests with tiny fixtures miss most problems here. Add regression checks on fixed sample histories and compare outputs before and after a change.

```python
# quick sanity check before merging changes to mastery-model
from sklearn.utils.validation import check_is_fitted
check_is_fitted(model)
```

## Rollout

Ship behind a switch if you can, and compare old and new outputs on real data first. Tell teachers when suggestions will visibly change.
