---
id: 01KRXDB64AWKW2DKVK8WBVJHX7
created: 2026-05-18T08:25-03:00
---

# battery-controller design

This note records how battery-controller is put together and why. It is written from the design as it stands, not from a fresh read of the code, so check details against the source before relying on a fine point. The broad shape is stable: battery-controller takes the solar forecast and the tariff schedule, decides when each site's battery should charge or discharge, and sends that decision to the site over MQTT as a charge setpoint.

## Purpose

GridHaven forecasts household solar output and schedules battery charging against time-of-use tariffs. battery-controller is the part that turns a forecast and a tariff table into an instruction a battery can follow. It does not forecast anything itself. It does not talk to the inverter directly. It sits between the forecasting side, which produces expected generation and expected household load, and the device side, which holds the real hardware.

The customer outcome is simple: charge when power is cheap or when surplus solar would otherwise be exported for little, and hold charge for the expensive periods. Installers care that the battery never ends up empty at the start of an evening peak. Customers care about the bill. The controller has to serve both without being clever in ways nobody can explain afterwards.

## Scope and non-goals

In scope: reading forecasts, reading tariffs, reading battery state, computing a schedule, publishing setpoints, recording what it decided and why.

Out of scope: fitting forecast models, managing tariffs as a product feature, firmware on the battery, and the installer-facing screens. The Svelte front end only reads what the controller wrote down. It does not call into the controller to change a decision in flight. If an installer wants a different behaviour, they change site settings and the next planning pass picks them up.

Also not a goal: squeezing out the last fraction of optimality. A plan that is slightly worse than the best one but easy to explain and robust to missing data is preferred.

## Inputs

There are three kinds of input.

Forecasts: expected solar output and expected household consumption for the planning horizon, per site. These come from the forecasting service and are read from the time-series store, InfluxDB, where they are written with a timestamp for the period they cover and a marker for when they were produced. The controller always prefers the most recently produced forecast for a period.

Tariffs: the time-of-use price bands for each site, including import price and export price per band. These change rarely, and the controller caches them per site and refreshes on a schedule and on explicit invalidation.

Battery state: state of charge, current power, and any limits the device reports, such as maximum charge rate and a reserve floor. These arrive over MQTT from the site via Azure IoT Hub and are also stored in InfluxDB for history.

## Outputs

The main output is the charge setpoint. battery-controller publishes charge setpoints as JSON to the MQTT topic `sites/{site_id}/battery/setpoint` with the retain flag off. The site identifier is substituted into the topic, so each site has its own setpoint stream and a device only subscribes to its own.

The secondary output is a decision record written to InfluxDB: the plan that was computed, the inputs it was based on, and the setpoint that was sent. This is what the Svelte views and any later debugging use. Without it, the question of why the battery charged at a given time cannot be answered.

## Setpoint message

The payload is a small JSON object. It carries the target power for the battery, signed so that charging and discharging are distinguishable, a validity window saying how long the setpoint should be honoured, and a timestamp of when the controller issued it. It also carries a plan reference so a device log line can be matched to a decision record. Field names should stay stable once released; adding fields is fine, renaming or repurposing is not, because devices in the field update slowly.

Devices must treat unknown fields as ignorable. The controller does not assume the device has read a message just because it was published; confirmation comes from the state the device reports afterwards.

## Why retain is off

The retain flag is deliberately off on the setpoint topic. A retained setpoint would be delivered to any device that reconnects, however old it was, and an old charge command applied at the wrong hour is worse than no command. With retain off, a device that reconnects waits for the next fresh setpoint, and meanwhile follows its local fallback behaviour.

The cost of this choice is that a device which reconnects just after a publish may wait for the next planning pass before it hears anything. That is accepted. The validity window in the payload backs this up: even a message delivered late is ignored once it has expired.

If someone proposes switching retain on to fix a slow reconnect, the answer in this design is no. Shorten the republish interval instead, or have the device ask for a fresh plan on connect.

## Planning approach

Planning is done in Julia. For each site the controller builds a horizon of consecutive periods, fills in expected solar, expected load, and the tariff band for each period, and then chooses a charge or discharge amount per period.

The choice is a constrained optimisation over the horizon: minimise expected cost, subject to battery capacity, charge and discharge rate limits, and the reserve floor. A linear formulation is enough for the main case, which keeps solve times small and results predictable. Round-trip efficiency losses are modelled as a simple factor rather than a detailed curve.

Only the first period of the plan is sent as the setpoint. The rest is thrown away and recomputed on the next pass. This receding-horizon style means forecast errors do not accumulate in the schedule; they are corrected every pass.

## Handling forecast uncertainty

