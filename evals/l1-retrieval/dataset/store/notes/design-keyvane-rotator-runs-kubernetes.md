---
id: 01KQG6GBSZVA2NDF25MVNSYCFN
created: 2026-04-30T19:00-03:00
sources:
  - "code: deploy/rotator-cronjob.yaml"
---

# keyvane-rotator design

keyvane-rotator is the part of Keyvane that rotates secrets before they expire, so application teams do not have to redeploy to pick up new credentials. It runs as a Kubernetes CronJob on the schedule `*/15 * * * *`. On each run it scans etcd for secrets that are inside their renewal window, renews or replaces them, and exits. This note records how it is put together and why. It is written from the design as it stands, so check the code before relying on a detail.

## Purpose and scope

Keyvane issues short-lived credentials to services. Short lifetimes only help if something replaces the credentials reliably before they lapse. keyvane-rotator is that something. It does not issue credentials for a first request from a service, and it does not decide policy. It looks at what already exists, finds the entries that are close to expiry, and drives the renewal.

Things it does:

- Runs on a fixed schedule as a Kubernetes CronJob, with the schedule `*/15 * * * *`.
- Scans etcd for secrets whose renewal window has opened.
- Asks HashiCorp Vault for new material where a secret needs to be replaced.
- Writes the result back so consumers see the new value on their next read.
- Reports what it did through logs and metrics.

Things it does not do:

- It is not a long-running daemon. There is no loop inside the process and no leader election.
- It does not hold secret values in memory longer than one run.
- It does not push to consumers. Consumers pull.

## Why a CronJob and not a daemon

We chose a CronJob because the failure model is simpler. A daemon needs leader election, a watch that survives etcd compaction, and a restart story that does not miss work. A CronJob gets a clean process each time. If a run crashes, the next scheduled run starts from the same state in etcd and finds the same work. Nothing carries over in memory, so nothing can be wrong in memory.

The cost is latency. A secret can sit in its renewal window until the next run picks it up. That is acceptable only because the renewal window is set to be much wider than the gap between runs. If someone narrows the window to something close to the schedule gap, rotation will start missing deadlines. Keep the window comfortably wider than the schedule gap, and change the two together.

A second reason is resource use. Most runs find little or nothing to do and exit quickly. A daemon would sit idle holding a connection and credentials for most of its life.

### The schedule

The schedule is a standard five-field cron expression: `*/15 * * * *`. Every field other than the first is a wildcard, so the job fires on a regular cadence all day, every day. The concurrency policy on the CronJob should forbid overlap. If a run is still going when the next one is due, the new one is skipped rather than started in parallel. Two rotators working the same secret at once is the worst case we are trying to avoid.

```yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: keyvane-rotator
spec:
  schedule: "*/15 * * * *"
  concurrencyPolicy: Forbid
```

## How a run works

A run has a fixed shape. It is a single pass and it ends.

1. Start up and load configuration. Fail fast if Vault or etcd settings are missing.
2. Establish identity. The process gets its workload identity from SPIFFE and uses it for mutual TLS to both Vault and etcd.
3. List candidate secrets in etcd under the prefix Keyvane uses for managed secrets.
4. For each one, read its metadata and compute whether it is inside the renewal window.
5. For those inside the window, perform the rotation (see below).
6. Write results and update status fields.
7. Emit a summary and exit with a status that tells Kubernetes whether the run was healthy.

The scan reads metadata first and fetches the full value only when it must. Most entries are outside the window and never need their value read, which keeps load on etcd low and keeps secret values out of the process when there is no reason for them to be there.

## The renewal window

Every managed secret carries an issue time and an expiry time. The renewal window is the part of the lifetime near the end, in which the rotator should act. A secret is eligible once the current time is past the start of that window and the secret has not already been replaced.

We define the window as a fraction of the lifetime, not a fixed duration, because secrets of different lifetimes should be renewed at a proportionally similar point. A very short-lived credential gets a proportionally short window. This is the main reason the schedule must be fast enough for the shortest lifetime Keyvane issues: if the shortest lifetime is close to the schedule gap, a single missed run can let it expire. Treat the shortest allowed lifetime as a constraint on the schedule.

Edge cases to remember:

- A secret already past expiry is still processed. It is logged at a higher severity because it means a consumer may have been using a dead credential.
- A secret with missing or malformed timestamps is skipped and reported, never guessed at.
- A secret that was replaced by something else between scan and write is detected through a revision check and left alone.

