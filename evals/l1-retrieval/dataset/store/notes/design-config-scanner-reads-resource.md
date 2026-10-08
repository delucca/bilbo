---
id: 01JXKMQHTR8HXHPCF9BSJ0M8R6
created: 2025-06-13T00:18-03:00
sources:
  - "code: src/auditmesh/scanner/collector.py"
---

# config-scanner design

config-scanner is the AuditMesh component that pulls cloud resource configurations and turns them into documents the policy engine can judge. This note records how it is shaped, what it was called before, and where its output goes. Written quickly, so it is plain.

## Names, read this first

The previous name of config-scanner was `cfgsweep`. The component is called `config-scanner` now. Old tickets, branch names, dashboards and some Lambda aliases may still say `cfgsweep`; treat them as the same component as `config-scanner`.

The internal codename of config-scanner is `hawkeye`. People say "hawkeye" in chat and in some log fields. It is a codename only, not a package or a deploy name. The deployed and documented name is `config-scanner`.

There is one more rename nearby that people mix up. `violations-v1` was the previous name of the findings-table; the component is called `findings-table` now. config-scanner does not own that table, but its output ends up there after policy evaluation, so old docs that mention `violations-v1` mean `findings-table`.

## Purpose

AuditMesh scans cloud infrastructure configs for policy violations and produces remediation tickets for cloud security teams. config-scanner is the first stage. It only collects and normalizes. It makes no judgment about whether something is a violation. That is Open Policy Agent's job downstream.

## Data source

config-scanner reads resource configurations through the AWS Config `SelectResourceConfig` API. It sends a SQL-like query, pages through the results, and handles each returned item as one resource configuration.

We chose this API over listing each service separately because it gives one uniform way to read many resource kinds, and the account's recorded configuration history is already there. The cost is that we depend on what AWS Config records; resources it does not record are invisible to us.

## Normalization

Each result from `SelectResourceConfig` is normalized into a JSON document keyed by `resource_type`. That key is what the policy side uses to pick which rules apply to a document. Keep it stable. Changing how `resource_type` is spelled or derived breaks rule selection silently, because rules simply stop matching and nothing fails loudly.

```json
{
  "resource_type": "AWS::S3::Bucket",
  "configuration": {}
}
```

The example is only a shape. The real documents carry more fields, but the keying by `resource_type` is the contract.

## Runtime

config-scanner runs as an AWS Lambda function. It is kept small and stateless, so a failed run can just be retried. Long scans are split into pages so one invocation does not need to hold everything in memory.

## Hand-off to policy evaluation

Normalized documents are passed to the Open Policy Agent evaluation step. config-scanner does not embed rules. Keeping rules out of the scanner means security teams can change policy without redeploying collection code.

## Where results land

After evaluation, violations are stored in the `findings-table`, a DynamoDB table. From there the ticketing step creates Jira remediation tickets. config-scanner never writes to Jira directly.

## Error handling

Throttling from the AWS Config API is the common failure. The scanner backs off and retries. A page that keeps failing is logged and skipped for that run, and the next scheduled run picks it up again. We prefer a partial scan with a clear log over a failed whole run.

## Things that bite

- Old references to `cfgsweep` in infrastructure code or alerts may point at stale resources. Check before assuming they are live.
- Docs saying `violations-v1` mean `findings-table`. Do not create a table by the old name.
- A normalized document with a missing or wrong `resource_type` will not match any rule and so will never raise a finding. That looks like a clean result when it is not.
- Searching logs for `hawkeye` finds codename-tagged lines; searching for `config-scanner` may miss them, and the reverse is also true.

## Open questions

- Whether to add a check that flags documents which match no rule at all, to catch the silent `resource_type` problem.
- Whether to cover resource kinds that AWS Config does not record, and by what second source.
- Retiring the last aliases that still carry the `cfgsweep` name.

## Testing notes

Test normalization with saved sample outputs of `SelectResourceConfig` and compare the resulting JSON documents, especially `resource_type`. Policy rules are tested separately in the Open Policy Agent side of the project, not here.

## Ownership and naming rule

When writing new docs, code comments or tickets, use `config-scanner`. Mention `cfgsweep` or `hawkeye` only to help someone searching old material, and mention `violations-v1` only alongside `findings-table`.
