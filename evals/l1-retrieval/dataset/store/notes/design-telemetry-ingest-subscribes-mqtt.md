---
id: 01K9SE2FS9B4WAWNA9HQZJ26AF
created: 2025-11-11T09:25-03:00
sources:
  - "code: src/subscriber.jl"
---

# telemetry-ingest design

telemetry-ingest is the service that takes inverter telemetry from sites and puts it into the time series store, where the rest of GridHaven reads it. It subscribes to the MQTT topic filter `sites/+/inverter/telemetry` with QoS 1 and writes points to the InfluxDB bucket `raw_telemetry`. The internal codename is `voltpipe`. You will see that name in older chat threads, dashboards and some log prefixes. This note uses `telemetry-ingest` throughout. If you search for either name, search for both.

This note is the design as it stands, written quickly so the next session does not have to work it out again. It says what the component does, why it is shaped that way, and where the sharp edges are. It does not repeat the forecasting or scheduling design. Those live elsewhere and only consume what this component produces.

## Purpose and scope

GridHaven forecasts household solar output and schedules battery charging against time-of-use tariffs. Both jobs need a steady, trustworthy record of what each installed inverter is actually doing. The forecasting side compares predicted output with measured output to correct its models. The scheduling side needs the current state of the battery and the present generation to decide whether to charge now or wait for a cheaper window. Neither wants to speak MQTT or deal with duplicate messages, so this component sits between the devices and the store.

The job of telemetry-ingest is narrow on purpose:

- Receive inverter telemetry messages published by sites.
- Check that each message is well formed enough to store.
- Turn each message into one or more points and write them to `raw_telemetry`.
- Report what it did, so that gaps and slowdowns are visible.

It does not forecast, aggregate for display, or decide anything about charging. It also does not clean data in a way that hides problems. The bucket is called raw for a reason: what lands there is as close to what the device said as we can keep it, with only the minimum shaping needed to store it. Anything smarter happens downstream, in jobs that read from the bucket and write somewhere else.

Installers and their customers never talk to this component directly. They see its effects through the web app, which is built in Svelte and reads processed data, not the raw bucket.

## Naming and history

The codename `voltpipe` came from the first prototype, which was a thin pipe from the broker to the database and nothing more. The name stuck in logs, in a few deployment labels, and in how people talk. The component was later given the descriptive name `telemetry-ingest`, which is what the repository, the design documents and the runbooks use.

Practical consequences:

- When reading old logs or alerts, treat `voltpipe` and `telemetry-ingest` as the same thing.
- New names, labels and metrics should use `telemetry-ingest`. Do not introduce a third name.
- If someone asks about voltpipe, they mean this component, not a separate service. There is no second ingest path under that name.

## Data flow at a glance

The path of one message, from the roof to the store, goes like this.

An inverter at a site reports its state through its local gateway. The gateway publishes a message to a topic that follows the pattern `sites/<site>/inverter/telemetry`, where the site segment identifies the installation. The broker holds the message for subscribers. telemetry-ingest holds a subscription on the filter `sites/+/inverter/telemetry`, so the single plus wildcard stands in for the site segment and one subscription covers every site. The message arrives with QoS 1, is decoded, validated, mapped to points, and written to the `raw_telemetry` bucket in InfluxDB. Only after the write is accepted does the service acknowledge the message back to the broker.

That last ordering is the heart of the design. Acknowledging after the write, not before, means a crash between receipt and write leads to redelivery rather than loss. The cost is that duplicates are possible, which the write side has to tolerate. See the delivery section below.

The service is written in Julia. The pieces are a subscriber loop, a decode and validate stage, a batching writer, and a small reporting layer. They are connected with bounded queues so that a slow store pushes back on the subscriber instead of growing memory without limit.

## MQTT subscription

The subscription is one filter, `sites/+/inverter/telemetry`, with QoS 1. A few notes on why.

### Why one wildcard filter

A single filter with a plus wildcard keeps the subscription list short and means new sites need no change to this component. When an installer commissions a new site, its gateway starts publishing under its own site segment and the data flows in at once. The alternative, one subscription per site, would need a registry that stays in sync and would fail quietly whenever the registry was behind.

The cost is that we receive everything that matches, including messages from sites we have never heard of. That is handled at the validation stage, not at the subscription. See the section on unknown sites.

### Why QoS 1

QoS 1 means at least once. The broker keeps the message until we acknowledge it, and may send it again if it does not see the acknowledgement. We chose it over the other levels for these reasons:

- The lowest level can drop messages without anyone knowing. For an energy record, silent gaps are worse than duplicates, because gaps look like zero generation to the forecasting models.
- The highest level gives exactly once at the protocol level, but costs more round trips on every message and still does not remove the need for an idempotent write, because the guarantee ends at the broker boundary and our write to the store is a separate step.
- At least once with an idempotent write gets the same practical result at lower cost and with simpler failure behavior.

