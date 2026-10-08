---
id: 01KP2FQMFWSP87GMNCMCEFB3H1
created: 2026-04-13T00:56-03:00
sources:
  - "code: ingest/tasks.py"
---

# standards-ingest reference

This is the working reference for standards-ingest, the part of ClassroomCompass that brings curriculum standards into the system. Everything else the product does for secondary school teachers hangs off the standards: progress tracking is measured against them, and the next-exercise suggestions are chosen by matching exercises to the standards a student has not yet met. If the standards are wrong, stale or duplicated, every screen downstream is wrong in a way that looks plausible, which is the worst kind of wrong. Read this before touching the job, and keep it up to date when behaviour changes.

The short version: the standards-ingest job is the Celery task `import_standards`, defined in `ingest/tasks.py`. Everything below is context around that task: what it is for, what it touches, how it fails, and what to check when someone says "the standards look off".

## What the job is

The standards-ingest job is the Celery task `import_standards`, defined in `ingest/tasks.py`. That is the single entry point. If you are searching the code for where standards get loaded, start in that file and follow the calls outward. Other code may trigger the task (an admin action, a scheduled beat entry, a management command wrapper), but the work itself happens inside that task and the helpers it calls.

The task is a Celery task, so it runs in a worker process, not in the Django web process. That matters in practice. A web request never waits on it. Whatever starts it gets back a task handle and moves on. Anything you want to observe about a run has to be observed through the worker logs, the task result backend, or the data the task leaves behind in the database and in the search index.

The name `import_standards` is what you see in worker logs, in monitoring, and in any queue inspection tooling. When someone reports a problem, ask them for the task name and the time of the run, and you can usually find the relevant log lines quickly.

## Why it exists

Curriculum standards are published by outside bodies. They change on the publisher's schedule, not ours. Schools adopt a revision at different times, and teachers need to see the version their school uses. We cannot hand-type standards, and we do not want teachers uploading spreadsheets of their own. So there is a job that reads an upstream source, normalises it into our shape, and keeps our copy in step.

The job is deliberately boring. It does not interpret standards, rank them, or decide which exercises fit. It loads, normalises, stores and indexes. The interesting logic, such as the scikit-learn based matching of exercises to standards, lives elsewhere and consumes what this job produces. Keeping the job boring is a design goal: when something is wrong with a suggestion, we want to be able to rule the ingest out quickly.

## Where it sits in the system

Django owns the relational data model for standards. The task writes through the Django ORM, so model validation and signals apply the same way they would for any other write. Do not bypass the ORM with raw SQL in this job; other parts of the app rely on signals and on model-level constraints being honoured.

Celery is the execution layer. The task is routed to a worker like any other; the broker and result backend are shared with the rest of the application. Long-running ingest work should not starve the queues used for interactive things such as recalculating a student's progress after a teacher records an assessment. If you change routing, check that interactive tasks still get picked up promptly while an ingest is running.

Elasticsearch holds the searchable copy of the standards. Teachers search and filter standards in the Vue.js front end, and those requests are served from the index, not from the relational tables. So a successful ingest means two things were updated: the database rows and the index documents. A run can succeed at one and fail at the other, and that is the most common source of confusing reports.

scikit-learn is a downstream consumer. The exercise-suggestion code builds features from standard text and metadata. A change in how the ingest normalises text can shift those features and change suggestions without any change to the suggestion code. Treat normalisation changes as changes to the recommender, and tell whoever owns that part before merging.

## What the task does, step by step

At a high level the run goes in this order. Fetch the upstream source. Parse it into records. Normalise each record. Compare against what is stored. Write the differences. Update the search index. Record the outcome.

Fetching is the step most exposed to the outside world. It can time out, return partial content, or return something that is no longer in the format we expect. The task should fail loudly in those cases rather than treat an empty or truncated response as "all standards were removed".

Parsing turns the upstream format into plain records. Parsing problems on a single record should be logged with enough detail to find the record, and should not abort the whole run unless the problem makes the rest of the data untrustworthy.

Normalisation puts text and identifiers into our conventions: whitespace, casing where relevant, the way hierarchy is expressed, and the mapping of the publisher's grouping onto ours. This is where most of the subtle bugs have come from, because small changes alter what counts as "the same record" on the next run.

Comparison decides, for each record, whether it is new, changed, unchanged or gone. Writing applies only the differences. Index update then reflects the same differences. Finally the task records what happened so someone can answer "what did the last run do" without reading logs.

## Idempotency and re-runs

The task is meant to be safe to run again. Running it twice in a row against the same upstream data should leave the database and the index in the same state as running it once. Several design choices follow from this: records are matched on a stable key from the publisher, not on our own row ids; writes are upserts in spirit; and the index update is driven by the same difference set as the database write.

This property is what lets us recover from almost every failure by simply running the task again. If you add a step, hold yourself to the same standard. A step that appends, increments, or sends a notification each time it runs will break the re-run story.

Be careful with anything that deletes. A standard that has disappeared upstream is not necessarily one we should remove, because students may already have progress recorded against it. The safe treatment is to mark it as retired or superseded so existing progress still resolves, and to stop offering it for new work. Hard deletion of a standard with recorded progress is not acceptable.

## Failure modes seen so far

Partial upstream content. The source responds, but with less than it should. If the comparison step trusts it, a large share of standards look removed. The guard is a sanity check on the size of the change before applying it, and a refusal to apply when the change looks implausible.

