---
id: 01K8TNXZSKWQR4895KNJEPMR8T
created: 2025-10-30T10:46-03:00
---

# producer-console moderation panel migration to React Server Components

This note replaces the earlier note about "producer console migrate moderation". The new value is that the producer-console moderation panel is to be migrated to React Server Components by 2027-02-26, and the earlier date of 2026-12-15 no longer holds.

The rest of this note is the working plan as it stands today. It is written for whoever picks the migration up next, whether that is an engineer or a coding agent. It is meant to be edited as things get settled, so correct it in place instead of adding a second note on the same subject.

## Goal

Move the moderation panel inside producer-console from a fully client-rendered Next.js surface to one built on React Server Components, with client components kept only where the panel truly needs browser state or a live socket. The target date is 2027-02-26. The point is not novelty. The panel is the heaviest screen in producer-console, producers keep it open for the whole length of a large virtual event, and its initial load and memory footprint are the things producers complain about most.

Success means three things. First, the first paint of the panel no longer waits on a large client bundle. Second, moderation actions (approve, reject, pin, merge duplicates, close a poll) behave exactly as they do now, with no change in what moderators see or in what the backend records. Third, the live behaviour over WebSockets is unchanged: a new question appears in the queue as fast as it does today.

## Why the date moved

The earlier date was 2026-12-15. It was pulled out to 2027-02-26 because the migration would otherwise land in the middle of the busiest event season, when producers are running large events back to back and nobody wants a risky change to the screen they depend on. Moving the date past the season also gives time to do the inventory properly and to run the new panel next to the old one on real events before switching.

If the date has to move again, change it here first and tell the producers and community managers who were told the previous date. Do not let the date drift silently in tickets while this note says otherwise.

## Scope

In scope is the moderation panel of producer-console: the question queue, the poll results view, the moderator action bar, the filter and search controls, the audit trail view for a single item, and the banner that shows connection health. Also in scope are the Next.js route segments that wrap the panel, the loading and error states for those segments, and the data fetching layer that feeds them.

Also in scope is whatever small adjustment the Phoenix side needs so the server components can fetch what they need in a shape that is cheap to render. That should be limited to read endpoints and to making the channel payloads easier to consume. It is not a rewrite of the backend.

## Out of scope

The attendee-facing views are not part of this work. Neither is the poll authoring flow, the event setup screens, or the reporting pages in producer-console. They may adopt the same patterns later, but this migration should not touch them except where they share a component with the panel and the shared component has to change.

The moderation rules engine in the Elixir backend is also out of scope. The migration changes how the panel is rendered and fetched, not what counts as a flagged question or how automatic filters score a submission. Changes to the database schema in CockroachDB are not planned and should be treated as a sign the scope has crept.

## Current state of the panel

Today the panel is a single large client tree. On load it fetches the current queue over HTTP, opens a WebSocket to a Phoenix channel for the event, and then keeps all state in the browser, including the full list of items, the selection, the filters and the optimistic updates for moderator actions. Almost every component is marked as a client component because the state lives high in the tree and everything below it needs to read it.

This works, but it has known costs. The bundle is large because the panel pulls in rich text rendering, date formatting, the audit trail viewer and several chart pieces for poll results, all of which are shipped even if the moderator never opens them. On a long event the in-memory list grows, and low-end laptops used by volunteer moderators start to stutter. Hydration of a big queue is slow when a moderator reloads in the middle of an event, which is exactly when speed matters most.

## Target architecture

The target splits the panel into a server-rendered shell and a small number of client islands. The shell, the static parts of the layout, the poll results summary, the audit trail view and the item detail pane become server components that fetch their own data on the server and send rendered output instead of code. The islands are the action bar, the live queue list, the filter controls and the connection banner, because those need event handlers, local state or the socket.

The guiding rule is to push the client boundary as far down as it will go. A component is a client component only when it must be, and the reason should be one of: it handles user input, it holds state that changes without a navigation, it subscribes to the socket, or it uses a browser-only capability. Anything else renders on the server. When in doubt, start on the server and move the boundary down only when a concrete need shows up.

