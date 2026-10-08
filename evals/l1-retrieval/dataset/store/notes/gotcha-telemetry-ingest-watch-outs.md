---
id: 01JTEV2K79AJ8HWER365ND8HTZ
created: 2025-05-04T20:46-03:00
---

# telemetry-ingest: things to watch when changing it

Notes for anyone touching telemetry-ingest. None of this is new, but each item has bitten people or is likely to. Read it before a change, not after the dashboards go odd.

## Where it sits

telemetry-ingest takes readings from household devices, which arrive over MQTT directly or through Azure IoT Hub, and writes them to InfluxDB. The Julia forecasting and charge-scheduling code reads from that store, and the Svelte front end shows the results to installers and customers. So a small change here shows up three steps later as a bad forecast or a strange chart. Nobody sees an error at the point of the change.

Do not assume the two inbound paths behave the same. Message shape, ordering, retry behaviour and metadata differ between a direct broker and the hub. A fix tested on one path can quietly break the other.

## Timestamps and ordering

Most of the trouble comes from time. Devices have drifting clocks, some report in local time without saying so, and messages can arrive late, twice, or out of order after a network drop. Things to keep in mind:

- Decide which timestamp wins: the device one or the receive time. Do not swap that casually, because forecasts and tariff windows depend on it.
- Duplicates must stay harmless. Writing the same point twice with the same key overwrites in InfluxDB; writing it with a slightly different timestamp creates a second point and double counts energy.
- Daylight saving changes and tariff boundaries are where off-by-one-hour bugs hide. Tariff periods are scheduled in local time, storage should not be.
- Backfilled data after an outage should not trigger the same side effects as live data.

## Schema and tags in InfluxDB

Tag and field choices are close to permanent. Changing a field type, renaming a measurement or turning a field into a tag does not migrate old data, and InfluxDB may reject writes whose type conflicts with what is already stored. Check what is already stored before changing anything.

Be careful with tag cardinality. Putting something unbounded in a tag, such as a message id or a raw timestamp, makes the store slow and costly over time. Per-device identifiers are fine; per-message ones are not.

Downstream queries, including the Julia ones, often rely on exact names and units. Search the consumers before renaming. Units matter: watts versus kilowatts, and power versus accumulated energy, are easy to mix up and produce plausible-looking wrong numbers.

## Payload parsing and device variety

Installed devices run different firmware generations, and old ones stay in the field for years. Payloads may omit fields, send them as strings, or use sentinel values for missing readings. Keep parsing tolerant: drop or flag a bad reading, never turn it into a zero. A zero for solar output at noon looks like a cloudy day to the forecaster and can push a bad charging schedule.

Keep malformed messages somewhere you can inspect them rather than silently discarding them. When tightening validation, expect real devices to start failing it, and check the rejection rate on live traffic before and after.

## Delivery, backpressure and restarts

MQTT quality-of-service levels and hub acknowledgement settings decide whether a restart loses data or replays it. Know which one you are relying on before you change connection, session or acknowledgement handling. Acknowledging before the write to InfluxDB succeeds loses data on failure; acknowledging after means replays, which loops back to the duplicate problem above.

If InfluxDB is slow or down, decide what happens to incoming messages: buffer, drop, or push back on the broker. Unbounded buffers in memory are a common way for this service to fall over later, away from the change that caused it. Batch sizes and flush intervals interact with this, so change them one at a time.

Credentials and device identity for the hub and broker are handled outside the code. Do not log payloads that could carry customer details, and do not paste connection secrets into config files that get committed.

## Testing and rollout

Unit tests with tidy sample messages are not enough. Replay a captured sample of real traffic, including messy and late messages, against a scratch bucket and compare it with what the current version writes. Look at the forecast side too, not only at whether writes succeeded.

Roll out gradually where possible. A bad change here corrupts stored history, and fixing history is much harder than reverting code. Before any change that alters what is written, work out how to tell old-format points from new ones, and tell whoever owns the forecasting and the front end.
