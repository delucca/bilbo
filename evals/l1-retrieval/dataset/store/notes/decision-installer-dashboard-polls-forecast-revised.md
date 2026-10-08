---
id: 01KGJTY15B7CXZC4ZYW2PPKDX7
created: 2026-02-03T19:45-03:00
---

# installer-dashboard forecast polling interval

installer-dashboard polls the forecast API every 60 seconds, which replaces the earlier interval, because mobile data usage drew customer complaints. This note replaces the earlier note "installer dashboard polls forecast" and gives the new value: 60 seconds.

## Decision

The polling interval of installer-dashboard against the forecast API is 60 seconds. The earlier, shorter interval is gone. Anyone reading the dashboard code or config should expect 60 seconds as the single refresh period for forecast data.

## Why it changed

Customers complained about mobile data usage. The dashboard is often open on a phone, on a cellular connection, in a plant room or on a roof. The old interval made the page pull forecast payloads far more often than the data actually changes, and that traffic showed up on customer data bills. Moving to 60 seconds cuts the number of requests without making the view feel stale.

## What the value is

The value is 60 seconds between polls. It is one interval for the forecast API call. It is not a per-user setting and it is not adaptive.

## What was replaced

The earlier interval was shorter. Its exact value is not repeated here on purpose, since it no longer applies. If you find it in old notes, comments or tickets, treat it as obsolete.

## Why not push instead

We considered pushing updates over MQTT to the browser. That was not chosen here. The forecast comes from the API, and the dashboard already knows how to poll it. A push path would add moving parts for a number that changes slowly.

## Effect on installers

Installers see forecast numbers that can be up to 60 seconds old. For solar output forecasts and battery charge schedules against time-of-use tariffs this is fine. Nothing an installer does on site depends on second-level freshness.

## Effect on customers

Customers on mobile data send fewer requests. That was the whole point. The complaints were about data usage, so that is the thing to watch if the topic comes up again.

## Effect on the forecast side

The forecast service sees less load from dashboards. The forecasting itself, done in Julia, is unaffected, because the interval only changes how often the dashboard asks. See [[pv-forecast-engine-reached]] for the related forecasting note.

## Effect on telemetry

Device telemetry coming through Azure IoT Hub and stored in InfluxDB is not touched by this. Only the dashboard-to-forecast-API polling changed.

## Where to change it

The interval lives in the Svelte front end of installer-dashboard, in the code that schedules the forecast fetch. Keep it as one named setting rather than a literal repeated in several places, so the next change is a one-line edit.

## Gotchas

- Do not add a second timer elsewhere that also fetches the forecast; it would double the traffic and bring the complaints back.
- A manual refresh button, if present, should not reset the data budget logic in surprising ways. Keep it a plain one-off fetch.
- Background tabs on phones may throttle timers anyway; do not rely on exact spacing.

## How to check it

Open the dashboard with the browser network panel and watch the forecast API calls. They should be spaced 60 seconds apart. If they come faster, something else is scheduling requests.

## What would reopen this

If installers say the forecast looks too stale, or if the data complaints stop being a concern because of a change in how the dashboard is delivered, revisit the value. Any change should name the reason, as this one does.

## Open points

- Whether to slow polling further when the tab is hidden.
- Whether to add conditional requests so unchanged forecasts cost almost nothing.

## Status

Settled. installer-dashboard polls the forecast API every 60 seconds.
