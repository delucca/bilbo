---
id: 01M2ZBS2Q6FXTJKK9YQRQEPN3R
created: 2026-09-20T09:13-03:00
---

# config-scanner per-region timeout

This note replaces the earlier note "config scanner must finish". The new value: config-scanner has a per-region timeout of 600 seconds. That limit supersedes the old timeout requirement, which is no longer in force. Anyone reading the old note should treat its figure as obsolete and use this one.

## Why the change

The old requirement was too tight for large accounts. A scan of a big account has many resources per region, and the policy evaluation could not complete inside the earlier limit. The result was partial scans: some regions were cut off, so violations in them were never reported and no remediation tickets were opened for them. Raising the limit to 600 seconds per region lets those accounts finish.

## What the requirement says

- The limit is per region, not per account and not per run. Each region gets its own budget of 600 seconds.
- A slow region does not eat into the budget of another region. Regions are scanned independently, so one large region cannot starve the rest.
- The timer covers the whole region scan: fetching the configs, running the Open Policy Agent evaluation against them, and writing the findings.
- If a region reaches the limit, config-scanner must treat that region as incomplete. It must not report the region as clean.

## Behaviour when the limit is hit

An incomplete region has to be visible. The scan result for that region should be marked as timed out, and the run as a whole should not look like a full success. Findings already produced for the region before the cutoff may be kept, but they must be flagged as partial. No silent drop is acceptable, because a cloud security team reading a clean result would assume the region was fully checked.

Tickets for partial findings still go to Jira like any others. The ticket or the run record should say the region was not finished, so nobody reads the absence of tickets as proof of compliance.

## Lambda and storage notes

config-scanner runs on AWS Lambda, so the function's own configured timeout has to be at least as long as the per-region limit, with some headroom for setup and for writing results to DynamoDB. If the function timeout is shorter than the region limit, the platform kills the invocation first and the region-level handling never runs. Check both values together whenever either one is changed.

If one invocation cannot cover several regions at this limit, scan regions in separate invocations rather than raising the function timeout further.

## Config shape

A sketch of how the setting should read. The key name here is illustrative; use whatever the code already calls it.

```yaml
config-scanner:
  region_timeout: 600 seconds
```

## Open points

- Whether the limit should become configurable per account for very large accounts is undecided. For now 600 seconds applies everywhere.
- The wording of the timed-out marker on tickets still needs agreement with the teams who read them.
- Any test or alert that encoded the old timeout should be updated to the new value.
