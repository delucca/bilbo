---
id: 01K66K34PW2EFCR4MJ9S0M030H
created: 2025-09-27T19:01-03:00
---

# weather-feed-client: overall structure

weather-feed-client is the piece of GridHaven that pulls outside weather data and hands it to the solar forecasting side. It is a Julia process. It does not forecast anything itself. Its job is to fetch, clean, store and republish weather inputs so the forecaster and the battery scheduler can read them from one place. This note is a map of how it is laid out, written quickly so the next session does not have to rediscover it.

## Purpose and boundaries

The component sits between external weather providers and the rest of the system. On one side are HTTP style provider APIs. On the other side are InfluxDB, where history is kept, and MQTT, where fresh values are announced. It knows nothing about tariffs or battery state. It also does not know about individual customer devices beyond a site location it is given.

Things it owns: provider access, normalisation, caching, writing weather series, announcing updates. Things it does not own: forecast models, scheduling, device telemetry from the inverters, user-facing screens in the Svelte app.

## Main pieces

The code splits into a few layers, each fairly small.

- Provider adapters: one per upstream source, each knowing the request shape and the response shape of that source.
- Normaliser: turns provider specific fields into one internal record type with consistent units and timestamps.
- Scheduler loop: decides when each site is due for a refresh.
- Store writer: writes normalised records into InfluxDB.
- Publisher: sends a short notice over MQTT when new data has landed.
- Config and site registry: tells the others which sites exist and where they are.

Keep these separate. Most past confusion came from provider quirks leaking into the writer.

## Provider adapters

Each adapter exposes the same small interface: given a location and a time window, return raw observations or forecast rows. Authentication, paging and retry live inside the adapter. Adapters do not write to the database and do not publish. If a provider changes its format, only its adapter and the matching normaliser mapping should need touching.

There is a notion of a primary and a fallback provider. When the primary fails or returns something clearly empty, the scheduler can ask the fallback. Which is which comes from config, not from code.

## Normalisation

Raw rows become one internal record with a site key, a valid time, an issue time, a kind (observation or forecast), and a set of fields such as irradiance, cloud cover, temperature and wind. Units are converted here and nowhere else. Missing fields stay missing rather than being filled with zeros, because the forecaster treats a gap differently from a zero. Timestamps are held in UTC internally.

Forecast rows keep their issue time so that several issues for the same valid time can coexist. That matters when comparing how forecasts drifted.

## Scheduling and refresh

A loop walks the site registry and works out which sites are due. Observations and forecasts have separate cadences. Sites that share a coarse grid cell from a provider can share one fetch, which cuts calls. The loop is meant to be tolerant: one slow or failing site must not stall the others, so fetches run concurrently with per-call timeouts and a bounded backoff on repeated failure.

Rate limits of providers are respected inside the adapter, but the scheduler also spreads work out so bursts are avoided.

## Storage in InfluxDB

Normalised records are written as points with the site as a tag and the weather fields as fields. Observations and forecasts go to separate measurements so queries stay simple. Writes are batched. Retention is handled on the database side and not by this component. The writer is idempotent for repeated points with the same identity, so a retry after a timeout does not duplicate rows.

## MQTT publishing

After a successful write the publisher emits a small message saying that a site has new weather data and of what kind. The message does not carry the full payload; consumers read from InfluxDB. This keeps messages tiny and avoids two sources of truth. The topic layout is per site and per kind. Messages go through the same broker setup that the rest of GridHaven uses, with the Azure IoT Hub bridge handling anything that must reach the cloud side.

## Configuration and secrets

Config is file based with environment overrides. It lists providers, the site registry source, cadences, timeouts and connection settings. Provider keys and database tokens come from the environment or a secret store, never from the repo. The site registry may be read from a file or from a service; the loader hides which.

## Failure behaviour

When every provider fails for a site, the component keeps serving the last stored data and marks the gap by simply not writing. It does not invent values. Consumers are expected to check the age of the latest point. Errors are logged with the site and provider named, and counters are kept for fetch success, failure and write latency so a dashboard can show staleness.

## Open questions and things to watch

- Whether the fallback provider should be queried in parallel instead of only after failure.
- How much of the shared grid cell logic belongs in the scheduler versus a separate module.
- Provider time zones and daylight saving edges have caused trouble before; keep all conversions in the normaliser.
- Tests mostly use recorded provider responses; new adapters should add the same kind of fixture.
