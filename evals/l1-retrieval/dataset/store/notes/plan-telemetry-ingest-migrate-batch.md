---
id: 01M0W6WG8SR103X0BAK1G49NDJ
created: 2026-08-25T07:19-03:00
---

# telemetry-ingest batch write migration plan

Plan to move telemetry-ingest from per-point writes to batched writes against InfluxDB. The target is 2500 points per request. The reason is write overhead: every request costs connection, parsing and commit work on the InfluxDB side, and at the current rate we pay that cost far more often than needed. Batching should cut it.

## Goal

Migrate telemetry-ingest to batch writes of 2500 points per request. Success means fewer write requests to InfluxDB for the same incoming telemetry, with no lost or duplicated points and no visible delay in the forecasts and charging schedules that read the data.

## Current behaviour

telemetry-ingest receives device messages from MQTT, which arrive through Azure IoT Hub for devices that are registered there. It turns each message into points and writes them out in small groups. Small writes are the source of the overhead. Exact current group sizes should be checked in the code before starting; I did not record them here.

## Proposed change

Buffer points in memory and flush when the buffer reaches 2500 points. Also flush on a timer so a quiet installation does not hold data for long. The timer value is still open. One request per flush, no splitting unless the server rejects the size.

## Open questions

- How long can data sit in the buffer before the Julia forecasting jobs notice a gap?
- Does the Svelte dashboard show live values that depend on the newest points? If so, the flush timer sets how stale it looks.
- What is the right timer interval?

## Failure handling

A failed batch must not drop points. Retry with backoff, keep the buffer bounded, and apply backpressure to the MQTT consumer when the buffer is full rather than discarding. A partial failure should be logged with the batch size and the reason. Retries are safe only if writes are idempotent for the same series and timestamp, which InfluxDB gives us when the tags and timestamp match. Confirm this holds for our tag layout.

## Shutdown and restarts

On shutdown, flush what is buffered before exiting. On a crash, points in the buffer are lost unless MQTT delivery is acknowledged only after a successful write. Prefer acknowledging after the write, accepting some redelivery.

## Interaction with rollups

Late or revised points matter for rollups. See [[influx-rollups-must-retain-revised]] before changing how timestamps or ordering are handled in a batch.

## Testing

- Unit test the buffer: flush at the size limit, flush on the timer, flush on shutdown.
- Replay a captured stretch of telemetry into a test bucket and compare point counts with the old path.
- Simulate InfluxDB errors and timeouts to check retry and backpressure.

## Rollout

Ship behind a config switch so batch size can be changed or the old path restored without a deploy. Try one installer's fleet first, watch write latency and error rates, then widen.

## Metrics to watch

Requests per minute to InfluxDB, write latency, buffer depth, flush age, retry count, and dropped-point count, which should stay at zero.

## Next steps

1. Read the current write path and note the present group size.
2. Decide the flush interval with the forecasting owners.
3. Implement the buffer with the config switch.
4. Run the replay test, then roll out to one fleet.
