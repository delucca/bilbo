---
id: 01JWCPC15EX93R0T621G2717CA
created: 2025-05-28T21:16-03:00
---

# frontdesk-ui week view hits H12 Request timeout on large clinics

The week view of frontdesk-ui triggers the Heroku error `H12 Request timeout` for clinics with `40 clinicians`. Smaller clinics do not see it, or not often enough to get reported. If a front-desk user at a large clinic says the week screen hangs and then shows a generic application error page, this is the first thing to suspect. Read this note before touching the view, the Heroku settings, or the Sidekiq setup.

The short version: the week view builds its whole page inside one web request. The work in that request grows with the number of clinicians. At `40 clinicians` the request runs past what the Heroku router will wait for. The router gives up, the user sees the failure, and the Rails process often keeps working on the abandoned request anyway.

```
Heroku error: H12 Request timeout
clinic size: 40 clinicians
view: week
```

## Symptom

The front-desk user opens the schedule and switches to the week view. The page spins for a long time. Then the browser shows the Heroku error page instead of the schedule. The day view for the same clinic usually still works. Reloading the week view usually fails the same way, though it can occasionally succeed when the caches are warm. That makes the bug look flaky, but it is not random. It depends on clinic size and on whether the data has already been loaded.

In the Heroku logs the router line carries the `H12 Request timeout` code. The Rails log for that request may show no completion line at all, or a completion line much later than the router gave up. Seeing the router error without a matching Rails error is normal for this problem. Do not read it as "Rails never failed, so the problem is in the router". The app was simply too slow.

Users often describe it as "the app is down". It is not down. Other screens keep responding. Only the week view for large clinics fails. Say this to support staff so they do not escalate it as an outage.

## Who is affected

Clinics with `40 clinicians` or more are the confirmed case. Clinics well below that size load the week view without trouble. We have not pinned down the exact edge, because the cost depends on more than head count: how many appointments are booked, how many rooms exist, and how much availability each clinician has defined. A clinic with fewer clinicians but very dense schedules could in principle hit the same wall. Treat the clinician count as the clearest marker, not the only one.

Small clinics are the main users of ClinicSlotter, so most customers never see this. The affected clinics are the larger ones, which tend to be the ones that care most about the week view, because planning across many clinicians is where a week overview helps most. So the failure lands on exactly the users who need the screen.

## Why it happens

The week view asks for a lot of data at once. For every clinician it needs availability across the whole week, the booked appointments, and the room constraints that decide which slots are actually usable. The page then renders a grid that combines all of that. With few clinicians this is cheap. With many, the amount of data and the number of queries both grow, and the rendering step grows with them.

The pattern we suspect, and have not fully proven, is per-clinician work repeated inside the request: a query per clinician, then a query per day, then checks against room constraints for each candidate slot. When the clinician count rises, the total work rises faster than linearly, because each clinician multiplies the room and slot checks. MySQL is not the only cost. Building the objects in Rails and rendering the template also take real time at this size.

The important point for anyone fixing it: the cause is the amount of work done synchronously in a single web request. It is not a bad deploy, not a Heroku outage, and not a MySQL outage. Raising a timeout somewhere would not fix it, and on Heroku the router limit cannot be raised by configuration anyway.

## How the Heroku router fits in

The `H12 Request timeout` error comes from the Heroku router, not from our code. The router waits a fixed time for the web dyno to start answering. If no answer begins in that time, the router returns the error to the client and closes its side. The dyno is not told to stop. It carries on with the request, uses memory and a database connection, and finishes later for nobody.

Two consequences matter. First, when a user reloads in frustration, several abandoned requests pile up on the same dynos, and the problem gets worse. A busy large clinic can slow itself down this way, and it can slow down other users who share the dynos. Second, the application-side timeout settings do not help unless they fire before the router limit. A Rack-level timeout that is longer than the router limit never gets a chance to act.

Because this limit is fixed by the platform, the fix has to be to make the request answer sooner or to take the heavy work out of the request.

## What does not fix it

Several things were tried or discussed and should not be repeated without a new reason.

