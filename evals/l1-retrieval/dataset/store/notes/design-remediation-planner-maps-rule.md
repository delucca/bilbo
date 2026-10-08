---
id: 01KE5Q322165PHXHDVP0GJCKD0
created: 2026-01-04T20:57-03:00
sources:
  - "code: templates/remediation.yaml"
---

# remediation-planner design

remediation-planner takes the findings that the policy scan produces and turns them into Jira remediation tickets. It does not evaluate policy itself. Open Policy Agent decides what is a violation; remediation-planner only decides what the ticket for that violation looks like. In code, commits and chat the component is usually called `remplan`, which is short for `remediation-planner`. Both names mean the same thing. This note uses the long name.

Related watch-outs about the policy bundles that feed this component are in [[policy-bundle-watch-outs]].

## What it does

Each finding carries a `rule_id`. remediation-planner maps the `rule_id` of each finding to a Jira issue template defined in `templates/remediation.yaml`. That file is the single place where the mapping lives. A template says what the issue should contain: summary, description text, labels, priority and similar fields. The planner fills in the placeholders with data from the finding, such as the resource, the account and the failing setting, and hands the result to the Jira client.

The key point for anyone changing it: the lookup is by `rule_id` and nothing else. The planner does not guess from resource type, severity or message text. If you add a new rule to the policy side, you also have to add a template entry for its `rule_id`, or the finding has nothing to map to.

## Where it runs

It runs as an AWS Lambda written in Python. It reads findings after a scan finishes, and it uses DynamoDB to remember which findings already have tickets. The Lambda is meant to be safe to run twice on the same input, because scans get retried and the same finding can show up again.

## Flow

- Load the templates from `templates/remediation.yaml` at cold start, not on every invocation.
- For each finding, read its `rule_id` and look up the template.
- Check DynamoDB to see whether a ticket already exists for that finding.
- If there is none, render the template and create the Jira issue, then record the issue key in DynamoDB.
- If there is one, skip creation. Updating an existing ticket is a separate concern and is kept simple on purpose.

## Unmapped rules

A finding whose `rule_id` has no entry in the templates file is the main failure case. The planner should not drop it silently and should not invent a generic ticket either. The current intent is to log it clearly with the `rule_id` and count it, so that a missing template shows up as a gap to fix rather than a lost finding. Whether to fall back to a catch-all template is an open question. A catch-all would reduce lost findings but would produce vague tickets that security teams dislike.

## Design choices and why

Templates live in a YAML file instead of code so that security staff can reword tickets without touching Python. The cost is that a typo in the file only shows up at load or render time. A validation step that checks every template for required fields would help, and the load step is the right place for it.

Keeping the mapping keyed on `rule_id` makes the behavior easy to explain and to test: one rule, one template. The downside is duplication when many rules want nearly the same ticket text. For now the duplication is accepted. Sharing text between templates is possible later, but only if the duplication starts to hurt.

## Things to check when changing it

- A new or renamed `rule_id` on the policy side needs a matching template entry.
- Placeholder names in a template must match fields the finding really has.
- Idempotence: a retry must not create a second ticket for the same finding.
- Jira rate limits and transient errors should be retried without losing track of which findings were already done.
- Keep the template load outside the per-finding loop.
