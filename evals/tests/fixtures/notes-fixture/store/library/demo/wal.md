---
id: 01KEZSGYW058RBWSA9D8DB9TRK
fetched: 2026-01-15
origin: "url: https://fixture.invalid/demo/wal"
digest: sha256:44aed0226ee4f0cff69a562a6f2dc35c4e69ed448bc95dd4453d9c3527bc33b6
kept: 3-11
capture: external
---
# Demo WAL

Intro nav line.

## Write-ahead log

The log records every change before the pages change.

### Checkpoints

A checkpoint copies pages back into the database file. It runs when the log passes 1000 pages.
