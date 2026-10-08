---
id: 01KPF9KS04B06MGV3V0JJ495YT
created: 2026-04-18T00:19-03:00
sources:
  - "doc: mastery-model evaluation notebook"
---

# mastery-model vs logistic regression: offline comparison

Short version: in an offline comparison, plain logistic regression reached an AUC of 0.81 on held-out terms, and mastery-model reached an AUC of 0.87. mastery-model is the better predictor on the data we tried. This note keeps what that comparison does and does not tell us, so nobody has to re-argue it from scratch. It is a research note, not a decision. Nothing here says the baseline should be deleted or that the gap is settled for every class and subject.

The rest of this is written quickly from what I know of the work. Where I am inferring rather than remembering, I say so.

## What was compared

The question was whether the mastery-model component earns its complexity over a simple baseline. ClassroomCompass tracks each student's progress against curriculum standards and suggests next exercises, so the thing both models predict is, roughly, whether a student will show mastery of a standard on a later attempt given their history on that standard and on related ones. The suggestion logic downstream leans on that probability: it decides which exercise to put in front of a teacher as a next step, and in what order.

The two candidates:

- Logistic regression, used as the baseline. It takes a flat feature vector per student and standard: recent correctness, counts of attempts, time since last attempt, and a few summary features of the exercise. It is built with scikit-learn, which is what the rest of the project already uses for classical models, so it was cheap to set up and is easy to explain to a teacher or a reviewer.
- mastery-model, the component this note is about. It keeps a per-student, per-standard state that is updated as evidence arrives, and it uses relationships between standards in the curriculum, so evidence on a prerequisite standard moves the estimate for a dependent one. It is more involved than the baseline, both to train and to run.

Both were scored on the same task and the same held-out data. The metric was AUC, which measures how well the model ranks students who later show mastery above those who do not. I picked AUC because the suggestion flow mostly cares about ordering, not about the absolute calibration of the probability. That choice has consequences, covered under the caveats.

## Result

On held-out terms, logistic regression reached an AUC of 0.81. On the same held-out terms, mastery-model reached an AUC of 0.87.

The difference is real in the sense that it is not a rounding artefact, and the direction is the one we hoped for. It is not huge. A baseline at 0.81 is already decent, which is worth remembering: the baseline is not a straw man, and a reader who sees only the headline might assume it is. What mastery-model buys us is a moderate improvement in ranking quality, not a different class of behaviour.

I do not have a confidence interval for the gap in my head, and this note should not pretend to one. If someone needs to argue the gap is significant, they should rerun with resampling over students, not over rows, and report the spread. Rows from the same student are not independent, so naive intervals over rows would look tighter than they should.

## What held-out terms means here

The held-out split was by term, not by random row. That matters. Training on earlier terms and scoring on a later term mimics how the system is actually used: the model is fitted on history and then asked about students in the term that is running now. A random split would leak information, because the same student's neighbouring attempts would land on both sides and make both models look better than they are.

So both numbers are on the harder, more honest split. I would expect both to be higher on a random split, and I would not quote those higher numbers anywhere. If someone reruns this and gets figures above the ones in this note, the first thing to check is whether the split is still by term.

A second effect of splitting by term: curriculum standards and exercise banks change between terms. Some standards in the held-out term may have little or no history in the training terms. Both models have to cope with that. The baseline copes by falling back on general features, and mastery-model copes by borrowing strength from related standards. I suspect, but have not shown, that a good part of the gap comes from exactly those cold or thin cases. That is a testable claim and is listed below.

## Why mastery-model probably does better

These are working explanations, not findings. None of them has been isolated by an ablation that I can point to.

### Structure between standards

The curriculum is not a flat list. Many standards depend on others, and a student who struggles with a prerequisite is likely to struggle with what comes after. The baseline sees only features about the standard in front of it, so it cannot use evidence from a related standard unless someone hand-builds a feature for it. mastery-model uses the relationship directly. If this is the main source of the gain, then the gap should be larger for standards with many prerequisites and close to nothing for standalone ones.

### State that updates over time

The baseline treats a student's history as a bag of summary numbers. mastery-model carries a running estimate and moves it with each new piece of evidence. That helps when the order of attempts matters, for example a student who got early items wrong and then recent ones right. A fixed summary like a recent-correctness rate can blur that. If this is the main source of the gain, the gap should be larger for students with long, uneven histories and smaller for students with only a handful of attempts.

