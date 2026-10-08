---
id: 01JSHTBKBA52RC5MXW42MF724K
created: 2025-04-23T14:15-03:00
---

# Producer-console moderation panel: React Server Components migration plan

The plan is to migrate the moderation panel of producer-console to React Server Components by 2026-12-15. That is the deadline for the whole migration, not for a first prototype. By that date the moderation panel in producer-console should render through server components wherever that makes sense, and the client-side parts that remain should be small, deliberate and easy to name. A reader of only this note should be able to tell what the target is, why it matters, what order to work in, and what could go wrong.

Producer-console is the Next.js front end that event producers and community managers use during live events. The moderation panel is the part where a moderator watches incoming questions and poll responses, approves, hides, merges or pins them, and sees what the audience sees. The backend is Elixir and Phoenix. Live updates reach the browser over WebSockets. Persistent state lives in CockroachDB. This plan touches only the producer-console side. The Phoenix channel contracts and the database schema are meant to stay as they are, and anything that would force a change there should be raised as a separate decision, not slipped in here.

## Goal, scope and what done means

The goal is to move the moderation panel in producer-console onto React Server Components. The reasons, in order of how much they matter to us:

- The panel currently ships a lot of JavaScript to the browser for things that are really just lists and read-only detail views. Moderators often work on shared event-day laptops and on venue networks that are not good. Less client code means a faster first paint and fewer stalls when a session opens mid-event.
- Data fetching for the panel is spread across client hooks that each fetch on mount. That produces waterfalls and flicker. Server components let the page fetch what it needs once, on the server, close to the API, and stream the result.
- Several access checks for moderation actions are done in client code and then repeated on the server. Moving the read side to the server makes it easier to keep one source of truth for who may see what.
- It lines up the moderation panel with the rest of producer-console, which already uses the Next.js app router in other areas. Having one panel on a different model is a maintenance cost.

Done means the following, and nothing less:

- The moderation panel's page shell, navigation, event header, queue summaries, filters that can be expressed as URL state, and read-only detail views are server components.
- Interactive pieces are isolated as client components with small, explicit props. Examples are the approve and hide controls, the multi-select toolbar, the live queue list that reacts to incoming messages, and the keyboard shortcut layer.
- The WebSocket connection is owned by one client boundary, not scattered across components. Server-rendered content provides the initial state; the client takes over for live changes.
- Moderation actions go through server actions or the existing API calls, but in either case they have a single, documented path, and the panel recovers correctly if an action fails or the connection drops.
- There is no regression in moderator-visible behavior: queue ordering, latency of new items appearing, undo behavior, and the audit trail of who did what.
- The old client-only implementation is removed, not left beside the new one. Leaving both would double the surface to maintain.

Out of scope: redesigning the moderation workflow, changing how the audience-facing app works, changing Phoenix channel message formats, changing CockroachDB schemas, and moving other producer-console areas that are not the moderation panel. If we notice something in those areas that blocks us, we write it down and decide separately.

## Starting point and constraints

Before starting, record what is true today, because the plan depends on it. This section is deliberately general; the first task in the plan is to fill in specifics by reading the code and replace any assumption here that turns out wrong.

The panel is mostly a client-rendered tree. A top-level client component owns state for the queue, selected items, filters and connection status. Children read that state through context or props. Data arrives in two ways: an initial fetch when the panel mounts, and a stream of events over the WebSocket after that. Moderation actions are sent as requests and the UI updates optimistically, then reconciles when the server confirms.

The main constraints come from what a live event needs:

- Freshness matters more than anything. A moderator who sees a stale queue may approve something that was already removed by a colleague. Whatever we render on the server is a snapshot, and the live layer must be able to replace it without a visible jump.
- Several moderators work at once on the same event. Concurrent actions on the same item are normal. The panel must show the winner of a race and not silently swallow the loser.
- Load is spiky. At a big event, queues can grow quickly in a short window. Server rendering must not make the panel slower to open under that load, and must not hammer the backend with a fetch per component.
- Sessions are long. Moderators keep the panel open for hours. Memory growth in the client matters, and so does a server component tree that re-renders more than needed on navigation.
- Authentication and event-level permissions already exist. The migration must reuse them, not reimplement them. Server components will need access to the session on the server, and the plan has to say clearly where that happens.
- Real-time moderation is the product's selling point for producers. A bad release during an event is far worse than a late release. That drives the rollout rules below.

