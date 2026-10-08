---
id: 01KZN4MPC3Q12T5CJ8Q6VR9G0G
created: 2026-08-10T03:09-03:00
---

# keyvane-rotator rotation spec

keyvane-rotator must rotate every secret at least 24 hours before its expiry. This is the one hard timing rule of the component, and everything else in this note is there to make it true in practice, or to say what happens when it cannot be met. If a secret reaches the point where fewer than 24 hours remain and it has not been rotated, that is a defect in keyvane-rotator or in the environment around it, and it has to be surfaced loudly. It is not a normal state to be tolerated quietly.

Written in a hurry after going through the design discussions again. The goal is that a later session does not have to rebuild the reasoning. Where something is not decided, it says so. Where a number would normally go and the number is not fixed yet, it is left out on purpose and the text stays general.

## Scope and the core rule

keyvane-rotator is the part of Keyvane that replaces secrets before they expire. Keyvane as a whole issues short-lived credentials to services and rotates secrets without redeploys. The issuing side hands out credentials to workloads. keyvane-rotator is the side that keeps long-lived or mid-lived secrets fresh so that application teams never see an outage caused by an expired secret, and so that nobody has to restart a service to pick up a new value.

The rule: every secret that keyvane-rotator manages is rotated at least 24 hours before its expiry. Read that literally. The deadline for a rotation to have completed is the expiry time minus 24 hours. It is not the time the rotation starts. It is not the time the new secret is written to the backend. It is the time the new secret is live and consumers can use it. A rotation that has written a new value but whose consumers still hold only the old one is not finished for the purpose of this rule.

Why 24 hours and not something tighter. A rotation can fail for boring reasons: the secret backend is briefly unavailable, a consumer is unreachable, a human has to approve something, a network partition separates the rotator from part of the fleet. A margin of 24 hours gives the on-call engineer a full working cycle to notice and fix the problem before any consumer is affected. A shorter margin would turn a small outage into an incident. A much longer margin would mean secrets are rotated far earlier than their intended lifetime, which wastes the short-lived design and increases churn. The margin is a floor, not a target: rotating earlier is always allowed, and the scheduler is free to rotate well ahead of the floor when load or policy suggests it.

What counts as a secret here. Anything keyvane-rotator is configured to manage and for which an expiry is known. That includes dynamic credentials whose lease has a known end, static secrets stored in HashiCorp Vault that carry a rotation policy, and certificates or identity documents that carry a validity end. If a secret has no expiry, the 24 hours rule does not apply to it, but it should still have a rotation policy based on age, and that is a separate concern from this spec.

What does not count. Credentials that a workload obtains directly from its own short-lived issuance path, and that it renews on its own schedule, are outside the rotator. Keyvane issues those, and the workload is responsible for renewing them. keyvane-rotator does not chase them.

## Where the data lives and who talks to whom

Three systems matter. HashiCorp Vault is the system of record for secret values and their expiry metadata. etcd holds the rotator's own coordination state: which secrets are scheduled, which rotation is in flight, which node holds the right to rotate a given secret. SPIFFE identities are how every party proves who it is, and mTLS is the transport for every call between keyvane-rotator and anything else.

The rotator is written in Go and runs as several replicas. Only one replica should act on a given secret at a time, and that exclusivity comes from etcd, not from any in-process lock. A replica that loses its claim in etcd must stop work on that secret promptly, even if it is halfway through, and must not write a new value to Vault after it has lost the claim. The reasoning is simple: two replicas rotating the same secret can produce two different new values, and consumers may end up reading a value that Vault has already superseded.

Identity. Each replica has a SPIFFE identity, and the access policies in Vault are written against that identity, not against network location. A replica can rotate only the secrets that its identity is allowed to touch. Consumers authenticate to the rotator with their own SPIFFE identity over mTLS when they ask for status or ask for an immediate rotation. The rotator never accepts a plain connection, and it never falls back to a weaker mode when certificate validation fails. A failed handshake is a failed call, and it is logged with the peer identity if one could be read.

