---
id: 01K92SACJ9ZG8QPJTHCFPJYN5E
created: 2025-11-02T14:19-03:00
---

# keyvane-audit-log: things to watch out for when changing it

Notes for anyone touching keyvane-audit-log. None of this is new news, but each item has bitten someone or is an easy way to bite the next person. Read it before you change event shapes, storage, or the write path.

## Why this component is different

keyvane-audit-log is the record security engineers rely on when something goes wrong. A bug elsewhere in Keyvane is usually an outage you notice. A bug here can be silent: events go missing or get altered, and nobody finds out until an investigation. Treat every change as one that could erase evidence.

## Never block credential issuance on logging by accident

The audit path sits next to issuing and rotating credentials. If a change makes the log write slower or makes it fail, check what the caller does about it. Decide on purpose whether a failed write should fail the request or not, and keep that behavior consistent across all call sites. Do not let it drift because one handler swallows errors and another returns them.

## Fail-closed versus fail-open

Auditors expect that an action with no record did not happen. That argues for failing closed. Availability argues the other way. Whatever the current behavior is, do not flip it as a side effect of a refactor. If you must change it, say so loudly in the change description and tell the security engineers.

## Event schema changes

Consumers parse these events outside this repo: dashboards, alert rules, retention jobs, people's scripts. Adding a field is usually fine. Renaming, removing, or changing the meaning or type of a field is not. Keep old fields until you have confirmed nobody reads them. Watch for changes in timestamp format and in how identities are written.

## Secret material must never reach the log

Log the fact of access, the path or name, the caller identity, and the outcome. Never log secret values, tokens, or full request bodies. Be careful with error strings and debug output that wrap a request, because they can carry a value into the log. Be careful with Go struct printing too, since a whole struct can include fields you forgot about. Add a test that feeds a fake secret through the path and checks it does not appear.

## Caller identity

Events are only as useful as the identity attached to them. The identity comes from the mTLS peer and its SPIFFE ID. Do not take identity from a request field the caller controls. When you touch the middleware order, check that the identity is already established before the event is built, and that unauthenticated and denied attempts are logged as well as successful ones.

## Ordering and timestamps

Events from several instances can arrive out of order. Do not assume the log is strictly sorted. Use one clock source consistently, and keep the time the event happened separate from the time it was written. If you add batching or buffering, make sure it does not reorder events from the same caller or hide the real time of the action.

## Durability and buffering

Any in-memory buffer is a place where events vanish on a crash or a restart. Check what happens at shutdown, during a rolling restart, and when the backing store is unreachable. Flush on exit paths, and make sure a drop is counted and visible, never silent. Test with a killed process, not only a clean stop.

## Interaction with Vault and etcd

Vault has its own audit devices, and keyvane-audit-log should not be assumed to duplicate or replace them. Be clear about which events come from which layer before you remove anything as redundant. If the log uses etcd in any way, remember that etcd is not a bulk event store. Large or frequent writes can hurt the rest of the system that shares it, so check the load before increasing write volume.

## Tamper evidence

If the log has hashing, chaining, or signing, a change to the order of fields or to serialization can break verification of old records. Keep serialization stable and test verification against records written by the previous version. Do not add a code path that rewrites or compacts past events without thinking about what it does to that guarantee.

## Retention and deletion

Cleanup logic is dangerous here. A wrong filter or a unit mix-up can delete far more than intended. Keep deletion behind explicit configuration, test it against sample data first, and never make a new default more aggressive in the same change as something unrelated. Retention may also be tied to compliance obligations that are not visible in the code.

## Testing habits

Unit tests that only check a mock was called will not catch dropped or altered events. Add tests that read events back from the real sink, including failure cases: sink down, slow sink, malformed input, and concurrent writers. Run with the race detector, since the write path is shared by many goroutines.

## Rollout

Deploy changes so old and new versions can run side by side, because instances do not all update at once. Watch event volume and error counts after a rollout. A sudden drop in events is a bad sign, not a quiet success. Keep a way to roll back that does not need the log to be healthy.

## When in doubt

Ask the security engineers who consume the log before merging anything that changes what is recorded. A short question is cheaper than a gap in the record.
