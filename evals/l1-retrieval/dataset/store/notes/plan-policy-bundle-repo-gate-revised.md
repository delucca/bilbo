---
id: 01KWJM68ZP8QZXZHK1GJ8DPMCD
created: 2026-07-02T20:56-03:00
---

# policy-bundle-repo coverage gate plan

This note replaces the earlier note about "policy bundle repo gate". The new value is 95%: policy-bundle-repo will raise its CI coverage gate to 95%, and that figure supersedes the threshold planned before. Anyone still working from the old number should drop it.

## Goal

The CI coverage gate in policy-bundle-repo moves up to 95%. The point is to keep the Rego policies and the Python helpers around them tested well enough that a bad rule does not reach the scanner and start producing wrong remediation tickets in Jira. A rule that misfires is expensive for cloud security teams, because every false positive becomes a ticket someone has to close by hand.

## What the gate covers

The gate applies to the policy bundle repository only. It does not change anything in the Lambda code or the DynamoDB tables that AuditMesh uses elsewhere. Coverage here means the policy tests run through Open Policy Agent plus whatever Python test code lives in policy-bundle-repo. If the two measurements are reported separately today, decide before the change whether the gate looks at the combined figure or at each one. My lean is the combined figure, since that is what the CI job already prints.

## Steps

- Check the current coverage figure on the main branch so we know the size of the gap to 95%.
- List the policies and helper modules with the weakest coverage and rank them by how often they fire in real scans.
- Write the missing tests first, starting with the high-traffic rules. Include both passing and violating config samples for each rule.
- Only after coverage is at or near the target, change the gate setting in the CI config to 95%.
- Tell the people who open pull requests against policy-bundle-repo that the bar has moved, so a failed check does not surprise anyone.

## Risks and gotchas

- If the gate goes up before the tests exist, every open pull request fails and people will start asking for exceptions. Do the tests first.
- Generated or vendored files can drag the number down. Make sure they are excluded from measurement, not tested for show.
- Chasing the last few points tends to produce tests that assert nothing useful. Review new tests for real assertions, not just line execution.
- Rego coverage tooling counts differently from Python coverage tooling, so the combined figure may jump when one side changes. Watch for that when comparing before and after.

## Open questions

- Whether the gate should be strict on every pull request or only on merges to main.
- Whether a small, documented exclusion list is acceptable for code that cannot sensibly be tested in CI.
- Who owns the follow-up if coverage drops under 95% after the gate is live. For now, assume the author of the failing change fixes it.