- Adding more web dynos. Each request is slow on its own, so more dynos only let more slow requests run side by side. It can even hurt, since more concurrent heavy requests put more load on MySQL.
- Using a bigger dyno type. It helps a little with rendering but not with the query pattern, and the cost is real.
- Asking users to reload. Warm caches sometimes make the second try succeed, but this is luck and it creates the pile-up described above.
- Raising timeouts in Rails or the web server. The router limit comes first.
- Blaming FHIR. The HL7 FHIR integration is not on the path of the week view render as far as we know. Do not go hunting there first.

If someone proposes any of these as the fix, point them to this note.

## Likely fixes, in the order I would try them

First, measure. Reproduce with a clinic that has `40 clinicians`, or a seeded copy of that shape, and record query counts and time spent per step for one week view request. Do not guess which step dominates. The per-clinician query pattern is the main suspect, but the numbers should decide.

Second, remove repeated queries. Load availability, appointments and room data for all clinicians in bulk and group them in memory. Use eager loading for associations the template touches. Check the template for calls that trigger a query inside a loop. These are cheap changes and may be enough on their own.

Third, cut the amount rendered. The week view does not need to build every cell in full detail for every clinician at once. Options are to paginate clinicians, to let the user pick a subset, or to load the grid in parts after the first paint. Splitting by clinician group fits how large clinics already think about their staff.

Fourth, move expensive computation out of the request. Slot usability against room constraints could be precomputed by a Sidekiq job when availability or bookings change, and stored so the week view reads a ready result. This is the largest change and brings a staleness risk, so it should come last. If this is done, the view must handle the case where the precomputed data is not ready yet, and say so to the user rather than show a wrong grid.

Fifth, cache. Fragment or data caching of per-clinician week data can help, but invalidation is the hard part. A booking in one room can change what other clinicians can offer. Do not cache without a clear rule for when entries are dropped.

## Things to watch when changing it

Correctness comes before speed here. Front-desk staff book real patients from this screen. A faster grid that shows a slot as free when the room is taken is worse than a slow one. Any change that moves room-constraint logic must keep the same results as the current code, and that should be checked with tests that cover clinician availability and room conflicts together.

Watch for double booking if precomputed data is used. Booking itself must still check the live state at the time of the write, not trust the grid. The grid is advice, the booking action is the authority.

Keep the day view working while changing the week view. They likely share code for availability. A change meant for the week view can alter day view results without anyone noticing. Run both after any change.

Also keep an eye on MySQL load. Moving from many small queries to a few large ones can cause new slow queries if indexes do not fit. Look at the query plans for the new bulk queries, not just the timing on a small dataset.

## How to check it is fixed

A fix is done when the week view for a clinic with `40 clinicians` returns without the `H12 Request timeout` error, repeatedly, and not only on a warm cache. Test cold: restart the dynos or clear caches, then load the page. Test while other users are active, because load from others affects the result. Test a few reloads in a row to be sure no pile-up occurs.

After deploy, watch the Heroku router logs for that error code for a while. If it disappears for large clinics and the day view stays healthy, close the item. If it persists only for the biggest clinics, the margin is too thin. Reduce the work further instead of declaring victory. Leave some headroom, since clinics grow over time and the number of clinicians only goes up.

Add a regression check if possible: a test or a script that builds a large clinic and asserts the query count for the week view stays bounded and does not grow with the number of clinicians. A query count assertion catches the return of the per-clinician pattern long before users see a timeout.

## Support notes

What to tell a clinic that reports it: the schedule is not lost, no appointments are missing, and the day view can be used in the meantime. Ask them not to keep reloading the week view, since that adds load. Ask which clinic it is and roughly how many clinicians it has, which confirms the pattern. If a clinic with far fewer clinicians reports the same error, that is new information; record it here, because it would mean the cost depends on something besides clinician count, such as booking density or room count.

If the error shows up on screens other than the week view, this note does not apply. That would point at a wider problem such as database trouble or a dyno shortage, and needs a separate investigation.

## Open questions

- Which step in the request dominates: queries, object building, or template rendering? This has not been measured cleanly.
- Exactly where is the edge in clinic size where the problem starts? Only the `40 clinicians` case is confirmed.
- Does booking density matter as much as clinician count?
- Would clinics accept a paged or filtered week view, or do they need to see every clinician on one grid?
- Is any part of the week view calling out to something outside the app that could stall the request?

Update this note when any of these get answered, and fix the claims above that turn out to be wrong.
