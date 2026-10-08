---
id: 01KEA4XPP440WQG2VQC9K6F0WG
created: 2026-01-06T14:15-03:00
sources:
  - "code: apps/console/next.config.mjs"
---

# producer-console design

producer-console is the web app that event producers and community managers use to run a live event on TownHall Pulse. It is a Next.js 15 application using the App Router, built from the apps/console directory. This note records how it is put together and why, so a later session does not have to rediscover it. It stays general on purpose. Check the code for exact names and values.

## Purpose

Producers use producer-console during a live session. They create and open polls, close them, and reveal results to the audience. They read the incoming Q&A stream, approve or reject questions, pin the ones to answer, and watch audience activity. The console is the control surface. It does not serve the audience; attendees use a different client. The console must stay responsive when the event is large, because the producer is the person who has to act quickly when something goes wrong.

## Where it sits in the system

The backend is Elixir with Phoenix. It owns poll state, question state, moderation decisions and fan-out to attendees. Persistent data lives in CockroachDB, which only the backend talks to. producer-console never reads the database directly. It talks to Phoenix over two channels: ordinary HTTP requests for commands and initial loads, and a WebSocket connection for live updates. The Next.js server side renders the shell and handles sign-in, but the live event data does not pass through it.

## Framework choice and layout

Next.js 15 with the App Router was chosen so the console can mix server-rendered pages with client components. Page-level routes, layouts and loading states follow the App Router conventions. Anything that needs a live socket is a client component. Anything that is mostly static, such as the event list or settings pages, stays a server component. The rule of thumb is to keep the client boundary as low in the tree as possible, so the live panels are small islands rather than a whole client-rendered app.

## Build and repo location

The app lives in apps/console and is built from that directory. Build tooling, environment files and the Next.js config belong there and nowhere else. Anything shared with other front ends should not be copied into the app. If shared code appears, it should move to a shared package rather than be duplicated. When running or debugging, change into apps/console first; running from the repository root tends to pick up the wrong config.

## Live updates over WebSockets

Live data arrives through a WebSocket connection to Phoenix channels. The console joins a channel scoped to the event it is showing and receives pushes for new questions, vote counts, moderation changes and poll state transitions. A single connection is shared across the whole page through a provider, so panels subscribe to topics rather than opening their own sockets. Opening a socket per panel was tried early on in design discussion and rejected because it multiplies reconnect storms.

## Client state handling

Incoming pushes are merged into a client-side store that the panels read from. The store holds the latest known state for polls and questions, keyed by their backend identifiers. Pushes are applied in the order they arrive, and each push carries enough information to replace stale state instead of patching it. The aim is that a dropped message can be repaired by the next one, or by a resync, without the producer seeing wrong numbers for long.

## Reconnect and resync

Sockets drop in real events, especially on producer laptops using venue networks. On reconnect the console rejoins its channels and then fetches a fresh snapshot over HTTP before trusting further pushes. Until the snapshot lands, the UI shows a visible stale indicator rather than silently showing old counts. Backoff on reconnect is jittered so that many producers on one network do not retry in lockstep. A producer should always be able to tell at a glance whether the screen is current.

## Moderation queue

The moderation queue is the busiest part of the console. Questions arrive, are held or flagged by the backend rules, and wait for a human decision. The queue must handle bursts without freezing, so rendering is virtualized and updates are batched per animation frame instead of per message. Approve, reject and pin actions are optimistic in the UI and confirmed by the backend; if the backend refuses, the item snaps back and a short notice explains why. Keyboard shortcuts exist because producers cannot always reach for the mouse.

## Poll control

Polls move through a small set of states, such as draft, open, closed and revealed. The console only requests transitions; Phoenix decides whether a transition is allowed and broadcasts the result. This avoids two producers in the same event putting a poll into conflicting states. The controls disable themselves based on the last known state, but the backend is the source of truth and the console treats a rejection as normal, not as an error to hide.

## Multiple producers in one event

Several producers and community managers may work the same event at once. The console shows who else is present and reflects their actions as they happen through the same live channel. There is no local locking. If two people act on the same question, the first decision the backend accepts wins, and the other console updates to match. Showing the acting person's name on each decision helps the team avoid repeat work.

## Authentication and roles

Sign-in is handled at the Next.js layer, and the session is passed to Phoenix when the HTTP calls and socket connection are made. Roles decide what a person may do in an event: a community manager may moderate but not necessarily change event settings, for example. The console hides controls a role cannot use, but hiding is only a convenience. Every command is checked again by the backend, and that is the check that counts.

## Data fetching conventions

Initial page data for non-live screens is fetched on the server in the App Router. Live screens load a snapshot on the client so that the same code path serves first load and resync. Responses are not cached across producers for live data. Caching is allowed only for settings and lists that change rarely. When adding a screen, decide first whether it is live or not, since that decides which path it takes.

## Performance concerns

Large events mean large question volumes and fast-moving vote counts. The main risks are render storms and memory growth in long sessions. Counts are throttled before display, lists are virtualized, and old resolved items are trimmed from the client store after a while. The console is expected to stay open for the full length of an event, so leaks matter more here than in a typical page-by-page app.

## Error handling and visibility

Failures should be visible to the producer in plain language: socket lost, command refused, snapshot failed. Errors are never swallowed to keep the screen tidy. Client errors are reported to the team's usual error tracking, with the event and the producer's role attached but without question text, since that is audience content. Error boundaries wrap each live panel so one broken panel does not blank the whole console.

## Testing approach

Component tests cover panel behavior against a fake socket that replays recorded pushes, including out-of-order and duplicate cases. A smaller set of end-to-end tests drives the console against a local Phoenix instance for the main flows: open a poll, receive questions, moderate, close. Reconnect behavior is tested by cutting the fake socket mid-stream and checking that the stale indicator appears and clears.

## Open questions

Things not settled yet: whether to move some moderation actions to bulk operations for very large queues, how much of the client store should survive a page reload, and whether a read-only view for observers should be a role or a separate route. None of these block current work. If one is decided, update this note rather than leaving the answer in chat.