If someone proposes changing the level, the question to ask is what happens to the write path, not just the subscription.

### Session behavior

The client keeps a persistent session with a stable client identity, so that messages published while the service is down are queued by the broker and delivered when it comes back. This only helps if the identity is stable across restarts and if the broker retains the queue long enough. A deployment that generates a fresh identity on each start will quietly lose the backlog. That is the first thing to check if a restart leaves a hole in the data.

## Delivery semantics and duplicates

Because delivery is at least once, the same message can arrive more than once. This happens after a reconnect, after a slow acknowledgement, or when the broker resends because it did not see our acknowledgement in time. The design assumes duplicates are normal, not rare.

The write side deals with this by making the point identity depend on the content of the message and not on when we received it. In InfluxDB a point with the same measurement, tag set and timestamp as an existing point replaces the earlier one. So if the mapping uses the device-reported timestamp and the identifying tags, a redelivered message produces the same point and the write is harmless. This is the property that makes the whole at-least-once design safe.

It also means there is a trap: if anyone changes the mapping to use the receive time as the point time, duplicates stop collapsing and every redelivery becomes a new point. Totals then drift upward in a way that is hard to spot. Keep the device time as the point time. If a message has no usable device time, see the section on validation for what we do instead.

## Decode and validate

Each message goes through a decode step and then a validation step before anything is written.

Decoding turns the payload bytes into a structured record. A payload that cannot be decoded at all is counted, logged with enough context to find the site, and then acknowledged and dropped. We acknowledge it because redelivery of an undecodable message will never succeed and would block the queue behind it. This is the one place where we deliberately discard something we received, and the count of such drops is one of the metrics to watch.

Validation checks that the fields we need are present and plausible. It is deliberately lenient on values and strict on structure:

- Required identifying fields must be present, otherwise the point cannot be given a stable identity.
- Numeric fields must be numbers. A value that is obviously out of range for the kind of reading is flagged, but it is still stored, because the raw bucket should show what the device said, and the downstream cleaning jobs are the place to decide what to do about it.
- Unknown extra fields are carried through where the mapping allows, not rejected, so that firmware that adds fields does not break ingestion.

### Unknown sites

Because the subscription is a wildcard, messages can come from sites that are not in the customer records, for example a gateway being tested at an installer's workshop. We still store them. Filtering by known site is a downstream concern. The reasoning is that dropping at ingest hides the problem of a site that was commissioned but not yet registered, which installers do run into. Storing it lets someone find the data once the registration catches up.

### Missing or bad timestamps

Some devices have a poor clock, especially just after a power cut. When the device time is missing or clearly wrong, the service falls back to the receive time and marks the point so that downstream jobs can tell. That breaks the duplicate collapsing described above for those points only, which we accept because the alternative is to lose them. The marking is the important part. Do not remove it.

## Writing to InfluxDB

Points go to the bucket `raw_telemetry`. The write path is batched: points are collected until either a batch is large enough or enough time has passed, and then sent together. Batching matters because per-message writes would overload the store at the message rates we see when many sites report in the same short period.

### Acknowledge after write

A message is acknowledged to the broker only when the batch containing its points has been accepted by the store. The writer therefore keeps track of which messages belong to which batch, and releases the acknowledgements together after a successful write. If the write fails, none of those messages are acknowledged, and the broker will redeliver them in time. This is slower to recover than acknowledging early, but it is what keeps the no-silent-loss promise.

### Measurement and tag layout

The layout is kept simple. The measurement groups readings by kind. Tags carry the site and the device identity, because those are what every query filters on and what must stay low in variety. Fields carry the numeric readings. Anything with high variety, such as free text or per-message identifiers, must not become a tag, since that inflates series count and slows the store for everyone. If you add a new reading, put it in a field.

### Retention

The raw bucket has a finite retention. It is meant as a working store for recent detail, not an archive. Longer-term history lives in downstream aggregates produced by separate jobs. When the retention was chosen, the reasoning was that forecasting needs recent raw detail to compare predictions with reality, and everything older can be kept at coarser resolution. If you need old raw data for a debugging session, check whether it still exists before assuming a bug.

## Relationship to Azure IoT Hub

Azure IoT Hub is part of the wider platform and is how some devices and gateways are managed and connected. Its role relative to this component should be stated plainly because it confuses people.

telemetry-ingest reads from the MQTT side. It does not talk to the hub's device management features, and it does not use the hub to decide who may publish. Device identity and credentials for the gateways are handled in the hub, and the messages that end up on the topics we subscribe to have already passed through whatever authentication the platform applies on the way in. For this component, the practical meaning is that we trust the topic structure and the broker's access rules, and we do not re-authenticate each message.

