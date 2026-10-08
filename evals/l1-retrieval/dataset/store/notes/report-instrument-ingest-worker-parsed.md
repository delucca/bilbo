---
id: 01M093WJZ44HKM7H866B514KWD
created: 2026-08-17T21:21-03:00
---

# instrument-ingest-worker CSV parse benchmark

instrument-ingest-worker parsed a 5000-row CSV file in a median of 62 ms across 200 benchmark runs. This note keeps that result, what it does and does not tell us, and what to check before anyone quotes it in a design discussion or a compliance conversation. It is a quick working note, not a formal performance report. The measurement is about the parsing step only, and the rest of this note is mostly about how far that statement reaches.

## Result

The headline: the median wall-clock time to parse one 5000-row CSV file in instrument-ingest-worker was 62 ms, taken over 200 benchmark runs. The median is the figure to quote. A single run is not the figure, and neither is an average pulled around by a few slow runs. Anyone reading this note alone should be able to answer these questions from it:

- Which component was measured? instrument-ingest-worker.
- What was the input? A CSV file with 5000 rows.
- What was the figure? A median of 62 ms.
- How many samples? 200 benchmark runs.

What the note does not say is just as important. It gives no tail latency, no memory figure and no throughput under concurrent load. Those were not part of this fact, so do not infer them. If someone asks for the slowest run or a high percentile, the answer is that this note does not have it and the benchmark would need to be rerun or its raw output looked at.

## What was measured

The unit of work was one CSV file of instrument output going through the parsing path of instrument-ingest-worker. In the real service that file arrives as a message on RabbitMQ pointing at a blob in Azure Blob Storage, and the worker pulls it, parses it, and hands rows on for persistence in SQL Server and for the audit trail. The benchmark number covers the parse of the rows into in-memory records. Treat it as a microbenchmark of the parser, not an end-to-end timing of a message from queue to database.

The file shape matters. A file of this row count is a typical mid-size export from a bench instrument such as a plate reader or a chromatography run. Real files vary a lot: some have wide rows with many channels, some have header blocks with instrument metadata before the data starts, and some have odd quoting. The benchmark used a single representative shape, so the result should be read as a reference point for that shape and not as a promise for every file.

## What was not measured

Several costs sit outside this number, and each one can dominate in production.

- Fetching the file from Azure Blob Storage. Network time and storage latency are far larger than local parsing time in most deployments.
- Queue handling in RabbitMQ: delivery, acknowledgement, redelivery after failure.
- Writing parsed rows to SQL Server, including transaction time and any constraint checks.
- Writing audit trail records. The audit trail is a compliance requirement, and each ingested entry produces records beyond the data rows themselves.
- Linking the parsed data to the electronic lab notebook entry it belongs to.
- Validation against instrument-specific schemas, if that step is separate from parsing.

So the correct reading is that parsing is cheap relative to everything around it. If ingest feels slow in production, the parser is unlikely to be the first place to look. Start with storage fetch and database writes, and measure them before changing any parsing code.

## Method notes

The run count of 200 benchmark runs is large enough that the median is stable against a handful of outliers, which is why the median was used rather than the mean. Some practical points on how these runs should be done and how to read them:

- Warm-up matters on .NET. The first runs pay for JIT compilation and cold caches. A benchmark harness should discard warm-up iterations or the early runs will drag the distribution. If the 200 benchmark runs were collected without a separate warm-up phase, the median is still fine, but the minimum and the tail would be skewed.
- Build configuration matters. Numbers from a debug build are not comparable with release builds. Only compare like with like.
- Machine matters. A laptop on battery and a build agent under load give different figures. The result here is a relative reference, and a regression check should be made on the same machine class.
- The input file should be read into memory once, outside the timed region, otherwise disk speed leaks into the figure.

I did not record the exact hardware or runtime build settings alongside the figure. That is a gap. When the benchmark is repeated, write those down next to the number in the same note so that the comparison is honest.

## How to use the number

The number is useful in three ways.

First, as a regression guard. If a later change to instrument-ingest-worker makes the same benchmark come out clearly slower than a median of 62 ms on comparable hardware, that is a signal to look at the change. What counts as clearly slower is a judgement call. Run-to-run noise on a shared machine can be considerable, so a small shift is not evidence of anything.

Second, as a sizing aid. Because parsing a file of this size takes so little time, capacity planning for the worker should focus on I/O and database throughput. Adding more worker instances to speed up parsing alone would likely do little.

Third, as a sanity check when someone proposes a rewrite of the parser for speed. At this level the parser is not the bottleneck, and the effort would be better spent elsewhere unless the benchmark shows a regression or the input shape changes a great deal.

## Caveats for compliance readers

Compliance officers who read this should not take the figure as a statement about audit completeness or timeliness. A fast parse says nothing about whether every row reached the audit trail, whether records were written in the right order, or whether the trail is tamper evident. Those properties are enforced and tested elsewhere in LabNotebook Sync. The benchmark only says the parsing step is not a time sink.

If the question is whether ingest can keep up with instrument output during a busy period, this note is not enough. That needs a load test through the full path with the queue, blob storage and database all in play.

## Open questions and next steps

Things worth doing next, roughly in order of value:

- Record hardware, runtime version and build configuration with the benchmark so the result can be reproduced.
- Add a benchmark for wider and narrower files, and for files with quoted fields and embedded separators, so the figure is not tied to one shape.
- Capture a high percentile and the maximum alongside the median.
- Measure memory allocation during parsing. Large files that are fully materialised could put pressure on the garbage collector, and that would not show up in a median time.
- Time an end-to-end ingest of a single file, from message receipt to committed rows and audit records, and compare it with the parse-only figure to see how small a share parsing really is.
- Decide whether the benchmark belongs in the regular build pipeline or stays a manual check run before releases. A manual check is cheaper but gets forgotten.

## Summary of the fact

In one line for later lookup: instrument-ingest-worker parsed a 5000-row CSV file in a median of 62 ms across 200 benchmark runs. It covers parsing only. It does not cover storage fetch, queue handling, database writes or audit trail writes. Reuse the number as a reference for regression checks on comparable hardware, and rerun the benchmark with fuller notes on environment before relying on it for any decision that matters.

If you update this note after a new run, keep the old figure next to the new one with the reason for the difference, instead of overwriting it. A change in the median is only meaningful if the reader can see what changed between the two runs: the code, the input, the machine or the runtime.
