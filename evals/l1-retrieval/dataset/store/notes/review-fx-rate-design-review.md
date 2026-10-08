---
id: 01KHW3HWSKSXH63H0ZF4MHZ0D0
created: 2026-02-19T20:25-03:00
---

# Design review of fx-rate-loader

The design review of fx-rate-loader is done. The main outcome: the job must fail when the newest fetched rate is older than 36 hours. A stale rate set is not something to carry on with quietly. Ledgerlark compares settlement amounts against ledger entries, and a currency conversion built on an old rate produces false mismatches or hides real ones. Finance operations would then review the wrong items, or miss the right ones. A loud failure is cheaper than that.

## Decision

fx-rate-loader fails the run when the newest rate it fetched is older than 36 hours. The check looks at the newest rate in the fetched batch, not at the time the job started. If the provider answered but handed back only old data, the job still fails. A successful HTTP response is not enough.

The threshold is 36 hours. It was chosen to tolerate one missed daily publication from the rate provider (for example over a weekend or holiday gap in the provider's own schedule) without letting a second miss go unnoticed. Anything tighter would page people for provider hiccups. Anything looser would let reconciliation run on rates that are two days old.

A short sketch of the intended check:

```go
// fail the run if the newest fetched rate is older than 36 hours
if time.Since(newest.FetchedAsOf) > 36*time.Hour {
    return fmt.Errorf("newest rate is older than 36 hours")
}
```

The exact error text is not fixed by the review. Pick wording that makes the age limit obvious to whoever reads the job log.

## Why failing and not warning

Options discussed in the review:

- Warn and continue with the old rates. Rejected. Nobody reads warnings until the mismatch report looks odd, and by then flagged items have already been sent to reviewers.
- Fall back to the last good rates indefinitely. Rejected for the same reason, since it hides the problem and the age of the data becomes invisible downstream.
- Fail the job and keep the previous rates in PostgreSQL untouched. Accepted. Downstream consumers can still read what is there, and the failure is visible in job status and alerts.

A failed run must not overwrite or delete the rates already stored. It leaves the table as it was.

## Operational notes

- A failing run should show up in the normal job alerting, defined in the Terraform for the scheduler. The review did not ask for a new alert path, only that the failure is not swallowed.
- Retrying is fine. If the provider recovers and publishes fresh rates, the next scheduled run passes the check without any manual step.
- If the provider is down for longer, the job keeps failing on each run. That is intended. Someone on the finance operations side needs to know the rates are stale before they trust the mismatch flags.
- Reconciliation consumers that read rates through the internal gRPC service should not invent their own staleness rules. The loader is the one place that enforces the limit.

## Open points

- Whether the reconciliation side should also expose the age of the rates it used in its output. Not decided. Useful for reviewers, but it is a separate change from this component.
- Whether events about a failed load should go onto Apache Kafka for other services. The review leaned toward not doing it yet. Job status and alerting are enough for now.
- Handling of currencies the provider stops publishing. The check as decided looks at the newest rate overall, so a single dead currency would not trip it. Worth a follow-up review if it becomes a real case.

## What to remember

If you touch fx-rate-loader, keep the stale check in place and keep the limit at 36 hours unless a new review changes it. Do not turn the failure into a warning, and do not let a failed run modify stored rates.
