---
id: 01K9SA9GMZ6ZVNM0QYS51XP4NK
created: 2025-11-11T08:19-03:00
---

# battery-controller status summary

This is where battery-controller stands as I understand it, written quickly so the next session does not have to rebuild the picture. It is a general status, not a spec. Where I am unsure I say so. Nothing here fixes a value; the actual settings live in the code and the config, and those win over this note.

battery-controller is the part of GridHaven that decides when a household battery charges and when it discharges. It takes the solar output forecast, the current state of the battery, the household load, and the time-of-use tariff, and turns them into a charging schedule. It does not forecast solar itself. It consumes what the forecasting side produces and acts on it.

## What works today

The core scheduling loop runs end to end in a development setup. It reads a forecast, reads the tariff windows, looks at the battery state, and produces a plan for the coming period. The plan favours charging when energy is cheap or when surplus solar is expected, and favours discharging when the tariff is expensive and the household needs power.

Commands to the battery go out over MQTT, and state comes back the same way. The message flow is steady in normal conditions. Devices at customer sites are managed through Azure IoT Hub, and battery-controller talks to them through that path for provisioning and for the less frequent configuration changes.

Historical readings are stored in InfluxDB. The controller writes its own decisions there next to the measured values, so it is possible to line up what it planned against what the battery did. That has been the main way I check behaviour.

The Julia code for the optimisation is in reasonable shape. It is readable, the main functions are split sensibly, and the scheduling step can be run on its own with recorded input, which makes it easy to try changes without live hardware.

## What is partly done

Handling of tariff changes is only half finished. The common case, a repeating daily pattern with a few windows, works. Seasonal differences and unusual tariff shapes are handled by special cases in places, and I do not trust them yet. A cleaner representation of tariffs would help, and I think that is the right next structural change, but nothing is settled.

Forecast uncertainty is used in a basic way. The controller takes the central forecast and applies a margin to stay safe. It does not yet use the spread of the forecast in any real sense. When the forecast is poor, plans can be too cautious and leave solar energy unused, or too optimistic and leave the battery short in the evening. Both show up in the data from time to time.

The Svelte dashboard shows the current plan and the battery state to installers and customers. It reads from the stored data and is reasonably accurate, but the view of why the controller chose a given plan is thin. Installers have asked for that, and I agree it is the most useful missing piece in the interface.

## Known weak spots

Reconnection after a lost link is the thing I worry about most. When a site drops off and comes back, the controller sometimes carries on from a stale view of the battery for a short while before the fresh state arrives. In practice the plan corrects itself, but I have not proven that it always does, and the stale window has not been bounded by a deliberate design.

Clock handling between the site, the cloud and the controller is another soft area. Schedules are expressed against local tariff time, and daylight saving shifts are the obvious trap. The current behaviour around those shifts has not been tested thoroughly enough for me to say it is right.

Battery limits are read from device configuration, and some devices report them differently or incompletely. The controller falls back to conservative assumptions when information is missing. That is safe, but it hides the fact that the data is incomplete, and it can lower the usefulness of a plan for that site without anyone noticing.

Logging is uneven. Some paths log decisions in detail and others log almost nothing. When something odd happens at a site, reconstructing it from InfluxDB works, but only if the relevant decision was written there.

## Testing

There are tests for the scheduling logic using recorded and constructed inputs. They cover the ordinary daily cycle well. They cover odd tariffs, missing forecasts, and delayed or duplicated messages poorly. The MQTT side is mostly checked by hand against a development broker, not by automated tests, so regressions there would likely be caught late.

There is no solid replay setup that takes a real day from a real site and runs it through the whole controller, including messaging. I would like one, because most of the interesting problems only appear with real data shapes.

## Operations

Deployment is manual enough that I would not call it routine. Configuration changes for a site go through the device management path and are applied reliably, but the process for rolling out a new controller build across many sites is not written down in one place. Someone new would need to ask.

Monitoring covers whether the controller is alive and whether messages are flowing. It does not yet tell us whether the plans are good. A measure of how well planned behaviour matched what happened would be more useful than liveness alone, and the data for it already exists in InfluxDB.

## Open questions

- How should tariffs be represented so that seasonal and irregular cases stop being special cases?
- Should the controller use forecast spread directly, and if so how cautious should it be by default?
- What should the controller do, by design, while its view of a site is stale?
- Who owns the rollout process, and where should it be documented?
- Which device quirks need explicit handling rather than a conservative fallback?

## Suggested next steps

Start with the tariff representation, since several other issues lean on it. After that, build the replay setup so changes can be checked against real days. Then address the stale-state window on reconnection and make the behaviour explicit and tested. Improving the explanation of plans in the dashboard can proceed in parallel, since it mostly depends on data already being stored.

I have not verified every claim above against the current code in this session; treat this as a working impression and check the source before relying on any detail.
