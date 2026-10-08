---
id: 01M0X9VJ2SY1GYMWJY4FS746RM
created: 2026-08-25T17:30-03:00
---

# frontdesk-ui spec

This note is the working spec for frontdesk-ui, the screen set that front-desk staff at small clinics use to book, move and cancel outpatient appointments in ClinicSlotter. The main requirement is that frontdesk-ui must let staff book an appointment in at most 3 clicks. Everything else here either supports that budget or says what we will not trade away to meet it. If a design choice adds a click to the booking path, it needs a reason written down here.

The people using it are receptionists and clinic managers. They are often on the phone while booking. They are interrupted a lot, they know their clinic's clinicians and rooms by name, and they do not want to learn a scheduling model. The screen has to match how they talk: a patient, a kind of visit, a clinician or any clinician, a day, a time. The room is almost always an outcome of those choices, not an input.

## What the click budget means

The budget is about clicks, meaning pointer clicks or taps on a control, counted from the moment the staff member is on the main schedule view with a patient already identified, to the moment the appointment is confirmed and saved. The limit is 3 clicks. We count it this way because patient lookup is a typing task and varies a lot, and we do not want to hide a slow lookup inside a fast click count.

What counts as a click:

- Choosing a slot on the schedule grid.
- Choosing or changing a value in a picker, such as visit type or clinician.
- Pressing the confirm control.

What does not count:

- Typing into the patient search box and selecting the matching patient from the result list. This is the identification step and is measured separately.
- Keyboard use. Staff who prefer the keyboard should be able to reach the same result with fewer pointer actions, and that is encouraged, but the budget is stated in clicks so that it can be tested the same way for everyone.
- Dismissing a toast or a banner. Those must never block the path anyway.

The intended shortest path is: click an open slot on the grid, which opens a compact booking panel with the patient, visit type, clinician and room already filled in as far as the system can infer them; adjust at most one thing if needed; click confirm. In the best case that is two clicks. The third click is the allowance for the one correction that staff most often need, usually visit type or clinician. If a typical booking needs more than that, the defaults are wrong and the fix is better defaults, not more room in the budget.

The budget applies to the common booking. It does not apply to recurring series, bulk rescheduling after a clinician is out, or bookings that need an override of a hard constraint. Those flows can be longer, but they should say clearly that they are a different flow and should not leak into the common path.

## Booking flow

The flow starts on the day or week schedule grid. Columns are clinicians, rows are time. Open time is visible as open time, not as a blank. Closed time, blocked time and time outside a clinician's availability are visibly different from each other, because staff will ask why a slot cannot be picked and the answer should be readable from the grid.

### Slot selection

Clicking an open slot opens the booking panel anchored to that slot. The panel does not navigate away from the grid, so the context of the day stays visible. The slot's clinician and start time are taken from where the click landed. The default length comes from the visit type, which is why the visit type default matters so much.

### Defaults

Defaults should do most of the work. The order of preference is:

- If the patient has a recent visit with this clinic, offer the same visit type and the same clinician as the most recent one, unless the clicked column says otherwise. The clicked column always wins for clinician.
- If the patient is new, default to the clinic's configured new-patient visit type.
- If the staff member started from a patient screen instead of the grid, default to the earliest open slot for the preferred clinician and show it as a suggestion that can be changed with a click.

Room is chosen by the server from the constraints, not by the user. The panel shows the chosen room as plain text with a small way to change it. Changing the room is allowed but is not part of the common path. If no room fits, the panel says so in plain words and offers the nearest alternatives, rather than failing silently.

### Confirming

Confirm saves the appointment and closes the panel. The grid updates in place so the new appointment appears at once. There is no separate success page. A short, non-blocking message confirms what was booked, with the patient, clinician and time in the same words used on the grid.

Confirm must be safe to press twice. A double click should not create two appointments. The request carries an idempotency token generated when the panel opens, and the server treats a repeated token as the same booking.

### What the panel never asks for

The panel should not ask for anything that the system already knows or that can be collected later. Contact details, insurance and notes are editable from the appointment afterward. Required fields in the panel are only those without which the appointment cannot be placed: patient, visit type, clinician, time. Anything else that someone wants required for a clinic goes through clinic configuration and is argued for on its own, because each required field is a threat to the budget.

## Availability and room display

The grid shows what the scheduling core has computed. The front end does not decide availability itself. It asks the server for the state of each slot and renders what comes back. This is deliberate: two copies of the availability rules would drift, and the errors would show up at the front desk in front of patients.

The server's answer for a slot has to carry enough for the grid to show why a slot is not open. The reasons we expect to show are: clinician not working, clinician already booked, no suitable room, clinic closed, and blocked by a clinic manager. These map to different looks on the grid, with a short label when the staff member hovers or taps the slot. Labels are written for receptionists, not engineers.

Rooms follow the same principle. A room constraint may be about the kind of room a visit type needs, equipment, or a room being shared between clinicians. The grid does not show every room at once. It shows the room that would be used if the slot were booked, and the room view is a separate screen for managers who want to see room use across the day.

When availability changes while a staff member has the panel open, for instance another receptionist books the same slot, the panel must notice before or at confirm, tell the user in plain words, and offer the nearest open alternatives with one click. The panel must not overwrite the other booking, and it must not lose what the user already chose.

## Errors and edge cases

The main rule is that a failure must leave the staff member knowing what to do next, and must not cost them the data they entered.

