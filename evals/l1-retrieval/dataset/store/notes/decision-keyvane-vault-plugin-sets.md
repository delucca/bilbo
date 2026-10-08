---
id: 01KR5F8J7M8ZQQGC9AV99P47HN
created: 2026-05-09T01:17-03:00
---

# keyvane-vault-plugin sets default_lease_ttl to 300s

keyvane-vault-plugin sets default_lease_ttl to 300s. The reason is that short leases limit the damage of a leaked credential. If a credential issued through keyvane-vault-plugin leaks, it stops working once its lease runs out, and the lease is short. This is the default for every credential the plugin hands out unless a role or a mount overrides it on purpose. Anyone reading this note alone should take away three things: the setting is `default_lease_ttl`, the value is `300s`, and the reasoning is blast radius.

This note records the decision so nobody has to rebuild the argument later. It also lists what the decision costs, what it changes for application teams, and what we would need to see before changing the value.

## Decision

The plugin's default lease length is `300s`, set through `default_lease_ttl`. In HashiCorp Vault terms, this is the lease a caller gets when it asks for a credential and does not ask for a specific duration. Callers can still renew, and Vault can still cap the lease with its own maximum. What we fixed is the starting point, not a hard ceiling.

```hcl
default_lease_ttl = "300s"
```

The decision is about the default. A default is what most callers will live with, because most application teams never touch lease settings. If the default is long, the typical credential in the fleet is long-lived. If the default is short, the typical credential is short-lived and teams have to opt in to anything longer. We want the safe behaviour to be the one that needs no effort.

The component is keyvane-vault-plugin and nothing else in Keyvane changes its meaning. The core service that issues short-lived credentials to services keeps its own timing rules. This note covers only what the Vault plugin hands out and how long Vault treats it as valid.

## Why short leases

The argument is simple. A credential is a bearer of trust. Once it leaves the plugin, we no longer control where it goes. It can end up in a log line, a crash dump, a debugging session, a pasted chat message, a misconfigured sidecar, or a backup of a host that nobody remembers. We cannot prevent every one of these paths. We can limit how long a leaked value is useful.

A short lease changes the shape of an incident. With a long lease, a leak found late is still a live problem, and the response has to include hunting for where the value was used, revoking it, and checking what it touched in the meantime. With a short lease, a leak found late is often already harmless, because the value expired on its own. The response shifts from emergency to hygiene: find the leak path and fix it, without racing a clock.

Short leases also reduce the value of the credential to an attacker who needs time. Many attacks need a window: reconnaissance, lateral movement, staging data. A credential that dies quickly forces the attacker to come back to the source, and the source is where we can see and rate-limit requests. Each return trip is another chance to notice something odd in audit logs.

There is a second benefit, which is less about attackers. Short leases keep the whole system honest about rotation. If every consumer must renew or re-fetch on a short cycle, then rotation is exercised constantly and not once in a while. A rotation path that runs all the time is a rotation path that works. A rotation path that runs rarely breaks silently and fails on the day we need it. Keyvane exists to rotate secrets without redeploys, so a design that exercises that path continuously fits the product.

The third benefit is about revocation. Revocation in a distributed system is never instant. Caches, connection pools, and long-lived sessions all hold on to a credential after it has been revoked. A short lease puts an upper bound on how stale any of those can be. We do not have to trust that every layer honours revocation promptly, because the lease expiry backs it up.

## What it costs

Short leases are not free, and this section is here so the cost is on the record and nobody is surprised by it.

First, more renewals and more issuance. Every consumer of a credential from keyvane-vault-plugin has to renew or re-fetch on a short cycle. That puts steady load on Vault and on the plugin. At small scale this is invisible. At larger scale it matters, and it is the main thing to watch. If Vault latency rises or the plugin falls behind, consumers feel it as failed renewals, and failed renewals are failed requests in the application.

Second, a dependency on availability. A consumer holding a short lease needs Vault and the plugin to be reachable in time to renew. If Vault is down for longer than a lease, consumers lose their credentials. With long leases, a Vault outage can hide behind the remaining lease time. With short ones it cannot. We accept this because it is better to find out about the dependency in a controlled way than to have long leases mask it. It does mean the Vault deployment and its storage need to be treated as critical infrastructure, and that monitoring on renewal failures is a requirement.

Third, burden on application teams. Libraries and sidecars must handle renewal properly. A team that fetches a credential once at startup and caches it forever will break after the first lease ends. That is a real cost during rollout, and it is the most common source of support questions. The fix is in the client, not in the lease: use the renewal support in the client libraries and do not hold on to a credential past its lease.

Fourth, clock skew and timing slack. A lease that is short leaves little room for clocks that disagree. Consumers should renew well before expiry, not at expiry, and should not assume their clock matches Vault's. We recommend that clients renew after part of the lease has passed, and that they retry with backoff on failure instead of failing at once. The exact fraction is left to each client and is not fixed here.

## How it interacts with the rest of the stack

Keyvane uses mTLS between services and SPIFFE identities to say who is asking. The identity of the caller is established first, and only then does the plugin issue a credential. The lease length applies after that decision. Identity checks are not weakened by short leases, and short leases do not replace identity checks. The two are layers: identity decides who may have a credential, and the lease decides how long it stays valid.

