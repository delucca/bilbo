---
id: 01K1G60KGCKM1EQVDNF7A2E5BP
created: 2025-07-31T09:06-03:00
sources:
  - "code: plugin/backend.go"
---

# keyvane-vault-plugin design

keyvane-vault-plugin is the Vault secrets engine for Keyvane. It is written in `Go 1.23` and mounted at the path `keyvane/`. In code, logs and chat the short name is `kvplug`, which means the same thing as `keyvane-vault-plugin`. This note records how the plugin is shaped and why. It is for whoever touches the plugin next, human or agent, so they don't have to rebuild the reasoning from the code.

Keyvane issues short-lived credentials to services and rotates secrets without redeploys. The plugin is the part that lives inside Vault. It does not try to be the whole product. Other Keyvane components decide who may ask for what. The plugin turns an approved request into a credential with a lease, and it carries out rotation when told to.

```
mount path: keyvane/
runtime:    Go 1.23
short name: kvplug
full name:  keyvane-vault-plugin
```

## Scope and boundaries

The plugin is a secrets engine and nothing else. It is not an auth method, and it is not a general key-value store. Vault already has good versions of both, and duplicating them would give security engineers two places to look when something goes wrong. If an application team wants a static secret that never rotates, the answer is the stock Vault engine for that. `kvplug` is for credentials that expire or get replaced on a schedule.

What the plugin does:

- Exposes roles under the `keyvane/` mount. A role describes what kind of credential is issued, for how long, and to whom.
- Issues credentials on read of a credential path. Each one is backed by a Vault lease, so revoking the lease revokes the credential.
- Rotates secrets that sit behind a role. Rotation can be triggered by a schedule, by an explicit request, or by a revoke.
- Records enough state to answer the question "what is currently live, and who holds it" without calling the downstream system.

What the plugin does not do:

- It does not decide policy for application teams. Vault policies on the mount paths do that, and Keyvane's own policy layer sits in front of them.
- It does not run its own service discovery. Callers already know where Vault is.
- It does not hold long-lived root credentials for downstream systems in plain form. Those are stored through Vault's own barrier, never in the plugin's etcd data.
- It does not push secrets into running processes. Services pull. The "without redeploys" part comes from services re-reading on lease expiry, not from the plugin reaching into them.

The boundary matters because the plugin runs inside Vault's trust domain. Anything it does is done with Vault's authority over the mount. Keeping the scope small keeps the review surface small, and security engineers can read the whole plugin in one sitting.

## Backend structure

The backend follows the usual Vault plugin layout. There is one backend object built with the Vault logical framework, a set of path definitions, and a set of handlers. The plugin binary is registered in the Vault plugin catalog and mounted at `keyvane/`. Nothing about the layout is unusual on purpose. A person who has read any other Vault secrets engine should be able to find their way around.

### Paths

Paths fall into four groups.

- Configuration paths hold mount-wide settings, such as how to reach the trust and storage layers and the default lease limits. Only operators write to these.
- Role paths define roles. A role names a credential type, a maximum lifetime, a default lifetime, and the identity pattern that is allowed to use it.
- Credential paths are what application teams read. Reading one issues a credential under the named role.
- Rotation paths let an operator request a rotation for a role or inspect the last rotation result.

Credential paths and role paths use the same role name, so the mapping from request to role is direct. There is no indirection through aliases. We tried an alias layer early and removed it, because it made it hard to answer "which role produced this credential" from a lease.

### Handlers

Handlers stay thin. A handler validates input, looks up the role, calls into an internal package that does the real work, and shapes the response. All issuing and rotation logic lives in internal packages that do not import the Vault framework types where it can be avoided. That keeps unit tests fast. Tests for issuing logic do not need a Vault test cluster. Only the handler tests do.

Errors from internal packages are typed. Handlers map the types to Vault's error responses: a bad request, a permission problem, or an internal failure. Anything that is not a known type becomes an internal failure and gets logged with the request id but without the credential material. We had a near miss where an error string included part of a secret, so error construction in the issuing code never formats credential values.

### Leases

Every issued credential gets a lease. The lease carries the role name and enough internal data for the revoke handler to find what to tear down. The revoke handler must be safe to call twice. Vault may call it again after a restart, and the downstream system may already have dropped the credential. A revoke that finds nothing to remove counts as success.

