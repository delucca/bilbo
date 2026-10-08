---
id: 01JRZ509P0XWE1D2E1JR6J3D78
created: 2025-04-16T08:16-03:00
---

# battery-controller glossary

Short glossary of words that come up around battery-controller in GridHaven. Written quickly so a new session does not have to guess what people mean. It is general on purpose: it names concepts, not settings. Where a term has a real value in config or code, look there; this note does not carry it.

battery-controller is the part of GridHaven that decides when a home battery charges and discharges. It takes the solar forecast, the household load estimate and the tariff schedule, and produces a charging plan. The plan is then pushed to the site over the messaging path. Most of the planning logic is written in Julia. Telemetry lives in InfluxDB, device traffic goes over MQTT, and the cloud side of device management goes through Azure IoT Hub. The installer and customer views are in Svelte.

## Energy and battery terms

**State of charge (SoC).** How full the battery is, as a fraction of usable capacity. The controller reasons in SoC, not in raw energy, most of the time. Be careful: some devices report SoC against nameplate capacity and others against usable capacity. Check which one a given source means before comparing two readings.

**Usable capacity vs nameplate capacity.** Nameplate is what is printed on the unit. Usable is what the controller is allowed to draw on after the manufacturer's reserve. Planning should use usable.

**Reserve.** A slice of SoC the controller keeps back, either for backup during an outage or to protect the cells. There can be a hard floor set by the battery's own management system and a softer floor set by the installer or customer. The controller never plans below the hard floor.

**Charge rate / discharge rate.** How fast energy can move in or out. Limited by the inverter, the battery and sometimes the grid connection. The planner treats these as ceilings, and real behavior is usually lower, especially when the battery is cold or nearly full.

**Round-trip efficiency.** The share of energy put in that comes back out. The planner uses it so it does not chase tiny tariff gaps that are eaten by losses.

**Degradation cost.** A rough cost attached to each cycle so the planner does not cycle the battery for marginal savings. It is a tuning idea, not a measured fact about a particular battery.

**Cycle.** One full charge and discharge worth of energy throughput. Partial movements add up to cycles.

**Self-consumption.** Using solar energy in the home, directly or via the battery, instead of exporting it. One of the two main goals the plan can serve.

**Export / import.** Energy sent to the grid versus drawn from it. Tariffs can price these differently, and sometimes export earns nothing.

**Curtailment.** Solar output deliberately reduced because there is nowhere useful for it to go, for example the battery is full and export is limited.

## Tariff terms

**Time-of-use (TOU) tariff.** A price schedule that changes by time of day, and often by day type or season. This is the main input that makes scheduling worthwhile.

**Peak / off-peak / shoulder.** Names for tariff bands. The controller does not care about the names; it cares about the price per interval. Different retailers use different labels for similar things, so the tariff loader maps them to a common shape.

**Tariff window.** A contiguous stretch of time where one price applies. Plans are built by comparing windows.

**Arbitrage.** Charging when energy is cheap and discharging when it is dear. Only pays if the price gap beats losses and the degradation cost.

**Demand charge.** A fee based on the highest draw in some period rather than on total energy. Not every customer has one. When present, it changes what a good plan looks like, because shaving a single spike matters more than total energy moved.

**Feed-in tariff.** What the customer is paid for exported energy.

## Forecast and planning terms

**Solar forecast.** Predicted household PV output over the planning horizon, built from weather inputs and site characteristics such as panel orientation and tilt. Forecast quality is the biggest source of plan error on cloudy days.

**Load forecast / load estimate.** Predicted household consumption. Usually derived from past telemetry in InfluxDB, grouped by time of day and day type.

**Net load.** Load minus solar. Positive means the home needs energy, negative means there is surplus.

**Horizon.** How far ahead the planner looks. The plan covers the horizon, but only the near part is acted on before the next replan.

