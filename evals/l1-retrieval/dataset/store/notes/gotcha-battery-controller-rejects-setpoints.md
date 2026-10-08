---
id: 01KGY0JF7DWPGBA8B7P7GPS9TQ
created: 2026-02-08T03:56-03:00
---

# battery-controller rejects stale setpoints (ERR_STALE_SETPOINT)

battery-controller refuses any setpoint whose message timestamp is older than 90 seconds. It answers with `ERR_STALE_SETPOINT` and does not apply the command. The usual cause is clock drift on the device that produced or relayed the message, not a slow network. The scheduler looks healthy, the forecast looks healthy, and the battery just does not do what the plan says. This note is what to check first when that happens.

In the tooling and in chat, `battctl` is short for `battery-controller`. If someone says battctl rejected a setpoint, they mean the same component and the same error. Search for either name when looking through logs or old notes.

## Symptom

The visible effect is that the battery sits idle, or keeps its previous mode, while the schedule says it should be charging against a cheap tariff window. Nothing crashes. The Julia scheduler publishes a plan, MQTT delivers it, and the controller logs a rejection with `ERR_STALE_SETPOINT`. Because the rejection is a normal, handled outcome, it does not page anyone and it does not show up as a service failure.

Customers notice this as a higher bill, not as an outage. An installer usually hears about it days later, when the customer asks why the battery did not charge overnight. By then the log lines may have rotated, so the first job is to find the rejections in whatever retention exists.

A useful tell: rejections come in runs for one site and not for others. If every site rejects at once, suspect the scheduler host or the broker path. If only some sites reject, suspect the device clocks.

## What the rule actually is

The check compares the timestamp carried inside the setpoint message with the controller's own notion of now. If the message is older than 90 seconds by that comparison, it is stale and gets rejected. The comparison uses the timestamp in the message body, not the time the broker received it and not the time the message was queued.

This matters because a message can be delivered promptly and still be judged stale. If the clock that stamped the message is behind the clock that checks it, the age looks larger than it is. The reverse also bites: if the stamping clock is ahead, messages look like they come from the future, and depending on how the code treats that, they may be accepted when they should not be. Do not assume the rule only catches slow delivery.

The threshold is deliberately tight. A setpoint is a statement about what the battery should do right now, and acting on an old one can fight the current tariff period or the current solar output. Do not loosen it to hide a drift problem.

## Why clock drift is the usual cause

Home devices are not great timekeepers. Many sit on consumer routers, boot without a reliable network time source, or lose sync after a power cut. Once the inverter or gateway comes back, its clock can be minutes off until it syncs, and it may never sync if outbound time traffic is blocked on the customer network.

The drift is often invisible from the cloud side. Azure IoT Hub accepts the telemetry, InfluxDB stores points with whatever timestamps they carry, and dashboards plot them. The data looks plausible but sits shifted in time. Only the age check in the controller turns the shift into a visible failure.

Other causes exist, such as a long stall in the scheduler between stamping and publishing, or a backlog on the broker after a reconnect. Those are rarer. Rule out drift first because it is both the most common and the cheapest to confirm.

## How to confirm drift quickly

Compare the timestamp in a rejected message with the time the rejection was logged on the controller side. If the gap is consistently large and stable, the clocks disagree by roughly that amount. If the gap grows over time, a clock is running fast or slow rather than being offset once. If the gap jumps, a sync or a reboot happened between messages.

Then compare the device's reported time against a trusted source. The telemetry the device sends to Azure IoT Hub carries its own timestamps, and the hub records an enqueue time of its own. A consistent difference between the two is a direct measurement of device skew. Pull a handful of recent points rather than one, since a single point can be a stale retry.

In InfluxDB, look at the latest points for the site. Points that land with timestamps visibly behind or ahead of wall time are a second confirmation. Check more than one measurement so you do not mistake a lagging sensor for a lagging clock.

## What not to conclude

A rejection does not mean the controller is broken. It is doing what it was built to do. Restarting it will not help, and restarting it can make things worse by dropping state that was fine.

It also does not mean the forecast is wrong. The Julia forecasting and scheduling side can be perfectly correct and still produce setpoints that never take effect. Do not spend time retuning the model because the battery did not charge.

Finally, a single rejection is not an incident. Occasional stale messages happen around reconnects. The pattern that matters is repeated rejections for the same site across tariff windows.

## Fixing the device clock

The real fix is on the device. Make sure it can reach a time source and that it actually syncs. On customer networks, the common blocker is a router or firewall that drops the time protocol. The installer may need to allow it, or point the device at a time server that is reachable.

After the clock is corrected, watch the next few setpoints for the site. The rejections should stop on their own with no change to GridHaven code. If they continue after the clock looks right, go back and measure again, because the clock may have been fixed in one place (the gateway) while the stamping happens somewhere else (the inverter or a bridge).

If a device cannot hold time at all, for example failing hardware or a dead backup cell, replacement is the answer. Do not paper over it with server-side tolerance.

## Do not widen the window as a workaround

