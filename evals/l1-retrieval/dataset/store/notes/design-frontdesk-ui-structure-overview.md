---
id: 01KD883CXD5J8P4P35ZQY86Q2G
created: 2025-12-24T10:17-03:00
---

# frontdesk-ui: overall structure

This note describes how `frontdesk-ui` is put together at a general level. It is the part of ClinicSlotter that front-desk staff at small clinics actually touch. It is a server-rendered Rails layer with a modest amount of browser-side behavior on top, and it talks to the scheduling core through the same models and service objects the rest of the app uses. Nothing here is meant as a spec. It is a map so a later session knows where to look first.

## Purpose and users

The people using `frontdesk-ui` are receptionists and clinic coordinators. They book, move and cancel outpatient appointments while a patient is on the phone or standing at the desk. That shapes everything: pages must be fast to read, the common action should be reachable without much navigation, and mistakes must be easy to undo. They are not scheduling experts, so the interface hides the constraint solving and shows only the outcome and the reason when something does not fit.

## Place in the larger system

The app is a Rails monolith backed by MySQL. `frontdesk-ui` is the human-facing edge of it. Behind it sit the availability and room logic, the background job layer run through Sidekiq, and the FHIR integration that exchanges patient and appointment data with outside systems. The UI should not contain scheduling rules itself. It asks the core whether a slot works and renders the answer. When rules change, the screens should mostly not need to change.

## Request flow

A typical interaction starts as a normal Rails request to a controller, which loads the records it needs, calls a service object for anything non-trivial, and renders a view. Partial updates, such as refreshing a list of free slots after a filter changes, are handled by returning a fragment that the browser swaps in. Full page reloads are still the fallback and should always work. The aim is that a flaky connection at a small clinic degrades into slower pages rather than broken ones.

## Controllers

Controllers in `frontdesk-ui` are kept thin. They handle authentication, find the clinic context for the current user, permit parameters, delegate, and pick a response. Appointment actions, patient lookup, calendar browsing and room views each have their own controller. Shared behavior, such as scoping everything to the signed-in user's clinic, lives in a base controller or concern so it is hard to forget in a new action. If a controller starts to hold business logic, that logic belongs in a service object instead.

## Views and layout

Views are ERB templates with a common layout that carries the navigation, the current clinic name, the signed-in user and a flash area. Pages are built from small partials: an appointment row, a slot cell, a clinician header, a room badge. The same partials are reused between the day view, the week view and search results, so a visual change happens in one place. Helpers format times in the clinic's own time zone, never the server's.

## Calendar and slot grid

The central screen is the calendar grid. It shows time down one axis and either clinicians or rooms across the other, with appointments as blocks and open capacity as clickable cells. The grid is the most expensive page to render, so it loads its data in as few queries as it can and avoids per-cell lookups. Unavailable time, such as leave or a closed room, is drawn differently from time that is merely booked, because staff need to tell those apart at a glance.

## Booking flow

Booking is a short sequence: pick or create a patient, choose an appointment type, choose a clinician or accept a suggestion, then confirm a slot. Each step is a normal page or a fragment of one, and state is carried in the request or a lightweight draft record rather than deep in the session. The final confirm step calls the scheduling core, which rechecks availability and room constraints at that moment. The UI must handle the case where a slot was taken between viewing and confirming, and it does so by returning to the choice step with the reason shown.

## Rescheduling and cancellation

Moving an appointment reuses the booking screens with the existing appointment preloaded, so the staff member only changes what is different. Cancellation asks for a reason from a short fixed list and keeps the record rather than deleting it. Both actions leave a visible history on the appointment so a colleague can see what happened. Notifications to the patient, where enabled, are queued through background jobs and are not sent inline from the request.

## Patient lookup

Patient search is a frequent first step. It accepts partial names and other identifying details and returns a short list, with the best match first. Because patient data can come from FHIR exchanges as well as local entry, the lookup page needs to cope with records that are partly filled in. The UI shows what it has and flags gaps instead of refusing to display the patient. Creating a new patient from the booking flow is possible but nudges the user to check for duplicates first.

## Constraint feedback

When the core rejects a booking, `frontdesk-ui` translates the failure into plain language: the clinician is not available then, the room is in use, the appointment type needs a room the clinic does not have free. Internal error codes are not shown. The mapping from core failure reasons to user messages sits in one place, so wording can be improved without touching each screen. Where the core can offer alternatives, they are shown as nearby slots the user can pick directly.

## Background work and live updates

Anything slow or not needed for the immediate response is pushed to Sidekiq: reminders, outbound FHIR messages, bulk recalculation after a schedule change. The UI does not wait on these. It shows a pending state and picks up the result on the next load or via a small periodic refresh. There is no promise of instant cross-user updates; two staff members looking at the same day may briefly see different things, and the confirm-time recheck is what protects correctness.

## Authorization and clinic scoping

Every user belongs to a clinic, and every query in `frontdesk-ui` is scoped to it. Roles are coarse: front-desk staff can book and move appointments, while coordinators or admins can also manage availability views and settings. Authorization checks happen in controllers and in query scopes, not only by hiding buttons. Any new screen should be checked for whether it could leak another clinic's data through an unscoped lookup.

## Front-end behavior

The JavaScript is intentionally small. It handles things like keeping focus sensible after a fragment swap, date pickers, keyboard shortcuts for common actions, and confirm dialogs for destructive steps. It does not own application state. Styles are plain and consistent, with a layout that works on a desk monitor first and a tablet second. Phones are not a design target for this component.

## Accessibility and speed of use

Staff use the screens all day, so keyboard navigation matters. Forms have proper labels, focus order follows the visual order, and color is never the only signal for status. Defaults are chosen to save keystrokes: the date is today, the clinic is the user's own, the first available slot is preselected where that is safe. Shortcuts are documented on a help page inside the app.

## Testing approach

Controllers and service objects are covered by ordinary Rails tests. For `frontdesk-ui`, the more valuable coverage is at the flow level: book, move, cancel, and the conflict case where a slot disappears. Those run against a real database rather than mocks, since constraint behavior depends on actual queries. Visual details are checked by hand when a layout partial changes, because the grid is easy to break in small ways.

## Deployment notes

The app runs on Heroku, with the web process serving `frontdesk-ui` and a separate worker process handling Sidekiq. Assets are compiled during the build. Because the UI relies on the database for most state, a deploy that changes schema needs care about ordering so a running page does not hit a missing column. Time zone handling is the thing most likely to differ between environments and should be rechecked when something looks off by an hour.

## Known rough edges

The calendar grid does more work than it should on busy days, and the partial structure for it has grown by accretion. Error wording is not fully consistent across the older screens. Patient duplicate detection is simple and can miss near matches. Live updates are weak by design. These are noted so nobody is surprised, not as a plan to fix them in any particular order.

## Where to start when changing something

For a visual change, find the partial that renders the element and edit it there. For a booking rule, do not edit the UI; change the scheduling core and then adjust the message mapping if the failure reason is new. For a new screen, begin from the base controller so clinic scoping comes for free, reuse the existing row and slot partials, and add a flow-level test before polishing.