## Scanning etcd

The scan is a ranged read over the managed prefix. It pages through results so that a large keyspace does not produce one huge response. Pages are processed as they arrive, and the rotator does not build the whole list in memory first.

Etcd is the source of truth for what exists and when it expires. Vault is the source of new material. This split matters: etcd tells the rotator what to rotate, Vault tells it what to rotate to.

Writes back to etcd use a transaction conditioned on the revision seen during the scan. If the revision changed, the write fails cleanly and the rotator moves on. The next run sees the new state. This is what makes overlapping or racing writers safe even if the concurrency policy is ever bypassed, for example by someone running the job by hand while a scheduled run is active.

## Talking to Vault

For each eligible secret the rotator asks HashiCorp Vault for fresh material through the engine that backs that secret type. It does not generate secrets itself. Vault owns generation, policy and audit, and the rotator is just a client.

The rotator authenticates to Vault with its SPIFFE identity over mTLS, and holds a Vault token only for the duration of the run. The token is not written to disk and is not logged. When the run ends the token goes with the process.

If Vault is unreachable or returns an error for one secret, the rotator records the failure for that secret and continues with the rest. One bad secret should not block the others. If Vault is unreachable for every request, the run exits unhealthy, so the failure is visible in Kubernetes and in alerting rather than hidden.

Old material is not revoked at once. There is an overlap period in which both the old and new values are valid, so consumers that have not yet re-read the secret keep working. Revocation of the old value is a later step, and it happens only after the new value has been written and confirmed.

## Identity and transport

All connections from keyvane-rotator use mTLS. Identity comes from SPIFFE: the rotator presents a workload identity and verifies the identity of the peer, both for Vault and for etcd. There are no shared static client certificates baked into the image.

Because the process is short-lived, it fetches its identity material at the start of each run and does not need long-lived renewal logic of its own. If identity cannot be obtained, the run stops before touching any secret. Do not add a fallback to a static credential. That would defeat the purpose of the whole system.

Authorization should be narrow. The rotator needs read access to metadata across the managed prefix and write access only for the rotation path. It does not need to read values for secrets outside the window, and its etcd role should not grant that.

## Failure handling and idempotence

The central rule is that a run can be repeated safely. Every step either finishes and is recorded, or leaves things as they were.

- If the process dies after getting new material from Vault but before the write to etcd, the material is simply unused. The next run asks again. Unused material expires under Vault's own rules.
- If the write to etcd succeeds but a later step fails, the next run sees a secret outside its window and does nothing further for it. Cleanup of the old value is retried by whichever run next finds it still pending.
- If a run overruns into the next slot, the concurrency policy skips the new run. A skipped run is not an error, but repeated skips are a signal that the scan has become too slow and need attention.

Exit status reflects health. A run in which some secrets failed but others succeeded should still be distinguishable from a clean run. The status should be non-zero when any eligible secret could not be rotated, so that Kubernetes job history and alerts show it.

## Observability

Each run logs a structured summary: how many secrets were scanned, how many were eligible, how many were rotated, how many failed, and how many were skipped and why. Individual secrets are identified by name or path in logs, never by value.

Metrics worth having, exported or pushed at the end of the run since the process is short-lived:

- Time of the last successful run, so an alert can fire when rotation has stalled.
- Count of secrets rotated and failed per run.
- Smallest remaining lifetime among all scanned secrets. This is the most useful early warning, because it shows how close the system is to an actual expiry.
- Run duration, to catch the scan growing toward the schedule gap.

Because a CronJob leaves no process behind, a push-style metric or a status record written to etcd is more reliable than expecting a scraper to catch the process while it is alive.

## Open questions and things to watch

- Whether the window fraction should be configurable per secret type rather than global. Right now one setting applies everywhere, and some consumers may want a wider margin.
- Whether the scan should use a secondary index or a watch-fed cache if the managed keyspace grows large. For now the ranged read is enough, but run duration is the number to watch.
- Whether revocation of old material should be a separate CronJob so that a slow revocation cannot delay rotation.
- Manual runs. Operators sometimes trigger the job by hand. The revision-conditioned writes make this safe, but the guidance should be written down for the runbook.
- Any change to `*/15 * * * *` has to be checked against the shortest secret lifetime and the window size first. Those three values form one constraint and should not be tuned separately.
