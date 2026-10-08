---
id: 01JXHBD8924ZEWZV0RVDSCRTEF
created: 2025-06-12T02:57-03:00
---

# mastery-model design

mastery-model is the part of ClassroomCompass that estimates, for a student and a curriculum standard, how likely it is that the student has mastered that standard. The estimate feeds the exercise suggestions that secondary school teachers see. It is a scikit-learn `GradientBoostingClassifier` with `n_estimators=200` and `max_depth=3`. This note records how it is put together, what goes in, what comes out, and the things that tend to bite when someone touches it.

Written quickly, from what the team knows about the component. Where a detail is not pinned down here, check the code before relying on it.

## Purpose

Teachers do not want a raw score table. They want to know which standards a student is secure on, which are shaky, and what to practise next. mastery-model turns the history of a student's attempts into a probability per standard. The suggestion logic then reads that probability and picks exercises.

The model answers one question only: given what we have seen so far, is this student likely to be at mastery on this standard. It does not pick exercises, rank teachers' preferences, or decide how a class is grouped. Those live elsewhere.

## Model configuration

The classifier is `GradientBoostingClassifier` from scikit-learn. The two hyperparameters we pin on purpose are `n_estimators=200` and `max_depth=3`. Everything else is left at the library default unless the code says otherwise.

```python
from sklearn.ensemble import GradientBoostingClassifier

model = GradientBoostingClassifier(n_estimators=200, max_depth=3)
```

Shallow trees keep each individual learner simple, and the ensemble size gives enough capacity to pick up interactions between recent performance, task difficulty and standard type. We have not seen a need to change either value since the model was first fitted. If someone changes them, treat it as a model version change and follow the retraining and rollout steps below.

## Inputs

Features are built per student and per standard. They are derived from attempt history and from curriculum metadata. The groups are:

- Recent performance: how the student did on the latest attempts for that standard, weighted toward the most recent.
- Longer-run performance: aggregate correctness over the whole history for that standard.
- Task difficulty: the difficulty rating of the exercises attempted, so a correct answer on a hard task counts for more than one on an easy task.
- Time effects: how long since the last attempt, and the spacing between attempts.
- Standard context: attributes of the standard itself, such as its subject area and where it sits in the curriculum sequence.
- Prerequisite signal: summary of how the student is doing on standards that the curriculum lists as prerequisites.

Features are numeric by the time they reach the classifier. Categorical attributes are encoded before fitting, and the encoding is part of the saved model artifact so that serving uses the same mapping as training.

## Outputs

The model produces a probability of mastery between zero and one for each student and standard pair. The application stores that probability and a coarser band derived from it (not yet secure, developing, secure). The thresholds for the bands are configuration, not part of the model, so they can be adjusted without refitting.

Teachers see the band. The probability itself is used by the suggestion code and for sorting. Do not show raw probabilities in the interface without checking with the product side, since teachers read a figure like that as a grade.

## Labels

Training needs a target: did the student master the standard. The label comes from teacher-confirmed outcomes, such as a teacher marking a standard as achieved after an assessment, together with rule-based outcomes from assessments that carry a clear pass criterion. Labels are noisy. A teacher may mark a standard late, or a pass may reflect luck on a short test. We accept that noise and do not try to clean it by hand.

Only labelled pairs are used for fitting. Unlabelled history still contributes to features but never to the target.

## Training data

Training data is assembled from the production database by a batch job. It takes attempt records, joins curriculum metadata, builds the features above at a cut-off point in time, and attaches the label observed after that point. The cut-off matters: features may only use information available before the label was determined.

Data from the same student can appear at many cut-off points. When splitting for evaluation, split by student, not by row, otherwise the evaluation looks better than the model really is.

## Leakage risks

Leakage is the main way this model can look good and be useless. Things to watch:

- Features that include the attempt which produced the label.
- Prerequisite signal computed using data after the cut-off.
- Splitting rows from one student across training and evaluation sets.
- Teacher confirmations that were themselves triggered by the model's own suggestion, which feeds the model back to itself.

The last one is subtle. Once teachers act on suggestions, some labels are partly a consequence of the model. We have not solved this; we only note it when reading evaluation results.

## Training pipeline