### Thin data

Secondary school classes are small. A teacher may have a few dozen students per class, and each student attempts only some of the standards in a term. Per-standard data per student is thin. A model that shares information across standards and across students is better placed than one that fits each feature independently. This is the same point as above seen from the data side.

### Things that probably do not explain it

I doubt the gap is mostly about model capacity in the raw sense. The baseline is a linear model on well-chosen features, and the improvement shows up on held-out terms, which argues against mastery-model simply memorising the training set. I also doubt it is down to the baseline being badly tuned, though I would not swear to that; see the caveats.

## Caveats on the comparison

- The baseline may be under-engineered. The feature set for logistic regression was reasonable but not exhaustive. A stronger baseline with hand-built prerequisite features could close some of the gap. If the real question is whether mastery-model is worth its cost, the fair comparison is against the best simple model we can build in a modest amount of effort, not the first one. This is the largest caveat.
- AUC measures ranking, not calibration. The suggestion flow uses the predicted probability to rank candidate exercises, which fits AUC. But if any part of the product shows the number itself to a teacher, such as a percentage-style mastery readout, then calibration matters and AUC says nothing about it. I did not compare calibration here. Do that before anyone quotes a model output as a literal probability to users.
- One comparison, one dataset. The result comes from the data we have, which comes from the schools using the system. It may not carry over to a new school, a different subject mix, or a different grading culture. Teachers differ in how they record outcomes, and the label for mastery is partly a product of that.
- The label is itself noisy. Mastery is inferred from attempts on exercises, not observed directly. A student can guess correctly or slip. Both models are scored against the same noisy label, so the ceiling on AUC is below one for reasons that have nothing to do with either model.
- No look at subgroups. I did not break results down by subject, year group, or class size. An average gain can hide a segment where mastery-model is no better or is worse. Class size in particular could matter, since tiny classes give less to learn from.
- Cost was not measured as part of this. mastery-model is heavier to train and to serve than the baseline. The AUC gain has to be weighed against that, and that weighing is not in this note.

## Where this touches the rest of the system

Training and refresh of models runs through Celery tasks in the Django backend, so a heavier model means longer or more frequent background jobs. If mastery-model needs to be refit often, for instance after each batch of new attempts, then the cost shows up there first. Whoever owns the scheduling should be told that the preferred model is the costlier one.

Suggested exercises are found and ranked using Elasticsearch together with the model output. The ranking quality gain from 0.81 to 0.87 matters only to the extent that the model score actually drives the final order. If the search side applies strong filters or boosts of its own, some of the benefit will be washed out before a teacher sees it. It is worth checking how much of the final order the model score actually controls.

The Vue.js front end shows progress to teachers. Nothing in this comparison requires a change there. If the readout is later changed to show the model's probability directly, see the calibration caveat first.

## What I would do next

In rough order of value:

- Strengthen the baseline. Add prerequisite-aware features to logistic regression and rerun on the same held-out terms. If the gap stays near its current size, the structure in mastery-model is earning its keep. If it shrinks a lot, the simpler model may be enough for most cases.
- Ablate mastery-model. Turn off the use of relationships between standards and rerun. Separately, turn off the running state update. This says which of the explanations above is real.
- Slice the results. Report AUC by subject, by year group, by amount of history per student, and by whether the standard had history in earlier terms. This tests the thin-data and cold-start suspicion directly.
- Check calibration. Compare predicted probabilities with observed rates for both models, and decide whether the product needs calibrated numbers.
- Quantify uncertainty. Resample by student and give an interval on the difference between the two AUC values.
- Measure cost. Time training and scoring for both, and note memory use, so the tradeoff can be stated plainly.

## How to use this note

If someone asks whether mastery-model beats a simple baseline, the answer from our offline comparison is yes: logistic regression reached an AUC of 0.81 on held-out terms against 0.87 for mastery-model. Say it with the caveats: the baseline was not exhaustively engineered, the metric is ranking only, and the result is from one dataset.

If someone proposes dropping the baseline entirely, hold off until the stronger-baseline rerun is done, because the baseline is also our cheap sanity check and our fallback for cases where mastery-model has nothing to work with.

If someone proposes using mastery-model output as a literal percentage in the interface, send them to the calibration caveat first.

If the numbers here are ever replaced by newer ones, keep the held-out-by-term split, and write down any change to the features, the label definition, or the data window, since each of those moves the AUC independently of the model.
