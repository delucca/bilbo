---
id: 01K3E7KDERK2RCN6BT3W7XCTZM
created: 2025-08-24T11:27-03:00
sources:
  - "code: config/settings/base.py"
---

# mastery-model reference

The current mastery-model artifact is stored at `/var/models/mastery/current.joblib`. Anything that needs the live model reads it from that location. The mastery-model is the part of ClassroomCompass that turns a student's recorded evidence against curriculum standards into an estimate of how well they have mastered each standard. The exercise suggestions are built on those estimates. This note is a quick reference for where things are, how they fit together, and what has gone wrong before. It is not a design document.

## Where the artifact lives and who touches it

The file at `/var/models/mastery/current.joblib` is the one serialized scikit-learn model the running system treats as live. It is a joblib dump, so it is only safe to load with a compatible scikit-learn and a compatible Python. Treat it as a build product, not source. It is not checked into the repository and should not be edited by hand.

Three kinds of process care about it:

- The Django web processes load it to answer requests that need a mastery estimate on the spot, for example when a teacher opens a student page.
- The Celery workers load it for batch work: recomputing estimates after new results come in, and preparing the data behind next-exercise suggestions.
- The retraining job writes a new artifact and then puts it in place as the current one.

The path is a convention shared by all of them. If the path moves, every consumer has to be told. Do not scatter copies of the path in new places. Read it from settings where the existing code already does, and follow that pattern when adding a consumer.

The file lives on a local volume of each host, not in object storage. That means each host that runs web or worker processes has its own copy, and the copies can drift if a rollout is half done. When estimates look different between two pages for the same student, check first that both hosts have the same artifact before suspecting the model itself.

## What the model does

Input is a student's history against a standard: which exercises they attempted, how they went, how recently, and how hard the exercises were relative to the standard. The curriculum standards themselves come from the curriculum data that Django owns. Output is an estimate of mastery per standard, plus enough information for the suggestion step to rank candidate exercises.

The model is deliberately simple and boring. It is a classical scikit-learn pipeline, not a deep model. A preprocessing stage turns raw attempt history into features, and an estimator sits behind it. Both live inside the one serialized object, so loading the file gives you the whole thing, preprocessing included. Do not build features outside the pipeline and feed them in. That breaks the moment the preprocessing changes and the artifact is replaced.

The suggestion step is separate from the mastery-model. It takes the estimates, asks Elasticsearch for exercises tagged with the relevant standards, and ranks them using the estimates plus a few rules about variety and difficulty progression. If suggestions look odd, it is worth working out whether the estimates are wrong or the ranking and search are wrong before touching the model. Most reports of odd suggestions have turned out to be tagging or search problems, not model problems.

The Vue.js front end never talks to the model. It gets estimates and suggestions from Django endpoints. There is no way to load the artifact from the browser and there should not be.

## Training and replacing the artifact

Training runs as a Celery task, on a schedule and on demand. It pulls attempt history and standards data from the database, fits the pipeline, evaluates it against held-out data, and only then writes the result. The point of the ordering is that a bad fit never replaces a good model. If evaluation fails the checks, the task stops and the existing artifact stays in place.

Replacing the artifact should be atomic from a reader's point of view. The job writes the new file next to the current one and then renames it over `/var/models/mastery/current.joblib`. Readers that already have the old model in memory keep using it until they reload. Readers that open the file during the swap get either the old file or the new file, never a half-written one. Do not change this to write in place. A reader that hits a partial joblib file fails with a confusing deserialization error, and it is hard to reproduce afterwards.

After a replacement, long-lived processes do not automatically notice. Web workers and Celery workers load the model into memory and hold it. Depending on how the process is set up, they either reload on a signal or on restart. If you replace the artifact by hand, restart or signal the workers and check that both web and Celery sides picked it up. A common confusion is a retrain that looks like it did nothing because the workers were still serving the previous model.

Keep a way back. Before replacing the artifact by hand, keep the previous file somewhere outside the live directory under a name that says what it is, so that a rollback is a rename and a restart. The automated job already keeps recent previous versions. Check what it keeps before relying on it, because the retention is a setting and not a guarantee.

## Compatibility and loading

Joblib files are tied to the library versions that wrote them. The artifact must be loaded by the same scikit-learn version, or a version known to be compatible, that produced it. When scikit-learn is upgraded in the project, the artifact has to be retrained with the new version and swapped in as part of the same rollout. Otherwise loading either fails outright or, worse, loads with a warning and gives subtly different numbers. Treat any version warning from scikit-learn on load as a real problem, not noise.

Loading is not free. The file can be large enough that loading it on every request is a bad idea. The existing code loads once per process and caches. Keep that. If you add a new entry point such as a management command or a new task, reuse the shared loader rather than calling joblib directly, so there is one place that knows the path, handles a missing file, and logs what was loaded.

