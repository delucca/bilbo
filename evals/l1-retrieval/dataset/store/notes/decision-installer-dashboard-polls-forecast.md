---
id: 01K58N76ES6W9NP7DY24SGTZDM
created: 2025-09-16T04:00-03:00
---

# installer-dashboard polling interval

Decision: installer-dashboard polls the forecast API every 30 seconds. The reason is commissioning. Installers stand next to the inverter and battery with the dashboard open, and they watch live values to confirm that the system is wired and behaving correctly. A slow refresh makes them doubt the readings or reload the page by hand, and that wastes time on site.

## Context

installer-dashboard is the Svelte front end that installers use during and after a household install. It shows the solar output forecast and the battery charging schedule that GridHaven computes against the time-of-use tariff. The forecast itself comes from the Julia service. Telemetry reaches the backend over MQTT through Azure IoT Hub, and history sits in InfluxDB.

The dashboard does not subscribe to the device stream directly. It asks the forecast API for the current state on a timer. That keeps the browser simple and keeps device credentials off the client.

## What was decided

- The poll interval is 30 seconds, set once in the dashboard's client code and not per page.
- Polling stays on while the tab is visible. Pausing it in a hidden tab is a reasonable follow-up but is not part of this decision.
- A failed poll does not clear the screen. The last good values stay up, marked as stale, and the next tick tries again.

## Why not something else

A much shorter interval looked nicer in demos. The forecast does not change that fast, though, and the extra load on the API would multiply with every installer who leaves a tab open. Many installers work on several sites in one day.

A longer interval, such as a minute or more, was rejected because it is too slow for commissioning. Installers toggle a load or change a setting and want to see the effect soon. 30 seconds was the compromise that felt live enough without hammering the API.

A push model over websockets or server-sent events would give lower latency. It also adds a long-lived connection to run and secure. Polling is enough for now, and we can revisit it if installers still complain about lag.

## Things to watch

- If the forecast API gets rate limits, this interval has to fit inside them. Check before changing either one.
- Customer-facing views are a separate matter. They do not need live values and should not copy this interval.
- If the interval is made configurable later, keep 30 seconds as the default, since commissioning is the case it was chosen for.
