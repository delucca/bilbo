---
id: 01KK0PK98SM62NKS1AE0GC7R8H
created: 2026-03-06T01:30-03:00
---

# config-scanner throttling when scanning many accounts

config-scanner fails with `ThrottlingException: Rate exceeded` when more than 10 accounts are scanned at the same time from one role. The limit is per role, not per account, so adding more Lambda concurrency makes it worse instead of better.

## Symptom

A scan run that fans out over many accounts starts fine, then some invocations die partway through with `ThrottlingException: Rate exceeded`. The rest finish. The result is a partial scan, and the tickets for the failed accounts are never created.

## Trigger

More than 10 accounts scanned at once, all through the same assumed role. Fewer than that and it does not show up. It is easy to miss in a small test setup.

## Why it happens

All the parallel scans call the cloud config APIs under one role identity. The API side counts calls per role, so the combined call rate from every parallel scan goes over the limit. Each Lambda looks fine on its own.

## What it looks like in logs

Search the config-scanner Lambda logs for the exact string:

```
ThrottlingException: Rate exceeded
```

Failures cluster in time, right after the fan-out starts.

## What not to do

- Do not just raise Lambda concurrency.
- Do not retry in a tight loop with no delay.
- Do not assume the failed accounts were clean. They were not scanned.

## Workaround

Cap the number of accounts scanned in parallel from one role at 10 or fewer. Queue the rest and run them in batches.

## Better fix

Use a separate role per group of accounts, so no single role carries the whole load. Add exponential backoff with jitter around the API calls as well.

## Effect on OPA

Open Policy Agent evaluation is not the problem. The failure is before evaluation, while config is being fetched. Policies never see the missing accounts.

## Effect on DynamoDB

Scan state in DynamoDB can show an account as started but never finished. Check for those rows after a throttled run and rescan them.

## Effect on Jira

No remediation tickets are made for accounts that failed. Do not close out a run in Jira as complete until all accounts show finished.

## How to check a run

Compare the list of accounts requested with the list that finished. Any difference means a throttled account.

## Testing notes

Reproduce with a test run that goes over 10 accounts through one role. A run under that will pass and tell you nothing.

## Open questions

- Whether the limit differs by region.
- Whether batch size should be configurable per deployment.

## Status

Known, not fully fixed. The batching workaround is the one to use for now.
