---
id: 01JYXRY8151T29DWSM9G27X9MB
created: 2025-06-29T09:00-03:00
---

# battery-controller: things to watch out for when changing it

This is a general list of the traps around battery-controller. It is not a spec and records no decisions. Read it before touching the component, and add to it when something new bites. The component sits between the forecast side, the tariff side and the physical battery, so most bugs show up somewhere other than where the change was made.

The short version: battery-controller turns a forecast and a tariff into commands for hardware that is in someone's house. Wrong forecasts cost money. Wrong commands can wear out a battery, or leave a customer without backup power when they need it. Treat every change as one that touches a real device, even when the diff looks like plumbing.

## Where it sits in the data flow

The rough shape, in the terms we actually use:

```text
InfluxDB history -> Julia forecast -> battery-controller -> MQTT -> device
device telemetry -> Azure IoT Hub -> InfluxDB -> Julia forecast
Svelte UI <- state and schedules from battery-controller
```

There is a loop in this picture. Device telemetry goes back into the store and feeds the next forecast. A change that alters what battery-controller commands will, a bit later, alter the data the forecast learns from. So a bad change does not just produce a bad schedule once. It can bend the history that later schedules are built on. Keep that in mind when you test with real or replayed data, and when you decide whether to repair bad history afterwards.

Things that follow from this:

- Do not assume a message you publish is the end of the story. Devices may ignore it, apply it late, apply it partially, or apply it and report something different back.
- Do not assume what you read back is what you sent. Telemetry reports what the device did, not what you asked for.
- When you change a field that goes out, check what reads it on the way back in. When you change a field that comes in, check what the forecast and the UI do with it.
- The Svelte side shows the state to installers and customers. If battery-controller changes what a state means, the UI will keep showing the old meaning until someone changes it too. People will make decisions from that screen.

## Time, tariffs and time zones

Most of the nasty bugs in this area are about time, not about energy.

Time-of-use tariffs are defined in local wall-clock time. The forecast, the stored series and the device messages are easier to keep in a single absolute time base. Every place where one is converted into the other is a bug candidate. When you change anything that builds a schedule, check which clock each value is on, and do not assume the helper you call is consistent with the one next to it.

Watch for these in particular:

- Clock changes in spring and autumn. A local day can be shorter or longer than usual. A schedule built by stepping through a fixed count of slots will drift against the tariff on those days. A tariff window can also appear twice or not at all in local time.
- Tariff boundaries that do not line up with the slot size used by the scheduler. If a boundary falls inside a slot, decide explicitly which price applies, and keep that choice the same everywhere.
- Tariffs that differ between weekdays, weekends and holidays, or by season. Changing the lookup can quietly break one of these cases while the common case still passes.
- Customers in different time zones under the same installer. Never use the server's local zone as a default. Always take the zone from the site.
- Devices with a clock that is wrong or has drifted. Do not trust a device timestamp for ordering without checking it against receive time. If you reorder events by device time, a bad clock will reorder your history.
- Tariff changes announced in advance. A schedule built before the change must not be applied after it without being rebuilt. Check how the controller knows that a plan is stale.

If a test only uses a typical day in the middle of a month, it proves very little here. Add the odd days.

## Forecast input and what to do when it is bad

The forecast comes out of the Julia side and the controller must not take it on faith. Forecasts are late, missing, flat, or plainly wrong in ways that look valid, such as a smooth curve that has nothing to do with the weather.

When changing how the controller consumes the forecast:

- Keep the fallback path working. If the forecast is missing or too old, the controller should still do something safe and boring. Test that path on purpose, because it only runs when things are already going wrong and so it rots first.
- Decide what counts as stale and keep that rule in one place. Do not let the schedule builder, the UI and the alerting each carry their own idea of it.
- Do not treat a forecast as a point value. If the forecast carries uncertainty, the scheduler should not plan as if the optimistic case is certain. Charging against a sunny forecast on a day that turns out overcast is the classic loss.
- Units and sign conventions. Power versus energy, per-slot versus cumulative, import positive versus export positive. A silent mismatch yields plausible numbers that are wrong by a constant factor or sign, and these survive review because nothing crashes.
- Missing values in the input series. Julia code that fills, interpolates or drops gaps can behave differently on a gap at the edge than in the middle. Check both.
- Be careful with `NaN` and `missing` crossing the boundary. They can turn into a zero, a null, or an error depending on the serialization, and a zero is read as a real value downstream.

