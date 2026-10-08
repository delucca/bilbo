---
id: 01KGJB3D9FZMVHJYY5RH0V4JK3
created: 2026-02-03T15:09-03:00
sources:
  - "doc: Bundle Load Time Study"
---

# policy-bundle-repo bundle size and evaluator cold load

Investigation result: bundles built from policy-bundle-repo with more than 400 rules add 1.2 seconds to the cold load time of the evaluator. Smaller bundles do not show the jump in any way we could measure. This note keeps the finding, how to read it, and what to try next. The internal codename of policy-bundle-repo is `rulebook`, so people and old docs may say that instead.

## Short answer

If a bundle goes past `400 rules`, expect the evaluator to start about `1.2 seconds` slower on a cold load. Warm invocations are not affected by this finding. The cost is paid once per cold start of the Lambda that runs the Open Policy Agent evaluator.

## Naming

The repo is called `policy-bundle-repo` in this note and in the AuditMesh docs. The codename `rulebook` shows up in older chat threads, branch names and some internal dashboards. They are the same thing. If you search for one name and find nothing, search for the other.

## What the component is

policy-bundle-repo holds the Rego policies that AuditMesh uses to check cloud infrastructure configs. It builds them into bundles. The evaluator loads a bundle, runs it against scanned configs, and the results turn into remediation tickets in Jira.

## What was measured

The measurement is cold load time of the evaluator, meaning the time from a fresh Lambda environment starting to the bundle being ready for queries. Bundles with more than `400 rules` were compared against smaller ones. The larger ones added `1.2 seconds`. The number is an added cost, not the total load time.

## Why it matters

Scans are triggered in bursts. A burst makes Lambda create many fresh environments at once, and each pays the extra load. For a security team waiting on a scan, that delay is visible. It also adds to cost and makes timeouts a little more likely on tight settings.

## Likely cause

Not confirmed. The working guess is that parsing and compiling the policies at load grows with rule count, and that something in that step stops being cheap once a bundle is large. Treat this as a hypothesis until someone profiles the load step.

## What is not known

- Whether the jump is a sharp step at the threshold or a steady slope that we only noticed there.
- Whether rule complexity matters more than rule count.
- Whether DynamoDB lookups during startup contribute.
- How it behaves with different Lambda memory sizes.

## Ways to reduce the cost

- Split a large bundle into several smaller ones, each under the threshold, and load only what a scan needs.
- Precompile or prebuild the bundle so the evaluator does less work at startup.
- Keep Lambda environments warm for the scan path.
- Remove dead or duplicate rules from the repo.

## Tradeoffs

Splitting bundles means the evaluator must know which ones to load, and it adds a routing decision. Warm environments cost money. Pruning rules needs a review by whoever owns each policy, since a rule that looks unused may cover a rare case.

## Check to run

Build a bundle just under and just over the threshold, then compare cold load times on the same Lambda configuration. A bundle build looks like this:

```bash
opa build -b policy-bundle-repo -o bundle.tar.gz
```

Run each case several times from a cold start, since one sample is noisy.

## Guidance for people adding rules

Watch the rule count in policy-bundle-repo. Before a change takes a bundle past `400 rules`, think about whether the new rules belong in a separate bundle. Mention the count in the review.

## Open follow-ups

- Profile the load step to confirm the cause.
- Decide on a splitting scheme or a prebuild step.
- Add a rule-count check to the repo's CI so growth is seen early.

## Status

Finding recorded. No fix is in place. Nothing here has been changed in the evaluator or in policy-bundle-repo.
