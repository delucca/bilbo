---
id: 01KJ4F9KYYZ8CB620SEWGWY2HR
created: 2026-02-23T02:24-03:00
---

# scan-scheduler nightly trigger design

This note replaces the earlier note "scan scheduler triggers nightly". The new value: scan-scheduler now triggers nightly scans with the schedule `cron(0 4 * * ? *)`, which replaces the earlier schedule to avoid the backup window. If you read the old note, drop its schedule and use this one.

## Decision

scan-scheduler now triggers nightly scans with the schedule `cron(0 4 * * ? *)`. That is the full value, in the AWS cron expression form with six fields, and it is what the scheduler rule must carry. It replaces the earlier schedule. The reason for the change is the backup window: the old time overlapped with it, so scans ran while backups were still working and read configs in a moving state. The new time sits outside the window.

Anyone asking "when does the nightly scan fire" should get this answer: once a day, at the time given by `cron(0 4 * * ? *)`, evaluated in UTC as AWS schedule rules are by default.

## Why the schedule moved

The earlier schedule fell inside the backup window. Two problems showed up from that overlap:

- Scans were slower, because the same accounts and data stores were busy with backup work.
- Some scan runs saw configuration snapshots that were in flux, which made results noisy and sometimes produced tickets that did not hold up a few hours later.

Moving to `cron(0 4 * * ? *)` avoids both. We did not change how scans work, only when they start. The policy evaluation by Open Policy Agent is untouched.

## How the trigger works

scan-scheduler is a small component. A scheduled rule fires on the cron expression and invokes an AWS Lambda function. That function reads the list of accounts and scan targets to cover, writes a run record to DynamoDB, and then starts the per-target scan work. Scan results later go through the policy step and end up as remediation tickets in Jira.

The schedule is the only thing this note changes. The Lambda handler, the run record shape and the ticket flow stay as they were.

## Things to keep in mind

- The expression uses a question mark in the day-of-week field, because AWS requires one of the day fields to be `?` when the other is set. Do not "tidy" it into a five-field Unix cron string; the rule will reject it or behave differently.
- The time is UTC. If someone reads local time into it, they will think scans run at a different hour than they do.
- If the backup window moves again, the schedule has to be reviewed. The decision here is tied to staying clear of that window, not to a particular hour for its own sake.

## Rollout notes

The change is a replacement of the rule's schedule value, not an additional rule. Check that only one nightly rule exists after deploy, otherwise scans run twice, which doubles Lambda cost and can create duplicate Jira tickets. Existing run records in DynamoDB from the old schedule stay as history and need no migration.

The first run after the change is the one to watch. Look at the run record for it and confirm it started on the new schedule, finished, and did not overlap with backups.

## Open questions

- Whether scan duration will ever grow enough to push the end of a run back into the next backup window. For now it does not, but nobody has set an alert for it.
- Whether teams in different regions want a different start time. Today there is one schedule for everyone, and this note does not change that.

## Related

Replaces the older note on the nightly trigger. Other scan-scheduler notes, such as retries and per-account fan-out, are separate subjects and are not affected by this schedule change.
