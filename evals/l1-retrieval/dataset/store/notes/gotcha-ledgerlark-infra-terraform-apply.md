---
id: 01K3T7MS98NSQ6VPZFFXD0R3G9
created: 2025-08-29T03:18-03:00
---

# ledgerlark-infra: state lock error when two CI pipelines overlap

A terraform apply in ledgerlark-infra fails with `Error acquiring the state lock` when two CI pipelines run for the same environment. The second apply cannot take the lock the first one holds, so it stops before changing anything. Nothing is wrong with the Terraform code itself; the problem is two runs touching one state at once.

## Symptom

The apply step in CI goes red early, right after init and before any plan output is applied. The log shows `Error acquiring the state lock`, followed by lock details: who holds it, the operation, and when it was taken. The pipeline that started first usually finishes fine. The one that started second is the one that fails.

This tends to show up when two merges land close together, when someone re-runs a pipeline while the earlier run is still going, or when a branch pipeline and a main pipeline both target the same environment.

## Why it happens

ledgerlark-infra keeps remote state per environment, and Terraform locks that state for the whole duration of an apply. Two pipelines for the same environment therefore compete for one lock. CI does not serialize them on its own, so whichever arrives second loses. Different environments have separate state and do not block each other.

## What to do

First, check whether the lock holder is still a live pipeline. If it is, wait for it to finish and re-run the failed one. That fixes most cases and is the safe default.

Only if the holder is dead (a cancelled or crashed runner that never released the lock) is a manual force-unlock reasonable. Confirm with whoever owns the environment first, and make sure no apply is really running. Forcing the unlock while a live apply is going can corrupt state, which is much worse than a failed pipeline.

Do not just retry in a loop. Repeated retries during a long apply only add noise and can leave stale lock entries if a runner gets killed.

## Prevention

Make CI run one apply per environment at a time. The usual way is a concurrency group or a resource lock keyed on the environment name, so a second pipeline queues instead of failing. Keep plan runs separate from apply runs so that reviewers' plans do not hold the lock. It also helps to avoid triggering apply from both branch and main pipelines for the same environment.

## Open items

- Not yet confirmed whether the current CI config already has any per-environment serialization; check before adding another.
- Decide who is allowed to force-unlock the shared environments, and write that down next to the runbook for ledgerlark-infra.
