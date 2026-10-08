---
id: 01KMJXKKJBEJ6RYRCZE69BZ41H
created: 2026-03-25T13:35-03:00
---

# standards-ingest download retries

This note replaces the earlier note "standards ingest retries failed". The new value is `max_retries=5` for failed framework downloads in standards-ingest. The older setting was max_retries=3 and no longer applies.

## Scope

standards-ingest pulls curriculum framework documents into ClassroomCompass so progress can be tracked against standards. This note covers only how it retries a download that fails. It says nothing about parsing or indexing.

## Current setting

Failed framework downloads in standards-ingest are retried up to `max_retries=5` times. After that the task gives up and reports failure. Anyone reading only this note should treat `max_retries=5` as the live value.

## Previous setting

The old value was max_retries=3. Any doc, comment or dashboard that still mentions three attempts is out of date.

## Why the change

Framework sources are sometimes slow or briefly unavailable. Three attempts was not always enough to ride out a short outage, so whole ingest runs failed for reasons that cleared up soon after. Five gives more room without changing the rest of the flow.

## Where it runs

The download step runs as a Celery task. The retry count belongs to that task, so a retry re-queues the same task rather than restarting the whole ingest.

## What counts as a failure

Network errors, timeouts and bad HTTP responses from the framework source count as failed downloads and are retried. A document that downloads fine but fails validation is not a download failure and is not retried by this setting.

## Backoff

Retries wait between attempts. This note does not change the backoff, only the number of attempts. Check the task definition for the current delay behaviour before assuming anything about timing.

## Worst case duration

More retries means a failing source takes longer to be declared failed. Expect a stuck download to occupy a worker for longer than before. Watch queue depth if many sources fail together.

## Effect on Django side

The Django app shows ingest status to staff. A source that is still retrying looks like it is in progress. It only flips to failed once all retries are used up.

## Effect on Elasticsearch

Nothing is indexed from a download that never succeeded. Standards already in Elasticsearch stay as they were, so teachers keep seeing the last good data while retries run.

## Effect on suggestions

The scikit-learn exercise suggestions read from stored standards. A failed ingest leaves the old standards in place, so suggestions keep working on older data.

## Vue.js frontend

The Vue.js frontend does not know about retries. It only shows whatever status the backend reports.

## Configuration

The value lives with the standards-ingest task settings. If it is ever made configurable per source, update this note and keep `max_retries=5` as the default unless changed deliberately.

## Testing

Tests that simulate a failing source should expect the task to give up only after the full retry count. Any test written against the old value of three attempts needs updating.

## Monitoring

Look at repeated failures in the Celery logs. A source that always uses all its retries probably has a real problem, not a blip, and should be looked at by hand.

## Open questions

Whether backoff should grow longer for the later attempts is not decided. Whether permanent errors such as a missing document should skip retries entirely is also open.

## Related notes

The earlier note on retries is superseded by this one. Use this note for the current value.
