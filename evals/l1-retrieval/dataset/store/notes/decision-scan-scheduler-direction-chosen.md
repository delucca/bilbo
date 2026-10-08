---
id: 01K52BYFTSKPTD6W95EJHQRK41
created: 2025-09-13T17:23-03:00
---

# scan-scheduler: general direction

We settled on a general shape for scan-scheduler and this note keeps the direction, not the tuning. Anything that needs a concrete value lives in config or in the code, not here. If a later session wants to change the direction, read the reasons below first and argue against them, not against the details.

scan-scheduler decides what gets scanned and when. It does not evaluate policy and it does not file tickets. It hands work to other parts of AuditMesh and records what it handed over.

## Context

AuditMesh scans cloud infrastructure configs for policy violations and produces remediation tickets for cloud security teams. Scans were being started in an ad hoc way, partly by hand and partly by timers that nobody owned. That made it hard to say what had been checked and what had not. We wanted one place that owns the answer to "what is due".

## Decision in short

scan-scheduler is a small, event-driven, mostly stateless component on AWS Lambda. Its state about what is due and what ran sits in DynamoDB. It fans work out in small units and leaves policy evaluation to Open Policy Agent and ticket creation to the Jira integration. It prefers being simple and predictable over being clever.

## Rough flow

```
scan-scheduler -> AWS Lambda -> Open Policy Agent -> DynamoDB -> Jira
```

This is the order things happen in, loosely. The scheduler picks targets, a Lambda does the scan work, Open Policy Agent judges the configs, results land in DynamoDB, and Jira gets tickets for what needs fixing.

## Why Lambda

The load is bursty. Most of the time nothing runs, then a lot of targets become due together. Lambda fits that and we do not want to keep idle workers around. We accepted the usual limits on run time and made the design work inside them instead of working around them.

## Why stateless

A scheduler invocation should be safe to kill and run again. Anything it needs to remember goes to DynamoDB before it moves on. This keeps recovery boring: if a run dies, the next one reads the table and carries on. We did not want an in-memory queue that could lose work quietly.

## Work in small units

The scheduler splits scans into small units per target or per group of targets, not one big job. Small units fail alone, retry alone, and are easy to reason about. The cost is more bookkeeping, and we accepted that.

## Idempotency

Handing out the same unit twice must be harmless. Each unit has a stable key derived from what is being scanned and which run it belongs to, and the table is checked before work is handed over. Duplicates can still happen, so downstream parts must also tolerate them, especially ticket creation.

## Where state lives

DynamoDB holds the schedule, the status of each unit, and a short record of outcomes. It is not a log archive and not a report store. Keep items small and keep access patterns simple. If a new query needs a scan of the whole table, that is a sign the design is wrong, not a sign to add a scan.

## Scheduling model

Targets are scheduled by policy of the owning team: how often, and how much it matters. The scheduler turns that into due times and picks up what is due on each pass. We chose a pull-on-tick approach over per-target timers because it is easier to see and easier to pause.

## Triggers beyond the clock

Time is the default trigger, but a config change or a manual request can also make a target due early. These go through the same path as scheduled work. We did not want a second code path for urgent scans, because two paths drift apart.

## Policy evaluation boundary

scan-scheduler never looks inside a policy. It passes the target and the policy set reference to the scan step and takes the result back. Open Policy Agent policies change often and are owned by security staff, so the scheduler must not depend on their content.

## Ticketing boundary

The scheduler does not talk to Jira directly. It only records that a unit finished and what the outcome class was. Ticket creation and de-duplication belong to the component that knows about Jira. This keeps the scheduler testable without a Jira instance.

## Retries and failure

Failed units are retried a limited number of times with growing delay, then marked as failed and left visible. We do not retry forever, because a stuck target should show up to a human. Transient cloud API errors and real config problems are kept apart in the status so people do not chase the wrong thing.

## Fairness and throttling

One noisy account or team must not starve the rest. The scheduler spreads work across owners and respects the rate limits of the cloud APIs it ends up calling. Limits are set in config and tuned from what we see, not fixed in this note.

## Observability

Every pass leaves enough behind to answer: what was due, what was handed out, what finished, what failed. Metrics and structured logs come from the Lambda side. Missing a due scan should be loud, since that is the exact failure the component exists to prevent.

## Alternatives we passed on

- A long-running service with its own queue: more to operate, and state in memory.
- Per-target timers: hard to inspect and hard to pause in bulk.
- Letting each team trigger its own scans: back to the original problem.
- Putting ticket logic in the scheduler: couples it to Jira for no gain.

## Risks we know about

DynamoDB access patterns can drift into expensive reads if people add fields casually. Lambda limits may bite for large targets, which pushes for even smaller units. Idempotency bugs would show up as duplicate tickets, which security teams notice fast. Review changes to the key scheme with extra care.

## Revisit when

Revisit this if scan volume changes the shape of the load a lot, if Lambda stops fitting the work, or if the boundaries with policy evaluation or ticketing start leaking into the scheduler. Until then, keep scan-scheduler small and keep the specifics in config.
