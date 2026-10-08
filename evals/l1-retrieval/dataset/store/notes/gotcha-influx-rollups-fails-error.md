---
id: 01M2N3F5XVTG97JP2PETGVT9FT
created: 2026-09-16T09:36-03:00
---

# influx-rollups fails on long ranges

influx-rollups dies with `error in query plan: memory allocation limit reached` when a single run covers a range longer than `30 days`. It is not a flaky failure and it is not a data problem. The query plan for the whole range is built in one piece, and past that length it no longer fits in the memory the query is allowed. If you see this error, check the range of the run first, before looking at InfluxDB itself, the MQTT feed or anything on the Azure IoT Hub side.

## Symptom

A rollup run that asks for a long window, for example a backfill after a gap in ingestion or a first run for a new installer site, stops early. The log shows the planner error and nothing is written for that run. Short runs for the same measurement and the same site work fine, which is the quickest way to tell this problem from a real outage.

The error text to search for:

```
error in query plan: memory allocation limit reached
```

Nothing partial is committed from the failed run, as far as we have seen. Still, look at the target bucket before assuming that. A rerun with a shorter range should fill it in either way.

## Why it happens

The limit is on the range of one run, not on how much data is in it. A quiet site with sparse readings still fails if the run spans more than `30 days`. A busy site with many series can get close to trouble sooner, so treat `30 days` as the ceiling that is known to be safe to stay under, not as a guarantee for every dataset. Keeping runs well under it is cheap.

We did not find a setting that raises this safely. Raising the memory limit on the InfluxDB side only moves the cliff and puts the shared instance at risk, because the same instance serves the Julia forecasting jobs and the battery scheduling reads. Do not do that to make a backfill go through.

## What to do

Split any long range into windows that are each `30 days` or shorter, and run them one after another. Run them in order, oldest first, so the later windows can build on the earlier rollups if the downstream tiers need that. Do not run many windows in parallel against the same instance. That brings back the memory pressure through a different door.

When you write a wrapper or a scheduled job around influx-rollups, put the window split in the wrapper and fail early with a clear message if a caller passes a longer range. Do not rely on the planner error to tell the caller. The planner error is slow to arrive and it is easy to misread as an infrastructure fault.

- Backfills: loop over windows, check each one finished, then go on.
- Regular scheduled runs: these are short already, so they are unaffected. Watch for the case where a scheduler was down for a while and the next run tries to catch up over the whole gap. That is how most people hit this.
- Ad hoc runs by hand: pick the start and end yourself and keep the span within the limit.

## Things that look related but are not

Memory errors from the Julia side, from the Svelte dashboard queries or from the MQTT bridge have different text and different causes. Only the exact planner message above points to this problem. If the message is different, do not apply this fix blindly.

If a window of `30 days` or shorter still fails with the same error, the cause is something else, probably a very high series count for that site. Write it down as a new finding with the site and the window, rather than editing this note to say the limit is lower.
