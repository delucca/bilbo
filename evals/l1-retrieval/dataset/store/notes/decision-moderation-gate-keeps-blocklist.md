---
id: 01JYF7E3AH5HKF51KV5S2FF5XT
created: 2025-06-23T17:24-03:00
---

# moderation-gate keeps its blocklist in ETS

moderation-gate holds its blocklist in an `ETS` table. It does not query CockroachDB for each message. We made this change because the per-message queries added `35 ms` of latency, and that cost landed on every message in the live path. The blocklist is now read from memory inside the Elixir node that is already handling the message. CockroachDB stays the durable store for the list. It is no longer on the hot path.

This note records the decision, the reasoning, what we gave up, and what to check if someone proposes undoing it. Read it before touching how moderation-gate loads or reads its blocklist.

```text
moderation-gate
  message in -> check blocklist in ETS -> allow or hold
  CockroachDB: source of truth for the list, not read per message
  reason: per-message queries added 35 ms
```

## Decision

moderation-gate checks every incoming poll answer, question and chat-style submission against the blocklist before it reaches the audience or the moderator queue. That check now reads from an `ETS` table owned by the moderation-gate process tree. The table is filled from CockroachDB and refreshed when the list changes. A message check does a local in-memory lookup and never makes a network round trip to the database.

The scope of the decision is narrow:

- The blocklist lookup that runs per message uses `ETS`.
- The blocklist is still stored in CockroachDB. Producers and community managers edit it through the normal product surfaces, and those edits are written to the database first.
- Loading and refreshing the table is a separate, non-hot-path job. It may talk to CockroachDB as often as it needs to.
- The decision does not cover other moderation data, such as per-event settings, moderator assignments or the audit trail of held and released messages. Those were not the problem and we left them alone.

We chose `ETS` over other in-memory options because it is built into the runtime, needs no extra service, supports concurrent reads from many processes without funneling through a single process, and fits how the rest of the Phoenix application already shares read-heavy data between channel processes. A single GenServer holding the list in its state would have serialized every lookup through one mailbox, which is the wrong shape for a gate that sits in front of every message at a large event.

## Why we did it

The product is live polls and Q&A for large virtual events, with real-time moderation. The whole point of the gate is that a message is judged quickly enough that the audience experience still feels live. When moderation is slow, one of two bad things happens. Either messages show up late, which makes a live Q&A feel broken, or the gate gets skipped or loosened to stay fast, which defeats moderation.

The original design asked CockroachDB on each message whether the text matched a blocked entry. That was simple and always consistent, and it was fine at small scale. Measured under realistic conditions, the per-message queries added `35 ms` of latency. That figure is the cost of the database round trip alone, on top of whatever the rest of the pipeline already spent. Three things made it matter more than it sounds:

- It is paid on every message, not on a sample. At a big event the number of messages in a short window is large, and each one pays it.
- It stacks with other steps in the path: the WebSocket receive, validation, the gate, persistence and the broadcast back out. The gate was the one step that added a database wait before anything else could proceed for that message.
- It adds load on the database that scales with message volume. During a spike, such as a host opening a poll to the whole audience, the gate would turn a burst of messages into a burst of reads against CockroachDB, competing with the writes we actually need to finish promptly.

The blocklist itself changes rarely compared with how often it is read. Reading an almost-static list from the database for every message is paying for freshness we mostly do not need. That mismatch between read rate and change rate is the core reason a local cache is the right tool here.

We considered keeping the database lookup and making it cheaper, for example by tuning the query or adding an index. The query was not the slow part. The cost was the round trip and the contention under load, and tuning could not remove either. We also considered a short-lived per-process cache in front of the database. That reduces the average cost but keeps a cold-path penalty and spreads the cached copies across many processes, with no single place to refresh them. A shared `ETS` table gives one copy per node with a clear owner and a clear refresh path.

## How it behaves

The table lives on each node that runs moderation-gate. Every node loads its own copy, so a lookup never crosses the network. The owner process creates the table, fills it from CockroachDB at startup, and keeps it current afterward. Readers are the many processes that handle messages, and they read concurrently without asking the owner for permission.

A few points about behavior that a later reader should not have to rediscover:

- Startup ordering matters. The table must be populated before the gate starts accepting messages for an event. If the gate came up with an empty table, it would let through content the list is meant to catch. The owner loads first, and the gate treats an unloaded table as not ready rather than as an empty list.
- Reads are local and fast, so the added cost of the gate on the hot path is now small compared with the database approach. That is the whole benefit and the reason this note exists.
- Updates are applied to the table when the list changes. An edit made by a producer or community manager is written to CockroachDB and then propagated to the nodes so their tables catch up. Until a node has applied the change, that node still uses the older list.
- If a node restarts, its table is lost with the process and is rebuilt from CockroachDB. Nothing about the blocklist is only in memory, so a crash does not lose data. It only costs a reload.
- The table is not the source of truth. If the table and the database ever disagree, the database wins, and the fix is to reload the table, not to edit the table by hand.

