---
id: 01KEPYXT1YFV2S4A4EKGJ2SEVH
created: 2026-01-11T13:41-03:00
---

# standards-ingest import time budget

This note replaces the earlier note "standards ingest must complete". The import time budget of standards-ingest is now 12 minutes per national framework. The old limit was 20 minutes, and it no longer applies.

## Scope

This spec covers only the time budget of standards-ingest, the part of ClassroomCompass that loads curriculum standards from a national framework into the system. Teachers rely on those standards to see student progress and get exercise suggestions, so a stale or half-loaded framework shows up directly in the app.

## The rule

One national framework gets one budget: 12 minutes. The budget is per framework, not per run and not per school. If a run imports several frameworks, each one gets its own 12 minutes.

## What it replaces

The earlier note set the limit at 20 minutes. That value is superseded. Anything that still quotes 20 minutes (comments, alerts, runbooks, config) is out of date and should be changed to match this note.

## What counts toward the budget

The clock covers the whole import for one framework: reading the source, parsing and normalising the standards, writing them to the database, and refreshing the search index. Time spent waiting in the Celery queue before the job starts is not counted.

## Where it is enforced

The import runs as a Celery task, so the budget belongs on the task as a time limit. Keep the value in one setting and have the task read it. Do not hard-code it in several places.

```python
# budget for one national framework
IMPORT_BUDGET = "12 minutes"
```

The snippet only shows the value. Convert it to seconds wherever Celery needs seconds.

## Why it was lowered

The import got faster than it was, so the old limit was too loose. A run that goes past the new budget is more likely stuck than slow. Failing early means a teacher is not left with old standards for long.

## What happens on a timeout

A framework that goes over 12 minutes is stopped and reported as failed. Its partial data must not replace the last good import. Other frameworks in the same run are not affected.

## Search index

The Elasticsearch index for standards should keep serving the last complete import until a new one finishes. Do not publish a half-built index when the budget runs out.

## Monitoring

Alerts on long-running imports should use the 12 minute figure. Check any alert threshold that was set near the old limit, since it would now fire too late.

## Tests

Tests that check timeout behaviour should use a short fake budget and not wait the real time. One test should confirm that the configured value is 12 minutes, so a silent change is caught.

## Open points

- Whether very large frameworks need a documented exception. None is granted now.
- Whether the budget should be split between parsing and indexing. Not decided.

## Related

The suggestion logic that uses scikit-learn reads the standards after import and has no part in this budget.

## Change history

The budget went from 20 minutes to 12 minutes. This note is the current source for the value.
