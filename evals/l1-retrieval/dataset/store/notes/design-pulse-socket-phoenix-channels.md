---
id: 01KFZ73TP69PQ0PEQDBM6YC2KE
created: 2026-01-27T04:53-03:00
sources:
  - "code: lib/townhall_web/endpoint.ex"
---

# pulse-socket design

pulse-socket is the realtime entry point for TownHall Pulse. It is a Phoenix Channels endpoint listening on port `4000` at the path `/socket/websocket`. Browsers running the Next.js front end connect there over WebSockets to receive poll results, Q&A updates and moderation events as they happen. Anything that needs to reach attendees live goes through this endpoint, so keep it thin and fast.

## What it does

Each client opens one socket and joins channels for the event it is attending. Producers and community managers join additional channels for moderation. The socket itself holds no business logic. It authenticates the connection, assigns the client to topics, and forwards messages to the Elixir contexts that own polls, questions and moderation state.

Channel topics are scoped per event, so a message for one event never reaches clients of another. Fan-out uses Phoenix PubSub, which keeps broadcasts cheap even when an event has a very large audience.

## Design choices

- Plain Phoenix Channels over WebSockets, no custom protocol. The stock Phoenix JS client works from Next.js without extra glue.
- State lives in CockroachDB, not in the socket process. A socket process can die and the client simply rejoins and catches up from the database.
- Moderation actions are broadcast as soon as they commit, so a hidden question disappears from attendee screens quickly. Moderators see the pending queue; attendees only see approved items.
- Votes are written through the poll context and then the aggregate is broadcast, rather than broadcasting every raw vote. This keeps message volume down during a spike.
- Joins are lightweight. Heavy loading happens after the join reply, so a reconnect storm does not block the channel process.

## Gotchas

- Reconnects come in waves when a network blip hits many viewers at once. Rejoin has to be idempotent, and the client backs off with jitter. Do not make join expensive.
- A load balancer in front of the endpoint must allow WebSocket upgrades and keep idle connections open longer than the heartbeat interval, or clients drop and rejoin in a loop.
- Do not push per-user data onto shared event topics. Per-user replies go back on the originating channel only.
- Slow consumers can back up their mailbox. Drop or coalesce poll aggregate updates for them instead of queueing forever, since the next aggregate supersedes the last.

## Related

Post-event exports are not handled by this endpoint; see [[export-worker-must-finish]] for why the export side must complete even if sockets are closed. When debugging, first check that clients are really reaching the endpoint on port `4000` at `/socket/websocket` before looking at channel code. A wrong path or a proxy that strips the upgrade headers looks like a channel bug but is not.
