---
id: 01M24HBJATHJ65723A0ZCZKSEY
created: 2026-09-09T23:11-03:00
---

# Exercise index rebuild command (notes)

Loose notes on rebuilding the `exercise-index` from scratch. Written from memory of how it behaves, not from rereading the code, so check details against the repo before relying on any of it. There may already be a tidier note on the same thing.

The `exercise-index` is the Elasticsearch side of ClassroomCompass. It holds the exercises that the suggestion step draws from when a teacher asks what a student should do next against a curriculum standard. The rebuild command is a Django management command that drops what is in the index and refills it from the database. It is the thing to reach for when search results look stale or the mapping has changed.

## What the rebuild does

It reads exercises from the Django models, builds one document per exercise, and bulk-writes them into Elasticsearch. Each document carries the standard tags, the difficulty band, the subject and year group, and the text fields used for matching. Afterwards the alias is pointed at the fresh index and the old one is removed. The idea is that readers never see a half-built index, but that depends on the alias swap actually happening, so watch for it.

## When to run it

- After a change to the mapping or analyzers for `exercise-index`.
- After a bulk import of exercises or a curriculum standards update.
- When teachers report that a newly added exercise does not show up in suggestions.
- After restoring a database from backup, since the index may be ahead of or behind the data.

Do not run it casually during school hours. The bulk write is heavy enough that suggestion requests slow down while it runs, even with the alias approach.

## Running it

Run it from the Django project root, in the same environment the web app uses. The usual flags are a dry run option and a batch size option; I do not remember the exact spellings, so use the command's help output. The rough flow:

```
exercise-index: rebuild -> Elasticsearch
```

If Celery workers are running, consider pausing the tasks that touch `exercise-index` first, otherwise they can write to the old index while the new one is being built and those writes get lost.

## Celery interaction

There are Celery tasks that update single documents when an exercise is saved. During a rebuild those tasks still fire. Two cases matter:

- A task writes to the alias that still points at the old index. The write is lost once the swap happens.
- A task fires for an exercise that the rebuild has already passed over. That one is fine, it just lands in the new index after the swap.

The safe approach is to drain the queue, run the rebuild, then run a short catch-up for anything saved during the window. I have not seen a built-in catch-up; it may be worth adding.

## Relationship to scikit-learn

The suggestion ranking uses scikit-learn models, and some of the features come from fields stored in `exercise-index`. If the rebuild changes how a field is computed, the model's inputs shift and suggestions can change even though the model itself is untouched. After a rebuild that changes fields, retrain or at least sanity check suggestions for a few known students before telling teachers it is done.

## Failure modes seen

- Elasticsearch unreachable or the cluster is yellow or red: the command fails early or partway. Fix the cluster first, then rerun, because the command is meant to be safe to repeat.
- Mapping conflict on a field whose type changed: the bulk write rejects documents. Look at the first few rejected items rather than the summary count.
- Memory pressure on the Django side when the batch size is too large. Lower it to the configured smaller value and retry.
- Timeouts on the bulk requests if the cluster is busy. The configured timeout can be raised, but a busy cluster usually means the run should wait.

## Checking the result

After it finishes, compare the document count in `exercise-index` with the number of exercises in the database. They should match except for exercises that are archived or hidden, which the build skips. Then open the Vue.js exercise browser and search for a couple of exercises you know, including one that was added recently. Also check that a suggestion request for a student with known gaps returns something sensible.

## Rollback

If the new index is bad and the old one was not yet removed, repoint the alias back. If the old one was already removed, the only way back is another rebuild from the database, which is why the database is the source of truth and the index is never edited by hand.

## Open questions

- Whether the rebuild should hold a lock so two runs cannot overlap. I think it does not.
- Whether the catch-up for tasks that fired during the run should be part of the command.
- Whether the dry run really skips the alias swap or only skips the delete. Read the code before trusting it.
- Whether the schedule, if any, for automatic rebuilds is still enabled in Celery beat. Worth confirming so nobody is surprised by a rebuild at an awkward time.

## Related

The Django models for exercises and standards are the input. The Celery tasks for single document updates are the other writers. The Vue.js browser and the suggestion endpoint are the readers. Any note about the mapping for `exercise-index` should be read together with this one.