Another constraint is the library ecosystem. Some components in the panel depend on browser-only libraries, such as drag-and-drop, virtualized lists, and rich text inputs. Those become client components and have to be wrapped carefully. If any of them cannot be used under a server component parent without trouble, we replace the library or keep that whole subtree on the client; we do not hack around it.

## Plan of work

The work is ordered so that each phase can ship on its own and can be turned off if it misbehaves. Do not start a phase before the previous one is verified in a staging environment with realistic traffic. The dates of the intermediate phases are not set, with one exception: everything must be finished by 2026-12-15. Work backwards from that and leave real slack at the end, because the final phase includes the removal of old code and a full dress rehearsal, and those are the things that slip.

### Phase one: inventory and decisions

Read the panel code and produce a short inventory that classifies every component as server-safe, client-only, or split. For each client-only component, record why: state, effects, event handlers, browser APIs, or a dependency. This inventory becomes a table in this note or a linked note, and it is the basis for the estimate. Also record, for each data dependency, where the data comes from, how fresh it must be, and who else reads it.

Take these decisions early and write each down as a decision note rather than burying it here:

- How the server side reaches the backend: direct calls to the Phoenix HTTP API from server components, or through a thin layer in producer-console. Prefer the simplest option that keeps one place for auth headers and error handling.
- Where the WebSocket client lives and how it hands live events to the tree. The likely answer is one client provider near the root of the panel that exposes a small subscription interface, with server-rendered children passed through as children so they stay on the server.
- How moderation actions are sent: server actions or the existing client request path. Server actions give us revalidation hooks and keep tokens off the client, but they behave differently under flaky networks. Choose one and test it with simulated disconnects before committing.
- What caching and revalidation policy applies to server-fetched moderation data. The default should be no stale cache for queue data. Anything cached must be justified in writing.
- How errors are shown. Server component errors and streaming failures need error boundaries and a visible fallback that tells a moderator what to do, not an empty panel.

Exit criteria for this phase: the inventory exists, the decisions are written, and someone other than the author has read them.

### Phase two: boundaries and scaffolding

Without changing behavior, restructure the panel so the client and server boundaries are in the places we want. Concretely: split the big top-level client component into a thin provider plus presentational pieces, pull data fetching out of leaf components, and make props serializable. This phase is mostly refactoring in the old model, and it is where most hidden coupling will show up, such as components reaching into shared context for things they should receive as props, or functions passed down that cannot cross the server and client line.

Add the app router structure the panel needs: layouts, loading states, error states, and not-found handling. Create the single client provider that owns the WebSocket and exposes connection status and a live event feed. Keep the old behavior behind the same entry point so that nothing changes for moderators yet.

Exit criteria: the panel still works exactly as before, the boundaries are where the inventory says, and the existing tests pass. Add tests where this phase touches untested behavior, especially around optimistic updates and reconnect handling, because later phases rely on them.

### Phase three: read-only surfaces on the server

Move the read-only parts first. These are the page shell, event header, navigation, counts and summaries, and detail views for a selected item that do not need to react live. They are the lowest-risk pieces and they deliver most of the bundle size win. Fetch on the server, stream where it helps, and use suspense boundaries so a slow section does not block the rest.

For anything that must stay live, render an initial server snapshot and hand it to a client component that subscribes to updates. The handoff is the delicate part: the client must not miss events that happened between the server fetch and the subscription. Use a sequence marker or timestamp from the backend, if one exists, to let the client discard events older than the snapshot and request a catch-up if there is a gap. If the backend does not provide something usable, raise it as a request to the backend side and do not paper over it with a client-side guess.

Exit criteria: moderators see no difference except faster opening, bundle size for the panel is measurably smaller, and the reconnect path has been exercised on purpose, not just assumed.

### Phase four: interactive core

Now handle the live queue and the moderation controls. The queue list stays a client component because it reacts to incoming events and supports selection, keyboard use and possibly virtualization. What changes is that it receives its initial data from the server and stops doing its own mount-time fetch. Controls for approve, hide, merge and pin become small client components that call the chosen action path. Their results update the shared client state and, where relevant, trigger revalidation of server-rendered summaries so that counts stay consistent with the queue.

