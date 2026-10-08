---
id: 01KSRDNCSSS2GZ51K7Z3BC4KVQ
created: 2026-05-28T20:10-03:00
---

# Installer dashboard polling and revised forecast

Notes on how the installer-dashboard gets its forecast numbers, and why we moved it off the old polling loop. This overlaps with [[installer-dashboard-locally-port]], which I did not reopen while writing this, so treat both as partial and reconcile later.

## Where this came from

Installers complained that the installer-dashboard showed a forecast that was visibly stale next to what the customer saw in their own app. The dashboard was polling on a fixed timer, and the timer did not care whether the forecast had changed. Most polls returned the same thing. Some polls landed just before a revision and the installer looked at the old curve for a full interval.

The Julia forecaster revises its output when new weather input arrives and when fresh inverter readings come in through MQTT. Those revisions are not on a schedule the dashboard knows about, which is the root of the mismatch.

## What we decided

Keep polling, but poll for the revision marker first and only fetch the full forecast when the marker moved. The dashboard asks a cheap question: has the forecast for this site been revised since the one I hold. If yes, it pulls the new series. If no, it does nothing and waits for the next tick.

We did not go to a pure push model for the installer-dashboard. Push would be nicer, but it means holding a live channel per open browser tab and the installers tend to leave tabs open all day. Polling the marker is dull and predictable.

## Why not push

Short version, because I argued this at length already and do not want to redo it.

- Azure IoT Hub is the device side of the system. It is not meant to fan out to browsers, and bending it to that would be a hack.
- A separate relay for browsers is another service to run and monitor for a dashboard used by a modest number of people.
- Polling a marker fails quietly and recovers by itself. A dropped push channel fails in ways that are hard to see from the installer's chair.

If the number of simultaneous dashboards grows a lot, revisit this. Until then, polling the marker is fine.

## The polling interval

The interval is the usual dashboard value and it is configurable. I left it alone on purpose. The change is in what each poll does, not how often. The marker check is small enough that the existing interval is fine, and shortening it would only help in the narrow window right after a revision.

There is also a back-off when the backend returns errors: the interval stretches rather than hammering. That behaviour was already there and I kept it.

## Revision marker

The marker is a small value stored alongside the forecast in InfluxDB. It changes whenever the forecaster writes a new series for a site. The dashboard compares the marker it last saw with the current one. Equality means skip.

Two things to remember. The marker is per site, not global, so a dashboard showing many sites checks many markers, and that should be batched into one request instead of one per site. And the marker must be written after the series, never before, otherwise the dashboard can see a new marker and fetch half-written data.

## Svelte side

The polling lives in one store in the Svelte app, not in individual components. Components subscribe and re-render when the store updates. That kept the change small: the components did not need to know that polling became two-step.

One gotcha: when the tab is hidden the browser throttles timers, so on coming back to the tab the dashboard can show an old forecast until the next tick. I added a refresh on visibility change so the installer does not stare at stale data after switching tabs.

## Tariff schedule display

The dashboard also shows the battery charging plan against the time-of-use tariff. That plan is derived from the forecast, so when the forecast is revised the plan can change too. The plan is fetched together with the series when the marker moves, so the two never disagree on screen. Earlier they could, because they were polled separately.

## Open questions

- Should the dashboard show that a revision happened, for example a small badge saying the forecast was updated. Installers might like it, or might find it noisy. Not decided.
- What to do when the marker moves very often for one site, for example when the weather input is flapping. Right now every move triggers a full fetch. A minimum gap between fetches may be needed.
- Whether the customer app should use the same marker approach. It has its own loop and I did not touch it.

## Testing notes

I checked it by forcing a revision in a test site and watching the dashboard. The old behaviour showed the stale curve for up to a full interval; the new one picked it up on the next tick. I did not load test it with many sites open. That is still to do before calling this finished.

Also worth a look: what happens when the marker request succeeds and the series fetch fails. The store should keep the old series and the old marker so the next tick retries. I believe it does, but I have not confirmed it with a deliberate failure.

## Follow-ups

- Batch the marker check across sites if it is not already.
- Add the minimum gap between fetches if flapping shows up in real use.
- Write down the ordering rule for marker and series in the forecaster, so nobody flips it later.
- Reconcile this note with the earlier one on the same dashboard.
