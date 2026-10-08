---
id: 01K0B1QE30D55834N9REDQTWTV
created: 2025-07-16T22:59-03:00
---

# battery-controller weekly recap

Quiet but useful week on battery-controller. Most time went into reading how the scheduler behaves against tariff windows and cleaning up places where it was guessing. Nothing here is a final call; it is what I would want to know on Monday.

## Where things stand

battery-controller still takes the solar forecast, the tariff schedule and the current state of charge, and produces a charging plan. The plan loop runs and the Julia side is stable enough to work on without constant restarts. The Svelte dashboard shows the plan, though a few labels are still confusing.

## Scheduler work

Spent time on how the planner treats the edges of a cheap tariff window. It sometimes starts charging a little late because it rounds the window inward. I traced the cause but have not changed the behaviour yet. Need to decide whether rounding outward is safe for the installers' customers.

## Forecast inputs

The forecast arrives from the solar side and is read from InfluxDB. When the latest forecast is missing, battery-controller falls back to the previous one without saying so loudly. I added a note to myself to make that fallback visible in logs and in the dashboard.

## MQTT handling

Reviewed how battery-controller subscribes to device topics. Reconnect behaviour looks fine in the happy case. After a longer broker outage the retained state messages can arrive out of order, and the controller may briefly act on an older state. Needs a closer look and a test that replays that sequence.

## Azure IoT Hub

Looked at the command path that sends charge and discharge instructions through Azure IoT Hub. Acknowledgements are handled, but a command with no acknowledgement is retried in a way that could duplicate an instruction. Not confirmed on real hardware yet.

## InfluxDB queries

Some of the queries that read battery history pull more data than the planner needs. Trimming them should make plan runs lighter. I held off because the dashboard shares a few of them and I did not want to break its charts.

## Dashboard

Small Svelte fixes only: clearer wording on the plan panel, and a warning banner when data is stale. The banner logic is simple and probably needs to agree with whatever the controller itself considers stale.

## Testing

Added a few replay-style checks for the planner using recorded tariff and forecast samples. They cover normal days. Cloudy days and tariff changes mid-day are still thin.

## Known problems

- Late start at the edge of cheap windows.
- Silent fallback to an older forecast.
- Possible out-of-order state after broker recovery.
- Possible duplicate commands on retry.

## Open questions

- Should battery-controller refuse to plan when its inputs are stale, or plan conservatively?
- Who owns the staleness rule, the controller or the dashboard?
- Do installers want to override a plan by hand, and how should that interact with the schedule?

## Next week

Write the replay test for the broker outage case first, since it is the riskiest. Then make the forecast fallback visible. After that, return to the window edge behaviour once there is a clearer answer on rounding.

## Notes for whoever picks this up

Keep changes to battery-controller small and test each against recorded data. Do not touch the shared InfluxDB queries without checking the dashboard afterwards. If hardware access is available, confirm the retry behaviour there before trusting the reading of the code.
