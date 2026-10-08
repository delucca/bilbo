---
id: 01M2SACQX4YJZ565HQD0MFR5XC
created: 2026-09-18T00:54-03:00
---

# Remediation planner: Jira due dates

Notes on how remediation-planner should set dates on the Jira tickets it creates. Written quickly, not complete. The open question is where a due date comes from and who owns it once the ticket exists. Right now the planner picks something close to the usual value and nobody has written down why.

## Where this came from

Cloud security teams read the tickets AuditMesh produces and complain that the dates feel arbitrary. A high severity finding and a low one sometimes land with the same due date. The remediation-planner builds the ticket payload from the OPA decision output, and the date is one of the last fields filled in. That is the part to fix.

## What the planner does today

It takes the violation record, looks up the severity, and adds a window to the current day. The window is a configured limit per severity. There is a fallback when severity is missing, and the fallback is the longest window. I did not re-check the exact numbers while writing this; look at the config before trusting any of this.

## Severity to window

The idea is simple: each severity class has one window. Critical gets the shortest, informational gets the longest. The mapping should live in config, not in the Lambda code. If a team wants a different window they change config, and the planner picks it up on the next run.

- Keep one table, keyed by severity.
- Do not compute windows inside the policy. OPA decides what is a violation and its severity. The planner decides time.
- Unknown severity should fail loudly in logs, not silently take the default.

## Business days versus calendar days

Open question. Some teams count calendar days and some want working days. The planner currently counts calendar days, I think. If we switch, weekends and holidays need a source. A holiday list per team is more state than I want in DynamoDB for now. Probably leave calendar days and say so in the ticket description.

## Time zones

The due date in Jira is a date, not a timestamp. The planner runs in Lambda in UTC. A scan finishing late in the day can push the date by one day for a team in another zone. Worth deciding whether to use the team's zone from their profile or just stay with UTC and document it. I lean toward UTC for now.

## Re-scans and existing tickets

If a violation is found again on the next scan and a ticket already exists, the planner should not move the due date. Moving it makes the clock restart and hides how long the issue has really been open. The planner should look up the existing ticket id in DynamoDB and leave the date alone. Only a severity change should recompute, and then only if the new window is shorter.

## Jira field handling

The due date goes in the standard due date field. Some Jira projects hide that field from the create screen, which makes the create call fail. The planner should check the project's field config once and cache the result, instead of finding out per ticket. A custom field might be needed for projects that hide it. Not decided.

## Updates and manual edits

People will edit the date by hand in Jira. The planner must not overwrite a manual edit. Idea: on update, compare the current Jira value against what the planner last wrote, kept in the DynamoDB record. If they differ, a human changed it, so skip it and log that.

## Example payload fragment

Rough shape of what the planner sends for the date. Field names come from the standard Jira issue API, not from our code.

```json
{
  "fields": {
    "duedate": "<computed from severity window>",
    "priority": { "name": "<mapped from severity>" }
  }
}
```

## Things to test

- Each severity gets the window from config, and changing config changes the output.
- Missing severity logs and uses the fallback.
- Re-scan of a known violation leaves the date unchanged.
- A manual date edit survives the next planner run.
- A project that hides the due date field gives a clear error, not a stack trace.

## Not decided yet

- Calendar versus business days.
- UTC versus team zone.
- Whether to add a grace period when a policy changes and many old findings suddenly become violations. Without one, a new rule floods teams with tickets that are already overdue on creation. That would be bad. Probably the window should start from the first time the finding was seen under the new rule.

## Next steps

Read the existing config and the planner's date code first, then write down the real windows in one place. After that decide the three open items above and add the tests. Check whether an earlier note already covers part of this before starting, because I may be repeating it.
