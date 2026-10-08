---
id: 01KJC6KM33CT5JZ3PYKSKJPEQE
created: 2026-02-26T02:26-03:00
---

# influx-rollups retention spec

This note replaces the earlier note "influx rollups must retain"; the new value is that influx-rollups keeps its downsampled aggregates for 730 days.

## Requirement

influx-rollups must retain its downsampled aggregates for 730 days. That is the whole retention rule for the rollup data, and it supersedes the shorter period written in the old note. Anyone reading the old note should treat its number as wrong and use 730 days instead.

The rule covers the aggregated series that influx-rollups produces from the raw device telemetry: the coarser solar output summaries and the battery state summaries that the forecasting and scheduling code reads. It does not describe how long raw points live. Raw retention is a separate setting and stays as it was, short, because the rollups are what we keep for the long view.

## Why the longer period

Forecasting in GridHaven needs more than one full seasonal cycle of household solar output to be useful. A single year leaves the forecaster with one sample of each season per household, and any odd year (a cloudy spring, a month with the inverter offline) then distorts the model. Keeping the aggregates for 730 days gives two samples of every season, so the Julia forecasting jobs can compare year over year and discard an outlier year instead of learning from it.

Installers also asked to show customers a year-on-year view of generation and of how much the battery shifted load against the time-of-use tariffs. That view in the Svelte dashboard only works if the previous year's aggregates are still there when the current year ends.

## What has to change to meet it

Retention is set on the storage side in InfluxDB, on whatever bucket holds the rollup output. The bucket policy for the rollup destination has to be set to 730 days. Check the actual bucket settings rather than trusting any config file, since the old shorter value may still be live in a deployed instance even after the repo is updated.

Points to check when applying it:

- The retention on the rollup destination bucket, in every environment where influx-rollups runs, not only production.
- Any provisioning or deployment script that creates the bucket, so a fresh install does not come up with the old value.
- Any cleanup or delete task that removes old aggregates on its own schedule. If one exists it must not remove data younger than 730 days.
- Dashboard queries and Julia jobs that assume a shorter window. They should still work, but look for hard-coded lookback limits that would hide the older data.

The rollup tasks themselves do not change. They keep reading raw data that arrives over MQTT through Azure IoT Hub, aggregate it, and write it to the rollup bucket. Only how long the output survives changes.

## Storage and cost notes

Doubling or more the lifetime of the aggregates grows the rollup bucket roughly in proportion. Aggregates are small compared with raw points, so this should be acceptable, but watch disk use on the first few weeks after the change. Growth will not show its full size until 730 days of data have built up, so do a capacity estimate now instead of waiting to see it.

Lowering the retention later would delete data past the new boundary and cannot be undone. Do not shorten it without a deliberate decision and a note that replaces this one.

## Open points

Whether data that was already dropped under the old shorter period can be rebuilt is not settled. If raw data for that range is gone, it cannot be. Do not promise customers a full two-year history for installs that predate this change.

Whether the downsampling resolution should differ for older data is not decided here. This note only fixes how long aggregates are kept.

If a later session changes the period again, update this note in place, or write a new one and say which one it replaces, as was done here.
