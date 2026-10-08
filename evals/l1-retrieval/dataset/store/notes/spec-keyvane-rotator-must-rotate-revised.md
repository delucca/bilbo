---
id: 01M1YYZYF8QGXYP1N2E3B04QY6
created: 2026-09-07T19:14-03:00
---

# keyvane-rotator rotation lead time spec

This note replaces the earlier note "keyvane rotator must rotate" and gives the new value: keyvane-rotator must rotate every secret at least 72 hours before its expiry. Whatever lead time the old note said no longer applies. Anything that still points at the old note should be read against this one.

keyvane-rotator is the Keyvane component that rotates secrets without redeploys. Its internal codename is rotor9. You will see rotor9 in log lines, dashboards, old branch names and chat. It is the same thing as keyvane-rotator. In docs and in code comments that other teams read, use keyvane-rotator. Use rotor9 only when you are matching something that already says rotor9.

## The rule

Every secret that keyvane-rotator manages must have a new version in place at least 72 hours before the current version expires. "In place" means the new version has been written to the backing store, has been checked, and is what a fresh consumer would receive. It does not mean "the rotation job has started". A rotation that starts inside the window but finishes after the deadline has missed the rule.

The deadline is computed from the expiry of the current version, not from when the secret was created and not from a fixed calendar schedule. If a secret was issued with a short life, the deadline for it may already be past at the moment it is issued. That case is covered below under short-lived secrets. It is a real case and it should not be handled by quietly ignoring the rule.

The value is a minimum. Rotating earlier than 72 hours before expiry is allowed and is often what happens, because the scheduler wakes up on a cadence and picks up work that is due soon, not only work that is due right now. What is not allowed is rotating later. Any design choice in this component that trades lead time for something else, such as batching or backoff, has to keep the 72 hours as a hard floor.

### What counts as expiry

Expiry is the time after which the credential stops working for the party that checks it. For a secret that Vault issues with a lease, that is the end of the lease or of its maximum time to live, whichever comes first. For a secret that is only a stored value with a recorded end date, it is that recorded date. For a certificate used in mTLS, it is the not-after time of the certificate. If a secret has more than one of these, the earliest one is the expiry for the purpose of this rule.

A lease that can be renewed does not move the expiry for this rule when the renewal is capped by a maximum. The rotator reads the effective end, meaning the cap, and plans against that. Do not plan against a renewable lease as if it were unbounded.

## Why the lead time exists

The lead time is there so that a failed rotation is found and fixed while the old secret still works. A rotation can fail for dull reasons: the target system is down for maintenance, a Vault policy was changed, the etcd cluster lost quorum for a while, a workload identity was revoked by mistake. If the margin is small, a failure of this kind becomes an outage the moment the old secret expires. With a long margin, there is time for retries, for a person to be paged, for the fault to be fixed, and for consumers to pick up the new version without anyone rushing.

The margin also covers consumers that are slow to pick up new secrets. Some application teams reload on a signal, some poll, and some only pick up changes on a restart that is itself scheduled. The rotator cannot force every consumer to reload at once, and it should not try. A lead time in days gives the slow consumers room to move over before the old version is gone.

The earlier lead time was judged too short on both counts. This spec raises the bar to 72 hours so that a rotation that goes wrong over a weekend or a holiday still has room to be handled. The people who asked for the change were security engineers and application teams, and the request came from incidents where the old margin left no room.

## How keyvane-rotator gets to the deadline

The rotator does not wait until the deadline minus 72 hours and then begin. It works backwards from the deadline and starts early enough that retries fit. The idea is that the start time of a rotation is a function of the deadline, the expected time a rotation takes, and an allowance for retries. The 72 hours is the end point that must be met, not the start point.

### Inputs

For each managed secret the rotator knows the following, and nothing about this spec requires more than this.

- The expiry of the current version, as defined above.
- The system that holds the secret and the way to write a new version to it. For most secrets this is HashiCorp Vault.
- The list of consumers that need to receive the new version, identified by their SPIFFE identities.
- A record of past rotations of this secret: how long they took and whether they needed retries.

State about which secrets are due, which are in progress and which are done lives in etcd. The rotator is written in Go. Talking to Vault, etcd and consumers is done over mTLS, and the rotator itself authenticates with its own SPIFFE identity. Nothing in this spec changes that. The lead time rule only changes when work is scheduled and when a miss is reported.

