---
id: 01M1T3GWEJXHB1JTWY32PBSNXZ
created: 2026-09-05T21:57-03:00
---

# carrier-sync-job report

This is a working report on carrier-sync-job, the nightly job in FreightWeave that pulls carrier schedule data into our own store so the route planner and the load rebalancer work from current timetables. I wrote it in a hurry after going through how the job behaves in practice. It is not a spec. It records what the job does, how long it takes, where it hurts, and what I would look at first when something looks off. The one hard figure to keep in mind: a typical nightly run of carrier-sync-job processes 1850 schedule rows in 3.2 minutes. Everything else here is described in general terms on purpose, because the exact settings move around between environments.

## Purpose

carrier-sync-job exists because dispatchers cannot plan a multi-leg truck and rail route against schedules that are a day stale. Carriers publish timetables, slot windows, and capacity notes in their own formats and on their own rhythm. The job collects those, normalizes them into one internal shape, and writes them where the planner can read them quickly. Without it the OR-Tools model would be solving against old departure times, and the rebalancer would shift loads onto trains that no longer run. The job is deliberately boring. It should finish well before dispatchers start their morning, and it should leave the store either fully updated or visibly not updated, never half and half without anyone knowing.

## Typical run profile

A typical nightly run processes 1850 schedule rows in 3.2 minutes. That is the baseline I compare against. If a run is far longer than 3.2 minutes with a similar row count, something upstream is slow or the job is retrying. If the row count is far below 1850 schedule rows, a carrier feed probably came back empty or partial, and that deserves attention even if the run finished quickly. The row count moves a little with the season and with how many carriers changed their timetables, so I treat the figure as a rough center, not a limit. Short runs are not automatically good news. A fast run with very few rows has fooled me before.

## What a schedule row is

A schedule row is one normalized record describing a carrier service on a lane: origin, destination, mode (truck or rail), departure and arrival windows, and any capacity or restriction note that came with it. One carrier timetable can expand into many rows because a service that runs on several days becomes separate rows. That is why the row count is the useful measure for the job and the number of carriers is not. When someone says the job handled a lot of data, ask how many rows, since the raw files can look large while the row count stays modest, and the reverse also happens.

## Pipeline stages

The job runs in clear stages. First it fetches the carrier data from each source. Second it parses each source into raw records. Third it normalizes those into the internal row shape and drops anything it cannot make sense of. Fourth it compares the new rows with what is already stored and works out what changed. Fifth it writes the changes. Sixth it announces that fresh data is available so other parts of the system can react. Keeping those stages separate in the code has paid off, because most failures can be pinned to one stage by looking at the logs, and a failure in an early stage should stop the later ones from touching the store.

## Where the data lands

The normalized rows end up in the shared cache that the planner reads, and Redis is the store involved. The planner is a FastAPI service, and it reads schedule data at request time, so the write needs to be consistent from its point of view. The job writes into a fresh keyspace and then switches readers over, instead of editing live keys one at a time. This avoids the case where a planner request sees a mix of old and new timetables for the same lane. If the switch does not happen, readers keep using the previous day, which is the safe failure.

## Change announcements

After a successful write the job publishes a message on Google Cloud Pub/Sub saying that schedule data changed, with enough detail for subscribers to know which lanes were affected. The rebalancer listens for these. When a timetable changes in a way that breaks a route already assigned to a load, the rebalancer can start looking for alternatives without waiting for a dispatcher to notice. Messages are small and carry no schedule content themselves. Subscribers read the data from Redis. That keeps the messages cheap and avoids two sources of truth.

## Scheduling and timing

The job runs once a night, in the quiet window after carriers have usually published their updates and before regional dispatchers start. At about 3.2 minutes for a typical run there is a lot of slack in that window, which is useful because carrier endpoints are sometimes slow and retries add time. I would not tighten the window just because the typical run is short. The slack is what lets a bad night still finish before morning. If runs ever start approaching the edge of the window, the first suspect is an upstream feed, not our own code.

## Upstream carrier feeds