We keep the table readable by other processes on the node and writable only by its owner. This avoids races where two writers update the list at once and leave it in a half-applied state. It also keeps the failure mode simple: if the owner dies, the supervisor restarts it, the table is recreated, and it reloads.

## Tradeoffs we accepted

The main thing we gave up is strict, instant consistency. With the per-message database query, a blocklist edit took effect on the very next message everywhere. With `ETS`, there is a short window after an edit during which some nodes still hold the old list. For a blocklist that is edited by humans between and during events, we judged that window acceptable. A newly blocked term might be let through, or a newly unblocked term might be held, for a brief moment on a node that has not caught up. Moderators also review held and flagged content, so a brief gap is covered by the human layer that already exists.

If an event producer needs an edit to apply immediately, for example when a harmful term starts spreading mid-event, the right response is to make the refresh path fast and reliable, not to put the database back on every message. Do not solve a freshness complaint by reverting this decision. Solve it by shortening the time to propagate.

Other costs, stated plainly:

- Memory. Each node holds a full copy of the list. For the blocklist sizes we expect, this is small next to everything else the node holds, but it grows with the list and is multiplied by the number of nodes. If lists ever become very large, per-event or per-community partitioning of what is loaded is the first thing to try.
- More moving parts. There is now an owner process, a load step, a refresh path and a readiness state to get right. The database-per-message approach had none of these. Each is a place a bug can hide, and the readiness handling in particular is easy to get wrong.
- Node-to-node drift. Two nodes can briefly answer differently for the same message. This should never show up as a persistent difference. If it does persist, the refresh path is broken and that is a bug to fix.
- Testing is slightly harder. Tests that touch the gate need the table to exist and be populated, so they set it up explicitly instead of relying on a database fixture alone.

We thought about what happens if CockroachDB is unreachable. Before this change, an unreachable database meant the gate could not decide, and the team had to choose between holding everything and letting everything through. After this change, the gate keeps working from the table it already has. The database being down affects loading and refreshing, not the per-message decision. That is a real improvement in availability for the live path, though it also means a node can run on a stale list for as long as the outage lasts. Surfacing that staleness in logs or metrics is worth doing.

## What to check before changing this

If you are about to alter this design, check these first.

- Is the complaint about latency or about freshness? If latency, the table is the fix and this decision stands. If freshness, improve propagation.
- Does the change put a database read back into the per-message path? If yes, you are re-creating the `35 ms` cost this decision removed. Measure before and after, and expect that cost to return.
- Does the change affect startup ordering? An empty or half-loaded table must never be treated as a valid list. Keep the gate not-ready until the load completes.
- Does the change give more than one process write access to the table? Keep the owner as the only writer.
- Does it make the table the source of truth in any way? Do not. CockroachDB stays authoritative.

If the blocklist grows large, or if per-event lists with very different content become common, revisit how much each node loads. Loading only what active events need would cut memory without bringing back per-message queries.

If we ever need stronger consistency, for example for legal or contractual reasons at a particular customer, the likely answer is a faster, push-based refresh with an acknowledgment step, not a return to querying on each message. Another option is a per-event switch that routes only that event through a slower, stricter path. We have not needed either, and neither is planned.

## Operational notes

When something looks wrong with moderation behavior, the first questions are whether the node's table has loaded and whether it is current. Check readiness before suspecting the matching logic. A gate that is letting through content it should hold has usually missed a refresh or come up before its load finished. A gate that is holding content it should allow has usually missed an unblock.

When the blocklist is edited, remember that the database write is the real change and the table update follows. If an edit seems not to have worked, check that the write succeeded before blaming the table, then check whether the affected node applied it. A reload on the affected node is a safe fix, because the table holds nothing that is not also in the database.

When adding new kinds of content to the gate, such as new message types from the Next.js client, make sure they go through the same table lookup. Do not add a side path that asks the database directly because it is easier. That would quietly bring back the cost this decision was meant to remove, and it would only show up under load at a large event.

The reason for all of this is short. Per-message queries added `35 ms` to a path that has to feel live, and the blocklist changes far less often than it is read. So the list is held in `ETS`, loaded from CockroachDB, and refreshed when it changes. Everything else in this note is detail about doing that safely.
