---
id: 01K50GJTZSYAAT0GBC6NSZF2KB
created: 2025-09-13T00:06-03:00
sources:
  - "code: Makefile"
---

# policy-bundle-repo design

policy-bundle-repo keeps Rego files under `policies/aws/` and builds the release bundle with the command `opa build -b policies/ -o bundle.tar.gz`. That is the whole layout in one line: source policies live in one tree, one command turns the tree into one artifact, and the artifact is what the rest of AuditMesh consumes.

## Purpose

AuditMesh scans cloud infrastructure configs for policy violations and produces remediation tickets. The rules that decide what counts as a violation are written in Rego and evaluated by Open Policy Agent. policy-bundle-repo is the place those rules live, separate from the Python scanner code, so security teams can change a rule without touching the scanner.

## Layout

All Rego files for AWS checks sit under `policies/aws/`. Group them by service inside that directory, one file per related set of rules. Keep test files next to the policy they cover so a reviewer sees both in the same diff. Other cloud providers would get their own sibling directory under policies/, but only AWS exists today.

Rules should return a small, consistent shape for a violation: a rule identifier, the resource, and a human-readable message. The ticket step downstream relies on that shape when it creates Jira issues, so changing it is a breaking change for the consumers.

## Building the bundle

The release bundle is built with `opa build -b policies/ -o bundle.tar.gz`. The `-b` flag treats the policies directory as a bundle root, and the output is a gzipped tarball named bundle.tar.gz. Run it from the repo root, otherwise the relative paths are wrong and the bundle comes out empty or with odd prefixes.

Run the OPA tests before building. A bundle that builds is not necessarily a bundle that passes, since the build only checks that the Rego compiles.

## Consumption

The scanner runs in AWS Lambda and loads the built bundle at start. The bundle is a build artifact, so do not commit it to the repo. Publish it from the release step and let the Lambda pick up the published version. DynamoDB holds scan state and findings, not policy, so a policy change never needs a data migration.

## Open points

- Versioning of bundles is not settled. For now each release overwrites the previous one, which makes rollback awkward.
- No decision yet on whether to split policies by team ownership inside `policies/aws/` or keep the per-service grouping.
- Should the build also run in CI on every pull request, not just on release? Probably yes, cheap to do.