Each carrier has its own feed, and they differ a lot. Some give clean structured data, some give files that need cleanup, and a few change their layout without warning. Most of the job's real complexity lives in the per-carrier adapters. A new carrier means a new adapter and a set of sample files for tests. When a feed changes shape, the symptom is usually a sudden drop in rows from that one carrier, not a crash, because the parser skips what it cannot read. That quiet failure mode is the main reason the row count is worth watching every night.

## Normalization rules

Normalization turns local conventions into one internal form. Times are brought to a single reference so a departure in one region compares correctly with an arrival in another. Location names are mapped to our own terminal identifiers. Modes are reduced to the truck and rail categories the planner understands. Records that cannot be mapped to a known terminal are set aside and counted, not guessed at. I prefer losing a row loudly to inventing one, since a wrong terminal mapping could send a load to the wrong yard and the dispatcher would only find out on the ground.

## Idempotency and reruns

The job is meant to be safe to run again. Running it twice on the same input should leave the same result, because writes are computed from the difference between the new rows and the stored rows, and an identical input has no difference. This matters when a run is interrupted or when someone starts it by hand to pick up a late carrier update. A rerun may announce changes again if the first attempt failed after writing but before announcing, and subscribers should tolerate duplicates. They do, since they re-read current data instead of applying the message as a delta.

## Failure handling

Failures are handled per stage and per carrier. A single carrier feed failing should not sink the whole run: the job keeps that carrier's previous rows, marks the carrier as stale in its report, and goes on. A failure in the write stage is more serious, and the job stops without switching readers. Retries are limited and use waiting between attempts, so a dead endpoint does not hold the run forever. The thing I want to avoid above all is a run that reports success while having quietly kept yesterday's data for most carriers.

## Monitoring and what to watch

I watch three things. One is the duration compared with the typical 3.2 minutes. Another is the row count compared with the typical 1850 schedule rows. The third is how many carriers were marked stale. Duration up with rows steady points at slow upstream. Rows down with duration normal points at a parser skipping records. Stale carriers above zero for several nights in a row means someone should contact that carrier or fix the adapter. Alerts should fire on the shape of these, not on exact thresholds copied from this note.

## Known gotchas

A few traps I have already met. Carriers sometimes publish a timetable that is valid only from a future date, and the job must not apply it early. Holiday schedules arrive as exceptions layered on top of regular ones, and getting the order of layering wrong gives departures on days with no service. Rail slot windows can be reissued with the same identifier but different times, so identity cannot rest on the identifier alone. And a feed that returns successfully with an empty body is not an error from the transport point of view, so the job has to check content, not just status.

## Testing approach

Tests use saved sample files from each carrier, including ugly ones. Each adapter has a test that feeds a sample through parsing and normalization and checks the resulting rows. There is also a test for the diff and write logic using a small fake store, and one that checks the job leaves the old data in place when the write stage fails. I would like a replay test that runs a full recorded night end to end, but that has not been built. For now the end to end check is looking at a real run in a staging environment and comparing duration and row count with the usual figures.

## Performance notes

At the current volume the job is not performance sensitive. Most of the 3.2 minutes goes to waiting on carrier endpoints, not to our own processing. Parsing and normalization are cheap, and the diff against stored data is fast. If volume grows a lot, the sensible first step is fetching carriers in parallel with a sensible cap, not rewriting the normalization. I would measure first. Speeding up work that is a small share of the run would not change the total much.

## Open questions

Some things are still unsettled. Whether stale carriers should be shown to dispatchers directly in the planner, so they know a lane is running on old data. Whether the announcement should include a short summary of how many rows changed per carrier, which would help the rebalancer decide how urgently to react. Whether the job should keep a short history of past runs' durations and row counts in a place that is easy to chart, instead of leaving them in logs. None of these block anything today, but the first one comes up often when dispatchers ask why a route looked wrong.

## Next steps

Short list for whoever picks this up. Build the recorded full-night replay test. Add the stale-carrier flag to what the planner exposes. Store duration and row count per run so trends are visible without digging in logs. Review each carrier adapter for the quiet skip behavior and make it count and report what it drops. And keep the typical figures in mind when judging a night: about 1850 schedule rows in 3.2 minutes. Update this report when those figures change noticeably, since the comparison is only useful while the baseline is true.
