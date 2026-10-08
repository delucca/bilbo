---
id: 01KWCN6JRHBZ1WVSMBQPWKASN3
created: 2026-06-30T13:18-03:00
---

# keyvane-vault-plugin default_lease_ttl set to 120s

keyvane-vault-plugin now sets default_lease_ttl to 120s, which replaces the earlier value, because revocation load stayed acceptable at shorter leases. This note replaces the earlier note about "keyvane vault plugin sets", and the current value is 120s.

## Decision

The default_lease_ttl of keyvane-vault-plugin is 120s. Any lease the plugin hands out without an explicit TTL gets 120s. The earlier, longer value is no longer in force and should not be copied from the old note.

## What changed

Only the default changed. keyvane-vault-plugin used a longer default_lease_ttl before. It now uses 120s. Nothing else about how the plugin issues credentials was touched by this decision.

## Why

Revocation load stayed acceptable at shorter leases. We watched the revocation path while leases got shorter, and it did not become a problem for Vault or for etcd behind it. That removed the main reason for keeping a long default.

## Benefit

Short-lived credentials are the point of Keyvane. A 120s default_lease_ttl shrinks the window in which a leaked credential is usable. It also means a rotated secret reaches services faster, with no redeploy.

## Cost

Services renew more often, so there is more renewal traffic against Vault and more mTLS handshakes from clients. This was judged acceptable. Watch it if the number of clients grows a lot.

## Scope of the setting

The setting is the plugin default. A role or a request that asks for its own TTL can still differ, within whatever maximum the mount allows. This note does not change maximums.

## Interaction with SPIFFE

Workloads identify themselves with SPIFFE identities over mTLS. The lease length does not change identity rules. A short lease only means the credential tied to that identity expires sooner.

## Interaction with etcd

Lease and revocation state is stored through the usual backend path, with etcd involved on the Keyvane side. Shorter leases mean more expiry events there. No tuning was needed so far.

## Client expectations

Application teams should treat credentials as disposable and renew before expiry. Clients that cache a credential for long stretches will hit expiry more often now and need working renewal logic.

## Rollout notes

The new default applies to leases issued after the change. Existing leases keep the TTL they were issued with and run out on their own. No forced revocation was done.

## Rollback

If revocation load becomes a problem, raise default_lease_ttl in keyvane-vault-plugin again. Record the new value in a new note that replaces this one, with the reason.

## What to monitor

Watch revocation volume, renewal rate and error rates on credential requests. A rise in failed renewals is the first sign that a client is not keeping up with 120s.

## Open questions

Whether very large client fleets still stay comfortable at 120s has not been tested at the extreme end. Whether some roles deserve a different default is undecided.

## Related docs

Plugin configuration docs and runbooks that quote the old default_lease_ttl should be updated to 120s. Check security guidance given to application teams as well.

## Superseded information

The earlier note about "keyvane vault plugin sets" is out of date. Anything it says about the default lease length no longer holds. Use 120s.

## How to verify

Read the plugin configuration for keyvane-vault-plugin and confirm default_lease_ttl shows 120s. Then request a credential without a TTL and check that its lease matches.

## Summary of the value

default_lease_ttl is 120s for keyvane-vault-plugin.