## Live data over WebSockets

The hard part is that a server component renders once per request and cannot hold a socket. Live behaviour therefore stays in client islands. The plan is for the queue island to receive its initial items as props from the server render, then open the Phoenix channel and apply incoming events to local state, the same way the current panel does but with a much smaller state surface.

Two things need care. The first is the seam between the server-rendered initial list and the first events from the channel: an event can arrive for an item that is not in the initial render, or for one that was already included. The island must reconcile by item identity and by a monotonic marker from the backend, not by arrival order. The second is refresh: when a moderator navigates within the panel, the server components re-render, and the island must not lose its socket or its place in the queue. Keep the socket owner high enough in the client tree that route changes inside the panel do not remount it.

## Server-side data fetching

Server components will call the Phoenix backend over HTTP from the Next.js server, forwarding the producer's session. Fetching should be done per component close to where the data is used, with request deduplication so that two components asking for the same event summary cost one call. Responses for data that changes constantly, like the queue, must not be cached. Responses for slow-moving data, like event metadata and the moderator roster, may be cached for a short time, and the cache lifetime should be decided per call and written down next to the call.

Authorization stays on the Phoenix side. The Next.js server must never decide on its own what a producer may see. It passes the credential through and treats a refusal from the backend as final, rendering the existing not-permitted state. Do not copy permission logic into the front end to save a round trip.

## CockroachDB considerations

The panel reads come down to CockroachDB through Phoenix, and moving rendering to the server changes the read pattern a little. Instead of one big fetch per client, there can now be several smaller reads per navigation, issued from one place. That is usually fine, but it can multiply under load when many producers reload at once during a large event.

Before switching, check the read endpoints behind the panel for query shape and for contention on hot rows, since moderation writes and queue reads touch the same items. Keep reads on the queue as simple range reads, and do not add new transactions that hold rows while a render waits. If a read needs to tolerate slightly stale data, prefer a bounded staleness read there over adding a cache layer in Next.js. Agree on any such choice with whoever owns the backend before changing it.

## Phased plan

The work is split into phases so that each one can ship behind a flag and be backed out alone. The order matters: inventory first, then boundaries, then the live seam, then the actions, and only then the switch. Nothing in the early phases changes what producers see.

Each phase should end with the old panel still available and still the default. The new panel is reachable through a flag that can be turned on per producer or per event, so that a few trusted community managers can use it on real events well before the target date.

### Phase one: inventory

List every component under the panel and mark why each is a client component today. For each, record which of the four reasons applies, or that none does and it is client only because of where it sits. This list is the main input for everything else, and it is cheap to produce. Also list every place that reads shared state from a context or a store, because those are the places that block moving a boundary down.

Also inventory third-party libraries used by the panel and note which ones assume a browser. Some of them will force a client island, and some can be replaced with a server-friendly alternative or loaded only on demand. Write the findings into this note or a linked note, not only in a ticket.

### Phase two: boundaries and shell

Build the server-rendered shell and the read-only server components first: results summary, audit trail view, item detail pane. These have no socket and few handlers, so they are the safest to move. Keep the old action bar and queue as client components inside the new shell, reading from their existing state sources, so the screen works end to end after this phase.

Check that loading and error states exist for each segment of the shell, so one slow read does not blank the whole panel. A producer should always see the queue and the action bar even if the audit trail is still loading.

### Phase three: live queue island

Rebuild the queue as an island that takes initial items as props and owns the channel subscription. Reconcile initial and live items as described above. Replace the large client store with local state scoped to the island, and move filters into URL state where that is natural, so a reload keeps the moderator's view.

This is the phase with the most risk to live behaviour, so it gets the longest soak on real events behind the flag, and it should not be merged with the next phase.

### Phase four: actions

Move moderator actions to server actions or to direct calls to the backend, whichever turns out simpler to keep correct. Optimistic updates stay on the client, but the source of truth is the backend response and the channel event that follows it. An action that fails must roll the item back visibly and say why, using the same wording producers already know.

