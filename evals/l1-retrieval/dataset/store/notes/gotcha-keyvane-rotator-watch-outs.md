---
id: 01K566T51Q2EJ83ZGMYP5R9CXA
created: 2025-09-15T05:10-03:00
---

# keyvane-rotator: things to watch out for when changing it

These are general traps for anyone touching keyvane-rotator, written quickly from what tends to go wrong. Nothing here is a spec. If a point looks stale, check the code and fix this note. The component sits between Vault, etcd and the services that consume short-lived credentials. Almost every bug in it comes from that position: it has to keep three or four systems agreeing about which secret is current, and any of them can be slow, wrong or gone at the worst moment.

Read the surrounding code before changing anything. The rotator looks like a loop that fetches and writes, but a lot of its behavior lives in ordering, timing and what it chooses to leave alone. A small tidy-up that reorders two calls can turn a safe rotation into one that locks consumers out.

```
keyvane-rotator -> Vault -> etcd
          \-> SPIFFE identity check over mTLS
```

## The order of steps is the contract

A rotation is a sequence: create the new secret, make it available, tell or wait for consumers, retire the old one. The order matters more than any individual step. Creating the new value before retiring the old one is what keeps services working during the change. If you refactor and the retire step ends up earlier, nothing fails in a unit test, because mocks do not care about order. It fails in production when a consumer still holds the old value and the backend already refuses it.

When you add a step, decide explicitly where it goes relative to the others and write that down in a comment next to the code. When you remove a step, check whether another step quietly depended on its side effects, such as a cache refresh, a state write or a delay that gave consumers time to pick up the new value.

Do not make steps parallel just because they look independent. Several of them share state in etcd or share a Vault path, and the independence is only apparent. If you do parallelize, think about what a half-finished run looks like at every point.

## Overlap window between old and new secrets

Consumers do not switch at the same instant. There has to be a period where both the old and new credentials are valid. Changes that shorten that period, or that retire the old credential on a trigger that is not tied to consumer progress, are the most dangerous ones in this component.

Watch for places where the overlap is implicit. For example, a lease on the old credential may simply run out on its own, and the rotator relies on that. If you change how leases are requested or renewed, you can shorten or lengthen the overlap without touching any line that mentions it. Check what the backend does when a new credential is issued: some backends revoke the previous one right away, and then the overlap does not exist at all unless the rotator arranges it.

Also remember slow consumers. A service that was restarting, partitioned or paused during rotation will come back holding something old. Think about what it sees when it returns, and whether the old value is still good enough for it to fetch the new one.

## Vault behavior is not uniform

Vault engines differ in how they handle leases, revocation, renewal and versioning. A change that works against one kind of secret can break another. Before assuming a call behaves a certain way, find which engine type it targets in the code path you are changing, and read the actual docs for that engine rather than relying on memory.

Token handling deserves its own care. The rotator authenticates to Vault and that token has its own lifetime. If you change timing, retries or long waits, make sure the rotator's own token cannot expire in the middle of a rotation. A rotation that creates a new secret and then loses the ability to finish is worse than one that never started.

Policies are another trap. The rotator should have only the Vault permissions it needs. When you add a new call, you will probably need a policy change, and that change lives outside this repository's code. Tests with a permissive dev setup will pass and the real deployment will be denied. Note the policy need in the change description so whoever deploys it knows.

Vault can also be sealed, unreachable or failing over. Do not treat those as ordinary errors with the same retry path as a bad request. They need patience, not a flood of retries.

## Leases, TTLs and clocks

Anything involving lifetimes is sensitive to clock skew and to the gap between when a lease is created and when the rotator thinks it was created. Compute expiry from values the backend returns where possible, not from the local clock plus a remembered duration. Mixed sources are a classic way to get a rotation that fires a little late.

Schedule rotation well before expiry with room for a failed attempt and a retry. If you change the margin, think about the case where the first attempt fails and the second has to succeed before the old credential dies. A margin that looks generous in the happy path can be too thin once retries and backoff are in the picture.

Be careful with renewal versus rotation. Renewing extends something that exists; rotating replaces it. Code that tries renewal first and rotates on failure has to cope with a renewal that half-worked. Also watch for maximum lifetimes: a renewable lease can hit a hard ceiling, after which only replacement works. If your change assumes renewal can go on forever, it will break at the ceiling, which may be long after the change ships.

## etcd state and what it records

etcd holds the rotator's view of what is current and what is in progress. Treat that state as the source of truth for recovery, and keep it in step with what actually happened in Vault. The dangerous case is a state write that claims progress the backend never made, or the reverse: the backend changed and the state never recorded it.

Think about writes that must be atomic. If two keys have to change together, use a transaction rather than two puts. A crash between two puts leaves a state that nobody planned for, and recovery code usually only handles the states someone thought of.

