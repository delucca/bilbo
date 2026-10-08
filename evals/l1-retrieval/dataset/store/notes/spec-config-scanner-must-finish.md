---
id: 01M196KVNNWCFWWYNPK01HT6XF
created: 2026-08-30T08:24-03:00
---

# config-scanner spec

This is the working spec for config-scanner, the part of AuditMesh that pulls cloud infrastructure configs, runs them through Open Policy Agent policies and hands violations to the ticketing step. Written quickly, so it covers the contract and not every detail.

## Purpose

config-scanner takes one AWS account region as its unit of work. It collects the configs for that region, evaluates them against the policy set and emits findings. Findings later become Jira remediation tickets for the cloud security team.

## Scan unit

One scan covers exactly one AWS account region. An account with several regions gets several independent scans. Do not batch regions into one run; a slow region should not hold up the others.

## Timeout

config-scanner must finish scanning one AWS account region within a timeout of 300 seconds. If the scan has not finished by then, the scan is marked failed. A failed scan is not partial success: nothing from it is treated as a complete result for that region.

## Failure handling

A scan marked failed is recorded as failed in DynamoDB with the region and account it belonged to. It is not retried silently inside the same run. Retrying is a separate decision made by the caller or the scheduler, so a repeatedly slow region stays visible.

## Runtime

The scanner runs as a Python AWS Lambda function. The timeout above is the budget the function plans around, so the Lambda configuration should leave room for startup and for writing the failed state.

## Policy evaluation

Policies are Rego, evaluated with Open Policy Agent. Evaluation is the part most likely to eat the budget on a first invocation. See [[opa-eval-cold-start]] for what we know about cold start cost.

## State

Scan status (running, succeeded, failed) and findings metadata live in DynamoDB. Writes should be idempotent per region scan, so a re-run overwrites its own earlier record instead of duplicating it.

## Output

Each finding names the resource, the violated policy and the account region. The ticketing step reads these and creates or updates Jira tickets. config-scanner itself never talks to Jira.

## Open questions

- Whether very large regions need to be split by resource type to stay within the budget.
- Whether a failed scan should raise an alert on its own or only after repeated failures.

## Non-goals

No auto-remediation. No cross-region aggregation inside the scanner. No retry logic beyond what the caller decides.
