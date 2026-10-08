---
id: 01KT1ZPT2290B884HBKVYMERWB
created: 2026-06-01T13:19-03:00
---

# keyvane-rotator schedule and run model

This note replaces the earlier note "keyvane rotator runs kubernetes". The new value: keyvane-rotator runs as a Kubernetes CronJob on the schedule `*/5 * * * *`, which replaces the earlier schedule. Anything that still describes the old interval is out of date.

## What changed

keyvane-rotator used to run on a slower schedule. It now fires every five minutes, using the cron expression `*/5 * * * *`. The change is only about how often a rotation pass starts. It does not change what a pass does, which secrets it covers, or how it authenticates.

The rotator is still a Go program. Each run starts, does its pass, and exits. Nothing stays resident between runs, so the CronJob is the only thing that decides when rotation happens.

## Why it matters for the rest of the system

A five minute cadence means a secret that is due for rotation waits at most one interval before the rotator picks it up. Before this, the wait could be much longer. Application teams who rely on short-lived credentials from Keyvane get a tighter bound on how stale a credential can get.

Each pass talks to HashiCorp Vault to write new secret versions, and reads and writes its coordination state in etcd. More frequent runs mean more of that traffic. It is small per run, but if Vault or etcd latency goes up, look at the schedule first as a possible contributor.

The rotator identifies itself with its SPIFFE identity and connects over mTLS. That did not change. Short runs mean a new connection and a fresh handshake each time, which is expected.

## Things to watch

- Overlap: if one pass takes longer than the interval, the next one can start while the previous is still running. Check the CronJob concurrency policy before assuming passes are serialized. Do not rely on schedule spacing alone for mutual exclusion; the etcd state is what should guard against two passes touching the same secret.
- Missed runs: if the cluster is down or the CronJob controller is behind, runs are skipped, not queued without limit. A gap in rotation after an outage is normal and catches up on the next pass.
- Job history: with a run every five minutes, finished Job objects pile up quickly. Keep the history limits on the CronJob small so the cluster is not filled with completed pods.
- Logs: each run is a separate pod, so a failure is found by looking at the most recent failed Job, not at one long-lived process.

## Open points

- Whether five minutes is the right long-term interval is not settled. It was picked to shorten the rotation delay, not from load measurements.
- If Vault rate limits or etcd load become a problem, the options are to lengthen the interval again or to make a pass skip secrets that are not yet due. Neither is planned now.
- Any docs, runbooks or alerts that mention the old schedule should be updated to `*/5 * * * *`. Alerts that fire on "no successful run for N minutes" need a threshold that fits the new interval.
