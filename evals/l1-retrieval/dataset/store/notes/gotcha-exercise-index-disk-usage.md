---
id: 01KKFMVFXZ5AQWYW1E2ZQGGP06
created: 2026-03-11T20:49-03:00
---

# exercise-index goes read-only when the disk fills

When disk usage passes the 95% flood-stage watermark, writes to exercise-index fail with index read-only / allow delete (api). The block does not clear itself in every case: after freeing space it must be cleared by hand. Free the space first, then remove the block, then check that writes work again.

## Symptom

Indexing jobs for new or changed exercises start failing. The error text in the Celery task log is `index read-only / allow delete (api)`. Reads and searches on exercise-index keep working, so teachers still see existing exercises and the suggestion screen looks normal. Only new exercises, edits and re-indexing are missing. That makes it easy to miss for days.

## Cause

Elasticsearch watches disk usage on each node. Once usage passes the 95% flood-stage watermark, it marks the indices that live on that node as read-only with delete allowed. Nothing in the app causes this. It is a disk problem that shows up as an indexing problem.

Freeing space alone is not enough. Depending on the Elasticsearch version and setup, the read-only block can stay on the index after usage drops. Assume it is still there until you have checked.

## Fix

1. Free disk space on the node. Old snapshots, old unused indices and logs are the usual candidates. Do not delete exercise-index itself.
2. Confirm usage is back under the watermark.
3. Clear the block on exercise-index through the index settings API.
4. Retry the failed indexing tasks.

```
PUT exercise-index/_settings
{ "index.blocks.read_only_allow_delete": null }
```

## Retrying the failed work

Tasks that hit the block are failed, and some may not be retried automatically. After clearing the block, re-run indexing for exercises changed since the first failure. A full reindex from the Django models is the safe fallback if the window is unclear. The database is the source of truth, so nothing is lost, only delayed.

## Detecting it earlier

Alert on disk usage well before the watermark, not at it. By the time the flood stage is hit, writes are already failing. Also worth alerting on a rise in failed indexing tasks in Celery, since that is the first visible sign.

## Things that do not help

Restarting the Django app or the Celery workers does nothing. The block sits on the index in Elasticsearch, not in the app. Retrying the tasks before clearing the block just fails again with the same error and adds noise to the logs.

## Open points

It is not settled how much headroom exercise-index needs as the curriculum content grows. Check disk capacity whenever a large batch of standards or exercises is imported.