When you change the shape of data that crosses between Julia and the rest, check both ends in the same change. The two sides are easy to update separately and then forget.

## Device commands over MQTT

Commands to the battery go out over MQTT, and MQTT gives fewer guarantees than people assume.

- Delivery semantics. Depending on the quality level used, a command can be lost or delivered more than once. Commands must be safe to apply twice. A command that means "set the target" is safe. A command that means "add to the target" is not. Prefer the first form, and keep it that way.
- Retained messages. A retained command is replayed to a device when it reconnects, possibly long after it stopped making sense. Be very cautious about retaining anything that tells a battery to charge or discharge. A device that comes back after an outage should get the current intent, not an old one.
- Ordering. Two commands sent close together may arrive in either order across reconnects or different paths. Put a sequence or validity window on anything where order matters, and have the device reject what is out of date.
- Topic structure and wildcards. Changing a topic name, or the way a site or device is encoded in it, changes who receives the message and who is allowed to. Check subscriptions and access rules in the same change. A topic that is too broad can deliver one customer's command to another customer's battery.
- Payload changes. Devices in the field run mixed firmware. Adding a field is usually fine, renaming or removing one is not. Do not assume every device has been updated. Keep the old form accepted for as long as any device might still send or expect it.
- Backpressure and bursts. After an outage, many sites reconnect at once and the controller sees a burst of telemetry and a burst of due commands. Check that the code copes with this and does not, for example, publish a full replan for every site at the same moment.
- Connection loss. Decide what the device should do when it hears nothing for a while, and make sure that is what actually happens. The safe default is usually to fall back to a local self-consumption behaviour, not to hold the last command forever.

When you test, use a broker that actually drops and reorders things. A happy-path local broker hides most of this.

## Azure IoT Hub, device identity and telemetry

Telemetry and device management go through Azure IoT Hub, and the controller depends on it for identity, for state and for knowing what a device is currently set to.

- Device twin or equivalent state can be out of date. The desired state and the reported state are different things, and the gap between them is where the interesting bugs live. If the controller reads reported state as if it were current truth, it will act on something that already changed.
- Throttling and quotas. Calls against the hub are limited. A change that adds a call per device per cycle looks harmless with a handful of test devices and becomes a problem with a real fleet. Think in fleet terms whenever a loop touches the hub.
- Credential and token expiry. Connections that work during a short test can fail after the credential ages out. Check that renewal is handled and that a failed renewal does not turn into a silent stall.
- Device identity mapping. The mapping from a hub device to a site, a customer and a battery is relied on everywhere. If you touch it, check that provisioning, replacement of a device, and removal still work. A replaced battery should not inherit the old one's limits by accident, and should not lose them either.
- Duplicate and late messages. Telemetry can arrive twice or well after the fact. Write to the store in a way that tolerates repeats, and do not let a late message overwrite newer state.
- Environment separation. Make sure test and development code can never reach real devices through a shared hub or shared credentials. Check configuration carefully when copying it between environments.

## Storing and reading time series in InfluxDB

InfluxDB holds the history that the forecast learns from and that the UI draws. The controller both writes to it and reads from it.

