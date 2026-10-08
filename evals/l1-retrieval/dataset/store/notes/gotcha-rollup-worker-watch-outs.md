---
id: 01KBE2PVY5KB8W4G7MQECK8QXR
created: 2025-12-01T20:07-03:00
---

# rollup-worker: things to watch when changing it

These are general traps around rollup-worker. Nothing here is a spec; it is what bites when you touch the component without reading around it first. rollup-worker takes per-student evidence (exercise attempts, teacher marks, assessments) and folds it into progress against curriculum standards. The suggestion logic and the teacher dashboards read what it writes. So a small change in how it aggregates shows up later as a odd recommendation or a strange-looking class view, and nobody connects the two for days.

## Task behaviour and Celery

Treat every task as something that will run twice. Celery with late acknowledgement, worker restarts, broker redelivery and manual retries all produce duplicates. If a change makes a rollup non-idempotent (adding to a counter instead of recomputing from source, appending rows without a natural key), you will get inflated progress after the first redelivery and it will look like students got better overnight. Prefer recomputing from the underlying evidence over incrementing.

Ordering is not guaranteed. Two tasks for the same student and standard can overlap or arrive swapped. Do not assume the later-arriving one is the newer one. If you need "latest wins", compare timestamps or version fields from the data, not arrival order. Check how existing code serializes work per student before adding a new task type; some of it relies on a lock or routing key, and a new task that skips it can race with the old ones.

Be careful with task signatures. Changing arguments of a task while messages are still queued breaks the messages already in flight. Add new arguments as optional with a safe default, deploy, and only later remove the old shape. The same goes for renaming a task: old messages keep the old name and get dropped or fail as unregistered.

Keep tasks small and bounded. A rollup that loads a whole school's history into memory in one go will work on the dev dataset and kill a worker in production. Chunk by student or by class, and make each chunk safe to retry on its own. Watch time limits and retry settings when you make a task slower; a task that now regularly hits the limit gets killed halfway and retried forever.

Do not call task code synchronously from a Django request path to "save a round trip". It ties web latency to worker work and hides failures that should be retried.

## Data and the Django side

rollup-worker shares models with the Django app. A migration that renames or drops a column is a deploy-ordering problem: workers running the old code against the new schema, or the reverse, fail in ways that only show up under load. Make schema changes backwards compatible for one release, in both directions, then clean up.

Watch transactions. Reading evidence, computing, and writing the result should not leave a window where a teacher edit lands in between and gets overwritten. Use the same locking or versioning approach the rest of the code uses, and check that it still holds after your change. Also check that a task does not dispatch follow-up tasks before its own transaction commits; the follow-up may read stale data and silently compute from the old state.

Be careful with how standards are matched. Curricula get revised, standards get merged, split or retired, and old evidence still points at the old ones. A change that assumes one evidence item maps to exactly one current standard will drop or double-count some students. Look at how retired standards are handled before simplifying anything there.

Null and missing values mean different things. A student with no attempts is not a student with zero progress, and an ungraded item is not a failed one. When you change an average or a threshold, check the empty and partial cases explicitly. Rounding and weighting choices are visible to teachers, who will compare numbers with their own gradebooks, so do not change them casually and do not change them quietly.

Time zones and school terms matter. Rollups that bucket by day, week or term depend on the school's calendar, not the server's. Be wary of anything that uses naive datetimes or the worker's local time.

## Downstream consumers: scikit-learn, Elasticsearch, Vue.js

The exercise suggestion code, which uses scikit-learn models, reads rollup output as features. If you change what a rollup field means, the models keep running and keep returning plausible suggestions, just worse ones. There is no crash to warn you. If the meaning, scale or range of an output changes, tell whoever owns the suggestion side and check whether the models need retraining or the feature code needs adjusting. Adding a new field is much safer than reinterpreting an existing one.

Elasticsearch holds a denormalized copy of some progress data for search and filtered views. Updates to it can lag, fail or be partial. When you change what rollup-worker writes, check what gets indexed too, and think about the mapping: a field whose type changes will be rejected for new documents or force a reindex. Make indexing failures visible and retryable; do not swallow them so the database commit looks clean. A database and an index that disagree are hard to notice, because each view looks fine on its own.

The Vue.js front end expects certain shapes from the API that sits on top of rollup output. Removing a field, changing a type or returning null where a number used to be can blank out a dashboard for a teacher mid-lesson. Keep the API output stable, and when it must change, version it or support both shapes for a while.

## Backfills, operations and testing

Any change to rollup logic leaves existing results computed the old way. Decide whether to backfill, and if so plan it: run it in batches, throttle it so it does not starve normal rollups, make it resumable, and keep it away from school hours when teachers are looking at the dashboards. A half-finished backfill leaves some classes on old rules and some on new, and that is confusing to explain.

Do not run a full recompute against production data just to test an idea. Use a small copy or a limited set of classes. Student data is personal data about minors; keep it out of logs, error reports and test fixtures. When logging in a task, log identifiers of the work, not names or marks.

Test with ugly data: duplicate evidence, evidence out of order, a student who changed class, a retired standard, an empty class, a very large class. Tests that only use one tidy student pass for nearly any change. Include a test that runs the same task twice and checks the result is the same. If you touch retries, test a failure partway through, not just success.

Watch the queue after a deploy. A change that makes tasks slower or makes them fan out into more tasks can back up the queue and delay everything else, including work that has nothing to do with your change. Look at queue depth, failure rate and task duration for a while after release, and keep a way to pause the new behaviour without a redeploy if you can.

Finally, read the existing code and comments around the part you are changing. A lot of the odd-looking conditions in rollup-worker are there because of a past incident with real class data, and removing them as cleanup is how the same incident comes back.
