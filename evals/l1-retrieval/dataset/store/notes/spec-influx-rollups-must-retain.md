---
id: 01K5NMPYN79XHJQTYVV7SJQD90
created: 2025-09-21T05:02-03:00
---

# influx-rollups spec: retention and downsampling

This note specifies what influx-rollups keeps, for how long, and why. The headline requirement: influx-rollups must retain 5-minute aggregates for 400 days. Everything else here is context and open detail around that one number. Anything not stated as settled is general on purpose and should be confirmed against the running deployment before relying on it.

## Purpose

influx-rollups is the layer in GridHaven that turns raw household telemetry in InfluxDB into coarser aggregates. The forecasting code in Julia and the Svelte dashboards both read from it. Raw points are too dense to keep for long, and too noisy to plot over a season. The rollups give a cheap, stable history for training forecasts, for showing customers how their panels and battery behaved, and for installers who check a system after commissioning.

## Retention requirement

The 5-minute aggregates must be kept for 400 days. That is the floor, not a target to trim. Anything that deletes, expires or overwrites those aggregates sooner than 400 days breaks this spec. Keeping them longer is allowed if storage permits, but nobody should assume it. A reader answering the question of how long the 5-minute aggregates live should answer 400 days.

## Why 400 days

The reason is year-over-year comparison. A full year of data is needed to compare the same season twice, and the extra margin beyond a year covers late-arriving data, a delayed start of a comparison window, and installers who review a system a little after its anniversary. Solar output is strongly seasonal, so forecast models that learn from last year's same period need the whole span available at fine resolution. Tariff schedules also change, and replaying a past tariff against real history needs the same span.

## What counts as an aggregate

An aggregate here is a bucketed summary of a raw series over a fixed time window. For each bucket we keep the statistics the consumers need: mean, minimum, maximum and count for power-like measurements, and sums for energy-like ones. Which statistics exist per measurement is decided by the task definitions, not by this note. The window width of the finest rollup is five minutes, and that is the tier this retention rule covers.

## Inputs

Raw readings reach InfluxDB from devices through Azure IoT Hub and MQTT bridging. Inverters, meters and battery controllers each publish on their own cadence. influx-rollups reads only what is already in InfluxDB. It does not subscribe to MQTT itself and it does not talk to Azure IoT Hub. If ingestion is late or gappy, the rollups reflect that until a backfill runs.

## Outputs

Outputs are written back to InfluxDB in separate destinations from the raw data, so retention can be set independently for each. The 5-minute tier is the main output. Coarser tiers may exist for long dashboards, but they are not covered by this requirement and must not be treated as substitutes for the finest tier when the forecaster asks for detail.

## Retention per tier

Raw data has its own, much shorter retention, set elsewhere. The 5-minute aggregates are held for 400 days. Coarser tiers, if present, may be held at least as long. The rule of thumb: a coarser tier never expires before a finer tier built from the same source, because otherwise a dashboard could lose history that the finer tier still has. If someone proposes a change to any tier, check it against this ordering first.

## Scheduling and freshness

Rollup tasks run on a regular schedule and process the most recent complete windows. A window is only rolled up once it is closed and a grace period has passed for late points. The grace period is a tuning choice and should stay modest. Consumers should expect the newest bucket to lag a little behind real time, and the forecaster must not treat an absent newest bucket as zero output.

## Late and out-of-order data

Devices buffer when connectivity drops and then flush in bursts. Those points land with old timestamps. The rollup tasks need to reprocess recent windows so late points are folded in. Reprocessing must be idempotent: writing the same bucket twice yields the same stored values, not doubled sums. Points that arrive after the reprocessing horizon are handled by an explicit backfill, never silently dropped without a record.

## Backfill

A backfill recomputes aggregates for a chosen time range from raw data. It only works while raw data still exists for that range. Since raw retention is shorter than 400 days, a backfill older than the raw window cannot be done, which is one more reason the 5-minute tier is the system of record for history. Do not run a backfill that overwrites older good aggregates with partial results from a thinned raw range.

## Downsampling rules

Downsampling must preserve what the consumers rely on. Energy totals must be conserved: summing fine buckets and summing coarse buckets for the same span should agree. Means of power must be weighted by the number of points, not averaged naively across buckets. Gaps stay gaps; we do not fill them with interpolated values inside the rollup layer, because the forecaster and the dashboards treat missing and zero differently.

## Time zones and tariff alignment

Storage is in UTC. Bucket boundaries are aligned to UTC, which keeps them stable across daylight saving changes. Time-of-use tariff periods are local, so the battery scheduler maps tariff periods onto UTC buckets itself. Because the finest window is short compared with any tariff period, the mapping is exact for the periods in use. Do not shift bucket boundaries to local time inside influx-rollups.

## Consumers

The Julia forecasting code reads the finest tier for training and for recent-history features. The scheduling code reads recent buckets to compare forecast against actual. The Svelte front end reads whichever tier suits the zoom level. All of them depend on the retention guarantee above, and none of them should cache assumptions about shorter history.

## Storage and cost

Keeping the fine tier for 400 days across all households is the main storage cost of this component. It is acceptable because aggregates are far smaller than raw points. If storage pressure grows, the answer is to compress or shrink fields, trim unneeded statistics, or add capacity. Shortening the 5-minute retention is not an option under this spec without changing the spec first and telling the forecasting owners.

## Monitoring

We want alerts for three conditions: rollup tasks failing or falling behind, a tier whose retention setting differs from this spec, and a sudden drop in bucket counts per household that suggests ingestion trouble rather than real quiet. The retention check should compare the configured period for the 5-minute destination against 400 days and complain if it is lower.

## Change control

Any change to retention, window width, statistics kept or destination layout goes through an edit to this note first. Record the reason and the date. Expiry is hard to undo, since deleted aggregates for old periods may not be rebuildable once raw data is gone. Treat a retention reduction as destructive and get explicit agreement from the owners of the forecasting and dashboard code before applying it.

## Open questions

Whether coarser tiers should be pinned to a longer period than the fine tier is undecided. The exact grace period for late data is still a tuning item. We also have not settled how to present gaps to customers in the front end. None of these affect the settled requirement: the 5-minute aggregates are kept for 400 days.
