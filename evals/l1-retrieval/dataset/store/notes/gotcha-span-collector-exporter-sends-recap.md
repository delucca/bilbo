---
id: 01KWSW0456BC9W555MTFQA9JCS
created: 2026-07-05T16:27-03:00
---

# span-collector exporter sends: what goes wrong

Notes on how the exporter in span-collector sends batches to ClickHouse and why it quietly loses or delays data if you leave the defaults alone. Written from memory of a couple of debugging sessions, so it is partial.

## What happens

The exporter in span-collector buffers spans in memory and flushes them to ClickHouse in batches. When the queue fills up, new spans are dropped, and the only trace of that is a counter on the collector's own metrics. Nothing fails loudly. On the Grafana side the latency panels just look thin right after a deploy, which is exactly when SREs are looking at them. That makes it easy to read a gap as "no traffic" when it is really "collector shed load".

## Why sends stall

Two causes came up more than once:

- The batch size is set larger than ClickHouse likes for a single insert, so inserts get slow, the sending queue backs up, and the retry logic keeps resending the same batch while newer spans pile up behind it.
- The retry window is longer than the pod's shutdown grace period. During a rollout Kubernetes terminates the old collector pod, and whatever is still in the queue goes with it. Deploys are the moment we most want complete traces, and they are also the moment collectors get restarted.

Neither shows up in a quiet staging environment. You need real deploy-time volume.

## Things to check first

Look at the exporter's queue size against its configured capacity, and at the dropped and failed send counters, before touching ClickHouse. If the queue is near the limit, the problem is on the send path. If the counters are flat and data is still missing, look upstream at the sampling config or at the SDKs in the Java services instead.

Also check whether ClickHouse is merging heavily. Many small inserts produce too many parts, and ClickHouse starts rejecting or delaying inserts. Larger, less frequent batches fix that, which pushes against the first cause above. The balance is a tuning call, not a single correct value.

## What to do about it

- Keep the batch size and flush interval so that inserts stay moderate and not tiny.
- Make the retry window shorter than the shutdown grace period, or raise the grace period to cover it.
- Alert on the dropped-spans counter instead of waiting for someone to notice a hole in a dashboard.
- If persistent queueing is available for the exporter, consider it, but verify the volume is backed by storage that survives pod rescheduling. An emptyDir does not.

## Open questions

I did not confirm the exact interaction between retry backoff and the queue limit under sustained ClickHouse slowness. Worth a load test before the next big release. Also unclear whether the existing note on this covers the shutdown ordering. Merge the two if they overlap.
