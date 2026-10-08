---
id: 01KY2S0P028Q297BB0A71F8KMA
created: 2026-07-21T13:44-03:00
---

# progress-api: common mistakes

These are the mistakes people keep making around progress-api. None of them is exotic. Most come from treating progress-api as a thin CRUD layer over student records, when it sits in the middle of several moving parts: Django models, Celery workers, scikit-learn models that rank exercises, and Elasticsearch for lookups. Read this before changing anything that touches progress data or exercise suggestions.

## Treating progress as a single stored value

The most common mistake is assuming a student has one progress number per standard that you can read and write directly. Progress against a curriculum standard is built from evidence: completed exercises, teacher overrides, and sometimes imported results. If you write a summary value without going through the code that records the evidence, the next recompute will quietly overwrite your change and nobody will know why the number moved back.

Related habits that cause trouble:

- Editing a student's standing in the Django admin or a shell to "fix" a case for a teacher. The fix looks fine until the background recompute runs. Record a teacher override as an override, so it survives.
- Assuming the standards list is static. Curriculum standards get revised, merged and retired. Code that hardcodes a standard's identity, or caches the list for a long time, breaks when the curriculum is updated mid-year.
- Mixing up a standard and the exercises tied to it. One exercise can touch several standards, and one standard has many exercises. Counting completions per exercise and calling it progress per standard double counts or undercounts.
- Forgetting that a class, a student and a teacher are separate things. A student can move between classes, and a teacher can see only some of them. Queries in progress-api that skip the permission filter will leak other classes' data. This is the mistake that matters most for a school product, so check it on every new endpoint, including the ones that look internal.

## Doing slow work inside the request

Anything that recalculates progress for a whole class, or reruns the suggestion model, belongs in a Celery task, not in the request handler. People get this right at first and then break it by adding "just one small recompute" to an update endpoint. It works on a test class with a handful of students and falls over for a real teacher with several classes.

Things to watch for:

- A handler that calls the suggestion logic directly because it was convenient. The model load alone can make the request slow, and under load the web workers get tied up.
- Tasks that are not safe to run twice. Celery can deliver a task more than once, and a worker can die halfway. A task that appends evidence or increments a counter will then double apply. Write tasks so that a repeat gives the same result: compute from the source data and set, do not add on top.
- Firing a task inside a database transaction before it commits. The worker starts, looks for the row, and does not find it, or reads the old state. Queue the task after the commit.
- Passing whole objects as task arguments. Pass identifiers and reload in the worker, so the task sees current data and the message stays small.
- Assuming the result is ready right after the call returns. The teacher UI in Vue.js has to cope with progress that is still being recomputed. If the API returns stale data without saying so, the teacher will think the student did not do the work. Make the response show that an update is pending, and make the frontend show it.
- No retry policy, or an infinite one. A transient Elasticsearch outage should delay a task, not lose it, and a bad record should not loop forever and block the queue for everyone else.

## Search, models and keeping them in step

Elasticsearch is a copy of the data, not the source of truth. Django's database is. Mistakes here are about letting the two drift apart.

- Writing to the database and forgetting to update the index, or the reverse. Reads through search then show something different from reads through the ORM, and the bug report says "the teacher sees two different answers". Route all writes through the one place that updates both, and make reindexing something you can run again safely.
- Changing a mapping or analyzer and expecting existing documents to follow. They do not. Plan a rebuild into a fresh index and a switch, not an in-place edit.
- Using search results as a permission check. Search can return documents the user should not see if the filter is missing from the query. Apply the class and teacher filter in every query, not only in the serializer after the fact.
- Treating the index as always available. Decide what progress-api does when search is down: fail clearly, or fall back to the database for the simple cases. Do not return an empty list that looks like "no results".

The scikit-learn side has its own traps. The suggestion model is trained on past data and loaded by workers. Typical errors:

- Retraining or swapping the model file without thinking about workers that already loaded the old one. Some workers will serve old suggestions until restarted.
- Feeding the model features built differently in training and in serving. A renamed standard, a new field, or a changed scale gives odd suggestions with no error at all. Keep the feature building in one shared function.
- Evaluating on data that includes the same students as training. The numbers look good and the suggestions are poor for new students and new classes.
- Not handling a student with little or no history. The model has nothing to go on, and a default that suggests the same exercise to everyone is worse than saying there is not enough evidence yet.
- Treating a suggestion as an instruction. It is a hint for the teacher. The API should keep it separate from recorded progress and never write it back as if the student had done something.

A rough picture of how data moves, to keep the order straight:

```
Vue.js -> progress-api (Django) -> Celery -> scikit-learn
                  |                   |
              database  <------->  Elasticsearch
```

Writes go to the database first, tasks do the heavy work, and the index follows. If you change the order, say why in the change.

## Testing and changing it safely

Tests for progress-api often pass while the real behaviour is wrong, because of how they are set up.

- Running Celery tasks eagerly in tests hides ordering and commit problems. Have at least some tests that go through the real queue path, or that check a task is queued only after the commit.
- Using tiny fixtures. Anything involving classes, many standards and many students behaves differently from a fixture with two students. Slow queries and repeated per-row lookups only show up with realistic size, so check the query count on list endpoints.
- Mocking Elasticsearch completely. Then mapping and filter mistakes never get caught. Keep a few tests against a real index.
- Not testing the permission boundary: a teacher asking for a student in someone else's class should be refused, and the test should say so.

When changing the data shape:

- Migrations on progress tables can be large and slow on a real deployment. Think about locking, and avoid a migration that rewrites every row while the app is serving teachers.
- Old clients exist. A Vue.js build cached in a browser may call the API for a while after you deploy. Changing a field name or meaning without a compatible period gives teachers a half broken page.
- Backfills should run as tasks that can be stopped and resumed, and they should be idempotent for the reason given above.

## Habits that help

Before you touch progress-api, find where evidence is recorded, where it is summarized, and where the index is updated. Those three places account for most of the surprises. If a number looks wrong, ask which of the three disagrees before you edit any value by hand. Keep the permission filter in mind on every query. When in doubt about a task, ask whether it is safe to run twice and whether it can run before the data exists. Write down in the change what you assumed about search and the model, since the next person will not be able to tell from the code alone.