State in etcd. The rotator keeps a record per managed secret: its expiry as last read from Vault, the computed deadline (expiry minus 24 hours), the time of the last successful rotation, the state of any rotation in flight, and a failure counter. The record is a cache and a coordination aid. Vault is authoritative for the expiry. If the cached expiry and the value in Vault disagree, Vault wins and the record is corrected, and the deadline is recomputed from the corrected expiry.

Clock handling. The deadline depends on wall-clock time, and so clock skew between replicas, Vault and etcd matters. The rotator should compare against the expiry using the time source it trusts most, and it should build in a safety allowance for skew. That allowance is taken from the margin before the deadline, never from the 24 hours itself. In other words the scheduler aims to finish earlier than the deadline so that skew cannot push completion past it. The exact allowance is a configuration value and is not fixed in this note.

## Scheduling

The scheduler decides when to start a rotation for each secret. The input is the expiry and the deadline. The output is a start time that leaves enough room for the rotation to complete, to be retried if it fails, and to be noticed by a human if retries run out, all before the deadline.

A rotation has a start window. The earliest start is bounded by policy: a secret should not be rotated so early that the short-lived property is lost. The latest start is bounded by how long a rotation takes plus how many retries we want to afford before the deadline. The scheduler picks a time inside that window. To avoid a thundering herd, start times are spread with jitter, so that many secrets created together do not all rotate in the same moment. Jitter must never push a start past the latest allowed time. If jitter and the latest start conflict, the latest start wins.

Prioritisation when the system is busy. Work is ordered by how close each secret is to its deadline. A secret closer to its deadline goes first, regardless of when it was queued. If the queue is long, the rotator does not drop low-priority items; it processes them in order and reports the backlog as a metric, because a backlog is a warning that the deadline is at risk for the items at the back.

New secrets and changed expiry. When a secret is first registered, or when its expiry is changed in Vault (for example a human extends or shortens a lease), the rotator recomputes the deadline and reschedules. A shortened expiry can put the deadline in the past at once. In that case the secret is already in breach of the rule on arrival, and the rotator starts the rotation immediately at top priority and raises the same alert as for any missed deadline, noting that the cause was a late registration or a shortened expiry rather than a failure.

Restart and failover. If a replica dies, its claims in etcd lapse and other replicas pick the work up. A rotation that was in flight is not assumed to have completed or to have failed. The new owner reads the state, checks Vault to see whether the new value was written, checks consumers to see whether they have it, and continues from the first step that is not confirmed. Each step of a rotation is written so that repeating it is safe.

Pause and freeze. Operators may pause rotation for a secret or for a group of secrets, for example during an incident in a dependent system. Pausing does not move the deadline. A paused secret that reaches the deadline still raises the alert. Pause is an operator decision to accept risk, and it must carry a reason and an owner in the record. The rotator should refuse to pause a secret in a way that silently outlasts the expiry itself.

## The rotation procedure

A rotation is a short sequence of steps. The point of writing them down is to say what must hold at each boundary so that a failure never leaves consumers without a working secret.

First, claim. The replica takes the claim on the secret in etcd. If the claim is held by someone else, it stops. If the claim is taken, it records the start of the rotation in the secret's record.

Second, generate or obtain the new value. Depending on the type of secret this means asking Vault to produce a new dynamic credential, generating a new key pair and asking the issuing path for a certificate, or asking the target system to create a new password. The old value stays valid throughout this step. keyvane-rotator never revokes the old value to make room for the new one. Both must be able to exist at the same time.

Third, store. The new value is written to Vault in a way that keeps the previous version readable for a period. Vault versioning is used for that. The write must be atomic from the reader's point of view: a consumer reading the secret sees either the old version or the new version, never a half-written one.

Fourth, distribute. Consumers get the new value. How depends on the consumer: some pull from Vault on their own schedule, some are notified by the rotator, some have a sidecar that reloads in place. The design goal of rotating without redeploys means consumers must be able to take a new value without a restart. If a consumer cannot, that is recorded as a property of the consumer, and the rotator treats such a consumer as higher risk and starts its rotations earlier inside the allowed window.

