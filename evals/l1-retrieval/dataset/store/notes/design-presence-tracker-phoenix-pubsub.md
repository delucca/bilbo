---
id: 01K9MR0HD5QM0PXNVWS1RA5J0A
created: 2025-11-09T13:43-03:00
---

# presence-tracker design

presence-tracker answers one question for TownHall Pulse: who is in this event right now, and in which room or session. It is built on `Phoenix.Presence`, which runs on top of Phoenix.PubSub. The PubSub instance is configured with `pool_size: 8`. That is the only sizing number this note records; everything else below is described in general terms on purpose, so check the code before quoting a figure.

This note covers the shape of the component and the reasons for it. It is not a spec of the wire format.

## Why Phoenix.Presence

Large virtual events mean a lot of WebSocket clients joining and leaving in bursts, often right when a session starts or a poll opens. We did not want to write our own heartbeat and conflict-resolution layer. `Phoenix.Presence` already gives us a CRDT-based replicated state, so each node tracks its own connections and merges with the others without a central owner. That fits the Elixir cluster we already run for the poll and Q&A channels.

Things we get for free:

- Tracking is tied to the channel process, so when a socket drops the entry goes away without cleanup code.
- Joins and leaves are broadcast as diffs, not full lists, which matters when a room is big.
- Netsplits heal by merging; we do not have to reconcile by hand.

## PubSub pool

Presence diffs go out over Phoenix.PubSub. A single dispatcher process becomes a bottleneck when many topics broadcast at once, so the PubSub instance is started with `pool_size: 8`. Broadcasts are spread across that pool by topic hash, which keeps one busy room from blocking the others.

```elixir
{Phoenix.PubSub, name: TownHallPulse.PubSub, pool_size: 8}
```

If presence diffs start lagging under load, this is the first knob to look at, but measure before changing it. A bigger pool costs more processes and does nothing if the real limit is elsewhere, such as the channel process doing too much work per join.

## What we track and what we do not

Metadata attached to each presence entry stays small: the role of the participant (attendee, moderator, producer) and a join time. Anything bigger, such as display names with avatars or per-user poll history, is looked up when needed instead of being copied into every diff. Large metadata multiplies across every subscriber on every join, so keep it out.

Presence is not the source of truth for who attended. It is live state in memory. Attendance records that producers want after the event are written to CockroachDB from the channel lifecycle, separately from the tracker. If a node restarts, presence rebuilds from reconnecting clients; the database record does not depend on it.

## Moderation and the front end

Moderators see the live participant list and can act on a person, so the tracker exposes roles to the moderation tools. The Next.js client subscribes through the same socket as polls and Q&A and applies diffs to a local list. It must handle a burst of joins as a batch; rendering one at a time made the participant panel janky in big sessions.

For very large rooms the client shows counts first and loads the full list only when a moderator opens the panel. That keeps the common path cheap.

## Gotchas

- Do not call the full presence list from a hot path. It is fine for a moderator opening a panel, not for every message.
- A user with several tabs shows up as several entries under one key. Group by user when counting people, or counts will be too high.
- Diffs can arrive for a leave right after a join of the same key during reconnects. Treat the state as the merge of both, not as a sequence.
- Changing the PubSub pool means a restart of that supervisor child; plan it outside live events.

## Open points

- Whether to shard presence topics per session in very large events is undecided. The current layout uses one topic per room.
- We have not decided how long a disconnected moderator keeps elevated status before the tracker drops it.
