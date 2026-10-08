---
id: 01K1N0PT0185QDB4VS4WH7V07Q
created: 2025-08-02T06:09-03:00
---

# frontdesk-ui week view gotchas (second pass)

Loose notes on the week view in `frontdesk-ui`. I wrote these from memory of a debugging session and did not check whether someone already wrote this up, so expect overlap with an older note on the same screen. Nothing here is a spec. It is a list of things that surprised us and that a front-desk user would notice before we did.

The short version: the week view looks like a plain grid of days and slots, but it is really three things drawn on top of each other. They are the clinician's availability, the room constraints, and the appointments already booked. Each comes from a different place and goes stale on its own schedule. Most of the confusing bugs are one of those layers being out of step with the others, not a rendering bug.

## What the week view is actually showing

The grid is built server side by Rails and sent as HTML. A small amount of client code handles navigation between weeks and the click-to-book popover. The grid is not a single query. The controller gathers availability windows for the visible clinicians, then the rooms they can use, then the booked appointments, and merges them into per-day columns.

Things worth remembering about that merge:

- A slot is shown as open only when the clinician is available and at least one allowed room is free. If either side is missing it looks the same as "fully booked" unless you look at the tooltip. Staff read grey as "full" when it sometimes means "no clinician window was defined for that day".
- The merge happens per request. There is no stored snapshot of what the week looked like. Two staff members refreshing a few seconds apart can legitimately see different grids.
- The visible range is decided by the clinic's configured working hours, not by the earliest or latest appointment. An appointment outside the configured hours exists in MySQL but is not drawn. This has caused a few "the patient is booked but I can't see it" reports.

If a report says "the slot is missing", first work out which of the three layers lost it. That is faster than reading the template.

## Week boundaries and time zones

This is the one that hurts most. The week starts on a day that depends on the clinic's locale setting, and the server may be running with a different default. Heroku dynos run in UTC unless the app says otherwise, so anything that uses the server's idea of "today" or "beginning of week" is wrong for clinics in other zones around the edges of the day.

Symptoms we saw:

- Late-evening appointments appearing on the next day's column.
- The "today" highlight sitting on the wrong column for a stretch of hours each day.
- Weeks that begin on the wrong day for one clinic but not for another, because one clinic had its locale configured and the other relied on the default.
- Daylight-saving weeks showing a slot that does not exist or hiding one that does. The grid is built from fixed-length steps, so the changeover day drifts by an hour unless the code builds each slot from the clinic's local time rather than adding durations.

Rule of thumb: always convert to the clinic's zone before computing the week range, and build slot times from local wall-clock values. Do not add intervals to a start timestamp. Store UTC, display local, and do the week arithmetic in local.

When testing, use a clinic whose zone is far from UTC and run the test around the changeover week. A test that runs in UTC passes and proves nothing.

## Room and clinician columns

There are two ways to slice the grid: by clinician or by room. They are not symmetrical, and that is where people get caught.

- In the clinician view, a column is one clinician. A slot's open state depends on that clinician's availability and on whether any suitable room is free.
- In the room view, a column is one room. A slot is open when the room is free, but a free room does not mean anyone can use it. A room can look open with no clinician available at all.

So the room view is optimistic and the clinician view is stricter. Front-desk staff who mostly use the room view sometimes try to book into a slot that the booking step then rejects. The rejection is correct. The grid is just not showing the clinician constraint there. If we ever fix this, the cheapest fix is a visual hint on room-view slots with no available clinician, not a change to the booking logic.

A second problem is rooms with restrictions, such as a room only suitable for certain appointment types. The grid colors a slot open if the room is free, but the restriction is only checked when an appointment type is chosen. Choosing the type after clicking the slot can therefore turn an "open" slot into a refusal. Ordering matters here: pick the type first, then the grid should dim the rooms that do not fit. Right now it does not re-render on type change unless the page is reloaded.

## Stale grids and background jobs

Sidekiq jobs change data that the week view reads. Examples are availability imports, recurring appointment generation, and cancellations that free a room. The grid does not subscribe to any of that. It is accurate at page load and then slowly wrong.

What this looks like in practice:

- A clinician's availability is updated by an import job. The front desk still has the old week open and keeps booking into a window that no longer exists.
- A recurring series is expanded by a job after the booking is confirmed. For a short time the later occurrences are not on the grid, and another user can book over them. The server-side conflict check catches the overlap, but the user sees a refusal for a slot that looked free in their window.
- A job fails and retries. The grid shows the state between attempts, which matches neither the before nor the after.

The working advice for staff is to refresh before confirming anything that involves a recurring series or a recently changed schedule. The better engineering answer is to include a version or last-changed marker in the grid response and have the booking request send it back, so the server can say "your view is out of date" rather than a generic conflict. We have not done that. Treat it as a known gap.

Be careful with the retry behavior of the jobs too. If a job is not idempotent, a retry can double-create availability windows, and the grid then shows a clinician as available in a doubled block that cannot be told apart from a single one. The only visible hint is that removing one window does not close the slot.

## Caching on Heroku

