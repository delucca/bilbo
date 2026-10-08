---
id: 01KGRA9BEG1H75NEJAHJMCFA0T
created: 2026-02-05T22:50-03:00
---

# ledger-matcher CPU profiling: JSON decoding hot spot

Profiled ledger-matcher because matching throughput on large settlement batches was lower than we expected and the consumers kept lagging behind the Kafka topic. The result was clear and a bit boring: 63% of CPU time was spent in json.Unmarshal while decoding settlement events. The matching logic itself, the PostgreSQL lookups and the gRPC handlers all together were well behind that. So the first thing to fix is decoding, not the matcher.

## What was measured

The profile was taken from a running ledger-matcher instance while it consumed a backlog of settlement events from Kafka. It was a CPU profile, not an allocation or lock profile, so it says where time goes on the core, not why memory grows. The flame graph shows json.Unmarshal as one very wide block under the event consumer loop, with reflection and map allocation underneath it. Time spent waiting on the database does not show up in a CPU profile, so it is not counted in the 63%.

I did not run a second profile with a different event mix. Treat the number as representative of a busy settlement window, not as a fixed property of the service.

## Why decoding is so expensive

Settlement events from the card processors are fairly wide records. Most fields are never read by the matcher, which only needs amounts, currency, processor reference, and a few timestamps. The decoder still parses and allocates for every field, because the standard library decoder works through reflection and builds the full structure each time.

Some events also carry nested detail blocks (fees, adjustments) that get decoded into generic maps in a few paths. Those are the worst offenders for allocation churn, and the garbage collector cost follows from it.

## Options to try

Ordered by how cheap they are to try:

- Decode only the fields the matcher needs, into a small struct, and skip the rest. This is the lowest-risk change and should cut most of the cost.
- Remove the generic map decoding in the nested detail paths and use typed structs.
- Try a faster JSON library that is a drop-in replacement, and compare against the standard library on real event samples. Check that numeric handling for money amounts stays exact; never let amounts go through floating point.
- Move the producer side to a schema-based encoding such as protobuf, since gRPC is already in the stack. This is the biggest win but needs coordination with whoever produces the events, and a migration period with both formats.

My recommendation is the first two, then re-profile before deciding on anything bigger.

## Open points

- Confirm with a benchmark on recorded events that the narrower struct gives the same match results as today. Mismatch flagging must not change.
- Re-profile after each change and record the new share here, so we can see whether decoding stops being the top item.
- Check whether batching of Kafka reads changes the picture. Larger batches may hide per-message overhead but will not remove the decoding cost.
- Find out whether any downstream code relies on fields that the narrow struct would drop, for example audit or review output.

## Where to look

The consumer loop and the event decoding code in ledger-matcher are the place to start. Keep the benchmark next to the decoder so the next person can rerun it without rebuilding the setup from this note.