Renewals are allowed only up to the role's maximum. The plugin does not extend the underlying credential beyond what the downstream system supports. If the downstream system cannot extend, renew returns an error and the caller has to read again for a new credential. We prefer that over pretending to renew.

## Issuing flow

A service asks for a credential by reading a path under `keyvane/`. The flow has five steps, and the order is deliberate.

1. Vault authenticates the caller and attaches a token. The plugin does not see how that happened. It sees the token's policies and metadata.
2. The handler resolves the role from the path and checks that the caller's identity matches the role's allowed pattern. The identity is the SPIFFE ID carried in the token metadata, and the plugin never accepts an identity from the request body.
3. The plugin asks the issuing package for a credential of the role's type, with a lifetime clamped to the role's limits and the mount's limits.
4. The credential is created downstream. The plugin records a small state entry that says a credential exists for this role and identity, and then returns the credential to the caller with a lease.
5. The response contains the credential, the lease, and the actual lifetime granted. Callers should use the granted lifetime, not the one they asked for.

The state entry in step four is written before the response goes out, not after. If the process dies between the downstream create and the state write, we could leak a credential that nobody knows about. If we write state first and the downstream create fails, we have a state entry with nothing behind it, and a cleanup pass removes it. We chose the second failure mode because it is harmless. The first one is a live secret with no owner.

### Lifetime rules

Lifetimes are short by design. The role sets a default and a maximum. The mount sets an upper bound over all roles. The granted lifetime is the smallest of what the caller asked for, the role maximum, and the mount bound. If a caller asks for nothing, they get the role default.

We do not allow a role to be configured with a maximum above the mount bound. The write to the role path fails with a clear message. Raising the bound is an operator action on the configuration path and is audited like any other write there.

### Idempotence

Reading a credential path is not idempotent. Each read issues a new credential. That is how Vault dynamic secrets normally work, but it surprises application teams who treat a read as a lookup. The docs for application teams say this in the first paragraph. Client libraries cache the result until shortly before the lease ends, so a normal service does not read in a loop.

There is no request-deduplication key. We considered one and dropped it, because the caller can already control churn by caching, and a deduplication layer would mean holding credential values somewhere longer than the lease needs.

## Rotation

Rotation replaces a secret that sits behind a role without the callers noticing, as long as callers follow the lease contract. It is the half of Keyvane's promise that is about not redeploying.

The model is overlap. When a rotation starts, the plugin creates the new secret downstream and begins issuing it. The old secret stays valid for a grace period that is at least as long as the longest lifetime a live lease could still have. After that the old secret is removed. A service holding an old credential keeps working until its lease runs out, then reads again and gets the new one. Nobody has to restart.

### Triggers

Rotation starts in one of three ways.

- Schedule. A role can carry a rotation period. A background routine in the plugin checks for roles whose period has elapsed and rotates them. The routine only runs on the active node of the Vault cluster, as Vault's framework intends, and does nothing on standbys.
- Request. An operator writes to a rotation path for a role. This is the normal reaction to a suspected leak.
- Revoke. Revoking a mount-wide or role-wide set of leases can start a rotation of the secret behind them, if the role is set to do that. Without that setting, revoke only removes the leased credentials.

Only one rotation per role can be in flight. A second request while one is running returns the status of the running one instead of starting another. This avoids two overlapping rotations racing over which secret is the "new" one.

### Failure during rotation

A rotation can fail halfway. The rule is that the old secret keeps being issued until the new one is confirmed working. The plugin does a verification step against the downstream system after creating the new secret. If verification fails, the plugin removes the new secret, records the failure on the rotation path, and keeps issuing the old one. An operator can read the failure there.

If the plugin dies during rotation, the state entry for the rotation says which phase it reached. On startup, or when a new node becomes active, the plugin reads the in-flight rotation state and either finishes it or rolls it back. Each phase is written to be resumable. We aimed for steps that can be repeated without harm over steps that need a transaction, since the downstream systems rarely offer transactions.

The case we worry about most is a rotation that completes downstream but whose final state write is lost. Then the plugin believes the old secret is current while the downstream system has dropped it. The verification step on the next issue catches this: if the current secret fails verification, the plugin looks for a newer one before returning an error. That path is rarely used, and it needs the most testing.

### Grace period

The grace period is computed, not configured separately. It comes from the longest lease that can still be live, plus a margin. A role with a long maximum lifetime therefore has a long overlap. That is the correct trade: it keeps old credentials alive for as long as callers might legitimately hold them. Operators who want fast rotation should shorten the role's maximum lifetime, not argue with the grace period.

