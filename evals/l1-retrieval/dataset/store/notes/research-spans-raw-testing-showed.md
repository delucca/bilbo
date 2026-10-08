---
id: 01K1VT9SQYQGCR2GQZGT9MHAB3
created: 2025-08-04T21:32-03:00
---

# spans_raw service column: LowCardinality test

Testing showed that declaring the service column of spans_raw as `LowCardinality(String)` cut on-disk size of that column by 31 percent. This note records what was tried, why it matters for TraceQuill, and what is still open. It is a research note, not a decision: nobody has agreed to change the production schema yet.

## Why we looked

spans_raw is the ClickHouse table where the OpenTelemetry collector pipeline lands every span before the deploy-regression summaries run. It grows fast. The service column repeats the same few values over and over, since a given cluster only has a limited set of services emitting spans. A plain String column stores each of those repeated values in full, so it looked like an easy place to save disk and some scan time.

## What was tested

The service column of spans_raw was declared as `LowCardinality(String)` instead of String, with the same data loaded into both variants. The rest of the schema, the ordering key and the partitioning were left as they were, so the comparison isolates the one column type.

Result: on-disk size of that column dropped by 31 percent. This is the size of the single column only, not the whole table. The table total shrinks by less, because trace ids, span ids, timestamps and attributes still take most of the space.

## Caveats

- The saving depends on how many distinct service names exist. LowCardinality uses a dictionary, and it works well when the distinct count is small. If a deployment has services that churn names a lot (per-build suffixes, for example), the dictionary grows and the benefit shrinks. That is worth checking before rollout.
- Only the storage size was measured as a firm number. Query speed for the Grafana panels and the latency summaries was not measured carefully, so no claim about it is made here.
- The test data was a sample, not a full production copy. A bigger sample may give a different percentage.

## Open questions

- How does the Java ingestion code behave when it writes into the changed column? Inserts should accept plain strings, but this was not confirmed against the real writer.
- Changing the type of an existing column on a live table means a mutation that rewrites data. We have not planned how to run that on the Kubernetes-hosted cluster without hurting ingestion.
- Do other low-variety string columns in spans_raw, such as span kind or status, give a similar saving? Not tested.

## Next steps

Repeat the measurement on a larger and more recent slice of data, record the distinct service count next to the result, and time the Grafana queries on both variants. If the saving holds, write a decision note and a migration plan before touching the production table.
