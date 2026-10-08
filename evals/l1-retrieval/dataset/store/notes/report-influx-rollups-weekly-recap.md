---
id: 01KG8W3Q9YKJR6132W4R8R273R
created: 2026-01-30T22:54-03:00
---

# influx-rollups weekly recap

This week on influx-rollups was mostly about making the downsampled series trustworthy enough that the Julia forecasting code can read them without special cases. Not much new surface area. Most of the time went into reading what the rollups actually contain versus what we assumed they contain, and fixing the gaps between the two. Notes below are rough and written fast; nothing here is a final call on any setting.

## What got done

The main thread was checking rollup output against the raw household readings that arrive through MQTT and Azure IoT Hub. For a handful of sample installs I compared raw points with the aggregated buckets and looked for places where they drift apart. The bulk of the mismatches came from late-arriving readings. A device reconnects after a network drop, flushes its buffer, and the points land in buckets that were already aggregated. The rollup task had long since moved on, so those buckets stayed stale.

I changed how the rollup task treats the trailing edge of the window. It now looks back over a recent stretch of buckets on every run, instead of only touching the newest one. That costs a bit more work per run, but the stale-bucket problem mostly went away in the sample installs. I have not checked it against the full fleet, so treat it as promising, not settled.

I also tidied the naming of the destination buckets and measurements so they read the same way across the different resolutions. Before this, the coarser rollups had picked up slightly different tag names than the finer ones, which meant the forecasting side needed a mapping layer. Most of that mapping can now be deleted. I left it in place for now because the Julia side still references it and I did not want to change both ends at once.

Smaller items:

- Cleaned up a few leftover task definitions that no longer matched anything in use.
- Added comments to the rollup definitions saying which downstream consumer depends on each one.
- Re-read the retention settings on the raw and rolled-up data to make sure raw data is not expiring before the rollups have caught up.

## Data flow, as I understand it now

Writing this out helped me see where the lag comes from, so I am keeping it here.

```
MQTT -> Azure IoT Hub -> InfluxDB (raw) -> influx-rollups -> InfluxDB (rolled up) -> Julia forecasts -> Svelte
```

The rollups sit in the middle. Anything wrong with them shows up twice: once in the forecast of household solar output, and again in the battery charging schedule that is planned against the time-of-use tariffs. A bad bucket in the rollup can make a charging plan look sensible when it is not, and nobody sees it until a customer asks why the battery charged at a poor moment. That is why I spent the week on correctness instead of new features.

## Problems and open questions

Several things are still unclear and I would rather list them than pretend they are closed.

**Gaps versus zeros.** When a device is offline, the raw series has no points. Depending on how the aggregation is written, the rollup either leaves a hole or fills it with a zero. For solar output, a zero at midday looks like a real, terrible day, not missing data. The forecasting code cannot tell the difference right now. I think holes are the safer representation, but the Julia side needs to handle them explicitly, and I have not confirmed it does.

**Time zones and tariff boundaries.** The tariff windows are defined in local time for each installation, while the stored timestamps are not. Bucket boundaries in the rollups are aligned to the stored clock. Around tariff changes and around daylight saving shifts, a bucket can straddle two tariff periods. I noticed this in one install and have not yet checked how common it is. It may be fine in practice if the buckets are fine enough, but I have not verified that.

**Backfill.** When I change a rollup definition, the old history stays in the old shape. I did a partial backfill for a few sample installs to test. A full backfill is a bigger job and could put real load on the database while the installers' dashboards are in use. Someone needs to decide on timing and batching before it is done. I did not start it.

**Duplicate points.** A few raw series seem to contain repeated readings, probably from the device retrying after a missed acknowledgement. InfluxDB overwrites points that share a timestamp and tag set, so many duplicates are harmless. Where the retry carries a slightly different timestamp, the sum-style aggregations double count. I have only looked at this on a small sample.

## Next week

- Confirm with the forecasting code how it handles empty buckets, and agree on one convention with whoever owns that side.
- Look at how often tariff boundaries and bucket boundaries disagree across the real install base, not just my samples.
- Remove the old tag mapping layer once both ends agree on the new names.
- Write down a backfill plan and get it reviewed before running anything against production data.
- Add a simple check that compares rollup totals to raw totals for a rotating sample of installs and flags drift. Right now I find problems only because I go looking.

## Notes for whoever picks this up

The rollup definitions are small but the assumptions behind them are not written down anywhere except in people's heads and in this kind of note. If you change one, check who reads it first. The comments I added list the consumers, but they may be incomplete. Also, be careful with the look-back change: it is cheap on a quiet system but I have not seen how it behaves when many devices reconnect at once, for example after a broker outage on the MQTT side. If the task starts running long, that is the first place to look.