Fifth, verify. The rotator confirms that consumers are using the new value, where the consumer offers a way to confirm, such as a status call over mTLS, or a check that the old value has stopped being used. Where no confirmation exists, the rotator waits out a grace period configured for that consumer type and records the rotation as complete without confirmation, and says so in the record. The 24 hours rule is judged against the time this step completes.

Sixth, retire. After the grace period, the old value is revoked or left to expire. Revocation is the last step and is never done before verification. If verification failed or was inconclusive, the old value stays valid until its natural expiry, and the rotation is marked as incomplete so that it is retried.

Failure at any step. A failure in the first three steps leaves the old value untouched and consumers unaffected, so retry is safe and cheap. A failure in distribution or verification is more serious because some consumers may have the new value and some the old. This is why the old value is kept valid: both versions work during the overlap, and the retry can finish distributing. The rotator must never respond to a partial failure by rolling back the new value in Vault, since consumers that already took it would then hold a value Vault no longer shows. It rolls forward.

Idempotence. Each step checks the current state before acting. Generating a new value twice for the same rotation attempt is avoided by recording the attempt's identity in etcd before the value is created, and by looking for an existing result on retry.

## Failure, retries and alerting

Retries use backoff with jitter. The backoff is bounded so that the retries are spread across the window between the start and the deadline instead of being exhausted early. A rotation that fails often at the start should still have retries left near the deadline. Because the deadline is the thing that matters, the schedule of retries is derived from it: the closer to the deadline, the more often and the more urgently the rotator tries, up to a ceiling that protects Vault and the targets from overload.

Classifying failures. Transient failures, such as a timeout or a brief refusal from a busy backend, are retried. Permanent failures, such as a denied permission, a missing target, or a consumer that no longer exists, are not retried blindly. A denied permission usually means a policy drifted, and retrying will not help. The rotator marks the secret as blocked, records the reason, and alerts a human straight away, without waiting for the deadline to come close. The same goes for a mismatch of identity: if a SPIFFE identity does not match what policy expects, the call fails, and the rotator does not try another identity.

Alert levels. There are three levels, described in words because the thresholds are configuration.

- Warning: a rotation has failed more than once, or the backlog is growing, but the deadline is still comfortably far. This goes to the owning team's channel.
- Page: the deadline is close and the rotation has not completed, or a secret is blocked by a permanent failure. This goes to the on-call security engineer.
- Breach: the deadline has passed and the secret is not rotated. This is a violation of the 24 hours rule. It pages, it opens an incident record, and it stays open until the secret is rotated and someone has written down the cause.

A breach does not mean the secret has expired. It means the margin is gone. The remaining time before expiry is still the buffer in which to fix things, and the rotator keeps trying throughout it with the highest priority. If the expiry itself is reached without a rotation, consumers will start to fail, and that is a separate, more severe incident whose record should link back to the breach.

Dependency outages. If Vault is unavailable, nothing can be rotated, and the rotator reports itself as degraded. It does not guess. If etcd is unavailable, no replica can confirm a claim, so no replica should start new rotations; ones already in flight should stop after their current safe step. It is better to stop than to risk two replicas writing different values. When the dependency returns, the scheduler recomputes everything from the stored expiry values, and the items closest to their deadlines go first. A long outage may therefore produce a burst of work and some breaches, and those breaches are reported honestly, not hidden.

Observability. The rotator exposes metrics per secret class and in aggregate: time remaining until the deadline for the closest secret, the number of secrets inside their start window, rotation durations, retry counts, and the number of breaches. The most useful single number is the smallest remaining time to deadline across all secrets; the dashboards and alerts should centre on it. Logs record each step with the secret's logical name and the rotation attempt identity, and they never contain secret values. Anyone adding a log line must check that no value, not even a fragment, can reach it.

## Security properties and constraints

Secret values never leave memory unprotected. Inside keyvane-rotator values are held only as long as a step needs them, are not written to disk, and are not included in errors, traces or metrics. Go makes it easy to leak a value through a formatted error, so error construction around secret material has to be reviewed with that in mind.

