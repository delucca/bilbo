---
id: 01KF1DRF1SGMQ9NS07X105694F
created: 2026-01-15T15:12-03:00
---

# blob-archive-client upload block size

This note replaces the earlier note about "blob archive client uploads". The new value: blob-archive-client now uploads in blocks of 4 MiB, down from the earlier block size of 8 MiB. The reason is memory use on small workers.

## What changed

blob-archive-client used to stage each block of an upload at 8 MiB before sending it to Azure Blob Storage. It now stages 4 MiB at a time. The old figure no longer holds anywhere in the client, so anything that quotes the earlier size is out of date, including the earlier note this one replaces.

## Why

Small workers were holding too much in memory while an upload was in flight. Each block is buffered whole before it is sent, so the block size sets the floor for per-upload memory. Halving it cuts that floor. Several uploads running at once on a small worker add up, and that was the pressure we wanted to relieve.

## Trade-offs

Smaller blocks mean more blocks for the same file, so more requests to the blob service and a longer block list to commit at the end. For the instrument output files and notebook attachments this project handles, that cost is acceptable next to the memory saving. Large files take a little longer to upload because of the extra round trips. Throughput on big workers may drop slightly; we have not treated that as a problem.

There is also a limit on how many blocks one blob can have in Azure Blob Storage. A smaller block size lowers the largest single file we can upload in one go. Check this limit against the biggest expected instrument file if someone starts handling much larger outputs than today.

## Effects on the rest of the system

The audit trail records an upload as one archive event, not per block, so compliance records are not changed by this. Retries work per block, so a failed block now resends less data than before. Anything that estimated upload time or memory from the old block size should be recalculated. RabbitMQ messages that announce a finished upload are unaffected, since they are sent once the whole blob is committed.

## Open points

- Whether the block size should be configurable per deployment rather than fixed. Not decided.
- Whether very large files should get a different size. Not decided.
- Nobody has measured the upload time change on real instrument files yet; do that before tuning further.
