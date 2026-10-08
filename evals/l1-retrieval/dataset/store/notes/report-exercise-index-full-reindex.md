---
id: 01KB1V3JHDQZ6ND895WH9VWAY1
created: 2025-11-27T02:03-03:00
---

# exercise-index: full reindex duration

A full reindex of exercise-index took 42 minutes. That is the one hard timing we have for rebuilding the whole index from scratch, and this note records it along with what it means for how we work on exercise-index. Nothing else here is a measured number; everything else is described in general terms because we did not record exact figures at the time.

## What was measured

The run was a full reindex of exercise-index: every exercise document was rebuilt and written into the Elasticsearch index that backs exercise suggestions in ClassroomCompass. The wall-clock time from start to finish was 42 minutes. This was a full rebuild, not an incremental update, so it is the worst case for the component.

We did not split the time into phases (reading from the database, building documents, bulk writes, refresh). So we cannot say which phase dominates. Treat 42 minutes as a total only.

## Why it matters

Teachers use ClassroomCompass during the school day. The suggestion of next exercises depends on exercise-index being present and reasonably fresh. A rebuild of 42 minutes means that if the index has to be recreated, suggestions are degraded or unavailable for that long unless we keep the old index serving while the new one builds.

It also matters for deploys. Any change that forces a full reindex, such as a mapping change or an analyzer change, carries that cost. We should plan those changes outside teaching hours.

## What touches the index

The pieces involved, in general terms:

- Django holds the source data for exercises and curriculum standards. The index documents are built from it.
- Celery runs the indexing work in the background, so a reindex is a queued job and not a web request.
- Elasticsearch stores exercise-index and answers the search queries.
- scikit-learn is used in the suggestion logic that ranks exercises for a student. It is not part of building the index itself, but its output is combined with search results.
- The Vue.js front end shows the suggestions to teachers and does not talk to the index directly.

## Operating guidance

Do not start a full reindex casually. Before doing one, check whether an incremental update would be enough. Most content changes touch a small number of exercises and do not need a rebuild.

When a full reindex is unavoidable:

- Schedule it outside school hours, with enough margin over the 42 minutes we saw.
- Build into a new index and switch over when it is complete, rather than clearing the live one. This keeps suggestions working during the build.
- Watch the Celery worker for the duration so a stalled task is noticed early.
- Confirm afterwards that document counts look right compared with the source data in Django.

A command sketch for the shape of the work, with a placeholder-free description rather than a real invocation:

```
# full reindex of exercise-index: took 42 minutes
```

The actual management command and its flags are not recorded here. Look them up in the repository before running anything.

## Open questions

- Which phase takes most of the 42 minutes? Needs a timed run with per-phase logging.
- Does the time grow linearly with the number of exercises, or faster? We have only one data point.
- Would more Celery workers or larger bulk batches shorten it? Not tried.
- Did anything else compete for Elasticsearch during the run? Not known.

## Next steps

1. Add simple timing logs per phase to the reindex task, so the next run tells us where the time goes.
2. Record the number of documents next to the duration each time we do a full reindex, so we can compare runs.
3. Check that the alias-based switchover described above is actually how the reindex task works today. If it is not, that is the first thing to fix, since it removes the downtime risk even if the duration stays the same.
4. Revisit this note after the next full reindex and replace the single data point with a small history.

## Caveats

The 42 minutes comes from a single run. Hardware, load on the Elasticsearch cluster, and the size of the exercise set all affect it, and none of them were written down. Use the figure as a rough planning number for exercise-index, not as a guarantee. If a later run differs a lot, update this note instead of adding a second one on the same subject.