**Interval / slot.** The time step the planner works in. Forecasts, tariffs and the plan are all resampled onto the same slots. Mismatched slot lengths between sources were a recurring source of off-by-one bugs, so resampling is done in one place.

**Plan / schedule.** The output: a sequence of target actions per slot, such as charge, hold, discharge or follow load. The word schedule is also used for the tariff, so say which one you mean.

**Replan.** Recomputing the plan when new information arrives, such as a fresh forecast, a changed tariff, or telemetry that diverges from what was expected. Replanning is normal and frequent.

**Set point.** A target the site device is asked to meet in a given slot, like a power level or a mode. The plan is turned into set points before it is sent.

**Mode.** The operating state of the battery or inverter, for example self-consumption, forced charge, forced discharge, or idle. Vendors name modes differently; the controller keeps its own small set and translates at the edge.

**Optimizer.** The Julia code that solves for the plan. It is a cost minimization over the horizon subject to the battery limits above. If someone says solver, they usually mean the same thing or the underlying library it calls.

**Fallback plan.** A simple safe behavior used when the forecast or tariff is missing or stale. Typically self-consumption with the reserve respected. It exists so a data outage does not leave the battery doing something odd.

## Messaging and device terms

**Site.** One household installation: PV, inverter, battery, meter. The controller plans per site.

**Gateway.** The small device at the site that talks to the inverter and battery and to the broker. Sometimes people say edge device; here it means the same thing.

**Broker.** The MQTT broker that carries messages between the controller and gateways.

**Topic.** An MQTT address messages are published to. Topics are organized per site and per message type. Do not hardcode topic strings in new code; use the shared helpers.

**Telemetry.** Measurements flowing up from the site: power, SoC, temperatures, status. Stored in InfluxDB.

**Command.** A message flowing down to the site asking it to change mode or set point. Commands should be safe to receive twice.

**Acknowledgement (ack).** The gateway's reply that it received and applied a command. A missing ack does not prove the command failed; compare with telemetry before assuming.

**Retained message.** An MQTT message the broker keeps and hands to new subscribers. Useful for last known state, risky for commands, since a stale retained command can be replayed on reconnect.

**QoS.** MQTT delivery guarantee level. Commands and telemetry may use different levels, chosen per message type.

**Device twin.** The cloud-side record of a device in Azure IoT Hub, with desired and reported properties. Used for configuration and for knowing whether a gateway is online, as opposed to the live data path over MQTT.

**Desired / reported properties.** The two halves of a twin. Desired is what the cloud wants, reported is what the device says it has. When they differ, the device has not caught up yet.

**Heartbeat.** A periodic message showing the gateway is alive. Absence for long enough marks the site as offline and triggers the fallback behavior.

## Data terms

**Measurement.** An InfluxDB table-like grouping of points, such as battery power or SoC.

**Tag vs field.** In InfluxDB, tags are indexed labels like site, fields are the values. Putting a high-variety value in a tag causes trouble, so keep site and device in tags and readings in fields.

**Downsampling.** Reducing raw telemetry to coarser intervals for long-term storage and for the load estimate.

**Backfill.** Writing late or missing data into the past after a gateway reconnects. Planning should tolerate backfilled points arriving out of order.

**Drift.** The gap between what the plan expected and what telemetry shows, for example SoC lower than predicted. Persistent drift is the usual trigger for a closer look at efficiency or capacity assumptions.

## Ways people use the words loosely

Plan and schedule get mixed up; the tariff has a schedule, the controller has a plan. Mode and set point get mixed up; a mode is a state, a set point is a target within it. Offline can mean the gateway is down, the broker link is down, or the twin shows disconnected, and these are not the same failure. When a report says the battery did not follow the plan, first ask which of those layers is meant before digging into the optimizer.

## Where to look

Planning logic is in the Julia side of battery-controller. Message shapes are shared with the gateway side, so a change to a command needs both ends checked. Tariff mapping and forecast resampling each live in one place; extend those rather than adding a second copy. If a term here turns out to be used differently in the code, fix this note.