All traffic is mTLS with SPIFFE identities. The rotator verifies the peer identity against an allow list tied to the operation: a consumer may ask for status of its own secrets only; an operator identity may pause, resume or force a rotation; a replica identity may rotate. There is no shared token, and there is no administrative backdoor that skips identity. Forcing a rotation is allowed because rotating earlier is always safe with respect to the rule, but it is audited, and it respects the same claim logic as scheduled rotations.

Audit. Every rotation, every pause, every forced rotation and every breach is recorded with the identity that did it and the time. The record lives with the rotator's state and is also shipped to the central audit trail used by security engineers. The audit trail must be sufficient to show, after the fact, that each secret was rotated at least 24 hours before its expiry, or to show exactly where and why it was not. That is the evidence an auditor would ask for, so the rotator should record both the expiry and the completion time for every rotation, not just a success flag.

Least privilege in Vault. The policy for the rotator allows writing new versions for managed paths and reading metadata such as expiry, and nothing more than needed. It should not be able to read values it does not rotate. Revocation rights are scoped the same way. When a new class of secret is onboarded, the policy change is reviewed as a security change, not as routine configuration.

Blast radius. A compromised replica should be able to harm only what its identity allows. Claims in etcd are per secret, so a replica cannot take over the whole fleet by taking a single lock. If a replica is suspected to be compromised, its identity is revoked, its claims lapse, and the remaining replicas take over, after which every secret that replica could touch is rotated out of schedule. That emergency rotation uses the same procedure and the same ordering rules, with priority set by exposure, not by deadline.

## Testing, rollout and open questions

What to test. The key property is the deadline rule, so tests are built around time. Use a fake clock and drive the scheduler through long periods quickly. Check that for any set of secrets and expiries, every rotation completes before the deadline when the backend behaves, and that breaches are raised when it does not. Include cases for shortened expiry, late registration, clock skew, a replica dying halfway through each step, loss of the etcd claim in the middle of a write, and a consumer that never confirms. Failure injection against Vault and etcd should be part of the standard test run and not a rare exercise.

Idempotence tests. Run each step twice and in the wrong order after a simulated crash and check that no second new value appears and that the old value is not revoked early. The most dangerous bugs here are not crashes but quiet double rotations and early revocations, so those get the most attention.

Rollout. New versions of keyvane-rotator are rolled out replica by replica. Because claims lapse and are picked up by others, a restart is harmless provided the steps are idempotent. During a rollout, the metric of the smallest remaining time to deadline is watched, and the rollout stops if it starts to shrink toward the margin. Changes to the scheduler get extra care because a scheduler bug can delay every rotation at once without any step failing.

Compatibility. Consumers of the rotator, whether they poll or are notified, should be tolerant of seeing a new version at any time inside the window. Changing how consumers are notified is a coordinated change with the application teams, who own the reload behaviour on their side.

Open questions, kept short.

- Whether the safety allowance for clock skew should be a fixed value or derived from observed skew. Leaning toward derived, with a fixed floor.
- Whether a secret with a consumer that cannot reload without a restart should be refused onboarding or just flagged. Leaning toward flagged with an explicit owner, since refusing would block teams that cannot change their consumers soon.
- How to treat secrets whose expiry is set by an external system the rotator cannot change. The rule still applies, but the rotator can only request a new value, so the schedule must account for the external system's own latency, and breaches there need a different escalation path.
- Whether pause should be allowed at all inside the final stretch before the deadline. Leaning toward requiring a second approver there.
- Whether the audit evidence for the rule should be generated as a periodic report for security engineers or only on demand. Periodic seems better, since a silent drift away from the rule is what we least want to find late.

Things that are settled and should not be reopened without a reason: the floor of 24 hours before expiry; roll forward, not back, on partial failure; old values are revoked only after verification; Vault is authoritative for expiry; etcd claims decide who may rotate; every call uses mTLS with SPIFFE identities; secret values never appear in logs, metrics or errors.
