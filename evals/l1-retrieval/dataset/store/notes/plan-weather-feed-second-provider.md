---
id: 01KBMTVJ4W74C0JNKZHDJG7YEB
created: 2025-12-04T11:04-03:00
---

# weather-feed-client: second provider fallback plan

The goal is to add a second weather provider to weather-feed-client as a fallback, and to have it done by the end of November 2026. Today weather-feed-client depends on a single upstream source for irradiance, cloud cover and temperature. When that source is slow, wrong or down, the Julia forecasting code gets stale inputs and the battery schedule degrades. The fallback is meant to remove that single point of failure. This note is the plan: what to build, in what order, and what to watch for.

## Why this is needed

Solar forecasts in GridHaven are only as good as the weather inputs. The charging schedule against time-of-use tariffs is built from the forecast. If the weather feed goes quiet, the forecast falls back on old data or on climatology, and installers see charging plans that ignore the actual day. Customers notice this as the battery charging from the grid when the sun would have covered it, or the reverse.

The single provider has had outages and some quiet periods where it returned data that looked valid but was clearly behind. A fallback has to cover both cases: a hard failure and a soft one. The soft one is harder, so the design should not treat a successful HTTP response as proof that the data is good.

The deadline is the end of November 2026. That is before the winter tariff changes that many of our installers' customers will be on, and winter is when the forecast matters most because there is less sun to waste.

## Scope

In scope for weather-feed-client:

- A provider abstraction so the client can talk to more than one source behind one interface.
- A second provider implementation, chosen from the shortlist below.
- A selection policy: primary first, fallback when the primary fails or looks stale.
- Normalisation of the second provider's units, time zones and field names into the shape the rest of GridHaven already expects.
- Metrics and logs that say which provider served each result.
- Tests that run against recorded responses, not live services.

Out of scope for now:

- Blending the two providers into one ensemble forecast. That is a modelling question for the Julia side and should be a separate plan.
- Changes to the Svelte dashboard beyond showing which provider is active, if that turns out to be cheap.
- Changes to the tariff scheduling logic.

## Current shape of the client

Before changing anything, re-read how weather-feed-client is structured today and write down the parts that assume one provider. Likely places: the configuration loader, the request builder, the response parser, the retry and backoff code, and whatever writes results to InfluxDB. Check also whether anything downstream reads provider-specific field names directly. If it does, those reads need to move behind the normalised shape first, otherwise the fallback will silently produce missing values.

Also check how results reach the forecasting code. If the client publishes over MQTT or through Azure IoT Hub, the message format must stay the same whichever provider served the data, with at most one extra field naming the source. Consumers should not need to change for this work.

## Choosing the second provider

Pick the provider on these criteria, in this order:

- Coverage of the regions where our installers operate, at a resolution close to what the primary gives.
- Irradiance data, not just cloud cover. Cloud cover alone forces us to estimate irradiance, which adds error we cannot separate from the model's own error.
- Licence terms that allow caching and storing in InfluxDB, and use in a commercial product used by installers and their customers.
- Rate limits and pricing that fit a fallback role. It will carry little traffic most of the time but must cope with a full load if the primary goes down for a day.
- Stability of the API and a clear deprecation policy.

Shortlist two or three candidates, compare them on a sample of real sites, and write the result up as a short research note so the choice is not lost. Do not pick on price alone. A cheap source with poor irradiance will make the fallback look fine in tests and bad in January.

## Design

Introduce a small provider interface with the operations the client really uses: fetch a forecast for a location and horizon, fetch recent observations if the provider offers them, and report health. Each provider maps its own response into one internal record type with explicit units and UTC timestamps. Do the unit conversion in one place per provider and test it.

The selection policy should be simple and explicit:

- Use the primary by default.
- Switch to the fallback when the primary returns errors past the retry budget, times out, or returns data judged stale.
- Judge staleness from the timestamp of the newest data point against the current time, with a configurable tolerance, not from the fetch time.
- Return to the primary only after it has been healthy for a sustained stretch, to avoid flapping between the two.

Keep the policy in its own module so it can be tested without any network. Make the switch observable: every returned record carries the name of the provider that produced it, and a switch writes a log line and a metric.

Do not merge data from both providers inside one forecast window in this iteration. If the fallback is active, the whole window comes from the fallback. Mixed windows hide discontinuities that the forecasting code is not built to handle.

## Data quality checks

A fallback that serves bad data is worse than none. Add sanity checks that apply to every provider:

- Irradiance must not be negative and must not exceed a physical upper bound for the location and time.
- Night hours should show zero or near-zero irradiance.
- Temperature and cloud cover must fall within plausible ranges.
- Series must be complete across the requested horizon, with no silent gaps.

When a check fails, treat the response as a failure for selection purposes. Log which check failed, without dumping the entire payload. Compare the two providers on the same days during the trial period and record systematic offsets. If the second provider is consistently biased, note it, and consider whether the Julia side needs a correction when the fallback is active.

## Rollout steps

Work in this order so each step can be merged and checked on its own:

- Write up the provider comparison and choose.
- Refactor weather-feed-client behind the provider interface with only the existing provider, and prove nothing changes in output. Compare stored InfluxDB results before and after for the same inputs.
- Add the second provider with recorded-response tests.
- Add the selection policy and its tests, including simulated failures, slow responses and stale data.
- Run both providers side by side in a staging setup, with the second one only logging, not serving. Review the differences.
- Enable the fallback in staging, then force the primary to fail and confirm the switch and the return.
- Enable in production for a small group of installers first, then for everyone.

Leave enough room before the end of November 2026 for the side-by-side period. It needs real weather days, and one calm week tells us little. If the schedule slips, cut the dashboard work and the extra metrics polish first, and keep the tests and the staleness check.

## Risks and open questions

- Credentials and quota for the second provider need to be stored the same way as the first, through the existing secrets handling. Do not put keys in the repository or in logs.
- Different providers define time steps and interval timestamps differently, for example whether a value refers to the start or end of an hour. A mismatch shifts the solar curve and is easy to miss. Check this explicitly with a clear-day example.
- Rate limits could be hit if many sites fail over at once. Cache by location area where the data allows it and spread requests out.
- The primary may recover in a degraded state. The return-to-primary rule must be conservative.
- Unclear who owns the provider contract and billing on the business side. Settle this early, since a delay there would block the whole plan.
- Open question: should the installer-facing app show the active provider? Probably only in a diagnostics view.

## Done means

The work is done when weather-feed-client can serve forecasts from either provider through one interface, switches to the second on failure or staleness and returns to the first when it is stable, tags every record with its source, passes the tests on recorded responses, and has run in staging against a forced primary failure without a gap in the data reaching the forecasting code. The target date for all of this is the end of November 2026. Update this note when the provider is chosen and when each rollout step lands, and move the decisions into their own notes as they are made.
