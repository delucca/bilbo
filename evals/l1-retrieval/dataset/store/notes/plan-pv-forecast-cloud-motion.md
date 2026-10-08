---
id: 01K43NG49BYNV1G7ZQXMDDXVPV
created: 2025-09-01T19:13-03:00
---

# pv-forecast-engine: cloud-motion nowcast plan

Plan to add a cloud-motion nowcast step to pv-forecast-engine, and to land it before the release tagged `v2.4.0`. The engine is also known internally by its codename `sunseer`, so if you see that name in a repo, a dashboard, a log line or a chat thread, it is the same thing as pv-forecast-engine. This note uses pv-forecast-engine throughout.

## Why this step

The engine forecasts household solar output and feeds the battery scheduler, which plans charging against time-of-use tariffs. Today the short-horizon part of the forecast leans on clear-sky models and a coarse weather feed. When clouds move through quickly, the first hour or two of the forecast is wrong in ways that cost customers money: the scheduler either charges from the grid when the sun would have covered it, or it waits for sun that does not come.

A nowcast step corrects that short horizon using what is actually happening over the site right now. The idea is to estimate how clouds are moving and project that motion forward a short time, then scale the clear-sky output accordingly. Longer horizons keep using the existing path unchanged.

## Scope

In scope:

- A new nowcast step inside pv-forecast-engine, written in Julia like the rest of the engine.
- Blending the nowcast output with the existing forecast over the first part of the horizon, fading to the old forecast as the horizon grows.
- A switch to turn the step off per site, so an installer can fall back to the old behaviour.
- Metrics so we can compare nowcast against the old forecast on real sites.

Out of scope for this plan:

- Changes to the battery scheduler itself. It should just consume a better short-horizon number.
- Any new hardware at customer sites.
- Changes to the Svelte installer dashboard beyond showing whether the nowcast is on.

## Inputs to decide

Open questions, to settle before coding starts:

- Which signal drives cloud motion: recent inverter output history from the site, a sky or satellite imagery source, or both. Inverter output is already in InfluxDB and needs nothing new, so start there unless it proves too noisy.
- How readings arrive. Device telemetry comes in over MQTT through Azure IoT Hub and lands in InfluxDB. The nowcast should read from InfluxDB, not subscribe to MQTT directly, to keep the engine's data access in one place.
- How far ahead the nowcast is trusted before it hands over to the existing forecast. Pick this from backtests, not by guess.

## Steps

1. Write down the current short-horizon error on a set of historical sites, so there is a baseline to beat. Use days with fast-moving cloud as a separate slice.
2. Build the motion estimate from recent site data and check it offline against stored history.
3. Implement the nowcast as its own step in the forecast pipeline, behind the per-site switch, defaulting to off.
4. Add the blend with the existing forecast and test the hand-over at the edge of the nowcast horizon. A visible jump there is a bug.
5. Add metrics and write them to InfluxDB so dashboards can compare old and new.
6. Roll out to a small group of installers first, with the switch on, and watch the error metrics for a couple of weeks.
7. If the metrics hold, turn it on by default in the release tagged `v2.4.0`.

## Timeline and release

The hard constraint is the release tagged `v2.4.0`. The nowcast step has to be merged and verified before that tag is cut. If it is not ready, the step ships off by default, or does not ship, and the tag is not held for it. Nothing in the rest of the release should depend on this work.

Rough order: baseline and offline estimate first, since they tell us whether the approach is worth continuing. Pipeline integration second. Pilot with installers last, because it needs real calendar time.

## Risks

- Noisy inverter data on small systems may make the motion estimate unreliable. Mitigation: fall back to the existing forecast when confidence is low.
- Missing or late telemetry from IoT Hub. The step must degrade to the old forecast, never fail the whole forecast run.
- Overfitting to the pilot sites. Compare on sites that were not used when tuning.
- Runtime cost. The forecast runs for many sites; the step has to stay cheap in Julia, so avoid allocations in the inner loop and measure before optimizing.

## Checking it worked

Success is lower short-horizon error than the baseline on the cloudy-day slice, with no worse error on clear days, and no increase in grid-charging that turned out to be unnecessary. Check these from the metrics in InfluxDB. If the nowcast is not clearly better, leave it off by default and write down why.

```text
component: pv-forecast-engine
codename:  sunseer
target:    v2.4.0
step:      cloud-motion nowcast
default:   off until pilot metrics hold
```
