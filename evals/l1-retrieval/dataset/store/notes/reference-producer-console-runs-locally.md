---
id: 01KM3ZZA1NEHENZEA2XS1JJ1SY
created: 2026-03-19T18:28-03:00
---

# producer-console reference

producer-console is the web app that event producers and community managers use to run a live event in TownHall Pulse. It is the Next.js front end. It talks to the Phoenix backend over WebSockets and HTTP, and the backend stores its data in CockroachDB. This note is a quick reference for working on it locally. It is not a design document.

## Codename

`kestrelui` is the internal codename of producer-console. You will see it in older chat threads, in some repository names, in dashboards and in a few package labels. When someone says kestrelui, they mean producer-console. Use `producer-console` in new notes, tickets and commit messages so searches keep working. If you grep and find nothing under one name, try the other.

## What it does

Producers open producer-console to create polls, queue audience questions, and decide what the audience sees. During an event it shows the live state: incoming questions, vote tallies, and the moderation queue. It is built for large virtual events, so the screens have to stay readable when volume spikes. Most of the work is reading a fast-moving list and acting on it quickly.

## Running it locally

Start the dev server with `npm run dev -- --port 3100`. The extra arguments after the double dash go to the Next.js dev server, which is how the port gets set. Run it from the root of the producer-console project, not from the backend project. Install dependencies first if the folder is fresh. The dev server reloads on file changes, so you rarely need to restart it.

## Why the port is explicit

The port is passed on the command line so that producer-console does not collide with other local services. The default Next.js port is often taken by another tool on a dev machine. If the app starts on a different port than you expected, something else grabbed the one you asked for, or the flag was dropped from the command. Check the command first before blaming the app.

## Backend dependency

The console does little on its own. It needs the Phoenix backend running locally, or a reachable shared environment, to show real polls and questions. Without it the pages load but stay empty or show connection errors. Start the backend first, then the console. The backend in turn needs a CockroachDB instance, so a missing database shows up in the console as a backend failure, not as a database error.

## Real-time connection

Live data arrives over a WebSocket channel from the Phoenix backend. The console joins a channel for the event being run and receives updates as they happen: new questions, vote changes, moderation actions by other producers. If the socket drops, the console should reconnect and resync. When debugging stale screens, look at the browser network tab for the socket first, before touching component code.

## Moderation view

Real-time moderation is the most sensitive part of the console. A moderator approves, rejects, or holds incoming questions, and the result has to reach the audience quickly. Several moderators can work at once, so the view must reflect each other's actions without a manual refresh. When two people act on the same question, the backend decides which action wins, and the console just shows the outcome.

## Polls

Polls are created and launched from the console. A producer drafts the question and options, then opens it to the audience, and later closes it and shows results. Live tallies update through the same socket as everything else. Keep poll editing separate from poll running in your head: edits while a poll is open are the usual source of odd bugs.

## Q&A queue

The Q&A queue lists audience questions waiting for review. Producers can promote a question to the stage list, mark it answered, or dismiss it. Ordering matters, so be careful when changing how the list sorts or when it re-renders. Reordering under a producer's cursor is a real usability problem during a busy event.

## Auth and roles

Only event staff should reach the console. Access is tied to a role on the event, such as producer or moderator, and what a person sees depends on that role. Do not assume every signed-in user can see every control. When adding a feature, decide which roles get it and check that the backend enforces it too, not just the UI.

## Environment configuration

The console reads its backend addresses from environment configuration. Keep these out of the source and out of notes. Local values differ from shared environments, so a setup that works on one machine may point somewhere else on another. If requests go to the wrong place, inspect the environment values the dev server was started with.

## Testing habits

Before calling a change done, run it against a local backend with a few fake participants and look at both the producer view and an audience view. Many problems only appear with more than one client. Pay attention to what happens when the socket reconnects mid-action. Unit tests help for formatting and state logic, but they do not replace watching a live session.

## Common problems

Empty pages usually mean the backend is not running or the socket did not connect. A wrong port usually means the start command was altered. Stale vote counts point at a dropped channel. Missing controls point at roles. Most of these are quick to rule out in that order.

## Performance concerns

Big events produce a lot of messages, so the console must not re-render the whole page for each one. Batch updates where possible and keep list items cheap. When profiling, use a busy simulated event, because a quiet one hides the problems. Memory growth in long sessions is worth checking too, since events can run for hours.

## Open questions

Things still worth pinning down later: how the console should behave when the backend is slow rather than down, what the exact reconnect policy is, and whether the old kestrelui name should be removed from remaining labels. Add answers to this note when they are settled.
