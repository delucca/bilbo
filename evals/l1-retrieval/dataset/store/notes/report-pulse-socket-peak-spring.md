---
id: 01KF30VEDPS04TEDDTT1SPEF5F
created: 2026-01-16T06:05-03:00
sources:
  - "doc: spring-summit-postmortem"
---

# pulse-socket peak load at the spring summit

At the peak of the spring summit, pulse-socket held 210000 concurrent connections across 14 nodes. This note keeps that figure as a reference point for capacity talks. It is a measured peak from a real event, not a target and not a load-test result.

## Headline figure

pulse-socket reached 210000 concurrent connections across 14 nodes at the busiest moment of the spring summit. Anyone asking how big pulse-socket has run in production can be given that pair of numbers together. The connection count alone says little without the node count.

## What the component does

pulse-socket is the WebSocket tier of TownHall Pulse. It is built on Elixir and Phoenix. It pushes poll results, Q&A updates and moderation actions to attendees in real time. The Next.js front end connects to it from the browser.

## Why the peak matters

Large virtual events are the main use case for the product. Event producers and community managers watch live polls and questions while moderators act on them. A drop or lag in pulse-socket is visible to all of them at once, so the peak is the number we plan around.

## Reading the numbers

The figure is concurrent connections at a single peak moment. It is not the total number of attendees over the whole event, and it is not a rate of messages. Averaging by node gives a rough per-node load, but connections were probably not spread perfectly evenly, so treat any division as approximate.

## What we did not record

We have no clean record here of memory use, CPU, or message throughput at the peak. Those should be pulled from the metrics for the summit before anyone claims headroom. Nothing in this note says the cluster was near its limit.

## Database side

State behind the sockets lives in CockroachDB. This note does not cover how the database behaved during the peak. If the database showed strain, that belongs in a separate note on the data layer.

## Moderation under load

Real-time moderation ran during the summit on the same tier. No moderation failures are recorded here. If someone remembers delays in hiding or approving questions at peak, add that as a finding.

## Using this for planning

When sizing for the next large event, start from the 210000 concurrent connections on 14 nodes as the known good case. Scaling beyond it is untested as far as this note knows. Plan a load test before promising more.

## Open questions

- How much spare capacity did the 14 nodes have at the peak?
- Were connections evenly spread across nodes?
- Did reconnect storms happen at any point during the summit?

## Follow-ups

Collect the summit dashboards and attach the key graphs by name to this note or a new one. Update this report if a later event beats the peak.

## Status

Figure recorded as observed. Not yet cross-checked against the raw metrics.
