---
id: 01JQS4V3W3S6JX6CMFX29G0SM6
created: 2025-04-01T14:02-03:00
sources:
  - "code: pkg/bridge/mapping.go"
---

# keyvane-spiffe-bridge design

keyvane-spiffe-bridge is the piece of Keyvane that gives workloads running in Kubernetes a SPIFFE identity, so they can authenticate over mTLS to the rest of the system and ask for short-lived credentials. This note records the design as it stands and the naming history, since old docs and dashboards still use the old name.

## Naming

The component used to be called `svidgate`. It is called `keyvane-spiffe-bridge` now. If you find `svidgate` in old runbooks, config comments, alert names or chat history, it is the same component. Use `keyvane-spiffe-bridge` in new code, docs and notes. Do not treat the two as separate services; there is only one.

## Identity mapping

The bridge maps Kubernetes service accounts to SPIFFE IDs. Every service account gets exactly one ID, built from its namespace and its name in this form:

`spiffe://keyvane.internal/ns/<namespace>/sa/<name>`

The trust domain is keyvane.internal. The namespace and the service account name are taken as they are in the cluster, with no aliasing and no per-team rewriting. Because the ID is derived and not configured, there is no table to keep in sync: create a service account and its identity exists, delete it and the identity stops being issued.

## How it fits with the rest

- Workloads present an X.509 SVID, and the peer checks it over mTLS. The SPIFFE ID above is what authorization rules match on.
- Vault policies for credential issuance are written against these IDs, not against pod names or IPs. This keeps rotation of secrets independent of redeploys: a pod that restarts keeps the same identity as long as the service account is the same.
- Any state the bridge needs to keep lives in etcd, not on local disk, so a replica can be replaced without losing mappings or issuance records.
- The bridge is written in Go, like the other Keyvane services.

## Things to watch

- Two workloads that share one service account share one SPIFFE ID. If they need different access, give them different service accounts; do not try to separate them inside the bridge.
- Changing a service account name or moving it to another namespace changes its ID, so any Vault policy or peer rule that names the old ID has to be updated at the same time.
- Anything that still refers to `svidgate` by name, such as metric labels or log filters, should be moved to `keyvane-spiffe-bridge` when you touch it. Check for leftovers before deleting any old alias.

## Open points

- Whether to allow an explicit override of the derived ID for special cases is undecided. For now the answer is no; the derivation stays the only path.
- Documentation outside this repo has not all been renamed yet.