The grid fragments are cached to keep the week view responsive for clinics with many clinicians. The cache key has to include everything that changes the output. We have got this wrong at least twice, and each time the symptom was "it shows the old thing until some unrelated change pushes it out".

Things that must be in the key, or the cache must be cleared when they change:

- The clinic and the visible week.
- The view mode (clinician or room).
- The last-updated marker of availability, rooms and appointments in that range, not just appointments.
- The user's role, if the role changes what details are visible.
- The clinic's locale and zone settings, since they move the week boundary.

The room and availability markers are the easy ones to forget, because the code that builds the key is usually written while thinking about appointments. A room being taken out of service does not touch any appointment row, so a key made from appointments alone will keep showing the room as usable.

On Heroku there is more than one dyno process, and the cache store needs to be shared between them. If someone configures a per-process memory store for convenience, each dyno has its own copy and users see inconsistent grids depending on which dyno served the request. This looks like random flicker and is hard to reproduce locally, where there is one process. Check the configured cache store before debugging anything else when the complaint is "it differs between refreshes".

Restarts and deploys clear whatever is held in process memory, which tends to make the problem disappear right after a deploy and come back later. That is a useful tell.

## FHIR sync effects on the grid

Appointments and schedules are exchanged through HL7 FHIR. Inbound resources can create, move or cancel appointments, and the mapping to our room and clinician model is not always one to one. A few observations:

- An inbound appointment may name a practitioner we have, but no room. We still need to draw it. It is placed in the clinician view normally and in the room view as unassigned, and in the room view that is easy to miss, because it appears in a separate strip rather than a column.
- Status mapping is lossy. Some external statuses collapse into the same internal state, so the grid cannot always tell a tentative booking from a confirmed one. Staff have asked for different shading and we cannot give it truthfully until the mapping keeps the distinction.
- Inbound changes arrive through background processing, so everything in the stale-grid section applies. An external system moving an appointment shows up after the job runs, not when the user thinks it should.
- Time values in inbound resources may carry their own offsets. If they are normalized at the wrong point, the appointment shows up on the right day in the data and the wrong day in the grid, which looks like a week-boundary bug but is an import bug. Compare the stored UTC value with what the source sent before touching the view code.

When a clinic says "the grid doesn't match the other system", ask which one they believe and when they last refreshed. Then compare the raw stored records. Do not start from the view.

## Small shape of the data flow

This is only to keep the order straight in my head; it is not a diagram of the real classes.

```
frontdesk-ui -> Rails -> MySQL
                  |
                  +-> Sidekiq -> HL7 FHIR
```

The grid reads from MySQL through Rails. Sidekiq and FHIR write into MySQL on their own timing. The browser sits furthest from all of that and sees the oldest copy.

## Things tried that did not help

- Polling the whole grid on a timer. It made the stale problem look smaller, but it also reset the popover while the user was in the middle of booking, which is worse than a stale grid. If polling comes back, it needs to leave an open popover alone and only update closed cells.
- Making the grid fully client rendered. This moved the week arithmetic into the browser, where the browser's time zone replaced the clinic's. Staff using a laptop set to some other zone, for example when travelling, saw the week shifted. Keep the week math on the server and in the clinic's zone.
- Shortening the cache lifetime as a catch-all. It hides missing keys without fixing them, and it adds load for the larger clinics. Fix the key.
- Locking slots when the popover opens. It avoids double booking in the short term, but locks are left behind when someone closes the tab, and then the slot looks taken for everyone else until the lock expires. If locks are added again, they need a clear expiry and a visible indicator of who holds one.

## What I would check first next time

In the order I would actually do it, when someone reports a wrong or missing slot:

- Ask which clinic, which view mode, and roughly when they last refreshed.
- Check whether the clinic's zone and week-start settings are set explicitly or left to defaults.
- Look at the raw availability, room and appointment rows for that clinician and day, ignoring the grid.
- Check whether a Sidekiq job touching that clinic ran or retried recently.
- Check the cache store configuration and whether the key covers rooms and availability.
- Only then look at the template and the client script.

The pattern in nearly every case so far has been that the data was right and the display was out of step with it, or the other way round: the display was right and the data had been changed underneath it. The template itself has rarely been the culprit.

## Open questions

- Should the grid carry a version marker so the booking step can reject stale views with a clear message? I think yes, but it touches every booking path.
- Should the room view show clinician availability at all? Hiding it is simpler. Showing it is more accurate. Staff are split, and the clinics that mostly use rooms want it left alone.
- Can the FHIR status mapping keep tentative and confirmed apart? That is a data question first and a UI question second.
- Is there a single place where the clinic's zone is applied, or is it applied by each caller? I suspect the second, which would explain why the boundary bugs keep reappearing in different places. Worth an audit of every place that computes a start or end of day or week.
- Who owns the cache key definition? Right now whoever adds a new data source to the grid has to remember to add it to the key, and nothing fails when they forget. A test that changes each input layer and asserts the output changes would catch this cheaply.

If the older note on this subject disagrees with anything above, trust whichever one was checked against the running app more recently, and merge the two rather than keeping both.
