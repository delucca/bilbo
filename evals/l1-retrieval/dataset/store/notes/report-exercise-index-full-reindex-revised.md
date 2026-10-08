---
id: 01KGARF2WCNZFR5MMH5357QCSP
created: 2026-01-31T16:28-03:00
---

# exercise-index full reindex time

This note replaces the earlier note about "exercise index full reindex": a full reindex of exercise-index now takes 17 minutes, after bulk settings tuning, where the old figure was 42 minutes.

## Current number

A full reindex of exercise-index takes 17 minutes. That is the figure to quote. The 42 minutes measurement is out of date and should not be used for planning, for estimating maintenance windows, or for telling teachers how long the exercise suggestions might be stale.

## What changed

The only change behind the drop was bulk settings tuning on the Elasticsearch side of exercise-index. No change to the exercise data, the Django models or the Celery task code was part of it. The speedup therefore comes from how the index accepts bulk writes during a rebuild, not from indexing fewer documents or skipping fields.

## Why it matters

ClassroomCompass suggests next exercises from student progress against curriculum standards. Suggestions come out of exercise-index, so while a full reindex runs, results can be partial or old. A shorter rebuild means a shorter window where teachers see stale suggestions, and it makes it cheaper to rebuild after a mapping change or a large import of curriculum standards.

## How the work is split

The reindex runs as a Celery job started from the Django side. It reads exercises and their standard tags, builds documents, and sends them to Elasticsearch in bulk batches. The scikit-learn part, which ranks suggestions, reads from the index at query time and is not part of the rebuild. The Vue.js front end only shows the results and is not touched by a reindex.

## Caveats on the measurement

This is one measurement of a full reindex, taken after the tuning. I did not record a spread over several runs, so treat 17 minutes as a typical value, not a guarantee. Load on the Elasticsearch cluster, the size of the exercise catalogue and other Celery work running at the same time can all move it. If the catalogue grows a lot, measure again.

## What was not measured

I did not break the time down by stage, so I cannot say how much is document building and how much is bulk writing. I also did not test whether the search latency during the rebuild changed after the tuning. Both are open.

## Settings to restore

The bulk tuning is meant for the rebuild. Settings that favour write speed during a reindex are usually not what you want for normal search traffic, so check that the index is put back to its normal serving settings once the job finishes. If a later reindex is slower than 17 minutes, first check that the tuning was applied again and not lost when the index was recreated.

## Next steps

- Re-measure after the next large catalogue change and update this note in place.
- Record per-stage timings so the next speedup has a target.
- Confirm the serving settings are restored after each full rebuild.

## Status

The number to use is 17 minutes. The older note on the full reindex is superseded by this one and should be treated as history only.