- Tags and fields. A tag value is indexed and a field value is not. Putting something with unbounded variety in a tag, such as a per-message identifier, makes the series count grow without limit and slows everything. Keep tags to things like site and device.
- Changing a field's type. Once a field has been written with one type, writing another to the same field is rejected or silently lost depending on the path. If you need a different type, use a new field name and handle both in the readers.
- Renaming measurements or fields. Old data does not move. Dashboards, the forecast and the UI that read the old name will see a cut-off, and anything trained on history will see a gap. Plan a transition, not a rename.
- Retention and downsampling. Check what the controller reads against what is still stored at full resolution. A query that worked on recent data can return coarser or empty results for older ranges, and a model that learns from them will behave differently.
- Timestamps and precision. Writing with a precision different from what the reader assumes shifts the data by a large factor in time, and you will find it as a flat line or an empty chart.
- Write amplification. A change that writes derived values on every cycle for every site can quietly multiply the write volume. Ask whether the value can be computed at read time instead.
- Queries in the control path. Do not make a command depend on a slow or large query without a timeout and a fallback. When the database is slow, the battery should not wait on it.

## Battery limits, safety and customer impact

This part matters more than the rest, and it is the part that tests cover least.

- Hard limits belong to the device and the installer's configuration, not to the scheduler's optimism. The controller must never command beyond them, even if the optimizer says the numbers work out. Enforce limits as the last step before sending, so a bug upstream cannot bypass them.
- Reserve for backup. Many customers expect a minimum reserve to stay in the battery for outages. Any change to how the schedule drains the battery must keep this in mind. Check whether reserve is a per-site setting and make sure the new code reads it and does not fall back to a default that differs.
- Cycling and wear. A schedule that chases small price differences by charging and discharging often can cost more in battery life than it saves in tariff. When changing the objective, look at how many cycles the new plan implies, not just the price outcome.
- Efficiency losses. Round-trip losses mean that a small price gap is not worth acting on. If the cost model ignores losses, or applies them on one leg only, the schedule will look better on paper than it is in the house.
- Grid export rules. Some sites have limits on export or different export prices. A change that charges from solar and discharges to the grid at the wrong time can break the site's agreement. Check that site-level rules are still honoured.
- Temperature and state of health. If the device reports conditions that restrict charging, the controller should respect them and not repeatedly retry. Repeated retries look like activity and hide the cause.
- Manual overrides. Installers and customers can set a mode by hand. A new automatic feature must not override a manual choice without telling anyone, and it must be clear how the manual choice is released.
- Rollout. Prefer a change that can be switched off per site without a deploy. Roll out to a few sites first, ideally installers' own, and watch the real behaviour for a full tariff cycle before widening it. Do not assume a simulation result carries over.
- Rollback. Know what a rollback leaves behind. Commands already sent, retained messages and schedules already stored will still be there after the code is reverted.

## Testing and review habits

Some habits that have paid off, or would have:

- Replay recorded days through the controller and compare the plan with the previous version. Look at the differences, not just whether it ran. Include a bad forecast day, a day with missing telemetry, and a day with an unusual tariff.
- Test the failure paths as carefully as the main path: broker down, hub throttling, database slow, forecast missing, device silent.
- Keep the pure scheduling logic free of I/O so it can be tested quickly and deterministically. If a change forces I/O into it, stop and think about whether it belongs there.
- Log the inputs behind each decision in a form that can be read later by a person. When a customer asks why the battery charged at a given time, the answer should be recoverable from the logs without guessing. Take care not to log credentials or personal data.
- Review changes to configuration and defaults as carefully as code. A changed default is a behaviour change for every site that did not set the value.
- When something looks odd in production, check whether it is the controller, the forecast, the tariff data or the device before changing anything. Most reports blamed on the scheduler turn out to be data.
- Write down what you learned. If you find a new trap while working here, add it to this note, with the reason it matters, in general terms.

## Quick checklist before merging

- Which clock does each time value use, and where is it converted?
- What happens if the forecast is missing, late or wrong?
- Is every command safe to apply twice and safe to receive late?
- Do the limits and the backup reserve still get enforced last?
- Did any name, topic, field or payload shape change, and have all readers and mixed-firmware devices been considered?
- Does anything now run per device per cycle against the hub or the database, and does it hold up at fleet size?
- Can the change be turned off per site, and what stays behind after a rollback?
- Does the UI still mean what it says?
