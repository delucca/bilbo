---
id: 01M2SWNR6JH8011YJMSYN3CBES
created: 2026-09-18T06:13-03:00
---

# settlement-ingester: how files get parsed and re-parsed when revised

Notes on how settlement-ingester handles a settlement file that the card processor sends again with changes. This is written from memory of how the pieces fit, not from a fresh read of the code, so check details before relying on them.

The short version: processors do revise files. A file shows up, we parse it, and later a file with the same logical identity shows up with different content. settlement-ingester has to treat the second one as a revision of the first and not as a new batch. If it treats it as new, finance ops sees double counted amounts and the reconciler flags everything as a mismatch.

## What a revision looks like

Usually one of three things happens:

- The processor fixes a few rows (a fee corrected, a refund added late) and resends the whole file.
- The processor resends the same file because their delivery job retried. Content is identical.
- The processor sends a file that covers the same settlement window but with extra rows appended.

The identical resend is the easy one. The other two need a decision about which rows win. The rule we have been using is that the later file replaces the earlier one for that settlement window, row by row, keyed on the processor's own transaction reference plus the line type. Rows that vanish in the later file are not deleted silently; they are marked superseded so the reviewer can see they existed.

## Parsing flow

The ingester reads the file, checks the header and trailer totals against the rows, and only then writes anything. A file that fails its own totals is rejected whole and a record of the failure goes out for review. That check matters more on revisions, because a half-corrected file is a common failure.

After parsing, each row is normalised into the internal shape, and a content hash of the file is stored next to the file identity. On a new arrival the ingester looks up the identity first:

1. No match: treat as a first version.
2. Match with the same hash: drop it, log that it was a duplicate delivery.
3. Match with a different hash: parse it, diff against the stored version, write the changes as a new revision.

Revisions are numbered per file identity, starting from the first. Old revisions stay in PostgreSQL; nothing is overwritten in place. The reconciler reads the latest revision only.

```sql
-- shape of the lookup, roughly
SELECT revision, content_hash
FROM settlement_files
WHERE file_identity = $1
ORDER BY revision DESC
LIMIT 1;
```

The table and column names above are from memory of the design and may have drifted from the real schema.

## Events out to Kafka

When a revision is accepted, settlement-ingester publishes an event saying which file identity changed and which revision is now current, plus a compact list of affected rows. Consumers must not assume one event per file. The reconciler needs to be able to receive a revision event for a window it has already reconciled and redo that window.

Ordering matters here. Events for one file identity should go to the same partition so revisions are seen in order. If two revisions are processed close together by different workers, the later one could be published first, and the reconciler would then end up on the older data. The partition key choice is what protects against that; do not change it casually.

Delivery is at least once, so the reconciler side has to be idempotent on file identity plus revision. The ingester does not try to guarantee exactly once.

## Open questions

- What should happen when a revision arrives after the review for that window is already closed? Right now it reopens the items that changed, but nobody has said whether finance ops wants that or wants a separate adjustment entry instead.
- The diff step loads both versions into memory. Fine for the usual file size, but a very large file from a big marketplace could be a problem. Worth streaming the diff if that shows up.
- Late-arriving rows with a line type the parser does not know. Currently they go to the rejected path with the whole file. That may be too strict for a revision, where most of the file is already known good.
- The gRPC surface that lets the review tool ask "which revision am I looking at" is thin. Reviewers have asked to see what changed between revisions, and the data is there, but the call to expose it is not.

## Things to watch when changing this

Do not make the parser mutate stored rows from an earlier revision. Every downstream audit story depends on old revisions staying as they were. Do not add a shortcut that skips the totals check for resends, even when the hash looks familiar; the hash is of the bytes we received, and a processor bug can produce consistent bytes with inconsistent totals.

Terraform only comes into it for the topic and consumer group setup. Changing partition counts there affects ordering of revision events, so coordinate it with whoever owns the reconciler.
