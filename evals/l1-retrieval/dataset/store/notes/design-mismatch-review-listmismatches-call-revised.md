---
id: 01KY70F6P5ZCE7N3R78JAD7TNR
created: 2026-07-23T05:11-03:00
---

# mismatch-review-api ListMismatches page size

This note replaces the earlier note about "mismatch review listmismatches call". The new value: the ListMismatches call of mismatch-review-api returns pages of 200 mismatches each, which replaces the earlier page size.

## Scope

This covers only the paging of ListMismatches in mismatch-review-api. Other calls on the service are not changed by this design. Reviewers in finance operations use this call to walk through flagged differences between card-processor settlement files and internal ledger entries.

## Decision

Every page returned by ListMismatches holds up to 200 mismatches. The last page of a result set can be shorter, and an empty result is a valid answer.

## What changed

The earlier page size is gone. Do not keep both values in client code or docs. Anything that still assumes the old size should be updated to read the page contents instead of counting on a fixed number.

## Why

Review queues at marketplaces can get long after a bad settlement day. A larger page cuts the number of round trips a reviewer tool makes. It also keeps each response reasonable for the gRPC channel, so we did not push it higher.

## Client impact

Clients that loop on a page token keep working without changes. Clients that hard-coded the old size for progress bars or offset math will be off and need a fix. Check any UI code that computes the total number of pages.

## Server side

The query behind the call reads from PostgreSQL. The limit applied to that query is the page size above. Ordering must stay stable between pages, otherwise a mismatch can be skipped or shown twice when new ones arrive during review.

## Kafka and ingestion

New mismatches arrive from the reconciliation pipeline through Apache Kafka. A page is a snapshot of what was stored at request time, so mismatches that land later show up on later requests, not in pages already served.

## Testing

Tests that build fixtures around the page boundary should use the new size. Cover three cases: exactly one full page, one full page plus a few extra, and no results. Watch for tests that quietly assumed the old size.

## Deployment

The change ships with the normal service rollout. Terraform config is not involved, since the size is a service default and not infrastructure. During a rolling deploy, clients may briefly see both sizes from different instances.

## Risks

Larger pages mean larger payloads when mismatch records carry many fields. If response size becomes a problem, trim fields before shrinking the page.

## Open questions

Whether the page size should be a client-settable parameter with a server cap is not decided. For now it is fixed.

## Related

The superseded note should be treated as outdated. Link future changes to paging here instead of starting another note on the same call.

## Status

Settled. Update this note if the page size changes again.
