---
id: 01K755HRAP1TSQSY8KAWH41SH1
created: 2025-10-09T16:00-03:00
---

# upgrade-scanner full scan timing report

A full scan of 412 repositories by the upgrade-scanner took 3m 40s with a concurrency of 8. This note records that one measurement, what it does and does not tell us, and what to check before relying on it. It is a single data point, not a benchmark suite. Treat it as a rough baseline for how long a full pass over the fleet takes today.

## The measurement

The upgrade-scanner walked 412 repositories in one run. Wall-clock time for the whole run was 3m 40s. Concurrency was set to 8, meaning up to eight repositories were being scanned at the same moment. The number is end to end: it covers the whole scan, not just the slowest repository and not an average per repository.

```text
upgrade-scanner full scan
  repositories: 412 repositories
  concurrency:  8
  wall clock:   3m 40s
```

The block is a plain restatement of the figures above. It is not captured tool output and not a command to run.

## What was being scanned

The scan covered the repositories PatchPilot manages for platform engineers. The upgrade-scanner looks at each repository for dependencies that have newer versions available, so that upgrade pull requests can be proposed later. The scan itself does not open pull requests and does not run the targeted tests. Those steps come after it, so this timing says nothing about how long a complete upgrade cycle takes.

## What the number includes

The 3m 40s is the time for the upgrade-scanner to finish its pass over all 412 repositories at concurrency 8. It includes whatever fetching and parsing the scanner does per repository, and any waiting on the remote side. It does not include the downstream work: no branch creation, no pull request creation, no test runs in GitHub Actions. If someone asks how long scanning the fleet takes, this is the figure. If they ask how long upgrading the fleet takes, it is not.

## Concurrency setting

Concurrency was 8 for this run. That is the only setting measured. We did not try lower or higher values in this report, so we do not know how the time scales. Do not assume that doubling concurrency halves the time. The scanner may be limited by the remote API, by local disk or SQLite writes, or by the slowest repositories rather than by the number of workers.

## Per-repository implication

A rough sanity check: the total time divided across the workers gives a per-repository cost that is small compared with a minute. This is only arithmetic on the reported figures and hides a lot of variance. Some repositories are larger, have more dependency manifests, or sit behind slower responses. The long tail of a few slow repositories can dominate the end of a run, so the real distribution is probably uneven.

## Where the time likely goes

We did not profile this run, so the following is a guess and not a finding. A scan like this tends to spend most of its time waiting on network calls to read repository contents and to look up the latest versions of dependencies. Local work, such as parsing manifests and writing results to SQLite, is probably a smaller share. If the guess is right, raising concurrency helps until a rate limit or the database becomes the bottleneck. Confirm with a profile before tuning.

## Caveats about the run

This was one run. We do not know the state of caches, the load on the remote service, or whether other jobs were competing for the same machine. A cold run and a warm run could differ noticeably. The set of repositories also changes over time, so a later scan of the fleet may cover a different count and take a different amount of time. Quote the figure together with the repository count and the concurrency, never alone.

## How to compare future runs

When a new full scan is timed, record the same three things: how many repositories were scanned, the concurrency, and the wall-clock time. Keep the definition of wall clock the same, from the start of the scan to the moment the last repository finishes. If the repository count differs from 412 repositories, compare time per repository roughly, not the raw total. If the concurrency differs from 8, say so up front, because the totals are not comparable otherwise.

## Risks if the fleet grows

If the fleet grows well beyond 412 repositories, the total time will grow with it, probably close to linearly at a fixed concurrency. That is fine for a scheduled job but could matter if scans are triggered on demand and a person waits for the result. Before raising concurrency to compensate, check rate limits on the services the scanner talks to, and check that SQLite writes do not serialize the workers. Running in Docker or in GitHub Actions may also cap CPU and network, which changes the picture.

## Open questions

- How does total time change at lower and higher concurrency than 8?
- Which repositories are the slowest, and why?
- How much of the 3m 40s is network wait versus local processing?
- Does a repeat scan of unchanged repositories finish faster, and by how much?
- Is the figure the same when the scan runs in GitHub Actions as when it runs locally or in Docker?

None of these are answered by this report.

## Suggested next steps

Repeat the full scan a few times at the same settings to see how stable 3m 40s is. Then vary concurrency one step at a time and record the results in the same three-field form. Add simple per-repository timing to the upgrade-scanner output so the slow tail is visible. Only after that decide whether concurrency of 8 should change. Update this note, or add a new one on the same subject, with the new figures.

## Summary of what to remember

The upgrade-scanner scanned 412 repositories in 3m 40s at concurrency 8. It is one run, it covers scanning only, and it has not been profiled. Use it as a baseline and re-measure before making decisions that depend on it.
