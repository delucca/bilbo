---
id: 01KYZEYVPCVKD1HJ2C89HDMP5K
created: 2026-08-01T17:06-03:00
sources:
  - "doc: Ingest load test report"
---

# telemetry-ingest load test result

One load test of telemetry-ingest gave a clear throughput number: it sustained 12000 messages per second on 4 vCPUs, with a p99 write latency of 85 ms. This note records that result, what it does and does not tell us, and what to check before leaning on it for capacity planning.

## Result

telemetry-ingest held 12000 messages per second on 4 vCPUs. The p99 write latency, measured on writes into InfluxDB, was 85 ms. The run was sustained, not a short burst, so the figure is a steady-state rate and not a peak that quickly fell off.

If someone asks what one node of telemetry-ingest can take, the answer from this test is: 12000 messages per second at 4 vCPUs, with p99 writes at 85 ms. Anything beyond that is extrapolation.

## What the pipeline looked like

Messages come in from devices through Azure IoT Hub and MQTT, are decoded and batched by telemetry-ingest, and are written to InfluxDB. The forecasting side in Julia reads from InfluxDB later, so it does not sit in the hot path of this test. The Svelte dashboard was not involved.

The latency number is about the write step, not end-to-end delivery from the household device. Time spent in the hub and on the broker is not included.

## Why the number matters

Installers add households in batches. Each household sends inverter and battery readings at a regular interval, so load grows roughly linearly with the customer count. The test gives us a ceiling per node to compare against projected fleet size. Battery scheduling against time-of-use tariffs needs fresh data, so write latency matters as much as raw rate.

## Caveats

- This is one load test, not a series. We have no run-to-run variance.
- Message payloads in the test were synthetic. Real payloads vary in size and field count, which can shift both throughput and write latency.
- The 85 ms figure is a p99. Tail behaviour beyond that (p99.9, worst case) was not looked at.
- Only the 4 vCPU shape was tested. Scaling to more or fewer cores should be measured, not assumed linear.
- Downstream read load from forecasting jobs was not running at the same time, so contention on InfluxDB is untested.

## Open questions

Does the rate hold when forecast queries run against the same InfluxDB instance? How does the system behave when devices reconnect all at once after a broker or hub outage? Those reconnect storms are the likeliest real-world spike, and the test did not model them.

Also unknown: where the limit sits. We do not know whether the bottleneck at 12000 messages per second was CPU in telemetry-ingest, batching settings, or InfluxDB write capacity. Finding this out would tell us whether adding vCPUs helps at all.

## Next steps

1. Repeat the load test a few times and record the spread.
2. Replay a sample of real payloads instead of synthetic ones.
3. Add a concurrent read workload from the forecasting side.
4. Simulate a mass reconnect and watch queue depth and latency.
5. Profile to find the actual bottleneck before changing instance size.

## How to use this note

Quote the 12000 messages per second and 85 ms figures together with the 4 vCPUs condition. Do not quote them as a general capacity claim. If a later test changes the numbers, update this note rather than adding a second one, and remove statements that no longer hold.