Mind the data model when you change it. Existing keys written by an older build must still be readable by the new one, and a rollback should not choke on keys written by the new one. Add fields in a way that older code ignores, and avoid renaming or repurposing existing ones. If a format change is truly needed, plan a transition where both are understood for a while.

Also respect etcd's limits on value size, request rate and watch behavior. Storing large blobs, or secret material itself, in etcd is almost never right here. Store references and status, and let Vault hold the secrets.

## Concurrency, leadership and double rotation

Assume more than one instance of keyvane-rotator can be alive at once, even if the deployment normally runs one. Restarts, rolling updates and network splits all produce overlap. A rotation done twice at the same time can issue two new credentials, retire the wrong one, or overwrite state with an older view.

If the component uses a lock or election in etcd, understand how it fails. A lock holder that stalls and wakes after its lease has gone must not keep acting as though it still holds the lock. Check that every write after a long operation re-verifies ownership or uses a guard in the write itself, such as a revision check.

Do not add long blocking calls inside a section that holds a lock without thinking about the lease on that lock. Slow Vault responses are normal enough that this will happen. Also avoid adding shared in-memory state accessed from several goroutines without clear ownership; races here show up rarely and are painful to reproduce.

When you shut down, finish or cleanly abandon the current rotation. Context cancellation should stop work at a safe point, not between a Vault write and the matching state write.

## Retries and idempotency

Every step should be safe to run again. The rotator will be restarted in the middle of things, and the retry logic will repeat calls whose first attempt actually succeeded but whose reply was lost. If a step creates something, it needs a way to tell whether it already did, or its leftovers need to be harmless and cleaned up later.

Be wary of retry loops that wrap more than they should. Wrapping a whole multi-step function in a retry will redo the early steps each time. Retry the smallest unit that is safe to repeat.

Backoff needs jitter and a ceiling. Many rotations are scheduled around the same times, and if they all fail at once and retry in step, they hit Vault together and make the outage worse. Also separate errors worth retrying from errors that will never succeed, such as a denied permission or a malformed request. Retrying a permanent failure only hides it and burns the time margin before expiry.

## Partial failure and cleanup

A rotation can fail anywhere. For each step, ask what is left behind if the process dies right after it. Orphaned credentials in Vault, stale entries in etcd and a new secret that nobody was told about are all realistic leftovers. The rotator should be able to find and deal with them on the next run, not leave them for a person.

Cleanup code is dangerous because it deletes things. Make sure it identifies leftovers precisely. A broad match that sweeps up a credential still in use will cause an outage that looks unrelated to the change. Prefer to retire things only when state positively says they are done with, not when they are merely absent from some list.

When a rotation cannot finish, prefer leaving the old credential valid and reporting loudly over pushing forward. Availability of existing consumers comes before freshness of the secret, unless the secret is known to be compromised, which is a different path with different rules. Do not blur the two when editing error handling.

## Identity: SPIFFE

The rotator and its consumers identify each other with SPIFFE identities. Changes that touch who may request or receive a secret should be checked against how identities are matched. Matching on a prefix or on a loosely built string can authorize more workloads than intended. Be exact, and prefer comparing parsed identity parts rather than raw text.

Identity documents rotate too. The rotator's own identity can change while it is running, and code that reads it once at startup and caches it will eventually present a stale one. Use the mechanism that delivers updates, and make sure new connections pick up the fresh identity.

Do not widen authorization to fix a test. If a test fails because an identity is not allowed, the usual right fix is in the test setup. Loosening the rule to make it pass can slip through review as a small change.

## mTLS and connection handling

Connections between the rotator and its peers use mutual TLS. When you change client or server setup, keep verification on in both directions. It is easy to disable checks while debugging and forget. Search the diff for any option that skips verification before you submit.

Certificate reload is a recurring problem. Long-lived connections keep using the certificate they started with. After a rotation of the rotator's own certificate, old connections may continue to work until the other side rejects them. Know which connections are long-lived and whether they get recycled. Make sure a reload does not briefly leave the process with no usable certificate.

Trust bundles matter as much as leaf certificates. A change in the trusted authorities can cut the rotator off from Vault, etcd or consumers, and the failure looks like a generic connection error. When touching trust setup, test the case where the bundle changes while the process runs.

Timeouts on connections should be set deliberately. A missing timeout turns a stuck peer into a stuck rotation, and a stuck rotation holding a lock turns into a stuck system.

## What consumers see

Consumers are the reason this component exists, and they are not under your control. Different applications pick up new credentials in different ways: some watch, some poll, some only read at startup. A change in how or where the new value is published can break the ones that read in a way you did not think about.

