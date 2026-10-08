---
id: 01KNEJH0EWACC05209C7NAQNTQ
created: 2026-04-05T07:20-03:00
---

# progress-api: general things to watch out for when changing it

Quick working list for anyone touching progress-api. Nothing here is a decision or a spec. These are the places where changes have a habit of going wrong, written down so the next person does not have to find them the slow way. progress-api sits between the teacher-facing Vue.js screens and everything that actually knows about students: the Django models, the Celery workers that recompute things, the scikit-learn based suggestion logic, and the Elasticsearch index used for finding standards and exercises. A change that looks local to the API usually is not. Read the whole note before a larger change, or skim the headings if you are in a hurry.

## What progress-api actually is

It is the read and write surface for student progress against curriculum standards. Teachers see a student, a class, a standard, and a suggested next exercise. All of that comes out of progress-api. It is easy to think of it as a thin CRUD layer over Django models. It is not thin. Some responses are computed on the fly, some are served from stored results that a Celery job produced earlier, and some are a mix of both. The same endpoint can behave differently depending on whether the background work has caught up.

So before changing anything, work out which of three kinds of data the endpoint returns:

- Data written directly by a teacher or by a user action, which is immediately consistent.
- Data derived by a background job, which can be stale for a while.
- Data that comes from the search index, which is a copy and can lag or drift from the database.

Most surprises come from mixing these up. A field that looks like part of the same object can come from different places with different freshness.

A rough picture of the flow, nothing more:

```
progress-api -> Celery -> scikit-learn -> Elasticsearch
```

The arrows are not strictly one direction. Results come back to Django storage, and the index is filled from the database. The picture is only there to remind you that a change at one end has effects at the other.

## Response shape and the Vue.js client

The front end is the first thing that breaks quietly. Vue components often destructure what they get, assume a field exists, and render nothing if it does not. No error shows. A teacher just sees an empty panel and assumes the student has no progress.

Things to watch:

- Renaming a field is a breaking change even if you also keep the old one on a different route. Search the client for the old name before and after.
- Changing a field from always present to sometimes null needs a client change in the same release, or the client has to be made tolerant first.
- Changing the order of items in a list can matter. Some screens assume the list order carries meaning, for example standards grouped in curriculum order, or exercises ranked by suggestion strength. Do not assume order is incidental just because the serializer does not mention it.
- Pagination changes break infinite scroll and class overview tables in ways that only show with a large class.
- Empty versus missing: an empty list, a null, and an absent key are three different things to the client. Keep whichever the client already relies on.
- Error response bodies are also a contract. The client probably reads a message or a field-level structure to show something to the teacher. If you change how validation errors are shaped, check the forms.

If you must change a shape, add the new thing next to the old one first, move the client, then remove the old one in a later change. Do not do it in one step unless you control both deployments and they go out together, which they often do not.

## Django models and migrations

Progress records are the core data and they are many. Migrations on those tables are the riskiest part of any change here.

- Adding a column with a default on a large table can lock it or take long enough to look like an outage. Prefer nullable first, backfill in batches, then tighten.
- Do not mix schema changes and big data backfills in one migration. Run the backfill as a separate step, ideally resumable.
- Removing a column while old code is still running will break the old code. Deploy order matters: stop reading, then stop writing, then drop.
- Celery workers run their own copy of the code. If workers are not restarted at the same time as the web processes, they may run old model code against a new schema, or the reverse. Think about which side deploys first.
- Choices and enumerations stored as strings: adding a value is easy, renaming one is a data migration plus a client change plus an index change.
- Foreign keys to curriculum standards matter. Standards get revised by the school or the curriculum body, and records pointing at old standards must still make sense. Never delete a standard row just because a new revision replaced it. Mark it, map it, keep it.
- Soft-deleted or archived students and classes should stay out of aggregates. Check that your new query respects whatever the default manager already hides, and that it does not use a manager that skips the filter.

Query changes deserve a look at the generated SQL, not just whether tests pass. A harmless-looking serializer field that follows a relation can turn one request into a flood of queries. Class-level endpoints multiply this by every student in the class.

## Celery tasks and background recomputation

A lot of what progress-api shows depends on tasks having run. When you change how a value is computed, you are usually changing a task, and the stored results from the old computation are still sitting in the database.

- A changed computation means old stored results disagree with new ones. Decide whether to recompute everything, recompute lazily on read, or let old values age out. Do not leave it unstated; teachers will see two students with results calculated differently and compare them.
- Tasks must be safe to run twice. Retries, redelivery after a worker dies, and manual re-runs all happen. A task that increments something instead of setting it will drift.
- Tasks must be safe to run out of order. Two updates for the same student can arrive in either order. Use the data in the database as the truth at run time, not the data in the message that was queued earlier.
- Passing whole objects in task arguments is a trap. Pass identifiers and reload. Serialized payloads from an older deploy may reach a newer worker.
- Changing a task name or its argument list while messages are still queued will strand or break those messages. Keep the old signature working until the queue has drained.
- Fan-out tasks that enqueue one task per student can swamp the queue at the start of term or after a bulk import. Think about what happens when the whole school is processed at once.
- Long tasks hold a worker. Keep heavy model work off the queue that handles quick progress updates, otherwise a teacher marks an exercise done and nothing happens for a long time.
- Time limits and retry rules are part of behavior. If you make a task slower, check the limits still fit.

