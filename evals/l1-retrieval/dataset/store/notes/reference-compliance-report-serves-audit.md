---
id: 01K61XX1WH33AJHKJER3Y536HA
created: 2025-09-25T23:33-03:00
sources:
  - "code: src/ReportApi/Controllers/ReportsController.cs"
---

# compliance-report-api reference

compliance-report-api serves the audit summary at `GET /api/v1/reports/audit-summary` and takes the query parameter `notebookId`. That is the one endpoint most callers need. Pass `notebookId` to say which electronic lab notebook the summary covers. This note collects what to know when touching or calling the service.

## Purpose

compliance-report-api is the read side of the audit trail in LabNotebook Sync. Research scientists and compliance officers use it to see what happened to a notebook: entry edits, instrument output attached to entries, sign-offs and sync activity. It does not write audit records. Those come from the sync pipeline and are stored in SQL Server.

## The audit summary endpoint

The route is `GET /api/v1/reports/audit-summary`. The query parameter is `notebookId`. The request has no body. The response is a summary of audit events for that one notebook, grouped by event type and by actor, with counts and the span of activity covered.

If `notebookId` is left out, the service should treat the request as invalid and answer with a client error, not fall back to a summary of every notebook. Keep it that way: a compliance report that silently covers everything is worse than a refusal.

## Query parameter notebookId

`notebookId` identifies a notebook as the rest of LabNotebook Sync knows it. Use the same identifier that appears on notebook records in SQL Server and in the messages the sync pipeline publishes. Do not invent a separate report-side id. An unknown `notebookId` should give a not-found style answer, not an empty summary that looks like a clean audit.

## Callers

Callers are the compliance dashboard, scheduled report jobs, and occasionally a scientist checking their own notebook. All of them go through the same route. There is no separate internal route with looser rules, so test against the public one.

## Data sources

Audit events are read from SQL Server. Large attachments, such as raw instrument files, live in Azure Blob Storage, and the summary only refers to them by reference. It never streams blob content. RabbitMQ is how audit events reach the store; compliance-report-api does not consume from it directly for this endpoint.

## Authorization

Reports expose who did what, so access is restricted to compliance roles and to the owner of the notebook. Check the authorization rules before adding any new caller. Do not widen access just to make a dashboard work.

## Consistency notes

Because events arrive through RabbitMQ, a summary can lag very slightly behind the latest edit. If a number looks short right after a change, wait for the pipeline to catch up before assuming a bug. Compare against the audit tables in SQL Server, not against the notebook UI.

## Performance

Summaries for notebooks with long histories can be heavy. Keep the queries aggregate-only and rely on indexes over the notebook key. Avoid loading individual events into memory in C# when SQL can aggregate them.

## Errors

Expect client errors for a missing or malformed `notebookId`, an authorization failure for callers without access, and a not-found answer for a notebook that does not exist. Server errors usually mean SQL Server is unreachable. Log them with the notebook id but never with entry contents.

## Testing

Test the endpoint with a notebook that has edits, instrument attachments and a sign-off, and one with no activity. Check that the empty case reads as empty and that the missing-parameter case is rejected. Run the integration tests against a disposable SQL Server database, never a shared one.

## Changing the contract

The path carries a version segment. Additive changes to the response are fine. Renaming or removing fields, or changing the meaning of `notebookId`, needs a new version, since compliance tooling parses these reports.

## Open questions

Whether the summary should also accept a date range is undecided. If added, it must be optional so existing calls to `GET /api/v1/reports/audit-summary` with only `notebookId` keep working.