Fitting runs as a Celery task, not inside a web request. The task builds the dataset, fits the classifier, evaluates it, and writes the artifact if it passes the checks. The Django side only enqueues the task and reads the result. Long fits must never run in the request cycle.

The task is idempotent in the sense that running it twice with the same data gives an equivalent model, as far as the library allows. Gradient boosting in scikit-learn is deterministic given a fixed random state, so the random state is set explicitly in the training code. Keep it set. A changing random state makes comparisons between runs meaningless.

## Evaluation

Before an artifact is accepted, it is compared against the model currently in service on a held-out set that is split by student. We look at discrimination (how well it separates mastered from not mastered) and at calibration (whether a predicted probability matches the observed rate). Calibration matters more than usual here, because bands are cut from the probability and a miscalibrated model shifts students between bands.

We also look at results by subject area. A model that is fine on average but poor on one subject is not acceptable, because teachers of that subject will see bad suggestions and stop trusting the tool.

## Serving

Predictions are made in two ways. A scheduled Celery job recomputes mastery for students whose history has changed, and stores the results. For an interactive request, such as a teacher opening a student page, the application reads the stored values rather than calling the model live. This keeps pages fast and keeps scikit-learn out of the web workers' hot path.

The model artifact is loaded by the worker processes. Loading is done once per process, not per task. If the artifact changes, workers need to pick up the new file, which in practice means a worker restart or the reload hook, depending on how the deployment is set up.

## Search and indexing

Mastery results are also pushed into Elasticsearch so teachers can filter and sort class lists by band, by standard, and by subject. The index holds the band and the probability next to the student and standard identifiers. It is a copy, not the source of truth. The database holds the real values.

If the index and the database disagree, the database wins and the index should be rebuilt from it. A reindex after a model version change is required, otherwise teachers see a mix of old and new estimates in the same list.

## Front end

The Vue.js interface shows the band as a coloured indicator per standard and a short explanation of what drives it, such as recent attempts or a weak prerequisite. The explanation is built from the feature groups, not from model internals, so it is coarse by design. Do not promise teachers a precise reason for a prediction.

## Versioning

Every stored prediction records which model version produced it. When a new artifact is promoted, new predictions carry the new version and older ones stay as they are until the recompute job replaces them. This is what makes it possible to tell, during a rollout, which numbers came from which model.

Changing the hyperparameters, the feature set, the encoding, or the label definition all count as a new version. Changing only the band thresholds does not need a new model version, but it does need a note in the release log.

## Rollout

New versions are promoted deliberately. The steps are: train and evaluate, compare against the version in service, promote the artifact, run the recompute job for all active students, then rebuild the Elasticsearch index. Skipping the last two leaves the system in a mixed state.

If something looks wrong after promotion, go back to the previous artifact and recompute again. Keep the previous artifact until the new one has been in service for a while and no one has complained.

## Monitoring

Things worth watching after any change:

- The share of student and standard pairs in each band. A sudden shift usually means a feature broke, not that students changed.
- Calibration on newly confirmed labels, which arrive over time.
- Failures in the training and recompute tasks, which show up as Celery task errors.
- Gaps between database and index counts.

A feature that silently becomes empty is the most common cause of odd outputs. Gradient boosting will not fail loudly on that; it will just predict worse.

## Known limits

- Cold start: a student with little history gets estimates that lean on standard context and prerequisites, and these are weak. The interface should say so rather than present a confident band.
- Label noise, as above.
- Feedback between suggestions and labels, as above.
- Curriculum changes: when standards are revised or renumbered, the standard context features shift and old history may not map cleanly. Plan a retrain when the curriculum data changes.
- The model has no notion of a particular teacher's grading habits. Two teachers with different strictness produce labels of different meaning.

## Things to check before changing it

Before touching mastery-model, read the feature construction code and the training task together. Make sure the cut-off logic is unchanged, the split is still by student, the random state is still pinned, and the encoding is saved with the artifact. Confirm that `GradientBoostingClassifier` is still constructed with `n_estimators=200` and `max_depth=3` unless the change is explicitly about them, and if it is, record the reason in this note.

## Open questions

- Whether a per-subject model would beat a single shared one is untested here.
- How to correct for labels influenced by the model's own suggestions.
- Whether the band thresholds should differ by subject area.
- Whether stored probabilities should be refreshed on a fixed schedule even with no new attempts, to account for the passage of time since the last attempt.
