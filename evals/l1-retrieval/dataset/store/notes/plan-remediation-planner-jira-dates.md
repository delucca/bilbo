---
id: 01KP6KGQ26XCEYDPYEVTM5YDT4
created: 2026-04-14T15:19-03:00
---

# remediation-planner: Jira due dates by severity

Plan for making remediation-planner set Jira due dates on the tickets it creates, driven by finding severity. The first step is critical findings, which get a due date of 3 days. Other severities come after that and are not settled yet. This note records the intent and the order of work so a later session does not have to rebuild it from scratch.

## Why

Right now tickets that remediation-planner produces from AuditMesh scan results carry no deadline, or whatever default the Jira project applies. Cloud security teams read the queue by age and guess at urgency. A critical finding, such as a publicly exposed storage resource, sits next to a low-risk tagging violation with nothing to separate them except the priority field. A due date makes the expectation explicit and lets Jira dashboards and filters show overdue work.

## Decision so far

- Due dates are derived from severity, not set by hand per ticket.
- Critical findings get a due date of 3 days.
- The 3 days is counted from the moment the ticket is created by remediation-planner, not from when the scan first saw the violation. Reason: scan time can be far earlier than ticket time if a finding was suppressed or batched, and teams should not receive tickets that are already overdue.
- Severity comes from the finding as produced by the Open Policy Agent policy evaluation. remediation-planner should not recompute or override it.

## Open questions

- Values for high, medium and low severities. Do not guess these in code; get them from the security team that owns the policy before adding them.
- Whether weekends and holidays should be skipped when counting days. Default for the first version is plain calendar days, and this should be stated in the ticket description so nobody is surprised.
- What happens when a finding is re-detected after its ticket was closed. A fresh ticket would get a fresh due date; a reopened one probably should keep the original. Not decided.
- Whether teams can configure the mapping per account or environment. Leave out of the first version.

## Implementation steps

1. Add a small severity-to-offset mapping in the Python code of remediation-planner, with only the critical entry filled in at first. Keep it as data, not branching logic, so adding the other severities is a one-line change each.
2. When building the Jira issue payload, compute the due date from the creation time plus the offset, and send it in the due date field. If a severity has no mapping, leave the field unset rather than inventing a value.
3. Keep the planner's output idempotent. The Lambda may retry, and the DynamoDB record that tracks which findings already have tickets should be the guard, so a retry does not create a second ticket or move the date on an existing one.
4. Write the due date into the DynamoDB record too, so later runs and reports can see what was promised.
5. Add tests for the critical case, the unmapped-severity case, and a retry of the same finding.

## Things to watch

- Jira projects can hide or lock the due date field on certain screens. If issue creation fails for that reason, the error should surface clearly instead of silently dropping the date.
- Time zones: compute in UTC and let Jira display it. A due date is a date, so check how the Jira API treats a timestamp sent where a date is expected.
- Existing tickets are not touched by this change. Backfilling is a separate job and should be agreed with the teams first.
