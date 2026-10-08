---
id: 01KSB7K3SQB675FW7HKH7W7T9H
created: 2026-05-23T17:14-03:00
---

# policy-bundle-repo coverage gate plan

policy-bundle-repo holds the Rego policies that AuditMesh evaluates against cloud infrastructure configs. Right now nothing in CI forces the policies to be tested well. The plan is to add a CI gate that runs `opa test --coverage` and fails the build when rule coverage is below `90%`. This note is the working plan for that gate: what we want, how to wire it, and what to watch for.

## Goal

Every change to policy-bundle-repo must keep rule coverage at or above `90%`. If a pull request drops it below that, the build fails and the change does not merge. The point is to stop untested rules reaching the scanner, because a wrong rule produces either missed violations or noisy Jira remediation tickets that cloud security teams then have to close by hand.

## Scope of the gate

The gate covers the Open Policy Agent policies in policy-bundle-repo and their tests. It does not cover the Python code that runs in AWS Lambda or the DynamoDB storage; those have their own checks. The gate only looks at Rego rule coverage, not line counts from other tools.

## Steps

1. Run the existing tests locally with coverage on and see where we stand today.
2. Write missing tests for the rules that are least covered, starting with the ones that create tickets for the most serious violations.
3. Add a CI job that runs the coverage command and checks the result against the threshold.
4. Make the job required on the main branch once it is green for a few runs.
5. Tell the cloud security teams who write policies what the new rule is.

## Command shape

The CI job should run the command below, then read the reported coverage and exit non-zero if it is under the threshold. The threshold check can be a small script step in the job.

```sh
opa test --coverage
```

## Open questions

- Does the coverage figure count helper rules, or only the rules that emit violations? Check the output format before choosing how to parse it.
- Should the threshold apply to the whole bundle or per package? Starting with the whole bundle is simpler; per package can come later if one package hides behind the others.
- Do generated or vendored policy files need to be excluded from the number?

## Risks

- Coverage can be gamed with tests that call a rule but assert nothing. Review tests in pull requests, not just the number.
- A sudden failure on the first run if current coverage is under the line. Fix that by writing tests first, not by lowering the threshold.
- Slower CI if the test set grows a lot. Watch the job time.

## Done when

The job runs on every pull request, fails correctly when coverage is under `90%`, passes on main, and is a required check. A deliberate test removal on a scratch branch should turn it red; try that once to prove the gate works.