An emergency rotation can cut the grace period short. This is a separate explicit request. It revokes the leases that depend on the old secret as part of the same operation. Callers see failures and have to read again. That is the intended behavior when the old secret is believed to be compromised.

## Identity and transport

Identity in Keyvane is SPIFFE. Services present a SPIFFE identity over mTLS, and the layer in front of Vault turns that into a Vault token whose metadata carries the SPIFFE ID. The plugin reads the ID from the token metadata and nowhere else.

The plugin also has to talk to other parts of Keyvane. Those connections use mTLS too, with SPIFFE identities on both sides. The plugin has its own identity, separate from any caller. Its identity is what downstream systems and the storage layer see. Callers' identities never leak onto outbound connections, so the downstream audit trail shows the plugin acting, with the caller recorded in the plugin's own records.

### Matching rules

A role's allowed identity pattern is matched against the caller's SPIFFE ID. Matching is exact or prefix-based on the path portion of the ID within one trust domain. We do not support regular expressions. They are easy to get wrong in a way that grants too much, and a security engineer reviewing a role should be able to read the pattern at a glance.

A role that matches no one is valid. It is how operators stage a role before opening it. A role with an empty pattern never matches. We do not treat empty as wildcard. That was a deliberate choice after discussing the failure mode of a forgotten field.

### Certificates and expiry

SPIFFE identities are themselves short-lived and rotate on their own. The plugin has to cope with its own certificate changing under it. Connections are rebuilt from the current identity source rather than held for the life of the process. A rotation of the plugin's certificate must not interrupt in-flight issuing. New connections pick up the new certificate, and old ones drain.

If the plugin cannot get a valid identity, it stops issuing and returns an error that says so. It does not fall back to anything weaker. Failing closed is the whole point of the plugin.

## State and storage

There are two places the plugin keeps data. Vault's own storage, reached through the framework's storage interface, holds anything that must be protected by the Vault barrier. That means configuration secrets, role definitions and the references needed to revoke leases. The second is etcd, which Keyvane uses for coordination state that several Keyvane components share.

The split follows one rule: if losing confidentiality of the data would hurt, it goes through Vault's barrier. etcd holds only what is safe to read by anyone who can read etcd, such as which role has a rotation in flight, which phase it is in, and timestamps. No credential values and no downstream root secrets go into etcd.

### Why etcd at all

Vault storage alone would be simpler. We use etcd because other Keyvane components, including the policy layer and the tooling that reports on rotation, need to see the same rotation and issuance facts without calling into Vault for each question. Vault is the system of record for secrets. etcd is the system of record for "what is happening". Having two systems means the two can disagree, and the design accounts for that.

### Disagreement between the two

The plugin treats Vault storage as authoritative for what exists and etcd as authoritative for what is in progress. After a crash, a reconciliation pass compares the two. If etcd says a rotation is running and Vault storage shows the rotation finished, etcd gets corrected. If Vault storage shows a credential that etcd has no record of, the plugin adds the record. If etcd shows a record with nothing behind it in Vault, the record is removed after a delay that avoids racing with a write that is still happening.

The reconciliation pass is meant to be boring. It should run at startup and on leadership change, and it should log what it fixed. A reconciliation that fixes many things on every start is a bug somewhere else, and the log line is there to make that visible.

### Watches and leader behavior

Only the active Vault node writes rotation state. Standbys do not touch it. Other components may watch the etcd keys to see changes quickly. The plugin does not depend on those watchers. Nothing in the plugin's own correctness relies on someone else reading etcd.

If etcd is unreachable, issuing from existing roles can continue, because issuing depends on Vault storage and the downstream system. Rotation cannot start, since it needs to claim the in-flight slot for the role in etcd. The plugin reports the rotation as blocked and does not try a rotation without the claim. That means an etcd outage delays rotation but does not stop credentials being issued. We accepted that: a delayed rotation is recoverable, and a rotation without coordination is how two nodes end up creating competing secrets.

## Operations and failure behavior

Operators interact with the plugin through Vault's normal tools, with the mount at `keyvane/`. The plugin does not ship its own admin CLI. If someone needs a helper, it should be a thin wrapper over Vault's API, and it belongs in the Keyvane tooling, not in the plugin.

### Logging and audit

