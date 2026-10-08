---
id: 01K8A0GF2SD9D837T4VGCF6BHE
created: 2025-10-23T23:24-03:00
---

# keyvane-audit-log: general direction chosen

We settled on a general direction for keyvane-audit-log. This note keeps the shape of it and the reasons, so nobody has to rebuild the argument later. It holds no tuned values on purpose. Those belong in config and in the places where they get reviewed.

## Context

keyvane-audit-log records who asked for which credential, who rotated what, and what the system decided. Security engineers read it after something goes wrong. Application teams read it to find out why a request was refused. These two groups want different things from it, and most of the argument was about that.

## What we chose, in one paragraph

Treat keyvane-audit-log as an append-only record that is separate from the services it describes. Services emit events and cannot edit or delete them. Events carry a workload identity, not a person's name or a shared account. Reading is a different permission from writing, and neither one is granted by default.

## Append-only first

We prefer a record that can only grow. If someone can quietly edit history, the log is worth nothing in an incident. Corrections go in as new entries that point at the older one. We accept that this makes the log a bit untidy, because a tidy log that can be rewritten is worse.

## Separation from the issuing path

keyvane-audit-log should not live inside the same failure domain as credential issuing. If the issuing side is compromised, the attacker should not be able to clean up behind themselves. So the writer side is narrow and the storage sits outside the services' own permissions.

## Identity on every event

Each event names the workload that acted, using the same identity system the rest of Keyvane already uses. We did not want a second identity scheme just for auditing. When an event cannot be tied to a workload identity, that is itself something to flag, not something to fill in with a guess.

## Transport

Events travel over the same mutually authenticated channels as other internal traffic. No plain side channel for logs, even inside the cluster. It is more setup, but it avoids a weak link that nobody remembers to review.

## What gets recorded

We record decisions and their inputs: the request, the policy outcome, the issuer, the result. We record the fact that a secret was rotated and which consumers were told. We do not record the secret material itself, nor anything from which it could be derived. This is the firmest rule in the note.

## What we keep out of the log

No credential values, no tokens, no key bytes, and no raw request bodies that might contain them. If a field might hold a secret, it is redacted at the source, not downstream. Redaction at the reader is too late, since the data is already stored.

## Failure behavior

When the log cannot accept an event, the preferred direction is to fail closed for sensitive operations like issuing and rotating, and to degrade more gently for low-risk reads. The details of which operations count as sensitive are to be reviewed with the security engineers, and are not fixed here.

## Buffering

Short local buffering is fine so that a brief outage does not stop work. But the buffer is not a place to hold events for a long time, and it must not be a place where events can be altered. Dropping events silently is not allowed. If something is lost, that loss is recorded.

## Ordering and time

We want a reliable order of events per workload and a rough global order. We do not promise strict global ordering, because the cost is high and nobody has shown they need it. Timestamps come from the writer side, not from the caller, so a caller cannot choose its own time.

## Storage

The backing store should be one we already operate. We lean toward reusing existing infrastructure over adding a new system, since each new system is one more thing to run, patch and watch. Vault's own audit facilities may feed into this, but keyvane-audit-log is the record the team treats as the main one.

## Retention

Retention is a policy matter and gets set by the people responsible for compliance, not hard-coded by developers. The code should make retention configurable and make expiry deliberate. Nothing should age out by accident.

## Access to read

Reading keyvane-audit-log is a permission that is granted on purpose. Security engineers get broad access. Application teams get a view limited to their own services. We want the narrower view to be the easy default, so that a team does not ask for broad access just to debug one request.

## Integrity checks

We want a way to notice tampering, such as chaining entries or periodic signed checkpoints. Which mechanism to use is still open. The direction is only that tampering should be detectable by someone who does not trust the writer.

## Rejected directions

- Letting each service keep its own local audit files and collecting them later. Too easy to lose or alter.
- Letting the log be edited for cleanup or privacy requests. Use new entries or controlled redaction at write time instead.
- Logging full payloads for convenience. Too risky.
- A separate identity scheme for auditing. Adds work, no real gain.

## Open questions

- Which operations are sensitive enough to fail closed.
- Which integrity mechanism fits best.
- How application teams search their own slice without heavy load on the store.
- How to test failure behavior without touching production data.

## Revisit when

Reopen this if the storage choice changes, if the identity system changes, or if an incident shows that the separation was not enough. Until then, new work on keyvane-audit-log should follow the direction above and raise exceptions in review instead of quietly working around them.
