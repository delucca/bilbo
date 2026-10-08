---
id: 01KK8NWYX1P29QNRMZQV5B1QWM
created: 2026-03-09T03:52-03:00
---

# standards-ingest design

standards-ingest is the part of ClassroomCompass that pulls curriculum standards frameworks from outside publishers, normalizes them, and makes them available to the rest of the app. Progress tracking and exercise suggestions both depend on it, so a stale or half-loaded framework shows up as wrong suggestions for teachers. This note records how it is put together and why, written quickly while the details are fresh.

## Purpose

Secondary school teachers map student work to standards. Those standards come from published frameworks that change from year to year. standards-ingest fetches each framework, turns it into our internal shape (framework, strand, standard, level descriptors), and stores it in the Django database. It then pushes the standards into Elasticsearch so teachers can search them by wording or code.

## Flow

A Celery task per framework does the work. Roughly:

- Download the framework document from the publisher.
- Parse it into standards records. Parsing is strict: a document that does not match the expected layout is rejected as a whole, not partly loaded.
- Upsert into the database inside one transaction, keyed on the publisher's own standard code, so reruns do not create duplicates.
- Queue an Elasticsearch reindex for the changed standards only.
- Queue a refresh of the features the scikit-learn exercise suggester uses, since it relies on standard text and level descriptors.

The ingest task does not compute any per-student numbers. Those belong to the rollup side; see [[rollup-worker-task-must]] for what that worker needs from us.

## Retry behaviour

A failed framework download is retried with max_retries=3 and exponential backoff. The backoff matters because publisher sites are often slow or rate limiting at the start of a school term, when everyone refreshes at once. After the last retry fails, the task gives up, logs the framework and the final error, and leaves the previously stored version of that framework untouched. Only the download step retries. Parse errors and database errors fail straight away, since repeating them gives the same result.

## Why these choices

Keeping the old version on failure is deliberate. A teacher with a stale but complete framework is better off than one with a partial framework. Strict parsing and one transaction per framework come from the same reasoning: either the whole framework updates or nothing does.

Exponential backoff was picked over a fixed delay so that many framework tasks failing together do not all hit the publisher again at the same moment.

## Gotchas

- Retrying the download does not help when the publisher has moved or renamed the document. That looks like a repeated failure but needs a config fix, not more retries.
- Standard codes from publishers are not always stable across years. The upsert key assumes they are within one framework version. A new version should be ingested as a new framework, not over the old one.
- The Elasticsearch reindex is queued after the database commit. If it fails, the database is right and search is stale. Rerunning the reindex fixes it without redownloading.

## Open questions

- Whether to alert someone when a framework has failed all its retries, rather than only logging. Right now nobody finds out until a teacher complains.
- Whether the suggester feature refresh should wait for the reindex to finish. At the moment the two run independently and that has not caused trouble.
- How to handle publishers that offer only a PDF. The current parser assumes a structured document.

## Where to look

The ingest tasks live with the other Celery tasks in the Django project. Start from the framework download task and follow the calls through parse, upsert and reindex. Retry settings are on the download task itself, not in a global Celery config.
