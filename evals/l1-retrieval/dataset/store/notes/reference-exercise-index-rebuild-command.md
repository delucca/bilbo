---
id: 01KGYRDPBSZXFZYY47HXMTK2PF
created: 2026-02-08T10:52-03:00
sources:
  - "code: exercises/management/commands/reindex_exercises.py"
---

# exercise-index reference

Working reference for exercise-index, the Elasticsearch-backed search index of exercises in ClassroomCompass. It exists so the suggestion code and the teacher-facing search can find exercises by curriculum standard, topic, difficulty and similar attributes without hitting the main database for every lookup. Short name in chat, tickets and shell history: `exidx`. It is short for `exercise-index`. If you see `exidx` somewhere, it is the same thing. In this note I use the full name `exercise-index`.

The quick answer most people come here for: to rebuild exercise-index, run `python manage.py reindex_exercises --batch-size 500`. More on when and why below.

## What it is

exercise-index is a derived copy of exercise data. The source of truth is the Django database. Exercises, their standards mappings and their metadata live there, and the index only holds a denormalised, search-friendly projection. Nothing should be written to the index that does not come from the database. If the two disagree, the database wins and the index gets rebuilt.

The index is read by two main consumers. First, the recommender that suggests next exercises for a student, which uses scikit-learn models for ranking but asks Elasticsearch for the candidate set. Second, the Vue.js front end, which lets a teacher browse and filter exercises when planning a lesson or assigning homework. Both go through Django views or services. The front end never talks to Elasticsearch directly.

Because it is a projection, it can be thrown away and recreated. That is the main property to remember when something looks wrong with it.

## Rebuilding

The rebuild command is `python manage.py reindex_exercises --batch-size 500`. Run it from the project root with the Django settings of the environment you want to rebuild, and with the virtualenv active. It reads exercises from the database in batches and sends them to Elasticsearch in bulk requests.

Things worth knowing before running it:

- It is meant to be safe to run against a live system. Searches keep working while it runs, though results can be a bit stale or incomplete for the duration.
- It is idempotent in the sense that running it twice gives the same end state. If it dies halfway, run it again from the start rather than trying to resume.
- The batch size is a trade-off between memory use and request overhead. The value in the command above is the one we use as the default. Lower it if the Elasticsearch node is under memory pressure or bulk requests start being rejected. Raise it only if you have a reason and have watched the cluster while doing so.
- Run it in a shell on a worker or management host, not inside a web request. It takes a while on a full catalogue.

After a rebuild, spot-check by searching for an exercise you know was edited recently and one that was recently deleted. The first should show the new content, the second should be gone.

## When to rebuild

Rebuild after any change that alters what gets indexed or how. The usual triggers:

- A change to the document shape or the field mappings in the indexing code. Mapping changes for existing fields generally cannot be applied in place in Elasticsearch, so a full rebuild is needed.
- A bulk import or bulk edit of exercises that bypassed the normal save path, for example a data migration or a script that used queryset updates.
- A change to the curriculum standards data that exercises are mapped against, if the index stores standards codes or labels.
- Restoring a database backup, or copying data from production into a staging environment.
- Recovering from an Elasticsearch outage or a lost index.
- Search results that are plainly out of step with what the database shows and incremental updates are not catching up.

You do not need a rebuild for ordinary edits of a single exercise. Those go through the incremental path described next.

## Keeping it in sync day to day

Normal edits reach the index through Django signals that queue a Celery task. When a teacher or content author saves or deletes an exercise, a task is queued to update or remove the matching document. This means the index is eventually consistent. Under normal load the delay is small, but if the Celery workers are backed up or down, the index lags and new exercises will not show up in search.

So the first question when someone says an exercise is missing from search is whether the Celery workers are healthy and the queue is draining. Only after that is it worth reaching for a full rebuild. A rebuild when the workers are dead fixes the symptom once and then the problem returns on the next edit.

Bulk operations that use queryset updates, raw SQL or fixtures do not fire the per-object signals. That is the most common reason the index drifts. If you write such a script, either trigger the reindex afterwards or make the script queue the update tasks itself.

## Document contents

Each document represents one exercise. In general terms it carries the identifying fields, the text a teacher would search on, the curriculum standards the exercise is mapped to, a difficulty indication, the subject and year group it targets, and a few flags such as whether it is active or retired. Retired exercises should not be suggested to students, and the query layer filters on that flag instead of relying on deletion alone.

Text fields use analysers suited to the languages we support. If search quality for a particular language is poor, look at the analyser settings before touching the ranking code. Ranking in the recommender is separate from Elasticsearch relevance: Elasticsearch narrows to candidates, then the scikit-learn side reorders them using student progress. Do not try to fix a ranking complaint by tuning index scoring unless you have confirmed the candidate set itself is wrong.

I am deliberately not listing exact field names here. Read the indexing code for the current mapping, since it changes more often than this note would.

## Gotchas

- Index drift after bulk changes is the top cause of confusion. See the sync section. Signals only fire on per-object saves.
- A rebuild does not fix bad source data. If an exercise has a wrong standards mapping in the database, the index will faithfully reproduce the error. Fix the data, then reindex or let the incremental path pick it up.
- Mapping changes that conflict with the existing index will make bulk requests fail with errors about the mapping. That is a sign you need to create a fresh index and rebuild into it, not retry harder.
- Do not edit documents by hand in Elasticsearch to fix something quickly. The next rebuild erases it and nobody will remember why the result changed back.
- Local development setups sometimes run without Elasticsearch. In that case searches return nothing or error out, and the reindex command fails to connect. Check that the service is up before assuming the code is broken.
- Different environments have their own indexes. Rebuilding in staging does nothing for production, and vice versa. Check which settings you are running with before you start.
- The rebuild competes with live traffic for Elasticsearch resources. Avoid running it at the start of a school day when teachers are planning lessons.

## Troubleshooting checklist

When search or suggestions look wrong, go in this order and stop when you find the cause.

First, confirm the exercise is correct and active in the database. Second, confirm Elasticsearch is reachable and the cluster is healthy. Third, confirm the Celery workers are running and the relevant queue is not backed up, and look for failed index tasks in the worker logs. Fourth, fetch the document for that exercise from the index and compare it with the database row. If it is missing or stale, trigger an update for that single exercise if you can. Fifth, if many exercises are affected or you cannot tell the scope, run the full rebuild with `python manage.py reindex_exercises --batch-size 500` and recheck.

If the rebuild itself fails, read the first error rather than the last. Typical causes are connection problems, rejected bulk requests because the batch was too heavy for the node, and mapping conflicts. Lower the batch size for the second kind, fix the cluster for the first, and create a new index for the third.

## Open questions

Things I have not pinned down and would like to see written here once someone has checked:

- Whether the rebuild should switch to an alias-based swap so that a rebuild never leaves a window of partial results.
- Whether a scheduled periodic reconciliation between the database and exercise-index is worth adding, to catch drift from bulk scripts without anyone noticing.
- A clear owner for analyser settings per language, since changes there need a rebuild and nobody currently owns that decision.
- Whether the recommender should degrade gracefully, falling back to a plain database query, when exercise-index is unavailable. Right now it mostly just errors.

If you resolve any of these, update this note instead of adding another one. Keep the name consistent: `exercise-index` in prose, `exidx` only as the short form in chat and tickets.