Action handlers must stay idempotent from the producer's point of view. A double click or a retry after a network blip must not approve, reject or merge twice.

### Phase five: switch and cleanup

Turn the new panel on by default, keep the old one reachable through the flag for a defined period, then delete the old code. Do the deletion as its own change so it can be reverted without touching the new panel. Remove the unused libraries from the bundle at the same time and note the size difference for the record.

## Risks

The main risk is a regression in live behaviour that only shows up under the load of a big event, where hundreds of items arrive quickly and several moderators act at once. Staging data rarely reproduces that. Mitigation is to replay recorded event traffic against the new island and to run it beside the old panel on a real but low-stakes event first.

The second risk is hidden coupling: some component deep in the tree reads a context that nobody remembers, and moving a boundary silently breaks it. The inventory is the defence. The third is schedule: the date has already moved once, and a second slip would push into the next busy period. Keep the phases small enough that a slip in one does not hide a slip in the rest.

A smaller risk is team habit. Server and client code now live side by side, and it is easy to import something browser-only into a server component or to leak a server-only secret toward the client. Use the framework's markers for server-only modules and keep the lint rules on.

## Testing

Unit tests cover the reconciliation logic of the queue island, because that is pure and easy to get subtly wrong: duplicates, out-of-order events, events for unknown items, and removal of items that were resolved elsewhere. Component tests cover each island with a fake channel. End-to-end tests drive the whole panel against a Phoenix instance with a seeded event and check the full moderation loop: a question is submitted, appears in the queue, gets approved, and shows up on the attendee side.

Add a load test that opens many simulated producer sessions against the new fetching path and watches database read behaviour in CockroachDB. The goal is not a benchmark number but to catch a pattern that multiplies reads. Also test the unhappy paths on purpose: the socket drops and reconnects, the backend returns a refusal, a server component read times out.

Accessibility must not regress. Moderators use keyboard shortcuts heavily, and moving to islands changes focus handling. Test focus after each action and after route changes inside the panel.

## Rollout

Roll out by flag, from the inside out. First the team, then a few community managers who agreed to try it, then a share of events chosen by the producers themselves, then everyone. At each step, watch the error rate on the panel routes, the time to first paint, the lag between a submission and its appearance in the queue, and the support messages from moderators.

Do not enable the new panel for an event that has already started on the old one; switching in the middle of a live event is an avoidable risk. Tell producers ahead of time about the change and about how to get back to the old panel while it still exists. Keep the announcement plain and short, and say what is different on screen, which should be very little.

## Rollback

Rollback is turning the flag off, which returns producers to the old client-rendered panel on their next load. For that to stay true, the old panel and the new one must keep working against the same backend endpoints and channel messages until the cleanup phase. Any backend change made for the new panel has to be additive and backward compatible.

If a problem appears in the middle of a live event, the moderator can reload after the flag is off. Write that into the runbook for producers so that nobody has to ask. Decide in advance who is allowed to flip the flag during an event, and make sure they can do it without a deploy.

## Open questions

Whether moderator actions should be server actions or plain calls to the backend is not decided; it depends on how session forwarding behaves in practice and on how errors can be surfaced. Whether filters should live fully in URL state is also open, since some filters are noisy and would clutter history. Whether the poll results charts can render on the server or need an island depends on the charting library found in the inventory.

It is also unclear how much of the cache policy for slow-moving data the backend team wants to own, and whether a bounded staleness read is acceptable for the roster. These need an answer before phase three starts, not after.

## Notes for the next session

Start with the inventory; nothing else can be sized without it. Keep the component name as producer-console everywhere, in tickets, flags and dashboards, so that searching finds everything. Do not reintroduce the old date from the earlier note: the only date that counts is 2027-02-26, and the earlier one of 2026-12-15 is history.

When a decision from the open questions is made, move it out of that section and into the relevant phase, with a one line reason. When a phase ships, record what actually happened, including anything that surprised you, so the next phase starts from what is true and not from what was planned.
