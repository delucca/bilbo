---
id: 01JRRV2A493VDGHTXHBGBX0CZ9
created: 2025-04-13T21:26-03:00
---

# settlement-events producer compression: zstd

Producers of settlement-events use zstd compression. We set `compression.type=zstd` on every producer that writes to the settlement-events topic. The reason is size: in our comparison, payloads came out `38%` smaller than with snappy. That is the whole decision; the rest of this note is context so nobody has to redo the comparison.

## Naming

`setev` is short for `settlement-events`. You will see `setev` in config keys, dashboards, alert names and chat. In code comments and docs prefer the full name `settlement-events`, so a search finds everything. They are the same topic and the same component, not two things.

## What we decided

- Producers of settlement-events set `compression.type=zstd`.
- Snappy is no longer the setting for this component.
- The choice is made on the producer side. Consumers do not need a matching setting, because Kafka consumers read the compression codec from the record batch.

```properties
# producer config for settlement-events
compression.type=zstd
```

## Why zstd over snappy

Settlement payloads are repetitive. Card-processor settlement rows repeat the same field names, currency codes, merchant references and status values again and again, so a compressor with a stronger dictionary and entropy stage does well on them. Snappy favours speed over ratio. On our sample payloads zstd gave output `38%` smaller than snappy.

Smaller batches help in a few places: less network traffic between the Go producers and the brokers, less disk used by the topic at the retention we run, and less replication traffic. For finance operations teams the volume spikes around settlement file drops, and those spikes are where the savings matter most.

## Costs and trade-offs

zstd uses more CPU than snappy on the producer, and consumers pay some CPU to decompress. We judged this acceptable: the Go services that produce settlement-events are not CPU bound, and reconciliation is batch-oriented, so a little extra latency per batch does not hurt. If producer CPU ever becomes a problem, the first thing to try is a lower zstd level on the client, not going back to snappy without re-measuring.

The 38% figure came from our own payload shapes at the time of the comparison. If the payload schema changes a lot (for example much more free-text or binary content), the number may shift. Re-measure before quoting it in new places.

## Rollout notes

Changing the codec is safe to do gradually. Kafka allows batches with different codecs in the same topic, so old snappy batches stay readable and new ones are written as zstd. No topic recreation and no consumer redeploy is needed. Old data keeps its original size until retention removes it, so do not expect disk use to drop right after the change.

The producer setting should live in the shared producer config for the Go services, and any Terraform that provisions topic-level settings should not override the codec with a different `compression.type` at topic level. A topic-level override of the broker's compression can make brokers recompress batches, which wastes CPU and hides the producer-side gain. Check for this when reviewing infrastructure changes.

## Open points

- Nobody has written down which zstd level the producers run at. Use the client default until someone measures otherwise.
- Other topics in Ledgerlark still use whatever they had before. This decision covers only settlement-events; do not copy it to other topics without measuring their payloads.
