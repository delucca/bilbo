---
id: 01KK88RPRW84DWFCX0325JNW0H
created: 2026-03-09T00:03-03:00
---

# frontdesk-ui: server-rendered pages with light progressive enhancement

We decided that frontdesk-ui stays mostly server-rendered Rails views, with small bits of JavaScript added only where a page really needs them. We are not moving it to a separate single-page app. This note records the direction and the reasons, so nobody reopens it without new facts.

## Context

frontdesk-ui is what front-desk staff use all day to book, move and cancel appointments. The people using it are not technical, often work on older shared machines, and are usually on the phone with a patient while they click. The clinics are small, so there is no dedicated frontend team. Whoever touches the Rails code also touches the UI. That shaped most of the choice.

We looked at a client-heavy rewrite for a while. The pitch was snappier slot browsing and drag-and-drop rescheduling. The cost was a second build pipeline, a JSON API surface we would have to keep stable, and duplicated validation. The scheduling rules (clinician availability, room conflicts) live on the server and must stay the single source of truth. Duplicating them in the browser invites the two copies to drift, and a wrong "this slot is free" shown to staff is worse than a slow page.

## What we chose

- Pages are rendered on the server. Forms post normally and the server answers with a full page or a partial that replaces a region.
- Interactivity is added in small pieces: slot pickers refresh a region, the calendar day view updates without a full reload, and confirmation steps do not lose what the person typed. Anything beyond that needs a reason written down first.
- Availability and room checks are never decided in the browser. The UI asks the server and shows what it says. If the browser cannot reach the server, it shows that plainly and does not guess.
- Pages must work with plain form submission as a fallback. Staff on a bad connection or a stale browser should still be able to book.
- Long or slow work, such as sending confirmations or syncing with outside systems over FHIR, is handed to background jobs. The UI shows a pending state and picks up the result later. It does not block the front desk while waiting.

## Why not the alternatives

A full single-page app would give nicer transitions, but the gain is small for a screen people use in short bursts. It would also make the Heroku deploy and release process more complicated, and it would put more of the validation logic in two places. A pure no-JavaScript approach was also rejected. Some interactions, mainly seeing open slots change as a clinician or room is picked, are too clumsy with full reloads and cause double bookings when staff race each other.

We also considered a component library for the UI. We held off. The screens are few and plain, and a library adds upgrade work that a small team will not keep up with. Shared styles and a handful of reusable partials are enough for now.

## Consequences and things to watch

Concurrency is the main risk. Two staff members can look at the same open slot, and a server-rendered page can be stale by the time someone submits. The server has to reject the second booking cleanly, and the UI has to explain what happened and offer the next open options. This is a rule for every new booking screen, not an optional extra.

Keep the JavaScript small and boring. If a page starts needing a lot of client-side state, stop and talk about it before building on it. That is the signal the decision may need to change, and it should be changed on purpose, not by drift.

Accessibility and keyboard use matter here. Front-desk staff move fast and many prefer the keyboard. New screens should be usable without a mouse and should have clear focus behavior after a partial update.

Testing follows the same line: most behavior is covered through server-side and request-level tests, with a small number of browser-driven checks for the interactive pieces like the slot picker. We do not want a large browser test suite to maintain.

## Open points

- Whether the calendar day view needs live updates when another user books, or whether a manual refresh and good conflict messages are enough. Not decided.
- How much of the pending-state handling for background jobs should be shared across screens versus written per screen.
- Whether a design pass on the shared partials is worth doing before adding more screens.

If any of these push toward heavier client code, revisit this decision as a whole rather than patching around it.