When the API endpoint triggers a task, decide what the response says. Returning stale data with no hint is worse than returning an explicit pending state, as long as the client knows how to show it.

## The suggestion logic and scikit-learn

Suggested next exercises come from a model or a scoring step built with scikit-learn. This is the part where a change is hardest to see and easiest to get wrong, because tests can pass while suggestions get quietly worse.

- Model artifacts and the code that loads them must match. If you change feature preparation, the saved model no longer fits. Loading may succeed and give nonsense, which is the worst outcome.
- Feature order and feature naming matter. A reordered column list can silently produce wrong results with some estimators.
- Library upgrades can change how saved models load or behave. Treat an upgrade of the numeric stack as a model change, not a dependency bump.
- Randomness: if anything uses random sampling, make sure seeding is deliberate. Teachers who reload a page and see different suggestions lose trust quickly.
- Cold start: new students and students with little data must still get something sensible. Check the empty and near-empty cases after any change, not only the typical one.
- Suggestions should respect what the teacher has already assigned or the student has already done. A change in how history is read can bring back exercises that were finished.
- Keep the suggestion step cheap enough to call during a request, or make sure it is only ever read from stored output. Do not introduce model loading inside a request path by accident.
- Fairness across groups of students deserves a thought. A change that nudges suggestions based on a feature correlated with something it should not be can go unnoticed in aggregate numbers.

How the mastery estimation compares between versions, and why it was built the way it was, is written up in [[mastery-model-offline-comparison]]. Read that before changing anything about how mastery is estimated, so you do not repeat an experiment or undo a conclusion without knowing it.

## Elasticsearch index and search

The index is a copy. It is filled from the database, and it can be wrong.

- Changing a mapping usually means building a new index and switching to it. In-place changes to existing field types mostly do not work. Plan for a rebuild and a switch, and plan what the API does while that happens.
- Anything that changes the text analysis, such as analyzers, synonyms, or language handling, changes which standards and exercises are found. Teachers search with curriculum wording and with their own wording. Test with both.
- Documents are written after database changes. If the write to the index fails or is delayed, search and the database disagree. Decide which one the API trusts for each field. Do not show a field from the index when the database has a newer value that the screen also shows.
- Deleted or retired standards must disappear from search results, or be clearly marked. Otherwise teachers pick something that no longer counts.
- Relevance tuning is a side effect machine. Boosting one field can bury another. Check a handful of real queries before and after, written down, not from memory.
- Result counts and pagination from the index can differ from database counts. If a screen shows both, they may not match.
- The index can be rebuilt from the database. Keep it that way. Do not store anything only in the index.

If search is down, the rest of the API should degrade, not fail. Check what happens to progress views that include a search call for decoration.

## Curriculum standards and their versions

Standards are the reference frame for everything. Progress means progress against a standard, so if the meaning of a standard changes, the meaning of old progress changes too.

- Mapping between revisions of a curriculum is rarely one to one. One old standard may split into several, or several merge into one. Decide how progress carries over and where it does not.
- Different schools or regions can follow different curricula. Code that assumes a single framework will work in a demo and fail in a real school.
- Ordering and grouping of standards come from the curriculum, not from alphabetical sort or from the database id.
- Imports of standards should be repeatable and not create duplicates when run again. Match on a stable external key rather than on display text.
- Display text changes are common and mostly cosmetic, but the index and any cached strings need to follow them.
- Prerequisite links between standards, if used by suggestions, can form odd shapes after an import. A loop or a dangling link can hang or distort the suggestion logic.

Any change touching standards should also check the stored aggregates. A class-level mastery figure computed from the old set will not match one computed from the new set, even if no student did anything.

## Permissions, privacy and school data

This is student data about minors, used by teachers inside schools. Treat every change as if it might widen who can see what.

- Every endpoint needs to check that the requesting teacher is allowed to see that class and that student. A new endpoint copied from an old one inherits that check only if you keep it. When you add a filter or a bulk route, test with a teacher from a different class.
- Listing endpoints are the usual leak. The detail route checks access, the list route forgets to filter.
- Search is another leak. Index documents may contain data that the detail route would hide. Filter at query time by what the user may see.
- Exports and bulk downloads need the same rules as the screens. Do not add a convenient dump route without them.
- Logs and error reports: keep student names and free-text notes out of them. Be careful with debug output left in after a bug hunt.
- Background tasks run without a user. They must not become a way to do things a user could not do, such as writing progress for a class the teacher is not part of.
- Data retention and removal requests: if you add a new place where student data is stored, such as a cache, a new table, or a new index field, it must be covered by whatever removes or anonymizes student data. Easy to forget, hard to fix later.
- Teacher notes may contain sensitive free text. Do not feed free text into suggestion features or search without thinking about it.

