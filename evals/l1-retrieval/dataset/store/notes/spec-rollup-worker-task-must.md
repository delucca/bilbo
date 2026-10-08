---
id: 01KQEAXJKQTW3XZ283ZH8DQ29P
created: 2026-04-30T01:38-03:00
---

# rollup-worker spec

This note specifies how rollup-worker behaves in ClassroomCompass. It is the Celery worker that turns raw student activity into progress figures against curriculum standards, so teachers see current numbers and the exercise suggester has something to read. The one hard rule so far is the time limit on each task. The rest is general and should be tightened when the details are settled.

## Purpose

rollup-worker takes the per-student results that Django writes when a student finishes an exercise and combines them into progress summaries. A summary says, for one student and one curriculum standard, how far along the student is. Teachers see these on the Vue.js dashboard. The scikit-learn suggestion step reads the same summaries to pick next exercises, and Elasticsearch holds the indexed copy used for search and filtering on the dashboard.

The worker does not decide which exercise comes next. It only produces and refreshes the summaries. Keeping that split makes it easy to rerun a rollup without side effects on suggestions.

## Time limit

Each rollup-worker task must have a `soft_time_limit` of `600 seconds`. This applies to every task the worker defines, not only the longest one. A task that reaches the limit gets the soft timeout exception inside its own code, so it can stop cleanly instead of being killed in the middle of a write.

What a task should do when the soft limit fires:

- Stop taking new work from its batch.
- Finish or roll back the write it is in the middle of, so no summary is left half updated.
- Report which part of the batch is not done, so the remainder can be queued again.

If a batch cannot finish inside `600 seconds`, the answer is to split the batch into smaller tasks, not to raise the limit. A task that regularly hits the limit is a sign the batch is too large or a query is slow, and that should be looked at first.

## Task design

Tasks should be safe to run twice. A rollup for a student and a standard is computed from the stored results, not added to a running total, so a repeated or retried task gives the same answer. This matters because Celery may redeliver a task after a worker restart or after the soft timeout.

Batches are grouped by class or by school so that one slow group does not hold up the others. Each task writes its summaries to the database first and updates Elasticsearch after that. If the index update fails, the database is still right and the next run fixes the index.

## Failure handling

Failures are logged with the student group and standard involved, so a teacher complaint can be traced to a task. Retries are limited and spaced out. A task that keeps failing is left for a person to look at and does not retry forever, since a bad record would otherwise block the queue.

A soft timeout is not treated as a plain failure. It is recorded as a partial result, and the unfinished remainder is queued as new, smaller work.

## Open points

- Whether the dashboard should show that a summary is still being refreshed after a partial result.
- How the batch size is chosen in practice, and whether it should adapt to load.
- Whether the Elasticsearch update should be its own task with its own handling.
- Monitoring: how we notice that tasks are often ending on the soft limit.

Anything beyond the time limit above is a working assumption until confirmed against the code.
