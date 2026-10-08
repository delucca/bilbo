---
id: 01KF2CK6P9PRJ9GBMVHG33XRBC
created: 2026-01-16T00:11-03:00
---

# Mastery model with scikit-learn

Rough notes on how the mastery-model component estimates what a student has mastered, and how scikit-learn fits in. Written quickly while reading through the code and talking it over, so it is partial. Some parts are things I am fairly sure of, some are guesses, and I have marked which where I remembered to.

The short version: for each student and each curriculum standard we want a number that says how likely it is that the student has mastered that standard. The exercise suggester reads those numbers and picks the next exercises. The model is a fairly plain supervised classifier trained on past attempt data, wrapped in a few layers of feature building and calibration. It is not deep learning and nobody should turn it into that without a good reason. Teachers need to be able to ask why a student was shown something, and a simple model lets us answer.

## What the model is for

The mastery score is not a grade. Teachers already have grades. It is an estimate of readiness, used for two things: deciding which standards to show as green, amber or red on the class overview in the Vue front end, and ranking candidate exercises for the "next exercise" suggestion. The thresholds for the colours are configuration, not something the model decides. The model only outputs a probability, and the Django side maps it to a band.

Because the output is used for ranking as well as for display, calibration matters more than raw accuracy. A model that ranks well but outputs probabilities that are all squeezed toward the middle will make the colour bands useless. We noticed this early on: the first version looked fine on ranking metrics and gave an overview page where nearly every standard was amber. That is why there is a calibration step after the classifier, described below.

The model also has to cope with sparse data. A student may have only a handful of attempts on a given standard, particularly early in a term. The design leans on features that borrow strength from related standards and from the class as a whole, so a new standard for a student does not start from nothing.

## Training data and labels

Training examples come from recorded attempts: a student, an exercise, the standards the exercise is tagged with, the outcome, how long it took, whether hints were used, and when it happened. These live in the main Django database. A Celery task pulls them into a flat table for training; we do not train straight from the ORM because it is slow and awkward for this.

The tricky part is the label. We do not have a ground truth for "mastered". What we have is a proxy: whether the student succeeded on later attempts at the same standard after the point we are making a prediction for. So each training row is a snapshot of a student's history on a standard up to some time, and the label is derived from what happened afterwards within a window. The window length is configured and has been tuned a couple of times. Rows where there is no later attempt inside the window are dropped, which introduces a bias: students who stop practising a standard are probably different from those who continue. Worth remembering when reading any aggregate numbers. I do not think it has been properly quantified.

There is also a teacher-assessed signal for some standards, where a teacher marks a standard as secured by hand. Those marks are treated as stronger labels in some experiments but are not in the main training set by default. I am not sure whether that has changed; check the training task configuration before assuming.

Splitting for evaluation is by student, not by row. Splitting by row leaks, because the same student's snapshots are highly correlated. This was a mistake at one point and the early numbers were too optimistic as a result. Splits are also kept time-aware where possible, so we evaluate on later periods than we train on.

## Features

Features are built in a separate step so that training and serving use the same code. This is the most important rule in the component: any feature used in training must be computable at prediction time from what the serving path has. Several bugs have been of the form "feature available in the batch job but not online".

Groups of features, roughly:

- Recent performance on the standard: success rate over the last few attempts, a recency-weighted success rate, and the number of attempts so far. The decay rate for the weighting is configured.
- Effort and behaviour signals: time taken relative to the usual for that exercise, hint usage, and whether attempts were spread over several days or crammed into one sitting. Spaced practice is a decent predictor of retention, and the label is about later success, so this helps.
- Exercise difficulty: an estimate per exercise, derived from the overall success rate across all students, shrunk toward the average for exercises with few attempts.
- Prerequisite signals: curriculum standards have a dependency structure. Performance on prerequisite standards is included as features, aggregated simply (mean and minimum). Missing prerequisites are filled with a neutral value plus an indicator that they were missing.
- Class context: the class's average on the same standard. This helps new students and makes the model somewhat sensitive to teaching differences between classes, which is partly good and partly a concern.
- Time features: how long since the last attempt on the standard, and how long since the start of the term. Forgetting is real, and scores should drift down if a student has not touched a standard for a long time.

Categorical features such as subject and year group are one-hot encoded inside the preprocessing pipeline. Numeric features are scaled where the estimator needs it. Everything sits in one scikit-learn pipeline object so the fitted preprocessing travels with the model.

We deliberately do not include anything identifying the student as a feature, and nothing demographic. That is a policy decision and not a technical one. Do not add such fields to make the metrics look better.

## Estimator and calibration

The main estimator is a gradient-boosted tree classifier from scikit-learn. We tried a regularised logistic regression as a baseline; it is still kept in the evaluation script as the thing to beat. The boosted model wins by a modest margin, mostly because of interactions between recency and attempt count. The logistic baseline is easier to explain and is the fallback if the boosted model misbehaves in production.

Hyperparameters are chosen by cross-validated search, grouped by student. The search space is small on purpose. We keep depth low and use a small learning rate with early stopping, because the data is noisy and deeper trees overfit to individual students. The chosen values are stored with the model artefact, not in this note, since they change on every retrain.