Pay particular attention to:

- Optimistic updates and rollback. If a server action fails or the connection is down, the item must return to its previous state with a visible message.
- Concurrent moderators. When another moderator acts on the same item, the local view must update and the in-flight local action must resolve to a clear outcome.
- Bulk actions. Multi-select with a bulk approve or hide is the case most likely to produce partial failures. Define what the panel shows when some succeed and some fail.
- Keyboard shortcuts, which are heavily used by experienced moderators during fast events. They must keep working across the server and client boundary and must not be lost when a server component re-renders.
- Focus handling after an item leaves the queue. Moderators work fast, and losing focus costs them seconds.

Exit criteria: a moderator can run a full simulated event on the new panel with the same speed and no new failure modes, as judged by the people who actually moderate.

### Phase five: rollout, cleanup and rehearsal

Roll out behind a flag that can be switched per event, so a producer can fall back to the old panel for a specific event if something goes wrong. Start with internal events and low-stakes sessions, then widen. Do not enable the new panel for a large or high-profile event until it has handled several smaller ones without incident. Do not flip anything during an event that is running; flags change only between events.

After the new panel has run cleanly for a reasonable stretch, remove the old implementation and the flag. This removal has to happen before the final date, not after it, which is why the buffer matters. Finish with a full rehearsal that mimics a large event: many simultaneous audience members posting questions and responses, several moderators active, deliberate network drops, and a backend restart in the middle.

## Risks, open questions and verification

The main risks, with what to do about each:

- Live data and server snapshots disagree. The mitigation is the handoff design in phase three and tests that inject events during the gap. If this proves hard, the fallback is to render the queue entirely on the client and keep server components for everything around it. That still counts as progress on the migration but we should note it plainly in the plan if we end up there.
- Server actions behave badly on poor networks. Moderators on weak connections could see actions hang or duplicate. Test with throttled and dropped connections, and make actions idempotent where possible. If they are not reliable enough, keep using the existing request path for moderation actions and use the server only for reads.
- Hidden client dependencies in third-party components. Found during the inventory; handled by wrapping, replacing, or leaving that subtree on the client.
- Server load. Rendering on the server moves work from browsers to our servers. With many moderators on many events at once, that load is real. Measure it in staging with realistic concurrency before widening the rollout, and watch it during early events.
- Team capacity. The migration competes with feature work for the same people and the date is fixed. If phase two or three runs long, the response is to cut scope from the interactive core, not to compress rollout and rehearsal. Shipping fewer components on the server is acceptable; shipping an untested panel to a live event is not.
- Behavior differences that moderators notice. Involve a few real moderators early, show them the staging panel at the end of phase three, and take their complaints seriously. They know which shortcuts and visual cues matter.
- Framework churn. Next.js and React server component behavior have moved quickly. Pin versions during the migration, read release notes before bumping, and do not upgrade the framework in the middle of a phase.

Open questions to settle early:

- Does the backend expose anything we can use as a sequence marker for the snapshot handoff, or do we need to ask for it?
- Which events or customers are good candidates for the first real rollout, and who owns the conversation with their producers?
- Is there an existing performance budget for the panel, or do we set one from today's measurements? Setting it from a measured baseline taken before phase two is the safer choice.
- Who signs off that the old implementation can be deleted?
- Are there accessibility requirements for the moderation panel that the new structure must preserve, such as screen reader announcements for incoming items? Check this before phase four.

How we verify, throughout:

- Take a baseline before changing anything: bundle size for the panel, time to first meaningful content on a throttled connection, time for a new item to appear after the backend emits it, and memory use after a long session. Repeat after each phase and keep the numbers in this note or a linked report.
- Keep automated tests for the logic around optimistic updates, reconnects and race handling. Add end-to-end tests that drive the panel with a simulated stream of messages and several simulated moderators.
- Run manual sessions with real moderators on staging before each widening of the rollout.
- Record every decision and every surprise as a note as it happens. If the plan changes, edit this note so it matches reality instead of adding a contradicting one beside it.

Status as of writing: nothing is migrated yet. The next action is the phase one inventory and the written decisions. Revisit the schedule once the inventory exists, and if the inventory shows that the date of 2026-12-15 is not reachable at full scope, say so right away and agree on what is cut, instead of finding out in December.
