---
id: 01M2YCKK2JNPX6DSZ46YBMQZ9F
created: 2026-09-20T00:08-03:00
---

# influx-rollups keeps failing

Notes on why the influx-rollups job fails now and then. Written quickly, not complete. The pattern is the same each time: the rollup task stops, the downsampled bucket has a hole, and the forecast in Julia reads stale or missing values.

## Symptom

The rollup run reports an error and the aggregated bucket stops getting new points. The Svelte dashboard shows a flat line for solar output after the gap.

## Where it shows up

Mostly after a burst of late data from devices that were offline. Telemetry comes in through MQTT and Azure IoT Hub, then lands in InfluxDB raw. influx-rollups reads that raw data.

## First suspect: late points

Points that arrive after the window has already been rolled up are not picked up again. The task only looks at the recent range, so older arrivals are skipped silently.

## Second suspect: query size

When a backlog is flushed, the range the task has to read gets large and the query hits the configured limit. The task then errors instead of returning partial data.

## Third suspect: retention

If the raw bucket retention is shorter than the catch-up range, the source data is already gone. Check the retention setting before blaming the task.

## What did not help

Just restarting the task. It reruns the same window and fails the same way.

## Things to check

- Task run log in InfluxDB for the last failed run
- Whether the raw bucket has the points for the failed window
- Whether the target bucket write was rejected
- Clock skew on the device that sent the late batch

## Quick look

```flux
from(bucket: "raw")
  |> range(start: -1h)
  |> aggregateWindow(every: 5m, fn: mean)
```

Use only as a sanity check that the raw data exists and aggregates.

## Backfill

Run the rollup by hand over the missing range in smaller pieces. Do not do one huge range, it fails for the same reason as above.

## Impact on scheduling

Battery charging schedules against the time-of-use tariff use the forecast. A gap in rollups makes the forecast fall back to defaults, which can charge at the wrong time.

## Mitigation ideas

- Split large backfills into smaller windows
- Add a delay offset so late points are included
- Alert when the target bucket has no new points for longer than usual

## Open questions

- Is the error always the query limit, or sometimes a write rejection?
- Does the Azure IoT Hub batching make the late bursts worse?

## To do

Confirm which of the suspects is the real cause in the most recent failure, then replace this note's guesses with facts.
