---
id: 01M3HPJ587GS3JP5YXATZQZ9WY
created: 2026-09-27T12:08-03:00
---

# pv-forecast-engine failures: what to check first

Notes from chasing the pv-forecast-engine falling over more than once. Not complete, written quickly so the next session does not start from zero.

## Symptom

The forecast job stops producing output for some sites, or the whole run dies partway. The battery scheduler then keeps using the last forecast it had, which looks fine until the tariff window shifts and charging is wrong.

## Where it usually breaks

Most of the time the failure is upstream of the Julia model itself. The model gets bad or missing input and throws. Look at the inputs before touching model code.

## Missing telemetry

If a site's inverter stops publishing over MQTT, the series in InfluxDB has a gap. The engine expects a continuous window of recent readings. A gap longer than the usual tolerance makes the feature step fail. Check the last write time for the site first.

## Late or duplicated messages

Azure IoT Hub can deliver messages late or twice after a device reconnects. Duplicated timestamps in the window have caused trouble in the resampling step. Late ones just show up as a temporary gap.

## Weather input

The irradiance and cloud cover feed is a separate dependency. When it returns an empty or short horizon, the engine fails instead of falling back. A quick look at the raw response tells you if this is the cause.

## Timezones and DST

Anything near a clock change is suspect. Timestamps are meant to be UTC end to end, but a local-time value slipped in once and shifted the whole day curve. Worth ruling out when failures cluster around a date.

## Site configuration

Wrong panel tilt, azimuth or capacity in a site record gives nonsense output, sometimes negative or above the physical limit. The engine has a sanity check that rejects those and marks the run failed. That is working as intended, fix the record.

## Julia side

First-run compile delay after a restart can look like a hang. Wait before assuming it is stuck. Memory use grows when many sites are batched in one process, so the batch size matters more than it seems.

## InfluxDB queries

Slow queries on wide time ranges hit the client timeout and surface as a generic failure. Narrowing the range or checking retention settings helps. The error text does not say it was a timeout, which cost time.

## Logs

The engine logs per site, so search by site first, not by time. The first error in a run is the useful one; later lines are fallout.

## Not yet confirmed

I have not verified whether a partial failure in one site can poison the shared batch state. It looked that way once but I did not reproduce it.

## Follow-ups

Add an explicit fallback to the previous forecast when inputs are short. Make the timeout error say it is a timeout. Alert on stale telemetry per site before the engine runs.

## Related

The existing note on pv forecast engine failures probably covers some of this with exact details. Merge the two if they overlap.