- Conflict at confirm. Someone else took the slot. Show the alternatives for the same clinician first, then other clinicians for the same visit type. Keep the patient and visit type.
- No room available. Say that no room fits, say which constraint is the problem if the server tells us, and offer the nearest times where a room exists.
- Clinician unavailable. Usually a grid issue, since unavailable slots should not be clickable. If it still happens because of a stale grid, refresh the grid and keep the panel contents where they still make sense.
- Server error. Show a plain message and keep the panel open with its contents. Offer a retry. Never clear the form on error.
- Lost connection. Disable confirm and say the connection is gone. Staff in small clinics often have patchy networks, so the panel should recover without a reload when the connection returns.
- Patient with an existing appointment at the same time. Warn before confirm, but let a manager override it. This warning is not part of the click budget when it does not fire, and when it fires it is an accepted extra step.
- Overrides. Overriding a hard constraint is restricted to roles that have the permission, is recorded, and lives in a separate flow from the common booking.

Accessibility is not optional. The grid and panel must be usable by keyboard and by screen reader, focus must move into the panel on open and return to the clicked slot on close, and color alone must never carry the meaning of a slot state. The labels used on hover must also be available without hover.

Time zones need care. The clinic's own time zone is the one shown. Staff should never see a time shifted into the browser's zone if that differs. All stored times stay unambiguous on the server; the front end is only responsible for showing the clinic's local time correctly, including across daylight saving changes.

## Implementation notes

frontdesk-ui lives inside the Rails application. It is server-rendered pages with targeted partial updates, not a separate single-page app. The grid and the booking panel are the two places where we allow richer client behavior, and each has to degrade to something usable if scripts fail to load. The reason is practical: small clinics have older machines and we do not have a front-end team to carry a heavy client.

The pieces, in general terms:

- A schedule grid view that renders a day or week for a set of clinicians, fed by a query object that returns slot states.
- A booking panel that is loaded on demand when a slot is chosen and posts to a booking endpoint.
- A patient search box with fast, forgiving matching on name and date of birth, with results that can be picked without leaving the keyboard.
- A small set of shared components for slot labels, so the grid, the panel and the room view all describe states in the same words.

The booking endpoint validates through the same scheduling code that the background jobs use. There is one place where the rules live. If a rule changes, the grid, the panel and the jobs change together. The endpoint returns structured reasons on failure, and the front end turns those into the plain messages described above, so wording can change without changing the server.

Performance matters for the budget indirectly. A click that takes a long time to respond feels like a failed click and users repeat it. The grid should render from a single query per view, avoid per-slot queries, and the panel should open from data already on the page where possible. The MySQL queries behind the grid are the usual suspects when it gets slow, so check indexes and eager loading there before touching the front end.

Background work is handled with Sidekiq and must not be on the booking path. Things like sending reminders, syncing to external systems and recomputing derived availability happen after the appointment is saved. The panel confirms on the saved appointment, not on the completion of those jobs. If a job fails, the staff member should see it only where it matters, such as a sync status on the appointment, not as an error at booking time.

Exchange with other systems uses FHIR resources. frontdesk-ui does not speak FHIR directly. It works with the application's own models, and the integration layer maps them. The one consequence for the interface is that fields we expose should be ones that map cleanly, so we do not collect something in the panel that cannot be sent on.

The app runs on Heroku, so anything that assumes a local file system or a long-lived process is suspect. Uploaded files, if any ever appear in this UI, go to external storage. Page responses should be fast enough that the dyno's request limits are never a concern for the booking path.

## Verification and acceptance

The click budget is a testable claim, so test it. The acceptance check is a scripted run through the common booking on the grid that counts pointer actions from the identified patient to the saved appointment, and fails if the count goes above the limit of 3 clicks. This check belongs in the system tests for frontdesk-ui and runs with the normal suite. A manual walkthrough by someone who works at a front desk is also worth doing before each release that touches the grid or the panel, because a script can pass while the screen is still awkward.

What the automated checks should cover:

- The best case: open slot, defaults right, confirm.
- The correction case: one picker changed, then confirm. This is the worst case that must fit the budget.
- Double confirm creates one appointment.
- Conflict at confirm keeps the entered data and offers alternatives.
- No room available gives the plain message and alternatives.
- Keyboard-only completion of the common booking.
- Time display in the clinic's zone, including across a daylight saving change.

What the manual check should cover: reading the grid with a quick glance, finding out why a slot is closed, recovering from a conflict, and whether the defaults match what staff would have chosen anyway. Record the findings against this spec rather than in chat, so the defaults can be tuned from real use.

A regression that adds a click to the common path is a bug, even if the feature it came with is useful. Either fix the defaults, move the new step off the common path, or write the trade-off down here with the decision and the person who made it.

## Open questions

- Whether the suggestion for a patient who comes from a patient screen should be a one-click booking or should go through the grid. The first is faster; the second keeps one mental model.
- How to treat visit types that need more than one resource at once, such as a clinician plus special equipment. Defaults work, but the panel may need a clearer way to show why a time was chosen.
- Whether clinic managers should be able to tune the defaults per clinic, or whether that is a way to produce clinics where the budget no longer holds.
- How much of the grid should work offline for clinics with poor connectivity. For now the answer is none beyond recovering cleanly when the connection returns.

Until these are settled, keep the common path as described above, keep the panel small, and treat the limit of 3 clicks as the thing that gets protected first.
