---
id: 01K7YT6V3151MFRDRPJMNXHAPB
created: 2025-10-19T15:02-03:00
sources:
  - "code: src/cache.jl"
---

# weather-feed-client design

weather-feed-client is the piece of GridHaven that fetches weather data for the solar forecast and hands it to the rest of the system. It sits between an external weather provider and the Julia forecasting code. This note records how it is put together, why the cache looks the way it does, and what to watch when changing it. It is written quickly, from the design as it stands, so treat small details as things to check in the code.

The short version: weather-feed-client asks the provider for irradiance, cloud cover, temperature and similar fields for each installation site, keeps responses on disk, and serves repeat requests from that cache until the entry expires. The disk cache lives under /var/cache/gridhaven/weather and entries have a TTL of 900 seconds. Everything else in the design follows from wanting few outbound calls, predictable behaviour when the provider is down, and forecasts that can be reproduced later.

## Purpose and scope

GridHaven forecasts household solar output and schedules battery charging against time-of-use tariffs. The forecast is only as good as the weather input. The installers and their customers see the result in the Svelte front end, but they never talk to the weather provider. All weather access goes through weather-feed-client.

What the component does:

- Takes a site location and a time window from the caller.
- Returns normalized weather records for that window, current conditions and a short-range outlook.
- Hides provider quirks: units, field names, missing values, pagination.
- Caches on disk so that several callers asking about the same site do not each cause an outbound call.
- Reports its own health, such as cache hit rate and provider errors, so that it shows up next to the other service metrics.

What it does not do:

- It does not run the solar model. That is Julia code downstream, which treats weather-feed-client as a source of records.
- It does not decide when the battery charges. The scheduler consumes the forecast, not the raw weather.
- It does not store long-term history. Long-term weather and forecast series go to InfluxDB through the normal ingestion path, not through the cache.
- It does not talk to devices. Device telemetry arrives over MQTT and through Azure IoT Hub, which is a separate path.

## Disk cache

Responses are cached on disk under /var/cache/gridhaven/weather with a TTL of 900 seconds. That is the one fixed number in the caching design, and both the directory and the TTL should be read from configuration with those values as defaults, not hard-coded at call sites.

Why on disk and not only in memory: the service restarts during deploys and when the host is rebooted, and a cold in-memory cache after a restart sends a burst of calls to the provider for every site at once. A disk cache survives restarts, so the first requests after a restart are usually hits. It also lets more than one process on the same host share the entries.

Why 900 seconds: the provider updates its current-conditions data on a cadence of that order, so asking more often returns the same answer and asking less often makes the forecast react late to a cloud front. It is a compromise between freshness and call volume. If the provider changes its update cadence, this is the number to revisit, and the reasoning should be updated here along with it.

How entries behave:

- An entry is keyed by the site and the kind of data requested, plus the requested window where that matters. Two requests that differ only in irrelevant parameters should land on the same key, so the key is built from normalized values, not from the raw query string.
- An entry is fresh while its age is below the TTL. After that it is expired and a lookup is a miss.
- Writes go to a temporary file in the same directory and are then renamed into place, so a reader never sees a half-written file.
- A file that cannot be parsed is treated as a miss and replaced, never as an error that reaches the caller.
- The directory is owned by the service account. Nothing else should write there, and nothing outside the service should depend on the layout of the files.

Cleanup: expired entries are not deleted at the moment they expire, because they are useful as a fallback (see the failure section). A periodic sweep removes entries that are far past expiry, so the directory does not grow without limit as sites come and go. The sweep should never be on the request path.

## Request flow

The normal path for a request:

- The caller asks for weather for a site and window.
- The client builds the cache key and looks on disk.
- A fresh entry is returned immediately, marked as coming from cache.
- On a miss, the client makes one outbound call to the provider, validates and normalizes the response, writes it to the cache, and returns it.
- If several callers ask for the same key at nearly the same moment, only one outbound call is made and the others wait for its result. Without this, a burst of forecast jobs at the start of a scheduling cycle would multiply provider traffic.

