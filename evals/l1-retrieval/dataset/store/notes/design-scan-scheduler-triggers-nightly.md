---
id: 01KEK91Y5HAP66AKC9D8KJDR57
created: 2026-01-10T03:21-03:00
---

# scan-scheduler design

scan-scheduler is the piece of AuditMesh that decides when scans start. It does not scan anything itself. It fires on a timer, works out what needs scanning, and hands that work to the scan path that runs Open Policy Agent against cloud infrastructure configs. Findings from those scans later turn into Jira remediation tickets. This note records how the scheduler is triggered and why it is built the way it is, so nobody has to rediscover it.

## Trigger

Nightly scans are triggered by an EventBridge rule named `auditmesh-nightly`. The rule uses the schedule expression `cron(0 2 * * ? *)`, which is EventBridge's six-field cron format. Read it as: minute 0, hour 2, every day of month, every month, any day of week, every year. EventBridge evaluates cron expressions in UTC, so the run happens at 02:00 UTC, not in the local time of any customer or team.

The rule targets the scan-scheduler AWS Lambda function. One invocation per night is the normal case. There is no second trigger for the nightly path. If someone needs an ad hoc scan, that goes through a separate manual invocation of the same function and not through a change to the rule.

```text
rule:      auditmesh-nightly
schedule:  cron(0 2 * * ? *)
target:    scan-scheduler (Lambda)
```

## What the scheduler does on each run

On invocation, scan-scheduler reads the list of accounts and config sources that are enabled for scanning from DynamoDB. It then creates one scan job per target and dispatches them, so that a failure or slow target does not hold up the others. The scheduler itself should finish quickly; the heavy work (pulling configs, evaluating policies) happens downstream.

Rough order of events:

- EventBridge fires `auditmesh-nightly` and invokes the Lambda.
- The scheduler loads enabled targets from DynamoDB.
- It writes a job record per target, so there is a trace of what was requested for that night.
- It dispatches each job to the scan workers.
- It returns without waiting for scans to complete.

Keeping job records in DynamoDB means a later run or an engineer can see which targets were queued on a given night, and which never got picked up.

## Design reasons

Why EventBridge rather than a loop inside a long-running process: Lambda plus a scheduled rule means nothing sits idle all day, and there is no scheduler process to keep alive or patch. The tradeoff is that timing is coarse and tied to the rule, and a missed invocation is only noticed if we look for it.

Why 02:00 UTC: it is a quiet period for most of the teams who use AuditMesh, so scan load on their cloud APIs is low and tickets are ready when people start work. If a customer base shifts to other regions, revisit the hour by editing the schedule on `auditmesh-nightly`, not by adding delay logic in code.

Why the scheduler only dispatches: scan duration varies a lot with the size of an account. If the scheduler waited on scans it would hit the Lambda time limit on big estates. Splitting dispatch from execution keeps the scheduler short and lets workers scale on their own.

## Gotchas and open points

- The cron expression has six fields. A five-field Unix cron pasted into the rule will be rejected, and the `?` in the day-of-week position is required because day-of-month is set to a wildcard.
- Times are UTC. Anyone reading logs in local time will think the run is off.
- EventBridge delivers at least once, so the scheduler can in rare cases be invoked twice for one night. Job creation should be safe to repeat, for example by keying job records on target and date so a duplicate does not queue a second scan.
- There is no built-in alert if `auditmesh-nightly` does not fire or the Lambda errors out. A missing-run alarm is worth adding.
- Disabling nightly scans means disabling the rule `auditmesh-nightly`, not removing targets from DynamoDB.

## Pointers

When changing the schedule, change the rule, and check that the new expression still reads as a valid EventBridge cron. When changing which targets are scanned, change the DynamoDB records. Keep those two concerns apart; mixing them is how scans quietly stop happening.