etcd holds some of Keyvane's own coordination state. The lease value for credentials issued through keyvane-vault-plugin is a Vault setting and is not stored by us in etcd as a source of truth. If someone is looking for where the value lives, it lives in the plugin's configuration and in the Vault mount configuration, not in etcd. Do not duplicate it elsewhere, because two copies of a timing value will drift apart.

The plugin is written in Go, as is most of Keyvane. Code in the plugin should read the configured value and should not hard-code its own duration. A hard-coded duration in a code path would make the configured `default_lease_ttl` a lie for that path. If a new code path needs a different duration, it should take it from configuration or from an explicit per-role setting, and the reason should be written down.

Vault has its own limits that sit above the plugin's default. A mount can have a maximum lease, and the system has a global maximum. Our default of `300s` must stay below both. If someone lowers a maximum below the default, Vault will cap the lease and the default becomes meaningless for that mount. That would be a surprise to callers, so lowering a maximum should be coordinated with whoever owns keyvane-vault-plugin.

## Overrides

A role may ask for a different lease when there is a clear reason. Reasons we consider acceptable include a batch job that cannot renew while it runs, a legacy client that cannot renew at all, and a one-time bootstrap step. Reasons we do not consider acceptable include convenience, a wish to avoid fixing client code, and a worry about load that has not been measured.

An override should be narrow. Prefer a per-role setting over a change to the mount default. Prefer a modest increase over a large one. Write down the owner of the override and the reason, so it can be reviewed and removed. An override with no owner and no reason is a candidate for removal.

Overrides should never go below the default without a reason either. A lease shorter than the default is allowed for especially sensitive credentials, but be aware that the load and availability costs above apply more strongly the shorter the lease gets.

The default itself, `default_lease_ttl` at `300s`, should not be raised to fix one team's problem. If one team needs more time, give that team a role-level override and keep the shared default as it is. Raising the shared default weakens the guarantee for everyone to help one consumer.

## What would make us revisit

We should reconsider `300s` if we see evidence, not just opinion. Signals worth acting on:

- Renewal traffic that measurably strains Vault or the plugin, shown by latency or error rates in monitoring and not by guesses.
- Repeated incidents where short leases caused application outages that a slightly longer lease would have avoided, and where fixing the client was not practical.
- A change in Vault behaviour that alters the cost of issuance or renewal.
- A change in our threat model, for example if leaks become much rarer or much more common than we assumed.

If we revisit, the question to answer is whether the damage-limiting benefit still outweighs the operational cost. The benefit is the reduced window for a leaked credential. The cost is renewal load, availability dependence, and client complexity. Moving the number in either direction trades one for the other, and the trade should be argued with data.

We should also revisit if the number turns out to be wrong in practice because of clock or network behaviour in some environment. That is a reason to adjust client renewal timing first. Only if that fails should the default itself change.

## Rollout and operations notes

When the setting first reaches an environment, expect a burst of renewal activity as existing consumers adjust, and expect some clients to break if they never renewed before. Roll it out in stages, starting with consumers that already renew correctly. Watch renewal failures and credential-expired errors from applications. Keep a way to apply a role-level override quickly for a consumer that breaks, so a client bug does not force a rollback of the default.

For on-call: if an application reports that a credential suddenly stopped working after running fine for a short while, check first whether it is renewing. This is the most likely cause by a wide margin. Check Vault and plugin health second. Check identity and mTLS third, since those fail differently, usually at connection time and not mid-session.

For security engineers: audit logs show issuance and renewal. A consumer that issues far more often than its peers may be failing to renew and re-fetching every time, or may be misbehaving. A consumer that renews for a very long time may be holding a credential longer than expected, and the maximum lease is what bounds that. Both patterns are worth a look.

For application teams: treat a credential as something that expires. Do not write it to disk, do not put it in a long-lived cache, and do not pass it to another process that cannot renew it. Fetch through the client, renew early, retry with backoff, and re-fetch if renewal fails. Test your service by letting a lease expire on purpose, and confirm it recovers without a restart. That test is cheap and finds most problems before production does.

## Alternatives we rejected

A long default, with short leases only for sensitive roles. This puts the burden of safety on whoever configures each role, and people forget. A missed role stays long-lived. We prefer to make the safe value the default and make long leases the exception that needs a reason.

No default, forcing every role to choose. This gives the same forgetfulness problem in a different shape, and it makes every new role harder to create. A sensible default removes a decision most people do not want to make.

Very short leases everywhere, to the point of one-use credentials. This maximises damage limiting, but the load and availability cost grows quickly, and most client libraries and application patterns are not ready for it. We keep this option for particularly sensitive cases through role-level overrides, and not as the shared default.

Relying on revocation alone and keeping leases long. Revocation is useful, but it needs someone to notice the leak first and then act, and caches delay its effect. Expiry needs no one to notice. That is the main reason we lean on lease length as the first line and treat revocation as the second.

## Open points

We have not fixed a recommended renewal fraction for clients. Different clients behave differently, and we would rather collect experience first. When we have enough, it should be written in a separate note and linked here.

We have not decided whether to publish a dashboard of renewal health for application teams. It would help them see problems early, but it is a separate piece of work.

We have not settled how to review existing overrides on a schedule. For now, review them whenever the plugin's configuration changes or an incident points at them.

The decision itself is settled: keyvane-vault-plugin sets `default_lease_ttl` to `300s`, because short leases limit the damage of a leaked credential. Everything above supports that line or records what it costs.
