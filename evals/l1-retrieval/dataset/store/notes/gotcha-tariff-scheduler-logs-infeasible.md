---
id: 01M1DP6ZG4NE79JNKM8V0R1FDV
created: 2026-09-01T02:14-03:00
---

# tariff-scheduler: INFEASIBLE when tariff periods share a boundary minute

tariff-scheduler produces no plan at all when two time-of-use periods share a boundary minute. It logs `INFEASIBLE: tariff window overlaps` and stops. It does not skip the bad period, and it does not fall back to the last good plan. The battery then gets no charging schedule from this component until the tariff data is fixed. The trap is that the data looks fine to a person. One period ends at a minute and the next one starts at that same minute. People read that as a normal handover, but the scheduler reads it as an overlap.

This note covers what the failure looks like, why it happens, how to confirm it, how to fix the data, and what to check before it happens again. It is written from what we know about the behaviour, not from a full read of the solver code. Where something is a guess, it says so.

## Symptom

The visible symptom is a missing plan. The forecast side still works: solar output forecasts keep arriving and the data keeps landing in InfluxDB. The charging schedule for the affected site just does not update. In the Svelte dashboard the schedule view stays stale, or empty for a site that is new. Nothing marks the site as failed, so the customer or installer usually notices first.

The log line to look for is:

```text
INFEASIBLE: tariff window overlaps
```

It is logged by `tariff-scheduler` once per scheduling run for the affected site. If the run is triggered often, the line repeats often. It is easy to scroll past if you only look for crashes, because the process does not crash. The run ends cleanly with an infeasible result and no plan.

Things that do not point to this problem:

- A device that stopped publishing over MQTT. That shows up as stale telemetry, not as this log line.
- A connectivity problem on the Azure IoT Hub side. That shows up as missing commands or missing device state, and again not as this log line.
- A forecast gap. That produces a different kind of failure, usually a thin or odd plan, not an infeasible result.

If you see the log line, go to the tariff data first. Do not start by restarting services.

## Cause

A time-of-use tariff is a list of periods, each with a start, an end and a price band. The scheduler turns these into a set of windows and then plans battery charging against them. It treats the windows as something that must not overlap. Two periods that share a boundary minute count as overlapping, because that one minute belongs to both of them. The scheduler cannot decide which price applies to that minute, so it declares the problem infeasible and gives up on the plan.

The usual sources of a shared boundary minute:

- A tariff entered by hand where the end of one period was typed as the same value as the start of the next. This is the most common case.
- A tariff imported from a utility document that writes periods as inclusive on both ends. Copying those values straight across gives a shared minute at every boundary.
- A tariff that was edited later. Someone moves the start of one period and forgets to move the end of the one before it.
- A period that wraps past midnight and meets the first period of the day at the same minute.

The last case is the easy one to miss. The wrap-around period and the first period of the next day look unrelated when you read the list top to bottom, but they meet at the day boundary.

The real rule is that the scheduler wants half-open periods: the start belongs to the period, and the end belongs to the next one. Our scheduler does not make that assumption for us. Check the actual input handling before assuming either way, because I did not confirm whether an exact end-equals-start pair is always rejected or only in some paths. In practice, treat any shared boundary minute as a failure.

## How to confirm

Start from the log line and the site it belongs to. Then look at the tariff periods stored for that site.

1. Find the site in the scheduler log by the `INFEASIBLE: tariff window overlaps` line. Note which site or tariff it was reported for.
2. Pull the tariff periods for that tariff, sorted by start time.
3. Walk the list in order and compare the end of each period with the start of the next. Do the same for the last period against the first one, across the day boundary.
4. If any pair shares a minute, that is the cause. If no pair does, this is probably not the plain boundary problem, even though the message says so. Look at whether the same period is listed twice, or whether two tariffs are active for the site at once.

A second tariff active at the same time as the first can produce the same message with no shared boundary at all. That is a different data problem with the same log line. It is worth checking when the boundaries look clean.

If the tariff comes from a source that is refreshed automatically, check when it last changed. A failure that starts right after a tariff refresh is a strong sign the new data has the shared minute.

## Fix

Change the tariff data so that no two periods claim the same minute. Pick the half-open convention and apply it everywhere: the earlier period stops just before the minute at which the next one starts. In the data, that means the end value of the earlier period is moved back by one minute, or the start value of the later period is moved forward by one minute. Either works. Which one to pick depends on what the utility document says the real price change time is. Keep the real price change minute attached to the period that is supposed to be charged at the new price.

After the edit:

- Re-read the whole list, not only the pair you changed. A tariff with one bad boundary often has several.
- Check the wrap-around across midnight again.
- Trigger a scheduling run for the site, or wait for the next one, and confirm the `INFEASIBLE: tariff window overlaps` line is gone and a plan is written.
- Confirm the plan reaches the device. The command path goes through Azure IoT Hub and MQTT, so a plan that exists but never arrives is a separate issue from this one.

Do not work around the problem by deleting one of the two periods. That hides the overlap but leaves a gap, and a gap in the tariff gives a different bad outcome: the scheduler has no price for those minutes and may charge at the wrong time.

## Why it stays silent

The scheduler refuses to guess. For a charging plan, a wrong guess costs real money: the battery might charge in the expensive band. Refusing to plan is the safe choice in the narrow sense. The cost of that choice is that the failure is quiet outside the logs.

What is missing today, as far as I know:

- No alert on the log line. Someone has to look for it.
- No status on the site in the dashboard showing that the last run was infeasible.
- No validation when a tariff is saved, so the bad data gets in and only fails later, at scheduling time.

The third item is the real fix. Rejecting a shared boundary minute when the tariff is saved moves the failure to the person who typed it, at the moment they can fix it. That is far better than a failure hours later in a service they never see.

## Checks to add

These are suggestions, not done work.

- Validate on save: reject any tariff where two periods share a boundary minute, including the wrap across midnight. Show the pair that collides in the error message.
- Validate on import: if a tariff arrives from an external document, normalise to the half-open convention before storing, and log what was changed.
- Surface the failure: show the last run result for each site in the Svelte dashboard, and mark a site whose last run ended with `INFEASIBLE: tariff window overlaps`.
- Alert on repeats: if the same site hits the same log line on consecutive runs, notify the installer or whoever owns the site.
- Add a test with a tariff that has a shared boundary minute, and one with the wrap-around case, and assert that the result is the infeasible one. The Julia side is where the scheduler logic lives, so that is where the tests go.

A regression test for the exact message is worth having. Other code and some runbooks match on the text, so changing it silently would break them.

## Notes for whoever hits it next

Check the data before the code. In every case I know of, the scheduler did what it was meant to do and the tariff was wrong. Resist the urge to relax the check in the solver so that shared minutes are allowed. That would turn a visible failure into a quiet pricing error, which is worse.

If you change how boundaries are interpreted, do it in one place, document the convention next to the tariff schema, and update every importer to match. Mixed conventions between importers are how this problem got in to begin with.

If the message appears and the periods look clean, say so in the ticket and look for a duplicate or a second active tariff before touching the solver. Update this note with whatever you find, in particular whether the exact end-equals-start pair is always rejected, since that part is still unconfirmed.
