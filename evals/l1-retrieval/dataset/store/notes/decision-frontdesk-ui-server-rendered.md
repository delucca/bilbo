---
id: 01K3NCY3H032E1NZ48HVXVDHJT
created: 2025-08-27T06:14-03:00
---

# frontdesk-ui: server-rendered Turbo, not a SPA

We build frontdesk-ui as server-rendered pages using Turbo, not as a single-page app. The reason is hardware: front-desk computers at the small clinics we serve are old and cannot run heavy JavaScript well. Keeping the work on the Rails server and sending HTML keeps the browser light. This note records that decision so nobody reopens it without new facts about the clinics' machines.

## Decision

frontdesk-ui is a Rails-rendered UI that uses Turbo for navigation and partial page updates. We do not ship a client-side framework that owns routing and state in the browser. Pages come from the server as HTML, and Turbo swaps in the parts that changed.

If someone proposes a single-page app, the answer for now is no, and the reason is the old front-desk computers.

## Naming

The component used to be called `fdui-v2`. It is called `frontdesk-ui` now. If you find `fdui-v2` in old branches, tickets, chat logs, dashboards or deploy settings, it means the same component as `frontdesk-ui`. Use `frontdesk-ui` in anything new: docs, commit messages, notes, alert names.

Do not treat `fdui-v2` as a second, separate component. There is no other UI behind that name. When you rename a leftover reference, check that it points at the same thing before you change it.

```text
fdui-v2  ->  frontdesk-ui
```

## Why not a single-page app

The users are front-desk staff at small clinics. Their machines are often several years old, with little memory and slow CPUs. A large JavaScript bundle has to be downloaded, parsed and run before the screen is useful, and on those machines that wait shows up directly in the booking flow. Staff are on the phone with patients while they book, so lag costs real time.

Server rendering moves the heavy work to Heroku, where we control the resources. The browser only has to draw HTML and run the small amount of script Turbo needs.

## What Turbo gives us

- Page navigation without a full reload, so the screen does not flash while staff move between views.
- Partial updates for the parts of a page that change, such as the list of open slots after a filter is applied.
- Logic that stays in Rails controllers, views and helpers, where the team already works.
- No separate client build with its own state management to maintain.

## Consequences for the code

Views, partials and Turbo-aware responses are where UI work happens. Validation and scheduling rules stay on the server, next to clinician availability and room constraints, so the browser never has its own copy of those rules. That avoids the two copies drifting apart.

Anything that takes a long time, such as bulk rescheduling or syncing FHIR data, should not run inside a request. Hand it to Sidekiq and let the page show progress or a result when the job finishes. Keep requests short so the UI stays responsive on slow machines.

## Consequences for performance

Because the browser is weak, budget for it. Keep the page weight small, avoid large client libraries, and do not add JavaScript for something HTML and Turbo can do. Each new script should be justified against the old-hardware constraint.

Server cost goes up a little because we render more on the server. That is accepted: a slower page on the Rails side is easier to fix than a slow browser we cannot upgrade.

## Trade-offs we accepted

- Every interaction needs a round trip to the server, so a poor network connection hurts more than it would in a single-page app.
- Rich, highly interactive widgets are harder to build. If a feature really needs one, keep it small and local rather than moving the whole UI to a client framework.
- Offline use is not supported.

## When to revisit

Reopen this only if the clinics' computers change, for example if most of them are replaced with modern machines, or if a required feature cannot reasonably be built with server rendering. Without evidence of that, the decision stands. Measure on a typical old front-desk machine before arguing either way, not on a developer laptop.

## Notes for agents

- Say `frontdesk-ui` when you refer to this component.
- If a task mentions `fdui-v2`, read it as `frontdesk-ui`.
- Do not introduce a SPA framework or a large client-side dependency without explicit approval.
- Prefer Turbo and server-rendered HTML for new screens, and test on slow hardware where you can.
- Put scheduling and availability checks on the server side.
