---
id: 01KKKX91YS570TZWKN24GCK5PY
created: 2026-03-13T12:33-03:00
---

# weather-feed-client design

This note replaces the earlier note "weather feed client caches"; the cache lifetime is now a TTL of 600 seconds for responses held by weather-feed-client, and the older value should be ignored wherever it still shows up.

weather-feed-client is the piece of GridHaven that fetches weather data from outside providers and hands it to the forecasting side. The solar output forecast is written in Julia and needs irradiance, cloud cover and temperature for each installation site. The battery scheduler needs the same forecast to decide when to charge against the time-of-use tariff. Neither of them should talk to a weather provider directly. They go through weather-feed-client, which owns the provider details, the caching and the failure handling.

## Purpose and scope

The client does three things: it requests weather data for a site, it keeps recent responses so the same question is not asked again, and it normalises the result into one internal shape. It does not forecast anything, it does not schedule anything, and it does not store history. History belongs in InfluxDB, written by the ingestion path, not by this client. If a change needs the client to understand panels, batteries or tariffs, the change is in the wrong place.

## Cache lifetime

Responses are cached with a TTL of 600 seconds. That replaces the earlier cache lifetime. The value applies to every kind of weather response the client holds, not only to one provider or one endpoint. The TTL counts from the moment the response was received by weather-feed-client, not from the timestamp the provider puts on the data. This matters because some providers publish data that is already a little old when we get it, and counting from the provider timestamp would make entries expire too early or inconsistently.

Once an entry is older than 600 seconds it is treated as missing for normal reads. A fresh fetch is attempted. If that fetch fails, see the section on stale data below.

## Why the lifetime was changed

Weather conditions that matter for rooftop solar, mainly cloud movement, change on a scale that makes a short cache worthwhile and a long one risky. The earlier lifetime was out of step with how often the forecast job and the scheduler actually ask. With 600 seconds, several consumers asking about the same site in a short window share one upstream call, and the data is still recent enough that a charging decision is not made on a clearly outdated sky. Provider rate limits were also part of the reasoning: fewer repeated calls means less pressure on quotas shared across many customer sites.

## Cache key

Entries are keyed by the site location and the kind of data requested. Locations are rounded to a coarse grid before they become part of the key, so two homes on the same street share one entry instead of causing two upstream calls. The rounding is deliberate and is the main reason the cache helps installers who manage many nearby systems. If a provider ever returns data that is meaningfully finer than the grid, the key needs revisiting, but nobody has needed that yet.

## Where the cache lives

The cache is in memory inside the process that runs weather-feed-client. It is not shared between processes and it is not persisted. A restart empties it, and the first requests after a restart all go upstream. We accepted that because the client starts rarely and a cold cache costs only a short burst of calls. A shared cache was considered and rejected for now; it adds a dependency and a new failure mode for a gain we cannot yet measure.

## Stale data and failures

When a refresh fails and an expired entry exists, the client may return the expired entry, but it must mark it as stale in the normalised result. Consumers decide what to do with stale data. The forecast job normally accepts it and widens its uncertainty; the scheduler prefers to fall back to a conservative charging plan when the data is stale for long. The client never silently presents an expired entry as fresh. Serving stale data does not extend the TTL of 600 seconds: the entry stays expired, and the next read tries upstream again.

## Provider handling

Several providers can sit behind the client. Each one has an adapter that deals with authentication, request shape, units and field names. The adapter output is converted to the internal shape before it reaches the cache, so the cache holds one format only. A provider switch is therefore an adapter change plus configuration, with no change in the consumers. Credentials for providers are kept in the deployment secrets, never in the repository and never in the cache.

## Interaction with MQTT and Azure IoT Hub

Device telemetry from the inverters and batteries arrives through Azure IoT Hub and MQTT, on a separate path from weather. The weather-feed-client does not subscribe to device topics. When the forecast job runs it joins the two sources by site and time. Keeping them apart means a weather outage does not stop telemetry ingestion, and a telemetry backlog does not slow weather fetches. If someone proposes pushing weather over MQTT to devices, remember that devices only need the resulting schedule, not the raw weather.

## Interaction with InfluxDB

The client does not write to InfluxDB itself. The forecast job writes forecasts and the inputs it used, so a later review can see which weather data a decision was based on. When debugging a bad forecast, check whether the weather input was stale at the time. The stale marker is carried through so that it can be stored next to the forecast. The cache lifetime is not stored; only the result is.

## Interaction with the Svelte front end

The installer and customer dashboards written in Svelte never call weather-feed-client. They read forecasts and schedules from the backend. If a dashboard shows a weather summary, it comes from the stored forecast inputs, which can be a little older than the live cache. This is fine for display. Do not add a direct browser path to the client to make the dashboard feel fresher; it would bypass the cache and spend provider quota.

## Testing notes

Tests for the cache should use a controllable clock rather than waiting in real time. The behaviours worth covering are: a read inside the TTL does not call upstream, a read after the TTL does, a failed refresh returns a stale-marked entry, and a successful refresh after a failure clears the mark. A test that hard-codes the old lifetime is out of date and should be changed to use the current value of 600 seconds, ideally through the configuration constant instead of a literal.

## Open questions

Whether the lifetime should differ by data kind is not settled. Current observations, such as cloud cover, change faster than slowly varying values, and a single TTL is a compromise. Whether to add a shared cache across processes is also open. Neither is urgent. If either gets decided, update this note rather than adding a second one, so there is one place that says what the cache lifetime is and why.