It is tempting to raise the staleness limit so the errors go away. Avoid this. A wider window makes the controller willing to act on commands that describe a situation that has already passed, which can mean charging during an expensive period or discharging when the household needed the energy later.

It also hides the underlying problem. A site with a drifting clock is also writing mis-timed data into InfluxDB, and that data feeds the next forecast. Fixing only the controller's tolerance leaves the history polluted and the forecasts quietly worse.

If a change to the limit is ever argued for, it should come with evidence about real delivery delays on the MQTT path and a review of tariff-window edges, not with a single drifting site as the reason.

## Effect on stored data

While a clock is off, anything it stamps carries the wrong time. Solar output and battery state points written during that period may sit in the wrong place on the time axis. Forecast models trained or evaluated on that data see a shifted day: production appears earlier or later than it was.

After fixing a device, decide whether the bad stretch should be excluded from training and evaluation for that site. Excluding it is usually safer than trying to shift points back by an estimated offset, because the offset is rarely constant.

Mark the affected range somewhere durable, such as a note on the site record, so a later person does not wonder why the curves look odd for those days.

## Where the timestamp comes from

The path for a setpoint goes from the scheduler, through MQTT, to the controller. The message is stamped at one end and checked at the other. Knowing which clock stamps it is the key to reading a rejection correctly.

When the scheduler stamps the message, the stamping clock is a server clock and is normally well synced. Then the controller side, usually closer to the device, is the likely culprit. When something nearer the device re-stamps or rewrites the message on the way, that component's clock enters the picture too.

Read the current code path before assuming either. This note records the behavior, not the exact stamping location, and that location can change. Check the code if the diagnosis depends on it.

## Telling drift from delivery delay

Drift gives a steady or slowly moving offset between stamp and check. Delivery delay gives a spread: most messages are fine, a few are late, and the late ones cluster around reconnects or broker restarts.

If the offset is the same for messages sent at different times of day, it is drift. If it correlates with load or with network events, it is delay. Plot the age at check time against the send time for a day of messages if the logs support it. Drift draws a flat or sloped line. Delay draws spikes.

Mixed cases exist. A device with a mild offset will reject only when delivery is also a little slow, which looks intermittent and is the most confusing version of this problem.

## Monitoring suggestion

Because the rejection is quiet, add visibility. A count of `ERR_STALE_SETPOINT` per site per day, plotted next to the schedule, would turn this from a customer complaint into an alert. A site with repeated rejections should be flagged to the installer before the bill shows it.

A second signal is the measured skew between device timestamps and hub enqueue times. That one catches drift before it grows large enough to cross the limit, so the fix can happen early.

Neither of these needs a change to the rejection rule. They only make the existing behavior visible.

## Talking to installers

Installers do not need the internals. What they need is a plain statement: the battery ignored commands because the device clock disagreed with ours, and the device needs a working time source. Give them the site, the period, and what to check on the customer network.

Be clear that GridHaven did not lose data and that the battery was not damaged. The cost was missed optimization, not a safety issue. Avoid vague language that makes it sound like an outage.

After the fix, tell them what to expect: rejections stop, and the next schedule applies as normal.

## Testing this behavior

When working on the controller or the scheduler, keep a test that sends a setpoint stamped older than the limit and expects `ERR_STALE_SETPOINT`, and one stamped just inside the limit that is accepted. Boundary tests are worth having because off-by-one or unit mistakes (seconds against milliseconds) are a classic way to break this check.

Also test a message stamped in the future. Decide what the expected behavior is and write it down, so a later change does not alter it by accident.

In simulation, inject a fixed offset into the device clock and confirm the failure shows up the way this note describes. That makes it easy to reproduce for someone new.

## Related components

The scheduler in Julia produces the setpoints. MQTT carries them. Azure IoT Hub handles device identity and telemetry ingest. InfluxDB holds the time series. The Svelte front end shows the plan and the battery state to customers and installers.

The front end is a place where this problem leaks out: it may show a planned charge that never happened. If the display can show whether the last setpoint was applied or rejected, that closes the loop for support staff.

None of those components enforce the age limit. Only battery-controller does, so that is where the rejection is logged.

## Quick checklist

First, find `ERR_STALE_SETPOINT` in the controller logs for the site. Second, check whether it is isolated to that site or global. Third, compare message timestamps with the check time and with the hub enqueue time to measure skew. Fourth, get the device clock fixed at the source. Fifth, confirm that rejections stop. Sixth, flag the bad period in the stored data so it does not mislead training.

If the age is under the limit and the message is still rejected, the cause is not this one. Look for a different error, a malformed message, or a mismatch in the unit of the timestamp.

## Open questions

It is not settled whether the controller should log the measured age along with the rejection. That would make diagnosis much faster, since the offset would be right in the log line.

It is also not settled whether a persistent run of rejections should trigger an automatic fallback mode, such as holding the last known good plan, or whether leaving the battery alone is the correct safe behavior. For now it leaves the battery in its prior state, and anyone changing that should check the effect on tariff windows first.