### Scheduling

The scheduler scans the due list on a regular cadence. A secret is due when the current time is past the point where the rule requires work to begin, which is the deadline minus the expected duration minus the retry allowance. The scheduler picks up due secrets in order of deadline, earliest first, so that a long queue does not push the most urgent one back.

When there are too many due secrets for the rotator to handle at once, it keeps to deadline order and does not skip ahead to cheaper work. A concurrency limit protects Vault and the targets, and that limit may slow things down, but it is not allowed to be the reason a secret goes past the deadline. If the limit would cause a miss, the rotator raises an alert about capacity instead of silently falling behind. The fix is then to adjust capacity, not to relax the rule.

### Claiming work

Only one rotator instance should work on a given secret at a time. Claims are kept in etcd with a lease so that a crashed instance releases its claim after a while and another instance can pick the work up. The claim lease must be short enough that a crash does not eat a large share of the margin. A crash that leaves a claim stuck for a long time is a way to miss the deadline without any rotation having failed, so claim duration is part of the budget, not a detail.

## Rotation steps

A rotation of one secret goes through the same steps each time. They are listed in order. The point of the order is that the old version stays valid until the new one is confirmed, so a failure at any step leaves the system working.

- Mint the new version. For Vault-backed secrets, ask Vault to create it or write it, according to the secret type. For a certificate, request a new one for the same SPIFFE identity.
- Validate the new version. Check that it is well formed and that the target system accepts it. Where the target can be probed without side effects, probe it.
- Publish the new version so that consumers can fetch it. Consumers that read from Vault get it from there. Consumers that are pushed to get a push over mTLS, to the identity they are registered under.
- Wait for consumers to acknowledge or, where they do not acknowledge, for the observed fetch of the new version. Record who has moved over and who has not.
- Mark the rotation complete in etcd, with the time it finished. Only then does the secret leave the due list.
- Retire the old version after the grace period, not before. Retiring early would defeat the purpose of the margin.

A rotation is complete for the purpose of the rule when the new version is published and validated. Consumer pickup is tracked and reported, but a slow consumer does not make the rotation late. It does make an alert, because a consumer that has not moved by a point well before expiry is a problem for its owner.

### Idempotence

Every step has to be safe to run again. A rotator that dies in the middle will be replaced by another one, and that one will not know exactly where the first one stopped. So steps check what already exists before they create anything. If a new version was minted but not published, the next attempt finds it and carries on, instead of minting another and leaving an orphan. This matters most for secrets that cost something to mint or that are limited in number at the target.

### Order across secrets that depend on each other

Some secrets depend on others, for example a client credential that is only useful together with a certificate. The rotator treats each secret against its own deadline. Where two must change together, the pair is held to the earlier of the two deadlines, and both must be in place by 72 hours before that earlier expiry. A pair is never allowed to be half rotated at the deadline.

## Failure and short-lived secrets

Failures are expected. The rotator retries with growing waits between attempts, but the waits are bounded by the time left. As the deadline gets closer, the waits get shorter, and the failures get louder. A rotation that has failed and has little margin left pages a person. A rotation that has failed with plenty of margin is logged and retried, and shows up on a dashboard, but does not wake anyone.

### Missed deadline

If the deadline passes and the new version is not in place, that is a miss. A miss is reported as an incident-level event, not as a normal error, even when the old secret is still valid. The rotator keeps trying after a miss. It does not give up, and it does not extend the deadline. The record of the miss stays, with the cause as far as it is known, so that the lead time and capacity can be reviewed later.

Do not suppress a miss alert because the old secret is still working. The whole point of the lead time is that the miss is raised while the old secret still works.

### Short-lived secrets

Some secrets are issued with a life that is shorter than the lead time plus the time a rotation takes. For these, the 72 hours rule cannot be met by waiting. There are two honest options and one dishonest one. The honest options are to issue such secrets with a life long enough to leave the margin, or to treat them as a different class that is not rotated by this component but issued fresh on demand. The dishonest option is to accept them and report them as compliant. The rotator should refuse to manage a secret whose life cannot satisfy the rule, and say so clearly, so that the owner decides which honest option to take.