Database written, index not. The ORM writes succeed, then the Elasticsearch update fails or is interrupted. Teachers then see old text in search while detail pages show new text. The fix is to re-run the task, which re-derives the index update from the stored state. If that does not repair it, a full reindex of the standards is the heavier option.

Index written, database rolled back. This should not happen if the ordering is respected, but it can if someone moves the index update inside the transaction. Keep the index update after the database commit.

Duplicate-looking standards. Usually caused by a change in normalisation that altered the matching key, so the next run sees every record as new and creates a second copy. If you must change normalisation, plan the migration of keys at the same time.

Worker restarts mid-run. Celery may redeliver the task depending on acknowledgement settings. Because the task is idempotent this is acceptable, but two copies of it running at once are not. See the section on concurrency.

## Concurrency

Only one run of `import_standards` should be active at a time. Two overlapping runs can interleave their writes and produce an index that matches neither. Prevent overlap with a lock around the task body, and make sure the lock has an expiry so a crashed worker does not block ingest forever. When investigating a report that nothing has been ingesting for a while, a stuck lock is one of the first things to look at.

Manual triggers and scheduled triggers go through the same task, so the lock covers both. If someone triggers a run by hand while a scheduled run is in progress, the second should exit quickly and say why, not queue up behind the first and run straight after for no reason.

## Scheduling and triggering

The task runs on a schedule from Celery beat, and can also be triggered by hand when a new revision of a standards set is published and we do not want to wait. Standards change rarely, so the schedule does not need to be frequent. Running it more often than the publisher updates adds load and log noise without benefit.

Avoid running it during peak teaching hours if the run is heavy on the search cluster. Index updates compete with teacher searches, and a slow search during a lesson is noticed immediately. Off-peak scheduling is the cheap fix.

When triggering by hand in a production environment, tell the team first. A run can change what teachers see, and if the upstream data has a problem it is better to have someone watching.

## Configuration

Configuration follows the rest of the Django project: settings come from the environment, with defaults for local development. The things that vary between environments are the upstream source location and any credentials for it, the Elasticsearch connection and the name of the standards index, and the Celery routing and schedule settings. Keep secrets out of the repository and out of task arguments, since task arguments can end up in logs and in the result backend.

If you add a setting, document its meaning here and give it a safe default. A setting that silently falls back to a production value in a development environment is a way to damage real data from a laptop.

## Observability

What to look at, in order, when a run seems wrong. First the worker log for the task name `import_standards`: did it start, did it finish, did it log warnings about records. Second the task result, which should tell you success or failure and the summary counts of new, changed, unchanged and retired records. Third the data itself: pick a standard that should have changed and compare the database row with the indexed document.

The summary counts are the most useful single signal. A normal run after a small upstream revision shows mostly unchanged records. A run where nearly everything is new or changed is almost always a normalisation or key problem, not a real upstream change. A run where a large share is retired is almost always a fetch problem.

Keep log messages specific. "Failed to parse record" is useless; "failed to parse record with this publisher key, reason" gets fixed. Do not log whole payloads, as they are large and noisy.

## Testing

Tests for the job should not call the real upstream source or a real search cluster. Use recorded sample payloads for parsing and normalisation tests, a test database for the write path, and a fake or test index client for the index step. Celery should run tasks eagerly in tests so the task body can be called and asserted on directly.

The cases worth keeping permanently: an unchanged re-run produces no writes; a changed record is updated and re-indexed; a record that vanishes upstream is retired rather than deleted and its progress still resolves; a truncated upstream response is rejected; and a failure in the index step leaves the database consistent and the next run repairs the index.

Add a regression test for every bug found in production. Most of the painful ones came from normalisation edge cases that nobody thought of until real data hit them.

## Working on it safely

Make changes in small steps and run the task against a copy of the data before anything reaches production. Compare the summary counts before and after. If a change is meant to be behaviour-neutral, the counts for an unchanged upstream should be all unchanged.

Do not rename the task or move it out of `ingest/tasks.py` without updating the beat schedule, any routing rules, monitoring that matches on the task name, and anything that triggers it by name. Celery resolves tasks by name at runtime, so a rename breaks scheduled runs silently: the schedule keeps sending a name that no worker recognises, and the only symptom is that nothing is ingested.

Tell the recommender owner about any change that affects the text or metadata of standards. Tell the front end owner about any change to the shape of indexed documents, since the Vue.js search screens depend on it.

## Open questions and known gaps

The retirement behaviour is correct but the front end does not yet explain to a teacher why a retired standard still appears on a student's history. That is a product question more than an ingest one, but it comes up in support.

There is no automatic alert when a run applies an unusually large set of changes; today someone has to read the summary. An alert on implausible change size would catch the fetch problems earlier than a teacher does.

Reindexing everything from scratch is possible but is a manual, heavier operation, and it is not yet documented as a runbook. If you have to do it, write down what you did and add it here.

Support for several versions of a standards set side by side is handled in the data model, but the ingest assumes each run covers a coherent set from one source. Ingesting from multiple sources in one run would need a clearer separation of keys, and nobody has designed that yet.

## Quick checklist for a bad-standards report

Find the run: look for the `import_standards` task in the worker log around the time in question. Check the outcome and the summary counts. Check whether another run overlapped or a lock is stuck. Compare a specific standard in the database and in the index. If they differ, re-run the task. If the counts look implausible, suspect the fetch or the normalisation before suspecting the upstream publisher. If the problem is in suggestions rather than standards, confirm the standards are right first, then hand over to the recommender owner with what you found.
