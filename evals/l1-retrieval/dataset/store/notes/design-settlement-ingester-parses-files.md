---
id: 01KR6FS7JF0CRRZ6FM0KN647RT
created: 2026-05-09T10:45-03:00
sources:
  - "code: internal/ingest/parser.go"
---

# settlement-ingester design

settlement-ingester is the front door of Ledgerlark. It takes card-processor settlement files, turns each row into a structured record, and puts those records on Kafka so the matching side can compare them with internal ledger entries. This note records how the component is shaped and why. It is written for someone who has to change it or debug it and has not read the code yet.

The core decision: settlement-ingester parses settlement files as a stream with `encoding/csv` and publishes them in batches of `500 rows`. It never loads a whole file into memory. Everything else in the design follows from that choice.

## Purpose and boundaries

settlement-ingester does four things and stops there. It accepts a settlement file, reads it row by row, normalizes each row into a common record shape, and publishes the records to Kafka. It does not match anything against the ledger. It does not decide what counts as a mismatch. It does not talk to reviewers. Those jobs belong to downstream services, and keeping them out of the ingester is deliberate, because the ingester has to stay simple enough that a bad file can be explained by looking at the file alone.

Inputs come from card processors. Each processor has its own column layout, its own date and amount formatting, and its own habits about trailing summary lines. The ingester hides those differences behind per-processor mapping code, so the rest of Ledgerlark sees one record shape no matter where the row came from.

Outputs are Kafka messages and a small amount of bookkeeping in PostgreSQL. The bookkeeping records which files have been seen, how far each one got, and whether it finished. The records themselves are not stored in PostgreSQL by the ingester; Kafka is the hand-off point.

The component is written in Go. It exposes a gRPC interface for submitting files and asking about their status. It is deployed with Terraform like the other Ledgerlark services, and nothing about the deployment is special beyond what is described in the operations section.

## Streaming parse with encoding/csv

Settlement files can be large. A busy marketplace produces a lot of rows per processor per day, and several processors deliver at around the same time. Reading a whole file into memory would make the memory footprint depend on the worst file we ever receive, and that is a bad property for a service that should be boring. So the parser reads through a reader and asks `encoding/csv` for one record at a time.

The standard library reader is enough for this. We use it with the settings that make it tolerant in the ways processors need: variable field counts are allowed at the reader level so that we can produce our own, clearer errors per row instead of a generic reader failure; leading whitespace is trimmed; and the reader reuses its record slice to avoid allocation. Because the slice is reused, the mapping code must copy any field it keeps. This is a known trap: holding on to a field string from a reused record is fine in Go because strings are immutable copies, but holding on to the slice itself is not. The mapping code only keeps strings, never the slice.

A few consequences of streaming are worth stating plainly.

- Row errors are local. A malformed row is reported with its position in the file and does not stop the rest of the file unless the error is one that makes later rows meaningless, such as a broken quote that swallows following lines.
- We cannot know the total row count before we finish. Progress is reported as rows read so far, not as a percentage. Some processors put a count in a trailer line; when they do, we compare it with what we read at the end and flag a difference, but we never rely on it up front.
- Header handling happens before the first data row. The header decides which mapping applies. If the header does not match any known layout for the processor, the file is rejected before any row is published.
- Trailer and summary lines are recognized by processor-specific rules and are not published as ledger-comparable rows. They are kept as file-level facts for the end-of-file checks.

## Batching: publish in batches of 500 rows

Parsed rows are not sent to Kafka one at a time. The ingester collects them and publishes in batches of `500 rows`. A batch is flushed when it reaches that size, and also once at the end of a file for whatever is left over, so the last batch of a file is usually smaller.

Why batches at all: publishing per row would make the producer overhead dominate and would flood the broker with tiny requests. Why this size and not something far larger: a moderate batch keeps memory bounded and keeps the cost of a retry small. If a publish fails, we resend a few hundred rows, not a huge slab. The number was chosen by feel and a little testing, not by a rigorous benchmark, and it lives in configuration so it can be changed without a code change. If someone changes it, they should watch broker request sizes and end-to-end latency for the largest files before keeping the new value.

Batches never span files. A batch always holds rows from a single file, in file order. That keeps the progress bookkeeping simple: the progress marker for a file is the position of the last row in the last batch that the broker acknowledged.

Within a batch the order of rows is preserved. Across batches of the same file the order is preserved too, because the ingester waits for the acknowledgement of a batch before it treats it as done, even though it may be reading ahead and filling the next one. The read-ahead is bounded to a small buffer so a slow broker pushes back on the parser instead of letting memory grow.

## Kafka publishing and keys

Each row becomes one Kafka message. The batch is a unit of producing and of progress tracking, not a message shape. Downstream consumers see individual records and do not need to know about batches.

Messages are keyed so that rows that belong together land on the same partition. The key is built from the processor and the settlement reference carried on the row, so a later correction row for the same settlement goes to the same partition as the original. This matters for the matcher, which relies on seeing the original and its corrections in order.