Normalization is where most of the code is. Providers disagree on units, on whether times are local or universal, on how they report missing readings, and on how coarse their grid is relative to a house. The client converts everything to one internal representation before it is cached, so the cache holds normalized data and not provider payloads. This means a change in normalization needs the cache cleared or versioned; otherwise old entries with the old shape would be served until they expire. Including a schema version in the cache key is the cleanest fix and is the preferred approach.

The returned records carry the time they were fetched and whether they came from cache. Downstream code needs this: the forecast should record which weather it used, so that a forecast can be explained after the fact.

## Failure handling

The provider will be slow or down sometimes. The client has to degrade without taking the forecast down with it.

- Timeouts are short and bounded. A slow provider must not hold up a scheduling run for long.
- Retries are limited and use backoff with some randomness, so many sites do not retry in lockstep.
- When the provider fails and an expired entry exists for the key, the client may return the stale entry, clearly marked as stale. For a solar forecast, weather that is a little old is far better than no weather. How stale is too stale is a policy decision for the forecasting side; the client exposes the age and lets the caller decide.
- When there is no entry at all, the client returns an explicit failure. It does not invent values and it does not return zeros, since a zero irradiance reads as a valid and very pessimistic forecast.
- Rate limit responses from the provider are treated differently from outages: the client backs off for the period the provider asks for and serves from cache meanwhile.
- Malformed responses count as failures and are not cached. Only validated data goes to disk.

Repeated failures should be visible. The client counts provider errors, stale serves and hard failures, and these are what an on-call person looks at first when forecasts look odd.

## How it fits with the rest of GridHaven

Forecasting jobs written in Julia call weather-feed-client for the weather inputs of each site, run the solar output model, and write the forecast onward. The scheduler then plans battery charging against the time-of-use tariff using that forecast. Results are stored in InfluxDB and shown in the Svelte dashboard for installers and customers.

Points worth keeping in mind:

- Device telemetry from the installed systems comes in over MQTT and through Azure IoT Hub. That data is used to compare actual output with forecast output. It is independent of the weather feed, so a weather outage does not blind the system to what the panels are really doing.
- When weather is stale or missing, the forecast should say so, and the scheduler should lean on conservative assumptions, not on the last good optimistic forecast.
- Many sites near each other can share weather data if the provider grid is coarser than the distance between them. Whether to key the cache by site or by grid cell is a possible optimization. Keying by site is simpler and is what the design assumes now; keying by cell would cut calls further but couples sites together in ways that complicate invalidation.
- The cache is per host. If the service is scaled to several hosts, each has its own cache, and total provider traffic grows with host count. A shared cache would fix that but is not part of this design.

## Operational notes

Things a person running this should know:

- The cache directory must exist and be writable by the service account. If it is not writable, the client should still work, just without caching, and it should log loudly. A silent fallback hides a large increase in provider calls.
- Disk usage is small per entry but scales with the number of sites. Watch it if the number of installations grows a lot.
- Clearing the cache is safe at any time. The cost is a burst of outbound calls afterwards, so do it outside busy scheduling periods when possible.
- Clock changes on the host affect freshness, since age is computed from timestamps. A host with a badly wrong clock will see entries as too old or too young. Keep time sync healthy.
- Provider credentials come from configuration and secrets handling, not from files in the cache directory or the repository.
- When debugging a strange forecast, first check whether the weather came from cache or from a fresh call, and how old it was. The records carry that information.

## Open questions and possible changes

- Whether the TTL should vary by data kind. The outlook changes more slowly than current conditions, so a longer lifetime may be fine there, while current conditions stay at 900 seconds. Nothing is decided.
- Whether stale serving should have a hard upper age, enforced in the client and not left to callers. Right now it is the caller's call, which risks inconsistent policies between jobs.
- Whether to add a second provider as a fallback. It would improve availability but doubles the normalization work and raises the question of which source wins when both answer.
- Whether to push a copy of every fetched record into InfluxDB so that past forecasts can be replayed against exactly the weather they used. This is attractive for debugging and for model evaluation, and it is the main reason the records already carry fetch time and origin.
- Whether the cache layout should be versioned on disk, with a marker file in the directory, so that a rollback to an older release does not read entries written by a newer one.

If any of these get settled, update this note and say what changed and why, rather than adding a new note on the same component.