Vault's audit device records requests to the mount. The plugin adds its own structured log lines for internal events such as rotation phases and reconciliation. Both carry a request id so they can be joined. Log lines never contain credential values, root secrets or the full token. They do contain the role name and the caller's SPIFFE ID, which is how a security engineer answers "who asked for this".

We keep log volume low on the issue path. One line per issue is enough. Debug detail is behind a flag that operators turn on temporarily, and the debug output still obeys the no-secret-values rule. That rule is enforced by review and by a test that scans the issuing code's log calls for forbidden fields.

### Health and visibility

The plugin reports its own state through a status path under the mount. It shows whether the plugin has a valid identity, whether it can reach etcd, and the last rotation result per role. It is meant for quick triage. When issuing fails for application teams, the first question is whether the problem is the caller, Vault, the plugin's identity, etcd or the downstream system. The status path answers the middle three.

### Upgrades

Upgrading the plugin means registering a new binary in the Vault plugin catalog and reloading the mount. Reloading must not drop leases. Lease data is in Vault storage and does not depend on the process. In-flight rotation state is in etcd and the new process picks it up through the reconciliation pass.

We keep the stored formats backward compatible across at least one upgrade step, so that a rollback to the previous binary works. A change that cannot be rolled back needs a note in the release description and an explicit operator step. The default is to avoid such changes. Format changes are done in two releases: the first reads both formats and writes the old one, the second writes the new one.

### Go version and build

The plugin is built with `Go 1.23`. Moving to a newer toolchain is a normal change but has to be checked against the Vault version in use, since the plugin links against Vault's SDK and the SDK's expectations about the toolchain can lag. The build produces one static binary. Vault verifies it against a checksum that the operator registers in the catalog, so a rebuilt binary with different bytes needs a new registration.

### Failure summary

The behavior under each failure, for quick reference:

- Downstream system down: issuing fails with an internal error that names the downstream system as the cause. Existing leases are unaffected until they expire. Revokes are retried by Vault.
- etcd down: issuing continues, rotation is blocked and reported as blocked.
- Plugin identity invalid: issuing and rotation stop, status says so, nothing falls back.
- Vault standby takeover: the new active node runs reconciliation, resumes or rolls back any in-flight rotation, then accepts work.
- Bad role configuration: the write is rejected at the role path. A role that was valid earlier and becomes invalid after a mount bound change keeps working for existing leases but refuses to issue new ones.
- Caller identity mismatch: permission error, logged with the caller's SPIFFE ID and the role name.

## Decisions worth remembering

A few choices look arbitrary from the code and should not be undone without the reasoning.

- State is written before the response, so a crash leaks a harmless record instead of a live secret nobody owns.
- Identity comes only from token metadata, never from the request body, because anything in the body is attacker-controlled.
- Matching uses exact or prefix rules, not regular expressions, so a role can be audited by reading it.
- An empty identity pattern matches no one, so a forgotten field fails closed.
- No confidential data goes into etcd. If a future feature needs shared state that is sensitive, it goes through Vault's barrier and other components get a reference or a derived non-secret fact.
- Rotation overlaps and the grace period is derived from live lease limits, so callers that follow the lease contract never see a failure from a normal rotation.
- Only one rotation per role runs at a time, and a second request returns the status of the first.
- The plugin is a secrets engine only. Auth and static secrets stay with the stock Vault pieces.

## Open questions

- Whether the reconciliation pass should run on a timer as well as at startup and leadership change. Right now it does not, and drift between etcd and Vault storage would only be noticed at the next restart or failover. A timer is cheap, but a timed reconciler that quietly repairs things could hide the bugs that cause drift.
- Whether to support several downstream systems behind one role, for credentials that span two systems. That would complicate the rotation model, since overlap and verification would have to hold for both at once. For now a role maps to one downstream system, and teams that need two use two roles.
- Whether the status path should expose per-role issue counts. It would help capacity thinking, but it means keeping counters, and counters are one more thing to get wrong across node changes. Vault's own metrics may be enough.
- How aggressively to shorten default lifetimes as downstream systems get better at fast creation and removal. Shorter is safer, but it raises load on both the plugin and the downstream system, and it makes any outage of either more visible to callers.

If you pick up work on `kvplug`, read the rotation section and the state split first. Most of the subtle bugs so far have been in the gap between what Vault storage says and what etcd says, and in what happens when a crash lands between two writes.
