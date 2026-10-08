---
id: 01KS4TFHNHWWM2KPSE89BASDYD
created: 2026-05-21T05:29-03:00
---

# upgrade-scanner full scan time with ETag caching

This report replaces the earlier note about "upgrade scanner full scan", which recorded 3m 40s. The new value is 1m 55s for a full scan of 412 repositories by the upgrade-scanner, measured after ETag caching was added.

## Result

A full scan of 412 repositories by the upgrade-scanner now takes 1m 55s. The earlier figure of 3m 40s no longer holds and should not be quoted. The difference comes from ETag caching: the scanner no longer re-downloads data that has not changed since the last scan.

## What changed

The upgrade-scanner now stores the ETag it gets back for each response and sends it on the next request. When the remote side says nothing changed, the scanner reuses what it already has instead of fetching and parsing the body again. Most repositories change little between scans, so most requests end up as cheap not-modified answers.

## How to read the number

The 1m 55s figure is for a full scan, meaning all 412 repositories in one run. It replaces the older number for the same kind of run, so the two are comparable. The figure is one measurement, not an average over many runs. Treat it as a ballpark and expect it to move with network conditions and with how many repositories changed since the previous scan.

## Caveats

- A cold cache will be slower, because there are no stored ETag values to send. The first scan after clearing the cache should not be compared against this number.
- If many repositories changed since the last scan, fewer requests will come back as not-modified and the time will rise toward the old figure.
- The measurement says nothing about the targeted tests PatchPilot runs after a scan; it covers only the scanning step.

## Quick reference

```text
component: upgrade-scanner
repositories: 412
full scan before ETag caching: 3m 40s (superseded)
full scan with ETag caching: 1m 55s
```

## Follow-ups

- Re-measure on a warm cache and on a cold cache and record both separately, so this note's number has a clear condition.
- Check whether the stored ETag values need an expiry or a cleanup rule, so stale entries do not pile up in the local storage.
- If someone changes the scan path again, update this note rather than writing a new one on the same subject.
