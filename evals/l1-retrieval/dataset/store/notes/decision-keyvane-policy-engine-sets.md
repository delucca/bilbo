---
id: 01K33N14BCXSFJWP0X6YH3RNX8
created: 2025-08-20T08:50-03:00
---

# Policy engine defaults to deny

keyvane-policy-engine sets `default_action: deny` so that a service without a matching rule receives no credential. That is the decision. If a request from a workload matches no rule, the engine answers with a refusal and nothing is issued: no token, no certificate, no secret lease. A missing rule means no access.

## Context

Keyvane issues short-lived credentials to services and rotates secrets without redeploys. The policy engine is the point where a request is checked against rules before anything is minted from HashiCorp Vault. Callers are identified by their SPIFFE identity, presented over mTLS. Rules are kept in etcd and read by keyvane-policy-engine, which is written in Go.

Two defaults were possible when no rule matches: allow the request, or refuse it. We needed one of them written down so that nobody has to guess what an empty or partly loaded rule set does.

## Decision

keyvane-policy-engine uses `default_action: deny`. The behaviour is:

- A service whose identity matches a rule gets what that rule allows, and no more.
- A service whose identity matches no rule gets no credential. The request fails closed.
- A new service has no access until someone adds a rule for it on purpose.
- An error while reading rules from etcd must not turn into an allow. If the rule set cannot be read, the engine denies.

The setting looks like this:

```yaml
default_action: deny
```

## Why deny

Allow-by-default means every typo in a rule, every missed service and every half-loaded rule set silently hands out credentials. That failure is invisible until someone abuses it. Deny-by-default makes the same mistakes visible: a service breaks, its owners notice, and a rule gets written. A broken deploy is cheaper than a leaked credential.

It also matches how security engineers reason about the system. They can review the rules and know that anything not listed is refused, without hunting for implicit grants.

Application teams pay for this with a bit of friction. Onboarding a service means asking for a rule before the first credential request works. We accepted that.

## Consequences and things to watch

- When a service reports it cannot get a credential, check first whether any rule matches its SPIFFE identity. A missing rule is the most likely cause, and it is the intended behaviour, not a bug.
- Do not "fix" such reports by changing the default to allow. Add or correct the rule instead.
- Tests for keyvane-policy-engine should include a case with no matching rule and assert that no credential is returned.
- Any future per-environment override of the default needs its own decision note. Do not slip one in through config.
- Rule changes in etcd take effect without a redeploy, so a newly added rule fixes a denied service quickly.

## Open points

Denials should be logged with the caller identity and the request, so operators can tell a missing rule from a real attack. Check that the logging is in place before widening rollout. Alerting thresholds on repeated denials are not decided here.
