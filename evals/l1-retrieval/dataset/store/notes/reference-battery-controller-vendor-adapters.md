---
id: 01KY8GVVJ07SD6RQJ2RCHHE8W7
created: 2026-07-23T19:17-03:00
sources:
  - "code: src/adapters/modbus_map.jl"
---

# battery-controller adapters and register map

Where the vendor-specific code of battery-controller lives, and how to find your way around it. Short version: every battery vendor adapter is under `src/adapters/`, and the Modbus register map is `src/adapters/modbus_map.jl`. If you are touching how battery-controller talks to a physical battery, start in that directory and nowhere else.

## Layout

battery-controller is the Julia component that decides when a home battery charges or discharges, based on the solar forecast and the time-of-use tariff. The decision logic is vendor-neutral. Anything that knows a vendor's quirks sits in the adapters directory, `src/adapters/`.

Each vendor gets its own adapter file in that directory. The scheduler never talks to a battery directly; it calls an adapter, and the adapter translates the request into whatever the vendor device understands. Keep that boundary. If a vendor detail leaks into the scheduler, move it back into the adapter.

## Modbus register map

The register map is one file: `src/adapters/modbus_map.jl`. It is the single place that says which register holds which value (state of charge, charge power limit, mode, and so on) for the Modbus-speaking batteries.

Rules of thumb when editing it:

- Change register definitions here, not inline in an adapter. Adapters read from the map.
- A register address that looks wrong is usually a vendor firmware difference, not a typo. Check the vendor documentation for the firmware in question before changing anything.
- Scaling and signedness matter. A value that is off by a factor of ten or flips sign at zero is almost always a scale or type entry in the map.
- If you add a new Modbus battery, add its registers to the map first, then write the adapter that uses them.

## Adding a vendor adapter

1. Add a new Julia file under `src/adapters/` named after the vendor.
2. If the device speaks Modbus, put its registers in `src/adapters/modbus_map.jl` instead of hardcoding them in the new file.
3. Implement the same set of operations the other adapters expose, so the scheduler can use it without special cases. Copy the shape of an existing adapter rather than inventing a new interface.
4. Register the adapter where battery-controller picks adapters by configuration, so an installer can select it.
5. Test against a simulated device first. Do not test a new register map against a customer battery.

A small illustration of the layout:

```text
src/adapters/
src/adapters/modbus_map.jl
```

## Data flow around the adapters

Commands come from the scheduler, which works from forecast output and tariff windows. Measurements read back from the battery go out over MQTT and are stored in InfluxDB, so a bad register read shows up there first as a flat or jumping series. When a battery reading looks implausible on a dashboard, check the adapter and the map before suspecting the pipeline. Devices in the field also report through Azure IoT Hub; that path does not go through the adapters and is a separate concern.

## Gotchas

- Two vendors can share a protocol but not a register layout. Do not reuse map entries across vendors just because both use Modbus.
- Read-only registers and writable registers can sit next to each other. Writing to the wrong one can put the battery in a mode the installer did not choose. Mark writable entries clearly in the map.
- Charging limits set by the adapter should never exceed what the vendor allows, whatever the scheduler asks for. Clamp in the adapter.
- Timeouts on a flaky serial or network link look like missing data, not errors. Keep retries bounded so a dead link does not stall scheduling.

## Where to look first

- Battery does not respond or returns nonsense: `src/adapters/modbus_map.jl`, then the adapter for that vendor in `src/adapters/`.
- New vendor request: the steps above, starting in `src/adapters/`.
- Schedule is right but the battery does the wrong thing: the adapter translation, not the scheduler.
- Schedule itself is wrong: that is not an adapter problem; look at the forecast and tariff inputs instead.

Keep this note in step with the directory. If the map is ever split per vendor or moved, update the two paths at the top.