If the file is missing or unreadable, the shared loader raises, and callers are expected to degrade rather than crash the page. The web side shows the student page without mastery estimates and logs the failure. The batch side retries later. Do not paper over a missing artifact by returning default estimates, because teachers would read made-up numbers as real progress.

Security note: joblib uses pickle underneath, so loading a file runs code from it. Only load artifacts from the controlled model directory, written by the training job. Never accept an artifact upload from a user or load one from a path that comes from request data.

## Known gotchas

A few things that have bitten people and will probably bite again.

- Two hosts with different artifacts. After a partial rollout, one host serves new estimates and another serves old ones. Symptoms: a teacher refreshes and the numbers jump back and forth. Check the artifact on each host and restart workers.
- Stale in-memory model. After a retrain the file on disk is new, but the processes are still serving the old one. Symptoms: new behaviour does not show up, or a fix seems not to work. Restart or signal the workers.
- Version drift. The environment used for training differs from the one serving, for example after a dependency bump in one image but not another. Symptoms: load warnings, odd numbers, or errors on load. Align the versions and retrain.
- Preprocessing outside the pipeline. Someone computes features in Django code and feeds them to the estimator directly. It works until the pipeline changes. Always pass raw inputs to the pipeline as it expects them.
- New standards. When the curriculum data gains standards that the model did not see in training, the model has little or no evidence for them. Estimates for those standards should be treated as low confidence until a retrain has happened. The suggestion step is supposed to handle low confidence by leaning on simple rules, but check that it does when you add standards in bulk.
- Small groups of evidence. A student with very little history gets estimates that are mostly the prior. That is correct behaviour, but teachers sometimes read it as the model being broken. The interface should say when there is little evidence, and any change there should keep saying it.
- Elasticsearch is not part of the model. Reindexing exercises or changing tags changes suggestions without touching the artifact. Do not retrain to fix a tagging problem.

## Checking that the model is healthy

When something seems off, work from the outside in.

1. Confirm the file exists at `/var/models/mastery/current.joblib` on the host you are looking at, and that it is readable by the user the processes run as. Permission problems after a manual copy are a classic cause of a worker failing to load.
2. Check the modification time against when you expect the last retrain or swap to have happened.
3. Check the logs of the shared loader for what it loaded and any warnings from scikit-learn about versions.
4. Compare a known student's estimates between a web process and a worker process. If they differ, one of them has a stale or different model.
5. Only then look at the training task and its evaluation output, to see whether the last retrain was skipped, failed its checks, or succeeded.

If you need to experiment, load a copy in a separate environment and leave the live file alone. Do not overwrite the live artifact with an experimental model, even briefly, since every consumer will pick it up on its next reload and teachers will see the effects.

## Evaluation and what counts as good enough

The evaluation step in training compares the new fit against held-out attempts and against the currently deployed model. The new model replaces the old one only if it is not meaningfully worse on the checks the team agreed on. The checks cover calibration as well as accuracy, because the interface presents estimates as levels of mastery and a model that is overconfident misleads teachers even when its ranking is fine. If you change the features or the estimator, update the checks together with the change and say in the commit why.

Be wary of evaluation data that leaks. Attempts by the same student appear in both training and held-out sets if the split is done naively, which flatters the model. The existing split groups by student. Keep it that way when touching the training task.

Fairness across classes and schools matters here in a practical way. Different schools use different exercises and different pacing, and a model that only works well for the most common pattern will under-serve the others. When looking at evaluation output, look at it broken down by school type and subject, not just the overall figure.

## Working on it

Changes to the mastery-model fall into a few kinds, and each has a different blast radius.

- Changing features or the estimator: retrain, evaluate, swap the artifact, restart workers. Update this note if the shape of inputs or outputs changes.
- Changing the retraining schedule or retention: this is a Celery configuration change plus a check that disk space on the model volume is still fine.
- Changing the location of the artifact: avoid it. If it is unavoidable, change the setting, update the shared loader, move the file, and restart every consumer in one rollout. Update the path in this note afterwards.
- Upgrading scikit-learn or Python: treat as a model change. Retrain under the new versions, test loading in the serving image, and swap together with the rollout.

Keep tests for the loader and for the pipeline's input contract. Tests should use a small throwaway model built in the test, not the live artifact, so they never depend on what happens to be on a developer's machine or a build host.

## Open questions

Things nobody has settled and that a later session might pick up.

- Whether to move the artifact to shared storage so that all hosts always agree, instead of per-host copies. It would remove the drift problem but adds a dependency at startup.
- Whether workers should check the file's modification time periodically and reload on their own, which would remove the manual restart step after a swap.
- Whether estimates should carry an explicit confidence value through to the interface, so that low-evidence cases are labelled by the model rather than inferred by the front end.
- How to version artifacts so that a stored estimate can be traced back to the model that produced it. Right now the only trace is the time it was computed and the deployment history.
