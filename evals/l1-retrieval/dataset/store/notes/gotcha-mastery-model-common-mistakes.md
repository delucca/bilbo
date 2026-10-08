---
id: 01K2CVDQ1N4668EHX0G7VJH4Z7
created: 2025-08-11T12:19-03:00
---

# mastery-model: common mistakes

This is a list of the ways people go wrong with mastery-model. It comes from reading the code, from review comments, and from the questions that keep coming back. None of it is about one particular failure. It is about habits. Most of the trouble comes from forgetting that mastery-model is a rough estimate that teachers lean on more than it deserves. They plan lessons around it, group students by it, and talk to parents about it. When it is slightly wrong in a quiet way, nobody notices until a teacher says the suggestions feel off for a whole class.

Read this before changing training, features, thresholds, or the way scores are shown. Most of the mistakes below look reasonable when you make them.

## Treating the score as a fact about the student

The first mistake is conceptual. The output of mastery-model is an estimate built from whatever evidence reached it: submitted exercises, marks entered by teachers, and sometimes quiz results. It is not a measurement of what a student knows. People read a number or a band on a dashboard and start treating it as a property of the student, the way a height is a property.

The practical effects:

- A student who was absent for a stretch looks like a student who stopped learning. The estimate drops or goes stale, and the next-exercise suggestions get easier or repetitive. A teacher then sees a student who "regressed" when the student simply had no evidence recorded.
- A student who did a lot of easy exercises looks strong on a standard. Volume of evidence and difficulty of evidence are different things, and the model only separates them as well as its features allow.
- A teacher who enters marks in bulk after a test produces a burst of evidence on one day. If the model weights recent evidence heavily, that one burst can override everything before it.

When you write code or copy that talks about mastery-model output, say "estimated" or "likely". Do not write labels such as "has mastered" in an API field name or a UI string. Those names spread. Within a few weeks someone builds a report on top of a field called something like mastered and nobody remembers it was a guess.

A related habit is comparing students on the raw estimate. Two students with the same estimate can have very different amounts of evidence behind them. If the confidence or evidence count is available, show it or use it. If it is not available, say so in the docs, and do not quietly rank students.

## Training data that does not match the classroom

The model is fitted with scikit-learn on historical records. The classic mistake is assuming the history looks like the classrooms it will be used in.

Several ways this goes wrong:

- **Schools differ.** Pacing, marking habits, exercise choice and how often teachers record anything vary a lot between schools and between departments in one school. A model fitted mostly on schools that record everything will read sparse data from another school as weak performance.
- **Selection effects.** Teachers do not assign exercises at random. They give harder ones to students they think are ready and easier ones to students they think are struggling. The history therefore contains the teacher's opinion, and a model trained on it can learn to echo that opinion back. It looks accurate offline and adds nothing new online.
- **Suggestions feed the data.** Once the suggestions are in use, the next batch of evidence comes from exercises the model chose. A model retrained on that evidence is learning partly from its own earlier output. This feedback loop is slow and easy to miss. Narrow suggestions produce narrow evidence, which produces narrower suggestions.
- **Mixed year groups.** Pooling younger and older secondary students without a feature or a separate fit makes the model average across populations that behave differently.

What to do about it: look at the data before training, grouped by school and by year group. Keep a note of which populations are underrepresented. When you evaluate, split by school or by class, not only at random. A random split over individual records hides all of the above.

## Standards changing under the model

The curriculum standards are not fixed. They get revised, split, merged, renamed and re-mapped to exercises. The mastery-model depends on a mapping from evidence to standards, and that mapping is where much of the damage happens.

Common mistakes:

- Treating a standard identifier as stable meaning. If a standard is reworded or split, the identifier may stay while the content changes, or the content may stay while the identifier changes. Either way, history attached to it no longer means what it did. Old evidence then counts toward a different skill.
- Re-mapping exercises to standards without recomputing the estimates that depended on the old mapping. The stored estimates are quietly out of date and nothing flags it.
- Using the standards hierarchy as if it were a prerequisite graph. Parent and child standards in a framework are an organising structure. They do not necessarily say what must be learned first. If the model or the suggestion logic uses the tree to infer readiness, it will sometimes infer nonsense.
- Assuming every exercise maps to exactly one standard. Many touch several, and some touch none cleanly. Spreading evidence evenly across several standards is a modelling choice with consequences, so make it on purpose and write it down.

