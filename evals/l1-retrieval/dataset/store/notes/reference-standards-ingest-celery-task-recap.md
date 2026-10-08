---
id: 01KY5P276A65B8CQGPXXEN0222
created: 2026-07-22T16:50-03:00
---

# standards-ingest Celery task: loose notes

Rough notes on the Celery task behind standards-ingest, the thing that pulls curriculum standards into ClassroomCompass so progress tracking and exercise suggestions have something to point at. Written from memory of how it behaves, not from a fresh read of the code, so check details against the task module before relying on any of it. I deliberately left out exact settings, names and limits; look those up in the Django settings and the task decorator.

The short version: a teacher never touches this directly. An admin or a scheduled job kicks off an ingest of a standards document set (a national or regional curriculum framework, or a school's own adaptation of one). The task parses the source, normalises each standard into our internal shape, writes or updates rows through the Django ORM, and then pushes the searchable form to Elasticsearch so the Vue.js side can find standards by text and by hierarchy. After that, a downstream step recomputes whatever the scikit-learn suggestion code needs from the standards, and that part is where most of the pain lives.

## What the task does, in order

The entry point is a Celery task that takes a reference to a source batch rather than the raw content. Passing a reference is on purpose: the payload stays small, the broker does not choke on big messages, and the task can be retried without the caller resending anything. The source batch is a database record that points at the uploaded file or the remote feed, plus a status field and some bookkeeping fields for who started it and when.

First stage is fetch and validate. The task loads the batch, flips the status to running, and reads the source. Validation here is shallow: is it a format we know, does it have the top-level structure we expect (subject areas, strands or domains, individual standards, and optional descriptors or performance levels). Anything that fails shallow validation marks the batch as failed with a human-readable reason and stops. It does not retry, because retrying a malformed file is pointless.

Second stage is parse and normalise. Each standard gets a stable external code from the source (the code the curriculum authority uses), a parent link to its strand or domain, a grade band or year range, the text, and sometimes a short label. The external code is the identity key. Everything else can change between versions of a framework, but the code is what we match on. This is the main thing to remember when debugging duplicates: if a source reissues codes, we treat them as new standards, and the old ones stay around until someone retires them.

Third stage is upsert. The task walks the normalised records and does create-or-update keyed on the external code plus the framework it belongs to. It works in chunks inside transactions, so a failure halfway through does not leave a half-written chunk, but it can leave earlier chunks committed. That partial-commit behaviour is intended. The batch record tracks progress so a rerun can skip what is done, though in practice people just rerun the whole thing because upserts are idempotent.

Fourth stage is index. After the database writes, the task sends the changed standards to Elasticsearch in bulk. Indexing is a separate step from the database write and can fail independently; when it does, the database is ahead of the search index, and the fix is to re-run indexing for the batch rather than re-parse. There is a reindex path for this. If you see standards in the admin that the search box cannot find, this is the first place to look.

Fifth stage is the follow-up. When the batch finishes cleanly, the task fires off a separate task that refreshes whatever derived data the suggestion model uses. It is a separate task so a slow or failing model refresh does not mark the ingest as failed. The ingest status only covers stages up to indexing.

## Celery specifics worth knowing

Queue routing: ingest runs on its own queue, not the default one, so a big ingest cannot starve the lightweight tasks teachers trigger interactively (like refreshing a class view). If ingest seems to be doing nothing, check that a worker is actually consuming that queue before assuming the task is broken. This has bitten more than once in local setups where the worker was started with defaults.

Time limits: the task has a soft limit and a hard limit, both generous but finite. The soft limit raises inside the task so it can mark the batch as failed with a sensible reason and clean up; the hard limit just kills the process and leaves the batch stuck in running. A batch stuck in running with no worker activity almost always means the hard limit hit or the worker was restarted mid-run. There is a housekeeping job that sweeps stale running batches to failed after a while, but it is slow on purpose, so do not wait for it; flip the status by hand in the admin and rerun.

Retries: only transient problems retry, meaning broker hiccups, database connection drops, and Elasticsearch connection or timeout errors. Retries use backoff with jitter and a bounded count taken from configuration. Validation failures and parse failures never retry. When retrying, the task re-reads the batch and relies on idempotent upserts, so a retry after a partial run is safe.

Acks: late acknowledgement is on for this task so that a worker crash returns the message to the queue. The consequence is that a task can run twice if the worker dies after finishing work but before acking. Because of the idempotent upserts and the bulk index being keyed on document id, running twice is harmless. Do not add anything non-idempotent to this task (sending emails, incrementing counters) without thinking about this.

Concurrency: only one ingest per framework should run at a time. There is a lock keyed on the framework, held in the cache backend, with an expiry longer than the usual run. If a second ingest for the same framework is submitted while one is running, the second one backs off and reschedules itself rather than failing. If the lock outlives its owner because of a crash, it expires on its own; if you cannot wait, clear the key. Different frameworks can ingest in parallel, but the Elasticsearch bulk step can get slow if several large ones overlap, so the usual advice is to stagger big ones.

Serialisation: arguments are simple identifiers only. Do not pass model instances or large dicts. This is a rule for the whole project, but it matters most here because the batches can be large.

Result backend: we do not rely on task results for anything user-visible. The batch record is the source of truth for status. The result backend is there mostly for debugging and for Flower-style inspection. If you are tempted to poll the task result from the frontend, poll the batch record instead.

## Data shape and the Elasticsearch side

Each standard document in the index carries the text, the external code, the framework, the subject, the grade band, the ancestry (a flattened path of parent labels so a search can match on strand names), and a few filter fields. The analyzer for the text fields is a language-aware one for the school's instruction language, with a keyword subfield for exact matches on the code. The code subfield matters because teachers often paste a code straight into the search box, and they expect an exact hit at the top.

Index management: the ingest writes into an alias-backed index. For a normal incremental ingest it writes to the live index through the alias. For a full rebuild, there is a path that builds a fresh index, fills it, and swaps the alias, so searches never see an empty or half-built index. The full rebuild is meant for mapping changes. If you change the mapping for a field, an incremental ingest will not fix existing documents; you need the rebuild. This is a classic trap and I have seen it described in a couple of places already.

Deletions: the ingest does not delete standards that are missing from a newer source. It marks them as not present in the latest version, and the search filters hide them by default. The reasoning is that student progress records reference standards, and deleting would orphan history. Hard deletion is a manual, admin-only operation, and it also has to remove the document from the index. If a deleted-in-source standard keeps showing up in search, check the flag on the row first, then the index document, since the two can drift apart when indexing failed.

Hierarchy: parents are created before children within a run, and the upsert step orders records accordingly. A source that lists children before parents is fine, because the task sorts, but a source with a dangling parent reference is not: the child is stored without a parent and flagged, and the batch reports a warning. Warnings do not fail the batch. They show up in the batch detail view, and someone should read them, because a dangling parent usually means a truncated source file.

Text normalisation: whitespace is collapsed, odd unicode quotes and dashes are normalised, and markup that sneaks in from word-processor exports is stripped. The original text is kept alongside the cleaned text so a mismatch with the authority's document can be traced. If a standard looks mangled in the UI, compare the two fields before blaming the parser.

## Gotchas and things I would check first

Stuck in running: covered above. Check the worker is alive and consuming the right queue, check the lock, check whether the hard time limit was hit. Then flip the status and rerun.

Duplicates after a framework update: the source changed its codes, or the framework reference on the batch was wrong, so the match key differed. The task cannot tell the difference between a renamed standard and a new one. A cleanup is a manual mapping step, and it should be done before students accumulate progress against the new copies.

Search out of sync: database ahead of the index. Re-run the index step for the batch. If the problem is the mapping, do the rebuild. If it is only some documents, look at the bulk response handling: the task logs per-item failures but does not fail the whole batch for a few bad items, which means a quiet partial index is possible. Look at the logs for that batch and not only at the status.

Suggestions look stale after an ingest: the follow-up refresh is a separate task and may have failed or not been picked up. The suggestion code reads derived data built from the standards, and if that has not been refreshed, new standards will not appear in recommendations even though they are searchable. Check the follow-up task before assuming the ingest lost data.

Memory on large sources: the parser streams where it can, but some formats need the whole document in memory. For very large frameworks the worker memory can spike, and the symptom is a worker killed by the OS with no Python traceback, which looks like a hard time limit but is not. The worker recycling setting helps a bit. If it keeps happening, split the source by subject or by grade band and ingest the pieces separately; the upsert keys make this safe.

Tests: the unit tests exercise parse and normalise on small fixtures and run the upsert against the test database. The task is run eagerly in tests, so locking and retry behaviour are mocked rather than exercised. Anything involving real broker behaviour, late acks, or the alias swap needs a manual run against a dev stack with real services. Do not trust a green test run as evidence the queue routing is right.

Permissions: only staff with the curriculum admin role can start an ingest from the UI. The scheduled variant runs under a service identity. The batch records who started it either way, so an audit question can be answered from the batch table.

Things I am unsure about and should confirm: whether the stale-batch sweeper also releases the framework lock or only changes the status; whether the follow-up refresh is triggered when a batch finishes with warnings or only when it is fully clean; and whether the rebuild path takes the same framework lock as the incremental path. I believe yes to the last one but have not verified it.

## Possible cleanups, not decided

Split the follow-up refresh into its own visible status on the batch so people stop confusing a stale suggestion model with a failed ingest.

Make the bulk index step fail the batch, or at least raise a visible warning, when the share of failed items crosses a configured threshold, instead of only logging.

Add an explicit mapping step for renamed codes so a framework update does not create duplicates silently.

Surface the lock holder in the admin so nobody has to go into the cache to find out why a second ingest is waiting.

None of these are scheduled. They are here so the next person who hits one of the gotchas above knows it has been noticed before.