Keep the published shape stable. Field names, formats and locations are an interface. If you must change one, support the old form for a long enough transition and tell the application teams before the change lands, not after.

Think about what a consumer gets if it asks mid-rotation. It should get something valid, not a half-written value or a gap. If the publish step is not atomic from the consumer's side, say so in a comment and make sure the consumer-facing order is safe.

Also remember that consumers may cache. A shorter lifetime on your side does nothing if a consumer holds the old value past it. When changing lifetimes, think about the slowest realistic consumer, not the typical one.

## Logging, errors and secret exposure

Never log secret values, tokens or full responses from Vault. This is easy to violate by accident: logging a whole response object or a struct that includes a secret field, or wrapping an error with the request body. When adding a log line near secret handling, look at every field that gets printed.

Error messages travel. They end up in logs, metrics labels, tickets and chat. Keep them useful without including sensitive content. Include which step failed and which logical target was involved, but not the material itself.

Also be careful with debug modes and panics. A panic that dumps local state, or a debug flag that prints requests, can leak what the rest of the code protects. Zero or drop secret buffers when you are finished with them where practical, and avoid copying secrets into long-lived structures, caches or test fixtures.

Test data should be obviously fake. Do not paste real values into tests, fixtures or comments, even for something that is supposedly expired.

## Configuration changes

Config affects timing and safety, so a changed default is a behavior change for everyone who did not set it. When adding an option, choose a default that is safe if nobody reads the docs. When changing an existing default, treat it as a breaking change and call it out.

Validate config at startup and fail early on nonsense. A rotator that starts with a bad interval or a missing target and only discovers it hours later is worse than one that refuses to start. Check relationships between settings too, such as a retry window that exceeds the time available before expiry.

Be careful with settings that can be reloaded at runtime. A reload in the middle of a rotation should not apply half the new values to the current run. Either snapshot config at the start of each rotation or make sure the reload is safe at any point.

## Observability

If rotation fails quietly, the first sign will be an outage when the credential expires. Keep metrics and logs that show last success, time until the next required rotation, current failures and retry state. When you change the flow, update them. A new step with no visibility is a blind spot, and a renamed metric silently breaks alerts that other people depend on.

Alert on approaching expiry, not only on errors. Some failure modes produce no error at all, such as a scheduler that stopped firing or a lock that nobody releases. A signal tied to how stale the current secret is will catch those.

Keep log lines structured and consistent so people can follow one rotation across steps. Include a correlation field for the rotation run, so that logs from the rotator, Vault audit entries and consumer-side messages can be lined up during an incident.

## Testing

Unit tests with mocks are not enough here. Mocks accept calls in any order and never expire anything. Add tests that exercise ordering, restarts in the middle of a rotation and repeated runs. Fault injection at each step is worth the effort: kill the process after every step and check that the next start recovers.

Run integration tests against real Vault and etcd instances where possible, with realistic lifetimes scaled down. Pay attention to cases that only appear with real timing, like a lease expiring while a retry is waiting, or a lock lost while a call is in flight.

Test the unhappy paths for identity and mTLS too: wrong identity, expired certificate, changed trust bundle. These are the cases that fail in production and are rarely touched by ordinary tests.

Do not rely on sleep-based timing in tests. Use controllable clocks so the test is stable and can cover long lifetimes quickly. Flaky tests in this component tend to be ignored, and then a real race hides among them.

## Rollout and rollback

Assume the new build and the old build will run side by side for a while, and that you may need to go back. Anything that changes state format, published shape or timing has to work in that mixed period. Ask what the old build does with state the new one wrote, and the other way round.

Roll out gradually where the setup allows, starting with low-risk secrets, and watch a full rotation cycle complete before extending. A change that looks fine until the first real rotation can sit unnoticed for a long time, because rotations are infrequent compared with deploys. Do not read a quiet dashboard right after deploy as proof of success.

Keep a plan for forcing a rotation by hand and for freezing rotation if something goes wrong. Make sure your change does not remove or break those manual paths, since people reach for them when things are already bad.

## Easy-to-forget items

- Update docs and runbooks when behavior changes; operators read those during incidents.
- Check that new Vault calls have matching policy and that new etcd keys have matching access rules.
- Re-read the diff for removed checks, skipped verification and widened matching.
- Think about the first run after a long idle period or after a restart with stale state.
- Check shutdown handling whenever you add a goroutine or a background loop.
- Look at who else reads the state or the published values before renaming anything.
- Keep dependency upgrades for the Vault client, etcd client and SPIFFE libraries separate from logic changes, and read their release notes for behavior changes in retries, timeouts and certificate handling.
- If you are unsure whether a change is safe for live rotations, say so in the review and ask someone who has seen this component fail before.