When the standards change, treat it as a data migration plus a model event. Decide explicitly what happens to old evidence, whether estimates are recomputed, and whether the fitted model is still valid. Do not let the mapping change in one deploy and find out about the effects from a teacher.

## Leakage and evaluation that flatters the model

Evaluation mistakes are the most common because they are the easiest to make without noticing. The numbers come out good, so nobody looks harder.

- **Time leakage.** Features computed using evidence that came after the moment being predicted. For example, a cumulative average per student per standard that includes the very exercise being used as the label. The model then looks excellent offline and mediocre live.
- **Student leakage.** The same student appears in both train and validation sets. Since a student's records are strongly related, the model memorises student habits rather than learning anything general. Split by student at minimum, and by class or school when you want to know how it travels.
- **Label leakage through teacher marks.** If a feature is derived from a mark the teacher entered after seeing the result, it carries the label.
- **Tuning on the test set.** After a few rounds of trying features and settings against the same held-out data, that data is no longer held out. Keep a final set you touch rarely.
- **Picking one headline metric.** A single accuracy-like score hides the cases teachers care about. Overestimating a struggling student is worse in practice than underestimating a strong one, because it removes support. Look at errors by group and by direction.
- **Ignoring calibration.** If the output is shown as a probability or is cut into bands, the values must mean something. A model can rank students well and still give probabilities that are far too confident. Check calibration, not just ranking.

A good habit: write down, next to every evaluation, what the split was, what the time boundary was, and which features could not have existed at prediction time. If you cannot fill that in, the result should not be trusted.

## Retraining in the wrong place

The mastery-model sits between a Django web app and Celery workers. The usual mistakes are about where work happens and who owns the fitted artifact.

**Fitting inside a request.** Someone adds a quick refit to a view or an admin action because it works fine on a small dev database. In production it holds a web worker, times out, or competes with real traffic. Fitting and bulk scoring belong in Celery tasks. A request should only read stored estimates or, at most, score a single student with an already loaded model.

```python
# fit and bulk scoring go in a Celery task, never in a Django view
@shared_task
def refit_mastery_model():
    ...
```

**Loading the fitted model on every call.** The opposite error. Deserialising a scikit-learn artifact for each task or each request wastes time and memory. Load it once per worker process and reload deliberately when a new one is published.

**Workers on different model versions.** During a rollout, some Celery workers hold the old artifact and some the new one. For a while the same student can get different estimates depending on which worker handled the task. Estimates then flip back and forth and a teacher sees it. Make the artifact identity part of what is stored with each estimate, and plan how workers pick up a new one.

**Non-idempotent tasks.** Celery can deliver a task more than once, and retries are normal. A task that appends evidence or increments a counter will double count. Write tasks so that running them twice leaves the same result. Key the work on the student and the standard, and overwrite instead of accumulating where you can.

**Unbounded fan-out.** Scheduling one task per student per standard at the same moment for a whole school floods the queue and starves the quick tasks. Batch it, and keep the long refits on their own queue away from the tasks that update a single student after a submission.

**Pickle and compatibility.** Artifacts saved by scikit-learn are tied to the library that wrote them. Upgrading the library and loading an old artifact can fail loudly or, worse, load and behave slightly differently. Treat a library upgrade as a reason to refit and re-validate, not as a no-op.

## Staleness between the stored estimate and the search index

Elasticsearch is used so that teachers can find students, standards and exercises quickly, and so that suggestions can be filtered and ordered. A copy of mastery-model output often lives in the index. That copy is the source of many confusing reports.

Typical mistakes:

- **Index treated as the source of truth.** The database holds the estimate; the index holds a copy for searching. If someone reads the index for a decision that matters, such as flagging a student, they act on whatever was indexed last, which may be old.
- **Partial reindexing.** A refit updates estimates in the database and the indexing step fails or lags for some documents. Some students then show new values and others old ones, and nothing says which are which. Record when each estimate was computed and indexed, and expose it.
- **Mapping changes done casually.** Changing a field type or analyser in the index mapping usually needs a full reindex. Doing it on a live index in place causes either rejected documents or fields that silently stop matching.
- **Ordering suggestions by an indexed score without a tie rule.** When many items share the same or nearly the same score, the order between them can change from one query to the next, so a teacher refreshes and sees a different list. Add a stable secondary ordering.
- **Stale suggestions after a standards change.** Suggestions indexed against old standards keep showing up until they are rebuilt. Rebuild them in the same step as the standards migration.
- **Search relevance mixed with mastery.** Blending a text relevance score with a mastery-derived score in one sort can make either one swamp the other depending on how the scales happen to line up. Keep them separate, or normalise on purpose and check what teachers actually get.

If an issue is reported as "the number is wrong", check first whether the database and the index agree, before touching the model.

## Cold start for new students, new classes and new standards

The model needs evidence, and the start of a school year has almost none. The mistakes here come from pretending otherwise.

- **Defaulting to a middle value and showing it as an estimate.** A new student gets an average-looking result that reads like a real assessment. Teachers then act on a number that came from nothing. Show it as unknown or low-confidence, and let the interface say "not enough evidence yet".
- **Defaulting to the bottom.** The reverse: new students shown as weak on everything because the missing evidence is encoded as zero. Missing is not the same as zero, and tree-based and linear models treat the encoding very differently. Decide how missing values are handled and test that path directly.
- **Prior from the wrong group.** Seeding a new student from a class or school average works only if that group is comparable. A strong class and a weak class will produce opposite biases for the same new arrival.
- **New standards with no history.** A standard added this year has no evidence from earlier students. The model cannot say much, and suggestions that depend on it will be erratic. Fall back to something simple and visible.
- **Transfers between schools or classes.** A student who moves keeps their evidence, but the context features (class level, pacing) change. Check that those features are derived from the current situation and not from stale joins.

The fix is usually not smarter modelling. It is being honest in the output about how much is known, and making sure downstream code handles "unknown" as a real value, not a null that crashes or gets coerced.

## Thresholds, bands and what the teacher actually sees

The model produces a continuous estimate, but most screens turn it into bands or flags such as "secure", "developing", "needs support". The cut points are where a lot of unintended behaviour hides.

- **Hard-coded cut points in several places.** The Django side, the Celery jobs, the index queries and the Vue.js components each carry their own copy of the thresholds. They drift. A student is "developing" in one view and "secure" in another. Keep the thresholds in one place and have everything else read from there.
- **Changing the model without revisiting the cut points.** A refit shifts the distribution of outputs. The same cut points then put far more or far fewer students in a band. Teachers see a sudden wave of students moving, caused by nothing the students did. After any refit, compare the band distribution before and after, and hold the release if it jumped.
- **Bands that flip on noise.** A student whose estimate sits near a cut point will flip bands on tiny changes. Without some hysteresis or a minimum change rule, the display flickers and teachers stop believing it.
- **Using band changes as alerts.** Notifying a teacher each time a student crosses a boundary generates noise, especially for students near it. Alert on sustained change with enough evidence, not on a single crossing.
- **Same cut points for every standard.** Standards differ in how much evidence a reasonable estimate needs and in how noisy the exercises are. One global threshold is convenient and often wrong for some of them.

When someone asks to "just tweak the threshold", ask what the distribution looked like before and after, and who will see the change.

## Display and interpretation mistakes in the Vue.js front end

The front end is where the estimate becomes a decision. Some mistakes are about code and some about wording.

