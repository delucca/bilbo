---
id: 01KKZN6K1GCY40GKTCV2MVF6EZ
created: 2026-03-18T02:02-03:00
---

# pv-forecast-engine forecast horizon spec

This note replaces the earlier note about "pv forecast engine must", which set the old horizon requirement. The new value is a 72-hour forecast horizon for every registered site. Anything that still quotes the earlier horizon is out of date.

In this note `pvfe` is short for `pv-forecast-engine`. Both names show up in logs, dashboards and chat, so treat them as the same component. In prose and in new docs, prefer the full name `pv-forecast-engine`.

## Requirement

`pv-forecast-engine` must produce a 72-hour forecast horizon for every registered site. This replaces the earlier horizon requirement. The horizon is not optional per site and not a premium feature: if a site is registered, it gets the full 72-hour forecast.

A site that cannot be forecast for the full horizon should be reported as failing the requirement. It should not quietly get a shorter forecast that looks normal downstream.

## Why the horizon changed

The battery scheduler plans charging against time-of-use tariffs. Tariff windows and weather patterns often span several days, and a short horizon forced the scheduler to guess about the later part of the plan. A longer horizon lets it decide earlier whether to charge from the grid or wait for solar. Installers also wanted to show customers a multi-day outlook in the Svelte front end.

## Scope of the component

The engine is written in Julia. It reads recent site measurements from InfluxDB, combines them with weather inputs and site configuration, and writes forecast series back for the scheduler and the UI to read. Device telemetry reaches the platform over MQTT and through Azure IoT Hub, but the engine does not talk to devices. It only consumes stored data.

Forecast output covers the whole 72-hour span in the same time resolution the scheduler already expects. Do not change the resolution as part of this work.

## What changes for consumers

- The scheduler must accept forecasts with the longer span and must not truncate them silently.
- The Svelte charts need to handle the longer series without cramming the axis. Zoom or scroll is fine.
- Anything that stores or caches forecasts needs capacity for the longer series per site.
- Alerts or checks that compared forecast length against the old horizon must use the new one.

## Quality expectations

Accuracy naturally drops toward the end of a long horizon. That is expected and not a bug. The requirement is about coverage, meaning every registered site gets values for the full span. It does not promise equal accuracy across the span.

Where weather inputs are weaker late in the span, the engine should fall back to a simpler seasonal estimate for that part and mark the values as lower confidence, rather than leave gaps.

## Open points

- How run time scales with the number of registered sites at the longer horizon has not been measured. Check it before rolling out to large installers.
- Decide whether confidence markers are exposed in the UI or only in the data.
- Confirm that old forecast data written under the earlier horizon is handled cleanly when it sits next to new data in the same series.

## Naming reminder

Use `pv-forecast-engine` for the component. Use `pvfe` only as a short form where space is tight, such as log prefixes or metric names. If a document states the earlier horizon, update it to the 72-hour figure and link here.
