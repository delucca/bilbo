---
id: 01KBE30PPC1EJG5ZZCJCBH6MDS
created: 2025-12-01T20:12-03:00
---

# policy-bundle-repo design notes

Quick write-up of how policy-bundle-repo is meant to work in AuditMesh, written from memory of the design discussions and what the code does today. It is partial. Check the code before relying on any detail here, and check whether an older note already covers some of this.

The short version: policy-bundle-repo is the single git repository that holds every Rego policy the scanner evaluates, plus the test data and the metadata that maps each policy to a remediation ticket template. Nothing runs from it directly. A build step packages it into an OPA bundle, and the Lambda scanners pull that bundle at start-up or when the cached copy is stale.

## Purpose

Cloud security teams want to change what counts as a violation without redeploying the scanner. So the policies live apart from the Python code. policy-bundle-repo is where a policy author works. The scanner code stays generic: it loads configs, hands them to OPA as input, gets back a set of violations, and files tickets.

Keeping this separate also means the policy history is its own audit trail. Who loosened a rule and when is just git log on this repo.

## Layout

The repo is organised by cloud service first, then by rule. Each rule has a Rego file, a test file next to it, and a small metadata file. The metadata carries severity, a human description, the ticket template to use, and an owner team label.

There is a shared library area for helper functions that several rules use, such as tag checks and CIDR matching. Rules import from it rather than copying. The rule of thumb: if two rules need the same helper, it goes in the library, and the library has its own tests.

There is also a folder of sample inputs, trimmed real configs with identifiers scrubbed. Tests use these so a rule is always exercised against something realistic and not only hand-made toy input.

## Bundle build

CI builds the bundle on every merge to the main branch. The build runs the formatter check, the OPA test run, and a lint pass, then produces a signed bundle archive and uploads it to the artifact location the Lambdas read from. A revision label derived from the git commit is written into the bundle manifest so a running scanner can report which policy set it used.

The build fails if any rule lacks a metadata file or a test file. That was a deliberate choice after a few rules shipped with no tests and nobody noticed for a while.

## How scanners consume it

The Lambda scanner fetches the bundle, keeps it in memory for the life of the execution environment, and re-checks the revision on the usual refresh interval. If the fetch fails it keeps using the last good bundle and logs a warning. It does not fail open and it does not stop scanning. Whether that is the right trade-off is still debatable; for now stale policy beats no policy.

The revision label gets stored with each finding in DynamoDB, so any finding can be traced back to the exact policy version that produced it. That is useful when someone asks why a ticket was raised on a config that looks fine today.

## Ticket mapping

Each violation returned by OPA carries a rule identifier. The scanner looks that up in the metadata shipped in the bundle to choose the Jira ticket fields: summary template, priority from severity, and component or label from the owner team. Keeping this in the bundle means a policy author can change the ticket wording in the same pull request as the rule.

Duplicate suppression is not done here. The scanner handles it by checking existing findings for the same resource and rule before opening a new ticket.

## Review and change process

Changes go through pull requests. The usual expectation is one reviewer from the security team and, for rules that touch high severity findings, a second from the owning team. New rules should land with tests for both a passing and a failing case, and ideally one edge case.

Loosening a rule, or raising the severity threshold so fewer findings qualify, should say why in the PR description. Reviewers have been asking for that and it has paid off.

## Versioning and rollback

Rollback is just pointing the scanners at an earlier bundle revision, or reverting the merge and letting CI rebuild. There is no separate release branch. The earlier idea of a staging bundle that scanners in a test account consume first is still attractive, but it was not finished, so right now merges go straight out to everyone on the next refresh.

## Open questions

- Should rule metadata be validated against a schema in CI, rather than only checked for presence?
- Do we want per-account overrides, and if so do they live here or in the scanner config? Leaning toward not here, to keep the repo free of account-specific data.
- The shared library is growing. At some point it may deserve its own package and versioning.
- Bundle size and load time on a cold Lambda start should be measured again once the rule count passes the usual comfortable range.
- Decide on a policy for deprecating a rule: keep it with a disabled flag for a while, or delete and rely on git history.

## Things to watch

Rego changes that look harmless can change which resources match, so always run the sample inputs and compare finding counts before and after. Also be careful with tests that depend on map ordering or on unstable input fields. And do not put real account identifiers into sample inputs, even in comments.
