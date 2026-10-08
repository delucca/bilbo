---
id: 01KF79SDT9JJSS8PAB86XVWGEZ
created: 2026-01-17T21:58-03:00
---

# presence-tracker reference

Quick reference for presence-tracker, the part of TownHall Pulse that knows who is connected to an event right now. It sits in the Elixir and Phoenix side of the stack, talks to browsers over WebSockets, and feeds counts to the Next.js front end and to the moderation tools. The one hard fact worth keeping at the top: presence-tracker exposes Prometheus metrics at `/metrics` on port `9568`. Everything else below is general orientation, written fast, so check the code before relying on any detail that is not that one.

## Metrics endpoint

presence-tracker serves Prometheus metrics at `/metrics` on port `9568`. That is a separate listener from the normal Phoenix endpoint, so it is not reachable through the public event URL and should not be. Scrapers need network access to that port on every node that runs presence-tracker, not only to one of them.

If a dashboard shows no data for presence-tracker, first check that the scraper target points at port `9568` and path `/metrics`. A wrong port is the usual cause, since the main app listens elsewhere and people copy the wrong target from the other services. A plain request to that path from inside the network should return text in the Prometheus exposition format. If it returns nothing or a connection error, the node is down or the metrics listener did not start.

The metrics are per node. To get a total across the cluster, sum in the query layer rather than expecting one node to report the whole picture.

## What it does

presence-tracker keeps track of which participants are in which event and which room or poll view they are looking at. It answers questions like how many people are watching right now, who is a moderator, and who has just joined or left. Producers see the live audience number on their dashboard. Community managers use it to judge whether a Q&A is worth opening up at a given moment.

It does not store questions, votes or moderation decisions. Those belong to other components. It only tracks that someone is here, and some light metadata about their role.

## How it fits in the architecture

Participants connect from the Next.js client over WebSockets to Phoenix channels. When a connection joins an event topic, the presence-tracker logic registers it. When the socket closes, the entry is removed after a short grace period so that a flaky network does not make people flicker in and out of the count.

State is held in memory on each node and merged across the cluster using the usual Phoenix presence approach, which is eventually consistent. CockroachDB is not on the hot path for presence. It is used only for durable things around it, such as event configuration and, where needed, coarse historical audience snapshots written on a slow cadence.

## Data model in brief

Each tracked entry is keyed by event and by participant session. The metadata is small: role, the view the participant is on, and a join time. Keep it small. Every change to metadata is broadcast, and in a large event that fan-out is the main cost.

There is a distinction between a person and a session. One person with two browser tabs shows up as two sessions. The audience count that producers see is deduplicated by person where an identity is known, and by session otherwise. Anonymous attendees are always counted per session, so the number can run a little high for them.

## Joins, leaves and the grace period

A leave is not applied immediately. A short delay lets a reconnecting client reclaim its entry without producing a leave and a join event. This matters at the start of a big event, when many clients reconnect at once after a load balancer shuffle.

If counts look too high after a network incident, the grace period is the first thing to suspect. If counts look too low or jumpy, check whether the client is reconnecting with a new session identity instead of reusing the old one.

## Scaling behavior

Large virtual events are the whole point of the product, so the interesting load is a very large number of joins in a short window, usually right when a keynote starts. The tracker handles this by batching broadcasts of count changes instead of sending one message per join. Individual join and leave diffs go to moderators and producers. Ordinary attendees get periodic count updates only.

Nodes are added horizontally. Presence state merges between nodes, and merge cost grows with the number of tracked entries, so memory and CPU on each node rise with the size of the biggest events it hosts. Watch the metrics for tracked entries and for broadcast queue depth before an event, not during it.

## Metrics worth watching

The names below are described in words, not quoted, because the exact metric names should be read from the `/metrics` output itself.

The useful signals are the current number of tracked entries per node, the rate of joins and leaves, the time taken to merge state from other nodes, the depth of the outgoing broadcast queue, and the usual Erlang VM figures such as process count, run queue and memory. Alert on a growing broadcast queue and on merge time drifting up. Those two warn of trouble before attendees notice.

A flat line on every presence metric during an event, while the app is otherwise serving traffic, usually means scraping broke, not that nobody is present. Cross-check against the dashboard's own audience number.

## Local development

Run the Phoenix app the usual way and open the Next.js client against it. With the app up, request `/metrics` on port `9568` from the same machine to confirm the metrics listener is alive. Opening a few browser tabs on one event is enough to see entries appear and disappear, including the grace period delay when a tab is closed.

Tests for presence logic use simulated sessions rather than real sockets where possible. Timing-dependent tests around the grace period are the most likely to be flaky, so give them generous timeouts instead of shortening the delay in the test configuration and forgetting to revert it.

## Operational notes

Rolling deploys cause every connected client on a node to reconnect. That is a mini join storm, and it shows up in the metrics as a spike in joins and leaves with the tracked-entry count dipping slightly. Prefer deploying between events. If you must deploy during one, go node by node and wait for the broadcast queue to drain before the next.

If a node is removed from the cluster abruptly, the other nodes eventually drop its entries once they notice it is gone. During that gap the audience count is too high. This is expected, not a bug.

## Known gotchas

Counts are eventually consistent, so two moderators may briefly see different audience numbers. Do not build features that need an exact, agreed number at a single instant.

The metrics listener is separate from the main endpoint. Health checks on the main endpoint tell you nothing about whether metrics are being served, and the reverse is also true. Keep both checks.

Metrics are per node and reset when the node restarts. Counters going backwards in a graph are a restart, not a fault, and the query layer should use rate functions that cope with resets.

Large metadata is expensive. Adding fields to the tracked entry multiplies the broadcast cost by the audience size, so think twice before adding anything.

## Troubleshooting checklist

Audience number is zero or missing: confirm the client joined the right event topic, then confirm the node is in the cluster, then check `/metrics` on port `9568` for the tracked-entry figure on that node.

Audience number is too high: look for a recent network incident, a node that left the cluster, or clients reusing no stable session identity and so leaving duplicates behind.

Dashboards empty: check the scrape target address and port `9568`, then the path `/metrics`, then network rules between the scraper and the node.

Slow updates to moderators during a big event: check broadcast queue depth and merge time, then the number of nodes relative to the size of the event.

## Open questions

Whether the historical snapshots in CockroachDB should be written by presence-tracker itself or by a separate job is not settled. Whether per-event metrics labels are worth their cardinality cost on the metrics endpoint is also open; for now keep labels coarse. Update this note when either is decided.
