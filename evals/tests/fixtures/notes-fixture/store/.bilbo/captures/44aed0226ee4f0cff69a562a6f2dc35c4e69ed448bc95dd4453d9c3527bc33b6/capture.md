# Demo WAL

Intro nav line.

## Write-ahead log

The log records every change before the pages change.

### Checkpoints

A checkpoint copies pages back into the database file. It runs when the log passes 1000 pages.