## Performance and load patterns

Use is bursty and follows the school day. Many teachers open class views at the start of a lesson, and many progress updates arrive near the end. Term start and report deadlines add their own spikes.

- Class-level endpoints are the heavy ones. Check them with a realistic large class and a realistic number of standards, not a small fixture.
- Avoid per-student queries inside loops. Prefetch, aggregate in the database, or read from stored results.
- Caching helps but ties you to invalidation. If you cache a value that a Celery job updates, make sure the job invalidates it, or accept a stated staleness. An unclear cache is how two screens show different numbers for the same student.
- Do not compute aggregates in Python over full tables when the database can do it.
- Expensive endpoints should be limited in what a client can ask for. Unbounded page sizes or unbounded date ranges will eventually be used.
- Index queries have their own cost. Wildcard-heavy or very broad queries on the index slow everything for everyone.
- Watch memory in workers that load model artifacts. Several worker processes each holding a copy can add up.

When something is slow, measure first. The cause in this component is more often a query pattern or a queue backlog than the code you were about to optimize.

## Testing and checking a change

Tests here tend to pass on a clean fixture and miss what real data does. Some habits that help:

- Test with students who have no progress, a little progress, and a lot of progress. Also with students who moved class, with retired standards, and with archived records.
- Test the API together with the background step, not only each in isolation. Many bugs live in the handoff, for example a task reading data before the transaction that created it has committed.
- Check that tasks are triggered after commit, not inside an open transaction.
- Test running a task twice and out of order.
- Compare suggestion output before and after on a saved set of realistic students, and read the differences by eye. Aggregate scores will not tell you if teachers would find the new ones odd.
- Run migrations against a copy of realistic data volume, not an empty database. Also check that they can be applied while the old code still serves requests.
- Try the Vue.js screens that consume the changed route, by hand, once. Contract tests do not catch everything a screen does with the data.
- Time zones and school terms: anything that groups by day, week or term can shift at boundaries. Test around a boundary.

If a test needs a model artifact or an index, make sure it fails loudly when that is missing instead of skipping and reporting green.

## Deployment order and rollback

progress-api is not deployed alone. Web processes, Celery workers, the model artifacts, the index, and the Vue.js client all move on their own schedules.

- Write the change so that old and new versions can run side by side for a while. That is the normal state during a rollout.
- Typical safe order: additive schema change, code that can read both, switch writes, backfill, then remove the old path. Skipping steps works until the one time it does not.
- Know how to go back. A migration that drops or rewrites data cannot be undone by redeploying the old code. If a change cannot be rolled back, say so in the review and prepare a plan before shipping.
- Model artifacts should be versioned and the code should be explicit about which one it expects. Rolling back code without rolling back the artifact, or the other way, gives the quiet kind of failure.
- Index rebuilds need a switch step and a way to go back to the previous index for a while.
- Feature flags help for suggestion changes, because you can turn them off without a deploy. Make sure the flag is read by workers as well as web processes.
- Do not deploy big changes before a school day starts, or during report deadlines. Teachers cannot tell you something is broken until they are in the middle of a lesson.
- After the deploy, watch the queue backlog and the error rate for a while. A slow queue is often the first visible sign.

## Smaller traps that keep coming back

A loose collection of things that do not fit above but cost time before.

- Rounding and display: mastery shown as a percent, a band, or a color is a presentation of an underlying value. Changing the cutoffs changes how every student looks, without any data change. Teachers notice and ask.
- Naming drift: the same idea is called progress, mastery, attainment and level in different places. Do not merge or rename them in a quick cleanup without checking the client, the index and the reports.
- Defaults in serializers and in models can disagree. The value a new record gets may depend on which path created it.
- Bulk update helpers in Django skip signals and per-object save logic. If something else relies on a signal to trigger a task or an index update, a bulk write will skip it.
- Management scripts and one-off fixes written under pressure tend to stay. Check whether one exists before writing another that does almost the same, and delete the ones that are obsolete.
- Settings that differ between environments, such as queue routing, index names and feature flags, are a common reason that something works locally and not elsewhere.
- Third-party integrations such as school information systems can send partial or repeated data. Do not assume a payload is complete, unique or in order.
- Comments and docstrings in this area are sometimes out of date. Trust the code and the data, then fix the comment while you are there.
- Leave a short note here when you hit something new, in plain words, with the area it affects. That is the point of this file.