Calibration is done with a held-out slice of students not used in fitting, using the usual scikit-learn calibration wrapper. I believe we use the sigmoid method rather than isotonic, because isotonic was jumpy with the amount of data in the calibration slice. If the data volume grows a lot, that choice should be revisited. After calibration we check reliability curves per subject, since a single global calibration can hide subject-level skew. Maths and science behaved differently from humanities at one point, which raised the question of per-subject calibrators. Not done yet as far as I know.

Evaluation metrics that we look at: log loss and Brier score for calibration quality, area under the ROC curve for ranking, and a precision-at-top style metric for the suggestion use case. Accuracy at a fixed threshold is reported but is not what we decide on.

## Training and serving flow

Training is a scheduled Celery job, run outside school hours. It builds the flat table, builds features, fits and calibrates, evaluates against the previous model on the same held-out students, and only promotes the new artefact if it is not worse by more than a tolerance. The tolerance is configured. If promotion is refused, the job logs why and leaves the previous artefact in place. A human should look at those logs; nobody is paged.

The artefact is a serialised pipeline plus a small metadata record: when it was trained, what data range, feature list, and evaluation numbers. The metadata is what the admin page shows. Serialisation uses the standard Python mechanism that scikit-learn recommends, which means artefacts must only ever be loaded from our own storage. Never load a model file that came from outside.

Serving has two paths. The first is batch: after a student's attempts are recorded, a Celery task recomputes the affected standards and stores updated scores in the database. The second is on demand, used when a teacher opens a class view and the stored scores are stale beyond a configured age. The on-demand path is slower and is meant to be rare. In practice most views read stored scores.

Elasticsearch is not part of the model itself. It is used by the exercise suggester to retrieve candidate exercises by standard, topic and difficulty. The mastery scores are then used to rerank those candidates. So the model never queries Elasticsearch, and a change to the index mapping does not require retraining. A change to how difficulty is represented in the index could, though, drift away from the difficulty feature the model was trained with. Keep those two definitions in step.

## Known problems and open questions

Cold start is the biggest practical issue. For a student's first few attempts on a standard, the model mostly leans on class-level and prerequisite features, and the output is uncertain. Right now we show the score anyway with the same colour banding. A better approach might be to show an "not enough evidence" state in the UI when the attempt count is below a configured minimum. The front end has some of this but the threshold is not shared with the back end, so they can disagree. This should be a single setting read by both.

Label delay is another. Since the label depends on later attempts, the freshest data cannot be used for training, so the model always lags behind the curriculum by at least the label window. When a standard is newly introduced or an exercise set is replaced, the model has little to go on until enough time has passed. We handle this crudely by falling back to class-level features.

Feedback loops: the suggester picks exercises based on the model, and the outcomes from those exercises feed the next training run. If the suggester over-serves easy exercises to students scored as weak, those students will accumulate easy successes and the model may inflate their scores. We have not measured this effect. A possible mitigation is to include the difficulty of the attempted exercise as a feature, which we do, and to weight examples to correct for selection, which we do not.

Fairness across classes and schools: because of the class-context feature, a class that is taught differently can have systematically shifted scores. We look at per-school calibration occasionally but there is no automated check. Worth adding one that flags a school whose reliability curve is far from the rest.

Explainability: teachers sometimes ask why a standard is amber. For the boosted model we can compute feature contributions, but we only expose a hand-written summary built from a few of the strongest features, such as recent success rate, time since last practice and prerequisite weakness. The mapping from contributions to wording is a bit ad hoc and should be reviewed with a teacher before it is extended.

## Things to be careful about when changing this

Keep the feature code shared between training and serving. If you add a feature, add it in one place, and make sure the on-demand path can compute it from the data it has.

Do not change the label definition without retraining and re-evaluating from scratch, and do not compare metrics across label definitions. The numbers are not comparable.

Always split by student. If a new evaluation script splits by row, treat its results as wrong.

When you retrain after changing the library version, expect small differences in outputs and check that the saved artefacts from the old version are not loaded by the new one. Artefacts and library versions need to move together, and the metadata record should note what produced the file. If an old artefact fails to load after an upgrade, retrain instead of trying to patch it.

Tests: there are unit tests for the feature builder, using small hand-made histories where the expected values can be checked by eye, and a smoke test that fits a tiny model on synthetic data and checks that the pipeline round trips through serialisation. There is no test that the calibrated scores are well calibrated on real data, because real data cannot be in the repository. That check lives in the evaluation job and its output is reviewed by a person.

Performance: fitting is not slow at the current data volume, and prediction for a class is fast. The expensive part is building the flat table, which is mostly database time. If training time becomes a problem, look at the extraction query and the feature builder before looking at the estimator.

## Ideas not yet tried

A per-standard hierarchical prior, so that scores for rarely seen standards shrink toward their parent topic rather than toward a global average. This might help cold start more than anything else on the list.

A survival-style treatment of forgetting, in place of the crude time-since-last-attempt feature. It would give a more principled way to say when a standard needs revisiting, which the suggester could use directly.

Using teacher-secured marks as an extra evaluation set, rather than as training labels. That would give us at least one signal that is independent of the attempt-based proxy, and it would let us say something about how well the proxy agrees with teacher judgement.

Per-subject calibration, as mentioned above, if the reliability curves keep showing subject-level skew.

A cleaner shared setting for the minimum evidence threshold, used by the model service, the Django views and the Vue components alike.

None of these is planned work. They are here so the next person does not have to think of them again from scratch.