Producer settings favor safety over raw speed. Acknowledgement from all in-sync replicas is required, and idempotent production is turned on so that a retry after a timeout does not create duplicates at the broker. Even so, we do not assume exactly-once end to end. A crash between the broker acknowledgement and our progress update can lead to a batch being sent again after restart. Consumers therefore deduplicate on a stable row identity, described in the next section. This is the main reason the identity exists.

The topic naming and partition counts are managed in Terraform with the rest of the Kafka configuration. The ingester does not create topics at runtime. If the topic is missing, publishing fails loudly and the file is marked as failed, which is what we want.

## Row identity and idempotency

Because a batch can be republished after a restart, every published record carries an identity that is derived from the file and the row, not from time or a random value. The identity combines the processor, a file fingerprint, and the row position within the file. Sending the same row twice yields the same identity, so downstream deduplication is a simple lookup.

The file fingerprint is computed while streaming, so it does not require a second pass. It is a hash over the bytes as they are read. If the same file is submitted twice, the fingerprint matches and the ingester recognizes the file as already seen. What it does then depends on the state of the first attempt: a completed file is acknowledged and ignored, an in-progress file is reported as in progress, and a failed file may be resumed or restarted depending on the request.

A file with the same name but different content is a different file. Processors do sometimes reissue a file under the same name after fixing something on their side. The fingerprint makes that visible, and the ingester treats it as new but records the relation so a human can see the two attempts side by side. We do not try to diff them automatically.

## Failure handling and resume

The failure modes we care about are a bad file, a failing broker, and a crashed process. Each has a different answer.

Bad file: row-level problems are collected with their positions and published to a rejects stream with enough context to fix them, rather than silently dropped. A file with too many row problems, or with a structural problem like an unrecognized header, is failed as a whole. The threshold for "too many" is configuration. The key rule is that a row is either published as a normal record or accounted for as a reject. Nothing disappears.

Broker trouble: publishing is retried with backoff. While retries are happening the parser is paused through the bounded buffer. If retries run out, the file is marked failed with the reason and the last acknowledged position. No partial batch is considered done.

Crash: on restart the ingester looks at the PostgreSQL bookkeeping for files that were in progress. It resumes from the last acknowledged position by re-reading the file and skipping rows up to that point. Skipping is cheap because it only parses and discards. The batch that was in flight at the time of the crash may be sent again, which the row identity makes harmless.

Resume depends on the source file still being available. Files are kept in storage until the ingest is complete and for a retention period after, so this is normally true. If the file is gone, the ingest is marked failed and someone has to resubmit.

## gRPC interface and bookkeeping

The gRPC surface is deliberately small. A caller can submit a file by reference to where it is stored, ask for the status of a file, and list recent files with their outcome. There is no call that streams the rows back; if you want the rows, read them from Kafka or from the source file.

Status has a handful of states: received, parsing, publishing, completed, failed. Parsing and publishing overlap in practice because of streaming, so the status shown to callers is a single state chosen by what the file is mostly doing. The detailed progress numbers, rows read and rows acknowledged, are reported alongside it.

The PostgreSQL side holds one record per file attempt with the processor, the fingerprint, the status, the progress counters, timestamps, and the failure reason if any. It also holds the per-row rejects summary, not the rejected rows themselves. Writes to this table are batched with the Kafka batches, so the progress update happens once per acknowledged batch, not once per row. That keeps database load low and is the reason progress is only as fine as a batch.

Schema changes to this table go through the normal migration process used by Ledgerlark. The ingester should be restartable on the previous schema during a rollout, so migrations are additive first and cleanup comes later.

## Operations and things to watch

Terraform defines the service, its permissions to read the file storage and to produce to the topics, and its database access. Configuration values that people tend to tune, such as the batch size, retry limits, reject thresholds and read-ahead buffer, are surfaced as variables rather than hard-coded.

Useful signals when something looks wrong:

- Rows read versus rows acknowledged for a file. A growing gap means the broker is the bottleneck, not the parser.
- Time a file spends in each state. A file stuck in parsing with no progress usually means the source reader is stalled, often a storage read problem.
- Reject rate per processor. A sudden jump for one processor usually means they changed their layout, not that our parser broke.
- Count of files resumed after restart. Frequent resumes point to instability in the process rather than in the data.

Known rough edges. Progress granularity is a batch, so a status check may lag by up to a batch worth of rows. The trailer count check only exists for processors that provide a trailer. Per-processor mapping code is the most change-prone part of the component, because processors alter formats without much notice; new layouts should be added as new mappings, with the old one kept until we are sure no more files arrive in it.

If you change the parser, keep the streaming property. Anything that needs the whole file in memory, such as sorting or whole-file validation, belongs somewhere else or has to be done in a way that still reads once and holds only small state.

## Open questions

We have not decided whether rejects should be retriable from the gRPC interface or only through a separate tool. We have also not settled whether the batch size should adapt to row width, since some processors have far wider rows than others and a fixed count means unequal message volume. Both are noted here so the next person does not assume they were already decided.