Short-lived credentials that Keyvane issues directly to services on request are not part of this rule. They are issued and used within their own life and are not rotated ahead of expiry. The rule applies to secrets that are held and kept current, which is what keyvane-rotator does.

### Clock problems

The deadline is a comparison of times, so a wrong clock moves it. The rotator uses a single source of time for decisions and records which one. If the clock of an instance is found to be far from the clocks of the stores it talks to, the instance stops taking new claims and raises an alert. A rotator that thinks it is early when it is late will miss deadlines without noticing.

## Configuration sketch

The lead time is configuration, not a constant buried in code, so that tests can use a different value and so that it can be raised later. The default and the production value are 72 hours. The setting is a floor and must not be set lower in production. A sketch of how it reads, using the names from this note:

```yaml
component: keyvane-rotator
codename: rotor9
lead_time: 72 hours
```

The real file has more in it, and the key names in the real file may differ from this sketch. What matters is that there is one place for the value and that every part of the rotator reads it from there. If you find the value written in a second place, such as in an alert rule or in a test fixture that is meant to mirror production, make it read from the same source or note why it cannot.

### Changing the value

Raising the lead time later is cheap in principle, but it brings forward work that is already scheduled, so a large increase can create a burst of due secrets all at once. Roll it out in a way that lets the scheduler absorb the burst, and watch the capacity alerts while it does. Lowering it is the thing to be careful about, and it needs a decision from the security engineers, since it removes margin. This spec does not allow it without a new note that replaces this one.

## Checking that it holds

The rule is only worth having if it is checked. There are three places to check it: ahead of time, as it runs, and after the fact.

### Ahead of time

A planning check looks at every managed secret and its expiry and asks whether a rotation can be done in time given what is known about duration and capacity. It reports secrets that cannot meet the deadline, and it reports the short-lived class described above. This check can run in a pipeline and should block a change that adds such a secret without a decision about it.

### As it runs

The rotator exposes, for each secret, how much time is left between now and the deadline, and the state of its rotation. Alerts are driven from the time left, so they scale with the rule: more urgent as the margin shrinks. The alerts must use the same lead time value as the rotator, from the same source, so that changing the value changes both together.

### After the fact

For each completed rotation the record in etcd keeps the time it finished and the time of expiry of the version it replaced. From those, the margin that was achieved is easy to compute. Review the distribution of achieved margins from time to time. If rotations regularly finish just inside the rule, there is no real slack, and capacity or the start time needs attention even though no deadline was missed. The target is to finish well ahead, not to land on the line.

### Tests

Tests should cover the following without needing real expiry times that last for days. Use a fake clock and set the lead time to a small value in test configuration.

- A secret is rotated before the deadline when everything works.
- A secret whose first attempts fail is still rotated before the deadline when a later attempt works.
- A secret that cannot be rotated in time raises a miss, keeps trying, and does not move its deadline.
- A crashed instance leaves a claim that another instance takes over in time.
- A secret with too short a life is refused with a clear reason.
- Pairs that must change together are held to the earlier deadline.

At least one test should run with the production value, 72 hours, on a fake clock, so that nothing hides a mismatch between the test value and the real one.

## Open points and who to ask

Some things are not settled by this spec and should be decided by the people who own them. They are listed here so that nobody assumes they were decided.

- The exact retry allowance and the expected duration used to work out the start time. These come from the recorded history of rotations and may differ by secret type. They need to be reviewed once the new lead time has been running for a while.
- The grace period before the old version is retired. It should be long enough for slow consumers and short enough that an old, leaked secret does not stay valid for long. Security engineers and application teams may weigh this differently, and the answer may be per secret class.
- How to handle consumers that never acknowledge and cannot be observed fetching. For now they are reported as unknown, which is not good enough for a long-term answer.
- Whether the short-lived class should get its own component or be handled by a mode of keyvane-rotator. The current view is that it should not be forced into the rule.
- Whether the planning check should be a hard gate everywhere or only for production secrets.

### Naming reminder

When writing about this later, call the component keyvane-rotator. If you search old chat, tickets and logs for history, also search for rotor9, since much of the older material uses the codename. If you find a page that states a lead time other than 72 hours for this component, it is out of date; correct it or link it to this note.
