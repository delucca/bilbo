---
id: 01JXY1F0P4G63PGFBBZ0MDN5YW
created: 2025-06-17T01:13-03:00
sources:
  - "code: src/auditmesh/jira/client.py"
---

# jira-client-lib design

jira-client-lib is the thin layer AuditMesh uses to talk to Jira. It wraps the Jira REST endpoint `/rest/api/3/issue` and reads its API token from the environment variable `JIRA_API_TOKEN`. Everything else in this note hangs off those two facts. If a ticket is not created, check the endpoint and the token first.

## Purpose

AuditMesh scans cloud infrastructure configs, evaluates them with Open Policy Agent, and turns violations into remediation tickets. jira-client-lib is the only place that knows how to create and update those tickets in Jira. Callers hand it a finding and get back a ticket reference.

## What it wraps

The library wraps the Jira REST endpoint `/rest/api/3/issue`. Create, read and update of issues all go through that endpoint. It does not wrap other parts of the Jira API. If we need something else later, add it deliberately and write it down here.

## Authentication

The API token comes from the environment variable `JIRA_API_TOKEN`. The library reads it from the environment and does not take it as a function argument. It is never written to logs, never stored in DynamoDB, and never put in the repo. In Lambda the variable is set in the function configuration, backed by a secret store.

## Names, old and new

The component is called jira-client-lib now. Its previous name was `atlasclient`. Old branches, old tickets and some dashboards still say `atlasclient`. Treat them as the same component. New code, docs and imports should use `jira-client-lib` only.

## Relationship to ticket-sync-worker

The internal codename `jiraferry` belongs to `ticket-sync-worker`. So when someone says jiraferry in chat or in a log, they mean `ticket-sync-worker`. That worker is the main caller of jira-client-lib. It picks up findings and pushes them to Jira through the library.

## Where it runs

It runs inside AWS Lambda, as part of the sync path. Functions are short lived, so the library holds no long lived state. Each invocation builds its client from the environment.

## Data flow

A finding is produced by a policy check and stored in DynamoDB. The sync worker reads the finding, asks jira-client-lib to create or update an issue, and writes the returned Jira key back next to the finding. That stored key is how we avoid duplicate tickets.

## Idempotency

Retries happen, because Lambda can run a handler twice. Before creating an issue the caller checks whether the finding already has a stored Jira key. The library itself stays simple and does not guess about duplicates.

## Error handling

Failures from Jira are turned into a small set of library errors: auth failure, validation failure, rate limit, and server error. Auth failures usually mean a missing or expired `JIRA_API_TOKEN`. Rate limit and server errors are retryable; the others are not.

## Retries and rate limits

Retryable errors use backoff with a cap, and the cap stays inside the Lambda timeout. When Jira asks us to slow down we honor that. If the budget runs out, the error goes back to the caller, which leaves the finding unsynced for the next pass.

## Field mapping

Severity, resource, policy name and remediation text from a finding map to issue fields. The mapping lives in one place in the library so ticket layout changes do not spread through callers. Custom fields differ by Jira project, so they come from configuration, not code.

## Testing

Unit tests use a fake HTTP layer and check the payload sent to `/rest/api/3/issue`. They do not need a real token. A manual check against a test Jira project is done before releases, with `JIRA_API_TOKEN` set locally for that shell only.

## Known gotchas

- A missing `JIRA_API_TOKEN` fails at the first call, not at import.
- Old references to `atlasclient` can mislead a search; search for both names.
- Searching for `jiraferry` finds worker logs, not library code.

## Open questions

- Whether to add bulk operations or keep one issue per call.
- Whether token rotation should be picked up without a cold start.

## Notes for later sessions

Keep the public surface small. Do not rename things again without updating this note, and keep the three names straight: `jira-client-lib` is the library, `atlasclient` is its old name, and `jiraferry` is the codename of `ticket-sync-worker`.