This has one consequence worth writing down. The site segment of the topic is taken as the identity of the publisher. If the broker's access rules were ever loosened so that one site could publish under another site's segment, this component would store the data under the wrong site without noticing. The check belongs in the broker rules. We do not duplicate it here, but we should notice if it ever changes.

## Failure modes

The cases below are the ones worth knowing, with what the design does in each.

### Store is slow or down

The batching writer retries with growing delays. While it cannot write, its queue fills, which stops the decode stage, which stops the subscriber from taking more messages. Since nothing is acknowledged, the broker holds the backlog. When the store recovers, the backlog drains. The risk is the broker's own limits: if the outage is long enough that the broker discards queued messages, those are lost. The remedy is to watch queue depth and alert on a growing backlog long before the broker limit is reached.

### Broker connection drops

The client reconnects and resumes its persistent session. Messages in flight at the moment of the drop may be redelivered, which the idempotent write absorbs. A flapping connection shows up as a burst of duplicates and a rise in reconnect counts, not as missing data.

### Process restarts

A restart mid-batch leaves unacknowledged messages, which the broker redelivers. Again this depends on the stable client identity. A restart is safe if and only if the identity is the same as before.

### Poison messages

A message that decodes but triggers an error in mapping is the awkward case. If it is retried forever it blocks everything behind it. The service therefore limits retries for mapping errors, records the failure with site context, and then acknowledges and drops it, in the same way as an undecodable payload. A write error from the store that is about the data itself, as opposed to the store being unavailable, is treated the same way. The distinction between the store being unavailable and the data being rejected is made from the kind of error the store returns, and getting that distinction wrong in either direction is a real bug: treating an outage as bad data drops good points, and treating bad data as an outage blocks the pipe.

### Clock skew

Devices with wrong clocks can send points stamped far from the present. These are stored at their stated time, flagged if the time is implausible, and left for downstream cleaning. They can land outside the bucket's retention window and be refused by the store, which then shows up as rejected writes in the metrics and not as a crash.

## Observability

The service reports what a person on call needs and not much more. The categories are:

- Messages received, decoded, dropped as undecodable, dropped after mapping failure.
- Batches written, batches retried, and write latency.
- Queue depths between stages, which is the earliest warning of trouble.
- Reconnect counts for the broker session.
- Per-site freshness, meaning how long since the last point arrived for each site, derived from the store and not from this process.

The freshness view is what installers care about, since a site that has gone quiet is usually a gateway or network problem at the customer's home. It is deliberately computed from the data and not from this service's counters, so that it still tells the truth if the service itself is the problem.

Logs carry the site segment and a short reason for any drop. They must not carry full payloads at normal levels, since payloads can include details about a household's energy use.

## Downstream consumers

Three kinds of reader depend on `raw_telemetry`.

The forecasting jobs, also written in Julia, read recent raw points to compare with predictions and to fit corrections. They tolerate small gaps but are sensitive to systematic bias, such as duplicated points inflating totals. This is why the duplicate handling above matters more than it first appears.

The scheduler reads the latest state for each site to decide on charging against the tariff windows. It needs recency more than completeness, so a backlog that arrives late is of little use to it. That is a reason to alert on lag, not only on loss.

Cleaning and aggregation jobs read the raw bucket and write the derived buckets that the web app uses. Those are the place for smoothing, outlier removal, and filling gaps. None of that logic belongs in telemetry-ingest, and changes that would put it there should be refused or moved.

## Decisions and reasons, short form

- One wildcard filter, so new sites need no change here.
- QoS 1 with an idempotent write, rather than the higher level, to avoid silent gaps without the extra protocol cost.
- Acknowledge only after the write is accepted, so crashes cause redelivery and not loss.
- Device time as the point time, so duplicates collapse. Receive time only as a marked fallback.
- Store everything that decodes, including unknown sites and odd values, and decide later downstream.
- Drop and count only what can never succeed: undecodable payloads and persistent mapping failures.
- Keep high-variety values out of tags.
- Keep the raw bucket raw. Cleaning is not done here.

## Open questions and things to check

Some points are not settled, and a later session should not assume they are.

- How long the broker keeps a queued backlog compared with the longest outage we expect from the store. We believe there is margin, but nobody has written the comparison down, and it decides whether a long outage means delay or loss.
- Whether the fallback to receive time for bad device clocks is worth its effect on duplicate collapsing, or whether those points should go to a separate place for inspection. For now they go in the same bucket with a marker.
- Whether the unknown-site data should be tagged at ingest, rather than discovered by joining against records later. Tagging would make workshop gateways easier to find, but it would couple this component to the customer records, which we have so far avoided.
- How much the batching parameters should adapt to load. They are fixed today, and they work for current volumes, but a large installer onboarding many sites at once could change that.

When one of these is settled, update this note instead of adding another on the same subject. If the component is ever renamed again, keep `voltpipe` listed here as a former name so old logs remain searchable.