- **Fake precision.** Showing many decimal places or a precise-looking percentage for a rough estimate. It invites teachers to compare two students on a difference that means nothing. Round, or use bands with a visible uncertainty hint.
- **Colour only.** Red, amber and green with no text. This is hard to read for some users and makes the bands feel like grades. Add a label.
- **Hiding the evidence.** An estimate with no way to see what it was based on cannot be challenged. Teachers should be able to open a student and see the recent exercises and marks that fed the estimate, including the missing stretches. Most disputes end quickly when the evidence is visible.
- **Caching estimates in component state.** A page that keeps an old value after the student submits more work shows something out of date next to fresh evidence. Refresh on the right events, and show a computed-at time somewhere sensible.
- **Optimistic updates that guess.** Updating the display locally after a submission, before the backend has rescored, shows a value the model never produced. Show that a rescore is pending instead.
- **Sorting a class list by the estimate by default.** It turns a rough signal into a league table. Default to name or to seating order, and make ranking something a teacher chooses.
- **Wording of suggestions.** "Next exercise" is a suggestion. If the interface makes it sound like an instruction, teachers either follow it blindly or distrust everything. Keep an easy way to ignore or dismiss a suggestion, and record when that happens, because it is useful signal.

## Privacy, fairness and who can see what

This is easy to leave for later and hard to retrofit. Mastery-model output is data about children, and secondary school teachers share devices, screens and sometimes accounts.

- **Features that stand in for something sensitive.** Even if no protected attribute is used directly, features such as school, attendance pattern, or language of instruction can correlate with one. Check error rates across groups where you are allowed to, and not just overall.
- **Logging estimates with names.** Debug logging of scores next to student identifiers ends up in log stores with broader access than the application. Log identifiers that cannot be traced back without the main database, and keep scores out of routine logs.
- **Copies in the index and in task payloads.** The search index and the Celery broker both hold copies of student data. Access controls and retention on them are separate from the database and are often forgotten.
- **Exports.** A convenient CSV export of estimates for a whole school is the kind of feature that gets built quickly and read widely. Think about who gets it and what it is used for.
- **Using the estimate for things it was not built for.** Reports to parents, setting decisions, or comparing teachers by their classes' estimates. The model was not validated for any of these. If a request goes in that direction, push back or get the validation done first.

A model that is a little less accurate but easy to explain and audit is often the better choice for this audience.

## Testing, monitoring and review habits that miss problems

The last group is about how the work is checked. Most of the earlier mistakes survive because the checks are weak.

- **Tests that only check shapes.** A unit test that confirms the model returns a value in range, or that a task runs, says nothing about quality. Add a small set of fixed, hand-made student histories with expected directions of change: more correct work should not lower the estimate, and a long gap should reduce confidence. Run them on every change to features or fitting.
- **Tests with random seeds ignored.** Results that move between runs make comparisons meaningless. Fix seeds in tests and record the seed used for a published fit.
- **Synthetic fixtures that are too clean.** Real data has gaps, duplicates, late entries, edited marks and students in several classes. Fixtures without any of that pass while production breaks.
- **No monitoring after release.** A model can pass every offline check and still degrade as the term goes on. Watch the distribution of outputs, the share of unknown results, the rate at which teachers dismiss suggestions, and the lag between evidence arriving and an estimate being updated. Sudden changes in any of these are worth a look before anyone complains.
- **Silent task failures.** A Celery task that fails and is swallowed leaves old estimates in place indefinitely. Make failures visible, and make the age of the newest estimate something that alerts when it grows.
- **Reviewing the code but not the data.** A change to a feature function can look harmless and shift every output. Review should include a comparison of outputs on a fixed sample before and after, not only the diff.
- **Reproducing a complaint with the current model.** When a teacher reports an odd estimate from last week, the model and data may have changed since. Keep enough history to rebuild what was shown: the artifact identity, the evidence, and the mapping that applied then.
- **Fixing the symptom in the interface.** If the output looks wrong for a student, it is tempting to patch the display. Find out first whether the cause is evidence, mapping, features, stale index, or the model itself. Each has a different fix, and patching the display hides the others.

If you change anything in mastery-model and cannot say how you would notice it getting worse, stop and add that check first.
