---
id: 01KM9Y6FDH5975CX6B64RWSCQK
created: 2026-03-22T01:52-03:00
---

# settlement-ingester crashes on settlement lines over 64 KB

settlement-ingester dies with `bufio.Scanner: token too long` when a settlement file contains a single line longer than 64 KB. The file is not skipped or flagged. The process fails, and nothing from that file gets reconciled until someone intervenes. Written down because it looks like a bad-file problem or a flaky deploy, and it is neither.

## Symptom

settlement-ingester stops partway through a file and exits with an error. Orchestration restarts it, it picks up the same file again, and it crashes at the same place. From outside this looks like a crash loop on one input. Other files queued behind the bad one wait.

## Trigger

One line in a settlement file longer than 64 KB is enough. The total file size does not matter. A file with a huge number of short lines is fine. A file with one very long line is not. Normal processor exports do not do this, so the trigger is usually an odd file: a processor that emits a whole batch as one line, a file with line endings stripped somewhere in transit, or a free-text field with a large embedded blob.

## Why it happens

The ingester reads settlement files line by line with Go's `bufio.Scanner`. The scanner has a default maximum token size. A line is a token, and when a line is bigger than the limit the scanner stops and reports the error `bufio.Scanner: token too long`. The limit is 64 KB by default. The code does not raise it, and it does not treat the error as a per-file failure, so the error bubbles up and the run ends.

## Where it bites

It hits on the read path, before any parsing or matching. That means no partial results are produced for the line, and rows read earlier in the same file may or may not have been handed on, depending on how far batching had got. Treat the file as unprocessed until proven otherwise.

## What the log looks like

The failing line is short and easy to miss among normal output. Search for the error text.

```text
bufio.Scanner: token too long
```

If you see this, the file named just before it in the logs is the culprit. The crash message itself does not name the file or the line, so the context lines around it matter.

## Impact on reconciliation

Entries from the affected file are never compared with internal ledger entries. Finance operations will see no mismatches for that settlement period, which looks like a clean result but is really missing data. This is the dangerous part: absence of flags is not evidence of a match. When the crash loop is going, check for files that were received but never finished.

## Impact on Kafka

Whatever settlement-ingester publishes to Apache Kafka for that file stops at the point of the crash. Downstream consumers see a truncated stream or nothing. If offsets or progress markers are committed before the file is finished, a restart may skip or repeat records. Check how far the file got before assuming a restart is safe.

## Impact on gRPC callers

Anything calling settlement-ingester over gRPC for that file gets an error or a dropped connection, not a structured rejection. Callers should not retry blindly. A retry reads the same long line and fails the same way.

## How to confirm

Find the file the ingester was working on at the time of the crash, then look for a very long line in it. Checking the longest line length with a standard shell tool is enough. If the longest line is above 64 KB, that is the cause. If it is not, this note does not apply and the crash is something else.

## Workarounds

Short term, split the long line in the source file so no line is above 64 KB, then let the ingester pick the file up again. Only do this on a copy, and only if the record boundaries are clear. Splitting inside a record corrupts the data and would produce false mismatches. If the processor can re-export in a normal line-per-record format, that is safer than editing.

## Move the bad file aside

If the file cannot be repaired fast, take it out of the ingest location so the crash loop stops and the files behind it can run. Keep the original untouched, and write down which settlement period is missing so it gets reconciled later. Do not delete it.

## Fix direction

Two reasonable options. One: raise the scanner's token limit to something that clearly covers real files. Two: stop using a line scanner for this and read with a reader that has no line cap. The second is more robust because there is then no limit to guess at. Whichever is chosen, a line that is still too large must become a recorded per-file failure with the file name, not a process crash.

## What not to do

Do not just restart the service and hope. Do not widen the limit to an enormous value without thinking about memory, since one hostile or broken file could then use a lot of it. Do not swallow the error and continue, because that silently drops the rest of the file and produces the same false clean result described above.

## Testing the fix

Add a test with a file that has one line just over 64 KB and normal lines around it. Expected behavior after the fix: either the whole file is processed correctly, or the file is rejected with a clear error naming it while the service keeps running. Also keep a test with a line just under the old limit, so the fix does not regress normal files.

## Operational notes

Alerting should key on repeated restarts of settlement-ingester and on received files that never complete, not only on the error text. The service is deployed through Terraform, but this is a code issue; no infrastructure setting changes the scanner limit. Raising memory for the service does nothing here.

## Open questions

Which processors have ever sent lines of this size is not known. Whether the earlier rows of a failed file were already published to Kafka should be confirmed against real behavior before relying on restarts. It is also unclear whether the database in PostgreSQL holds any half-written state for a failed file; check before re-running.

## Related

This is a read-path limit, separate from any parsing or matching bug. If a file ingests but produces odd mismatches, look at the parsing and matching side instead. If the process dies on read with `bufio.Scanner: token too long`, come back to this note.
