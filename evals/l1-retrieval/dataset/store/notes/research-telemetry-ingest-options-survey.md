---
id: 01KEQ6E82RHVHZB2WXTEVADPEF
created: 2026-01-11T15:52-03:00
---

# telemetry-ingest: options survey

This is a survey of the general options we looked at for telemetry-ingest, the part of GridHaven that takes readings from home inverters, meters and batteries and lands them where the forecasting and charging scheduler can use them. Nothing here is a decision. It is a list of what exists, what each option costs us, and what to check before picking.

## What telemetry-ingest has to do

Devices at customer homes send power, state of charge and tariff-relevant readings. Telemetry-ingest receives them, checks them, tags them with the site, and writes them to storage. The Julia forecasting code reads from storage, and the Svelte dashboards read from it too. It has to cope with flaky home networks, devices that go quiet and then send a burst, and clocks that are off.

## Device-to-cloud transport

Two families came up. One is plain MQTT against a broker we run or rent. The other is Azure IoT Hub, which speaks MQTT too but adds device identity, per-device credentials and routing. Plain HTTP posts from the device are a third option, simpler on the device but worse for bursts and for pushing commands back down.

## Self-run MQTT broker

Full control over topics and retention, and cheap at small scale. We carry the operations: clustering, upgrades, certificate rotation, and capacity. Per-device authentication is possible but needs tooling we would write or glue together. Worth it if installers want on-site brokers or if cloud cost becomes the main complaint.

## Azure IoT Hub as the front door

Device registry, per-device keys or certificates, and built-in message routing come for free. Cloud-to-device messages and device twins help with pushing charging schedules. The downside is cost that grows with device count and message volume, some lock-in to its quotas and message size limits, and a less flexible topic model than raw MQTT. We already use Azure, which lowers friction.

## Hybrid: local broker plus cloud bridge

A small broker at the site collects from several devices, buffers when the uplink drops, and forwards upstream. This helps with flaky links and local control of the battery when the cloud is unreachable. It also adds a thing installers must deploy and support. Not clear yet how many installs would accept that.

## Getting data out of the transport

Whichever transport wins, something has to consume messages and write to storage. Choices: IoT Hub routing straight to an endpoint, a small consumer service we write, or a stream processor. A consumer in Julia keeps one language but Julia services are less common to operate. A thin consumer in a more common service language is an option if the team agrees on one. Routing alone is attractive until we need validation or enrichment on the way.

## Storage choice

InfluxDB is the working assumption for time series: good fit for per-site readings, downsampling and retention policies. Things to watch are cardinality from tagging, how late or out-of-order points are handled, and the clustering or hosting story. Alternatives are a general SQL database with a time series extension, or object storage with columnar files for history plus a small hot store. We have not benchmarked these on our own data.

## Validation and cleaning

Options: reject bad readings at the edge of ingest, store everything raw and clean on read, or do both with a raw copy kept aside. Keeping a raw copy costs space but makes reprocessing possible when we find a unit or scaling bug. Leaning toward keeping raw, but this is a preference only.

## Time handling

Device clocks drift and some have none until they sync. Options: trust device timestamps, stamp on arrival, or store both and reconcile. Storing both is the safest for forecasting accuracy and for billing-adjacent tariff windows, at the price of extra fields and rules for which one wins.

## Backpressure and bursts

After an outage many devices reconnect and replay buffered data together. Options include broker-side queues, rate limits per device, batching writes to storage, and a buffer between consumer and database. We need to know whether replayed data should have lower priority than live data so the scheduler is not starved.

## Delivery guarantees

At-most-once is simple and loses data during faults. At-least-once needs idempotent writes, which time series stores often give by overwriting a point with the same series and time. Exactly-once end to end is mostly not worth chasing here. Needs a check against how the chosen store treats duplicates.

## Security and tenancy

Each device should have its own identity so one stolen credential does not expose a fleet. Installers should see only their own customers, so site and installer tags have to be set by ingest from the registry, not trusted from the payload. Rotation of credentials in the field is the hard part with any transport.

## Observability

We want counts of received, rejected and written messages per site, lag between device time and write time, and a way to spot silent devices. Options are metrics from the consumer, a dedicated dead-letter store for rejected messages, or alerts driven from the database itself. Silent-device detection probably belongs in a separate job, not the hot path.

## Open questions

- How many devices are expected over the near term, and how fast does that grow?
- Do installers need local control when the internet is down?
- Is a raw archive required for audit or only convenient?
- Who operates this at night, and does that rule out self-run pieces?
- Which language should the consumer be written in?