Forecasts are wrong, especially for cloud. The controller does not try to model the full distribution. It uses the central forecast and then applies a conservative adjustment to the evening reserve: when the next peak band is near and the forecast of remaining solar is uncertain, it keeps more charge than the pure cost minimum would.

The adjustment is a setting, not a hidden constant, and the decision record notes when it affected the result. Installers can change it per site. The default leans toward having charge available because an empty battery at peak is the failure customers notice most.

## Scheduling and timing

The controller runs planning on a regular cadence per site, and also when something material changes: a new forecast lands, a tariff update arrives, or the reported battery state differs from what the last plan expected by more than a tolerance. Passes for different sites are independent and can run concurrently.

Setpoints are republished each pass even if unchanged, so a device that missed a message recovers on its own. This pairs with retain being off: freshness comes from regular publishing, not from the broker remembering.

Clock handling matters. Tariff bands are defined in the site's local time, while storage and messages use a single absolute time base. Conversion happens at the edge of the planner and nowhere else. Daylight saving transitions are a known source of bugs, so periods are built from absolute timestamps and only labelled with local bands afterwards.

## Device connectivity

Sites connect through Azure IoT Hub, and MQTT is the protocol on the wire. The controller side connects as a trusted publisher and does not hold per-device credentials beyond what the hub integration needs. Device identity and authentication stay in the hub; the controller relies on topic scoping so that a site's device sees only its own setpoint stream.

If publishing fails, the controller logs it, records the failure in the decision record, and retries on the next pass rather than building a backlog of queued old setpoints. Stale queued commands are the same hazard as retained ones.

## Storage in InfluxDB

InfluxDB holds forecasts, tariffs as applied, battery state history, and decision records. The controller writes decision records with the site as a tag and the plan details as fields. Tags are kept low-cardinality; per-plan identifiers go in fields, not tags, to avoid blowing up the series count.

Retention is generous for decision records, because support questions arrive weeks after the event. Raw high-rate battery telemetry is downsampled sooner. Queries from the controller itself only ever look at a recent window, so retention policy does not affect planning.

## Failure modes and fallbacks

Missing forecast: use the last good forecast for the period if it is recent enough, otherwise fall back to a simple profile based on typical days for the site, and flag the plan as degraded.

Missing battery state: do not send aggressive setpoints. Hold the last known safe behaviour, usually a neutral setpoint, until state returns.

Tariff unknown or inconsistent: skip optimisation and send a neutral setpoint. Charging against a wrong price table is worse than idling.

Device offline: keep planning so the record exists, keep publishing, and rely on the device's own local fallback when it returns. Devices are expected to have a safe default mode when no valid setpoint is in force.

Controller restart: nothing essential lives only in memory. State is rebuilt from the store and the next pass produces a fresh setpoint.

## Safety limits

The controller clamps every setpoint to the limits the device has reported, and to the site's configured limits when those are tighter. It never asks for discharge below the reserve floor. These checks happen last, after the optimiser, so a bug in the planner cannot push a request outside the physical envelope.

The device is the final authority. If it refuses or clips a setpoint, that is expected behaviour and shows up in the reported state, not as an error in the controller.

## Testing

Planner logic is tested with synthetic days: a clear day, an overcast day, a day with a sharp afternoon peak, and a day with an unexpected load. Each has a known cheap answer that the planner should find. Edge cases get their own tests: battery already full, battery at the floor, a tariff band boundary falling inside a period, and a forecast with a gap.

The message layer is tested against a local broker to confirm topic names, that the retain flag is off, and that payload fields match what devices expect. A test on the retain flag is worth keeping because a library default change could flip it quietly.

## Observability

Every pass logs the site, the plan reference, whether the plan was degraded, and the setpoint chosen. Counters track passes, publish failures, degraded plans, and clamped setpoints. A rise in clamped setpoints usually means the planner and the device disagree about limits, which points at stale device metadata.

A useful check when a customer complains: find the decision record for the time in question, compare the forecast it used against what actually happened, and see whether the setpoint was clamped or the plan degraded. Most complaints end at one of those three.

## Open questions

Whether the devices should be able to request a fresh plan on connect, which would soften the cost of retain being off. Whether the uncertainty adjustment should become learned per site instead of a setting. Whether to model battery degradation cost in the objective; today it is ignored, which slightly favours cycling.

None of these should change the setpoint topic or the retain decision without a deliberate review, since devices in the field depend on both.

## Things to remember

Setpoints go to `sites/{site_id}/battery/setpoint`, JSON, retain off. Only the first period of each plan is sent. Safety clamping comes last. A neutral setpoint is the answer to bad input. When in doubt, keep the evening reserve.
