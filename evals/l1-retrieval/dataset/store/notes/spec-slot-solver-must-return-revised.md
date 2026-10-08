---
id: 01M319D24WKR8XFJECVGJCS4TD
created: 2026-09-21T03:10-03:00
---

# slot-solver latency budget spec

This note replaces the earlier note about "slot solver must return". The new value is a 95th percentile budget of 1200 ms per proposal for slot-solver, up from the earlier 800 ms budget. Anything that still quotes the old figure is out of date and should be corrected when you touch it.

One naming point so nobody searches for the wrong thing: fitr-v1 was the previous name of slot-solver. The component is called slot-solver now. Old tickets, old branches, old dashboards and some log lines may still say fitr-v1. They mean the same component. Do not treat fitr-v1 as a separate service, and do not reintroduce the name in new code, docs or alerts.

```
component: slot-solver
formerly: fitr-v1
p95 budget per proposal: 1200 ms
previous budget: 800 ms
```

## What the budget covers

The budget is about one proposal. A proposal is what slot-solver hands back to the front-desk screen when a staff member asks for a place to put an appointment: one or a few candidate slots, each tied to a clinician and a room, ordered by how well they fit. The clock starts when the request reaches slot-solver and stops when the ranked candidates are ready to be returned to the caller. Time spent in the browser, on the network between the browser and Heroku, and in page rendering is not counted here. Those have their own concerns and are not part of this spec.

The number is a 95th percentile. That means roughly nineteen out of twenty proposals should finish inside 1200 ms, and the slow tail is allowed to exceed it. This is not a hard timeout. A proposal that takes longer than 1200 ms is not an error by itself and must still return a correct answer. What the budget gives us is a target for monitoring and a pressure on design: if the measured 95th percentile climbs past 1200 ms and stays there, that is a defect to be fixed, not a figure to be argued about.

The budget applies to proposals as a whole population, not to a single clinic or a single day. A small clinic with a quiet calendar will usually be far under it. The case that matters is the busy clinic at the start of the week, with many clinicians, shared rooms, and a calendar already crowded with existing bookings. If the budget holds there, it holds everywhere. When checking, look at the busiest realistic clinic, not the average one.

Measure the budget on the solving work as seen by the application, not on a synthetic micro-benchmark of the search routine alone. The reason is simple: front-desk staff feel the whole proposal, including loading the availability data and the room constraints, and a fast search that sits behind a slow data load is still a slow proposal. So the clock includes data loading for the request, constraint preparation, the search itself, and ranking.

## Why it moved from 800 ms to 1200 ms

The earlier budget was set when the solver considered clinician availability and a fairly simple room model. Since then the room constraints grew richer, and the availability data now reflects more of what clinics actually do: partial days, blocked times, shared equipment tied to rooms, and clinicians who work across more than one room. Honouring all of that costs more time per proposal. Holding the old figure would have meant dropping constraints or returning weaker answers, and neither is acceptable for a scheduling tool where a wrong slot means a patient arriving at a room that is not free.

The decision was to keep correctness and relax the budget. Front-desk staff are usually on the phone or talking to a patient when they ask for a slot, and the difference between the old and new figure is small enough that it does not change how the screen feels in practice. A proposal that is a little slower and right is better than one that is fast and needs to be corrected by hand.

This was a deliberate, one-time change of the target and not a sign that the solver is allowed to drift. The new figure is a ceiling for the tail, and we expect the typical proposal to remain well below it. If someone proposes raising it again, that should go through the same kind of discussion, with measurements attached, and this note should be updated in place rather than a second note created.

## Where the time goes

The cost of a proposal splits into a few parts, and it helps to keep the split in mind when something gets slow.

First, loading the data the solver needs. That includes the existing appointments in the window being searched, the working hours and exceptions for the clinicians involved, and the room definitions with their constraints. This comes out of MySQL through Rails. It is the part most likely to be hurt by missing indexes, by loading more rows than the window needs, or by repeated per-record queries. When a proposal is slow for one clinic and fine for another, suspect this part first.

Second, preparing constraints. Raw availability has to be turned into free intervals per clinician and per room, and the intervals have to be intersected where an appointment needs both. This is plain interval work and should be linear in the amount of data, not worse. If preparation starts to dominate, something has probably been recomputed that could have been computed once per request.

Third, the search itself. The solver walks candidate slots in a sensible order and stops once it has enough good candidates, instead of enumerating everything. The stopping rule matters: it is what keeps the tail under control on a crowded calendar. Changes to the search should be checked against the crowded case, not the empty one.

Fourth, ranking and shaping the response. Candidates are scored and ordered, and then turned into the structure the caller expects. This is normally cheap. If it is not, something is being serialized more than once.

Where FHIR data is involved, for example when availability or appointment information arrives in or is exchanged as HL7 FHIR resources, the cost of parsing and mapping those resources belongs inside the budget as well, because it happens on the path of the request. Do not move that work out of the timed path just to make the number look better; if it can be done ahead of time, do it ahead of time for real, and say so.

## What counts and what does not

Counted inside the 1200 ms: loading data for the request, building free intervals, running the search, ranking, and building the response object. Also counted: waiting on the database, since the user waits on it too.

Not counted: queueing time before the request reaches slot-solver at the web tier, and any work done later in the background. Sidekiq is used for work that does not need to block the front-desk screen, such as notifications and other follow-up after a slot has been chosen. Such jobs are not part of a proposal and must not be moved into the proposal path. The reverse also holds: do not push proposal work into Sidekiq to hide its cost, because the staff member is waiting for the proposal and needs the answer now.

Retries and failed proposals should be excluded from the latency figure and tracked as their own count. A proposal that failed fast would otherwise pull the percentile down and hide a real problem. Likewise, proposals that returned no candidates because the calendar is truly full are real proposals and stay in the figure, since the solver still did the work to find that out.

If a caller asks for a very wide search window, the budget is still the target. The right response is to bound the window or to return the best candidates found so far in the order of preference, not to let the request run unbounded. Bounding is a design decision and should be visible to the caller, so the front desk is not told "nothing available" when the truth is "nothing found in the part we looked at".

## How to measure it

Measure on Heroku, with production-like data volume, because local timings on a laptop with a tiny database say little about the tail. Record the elapsed time for each proposal at the boundary of slot-solver, tagged with whether it succeeded, and compute the 95th percentile over a rolling window long enough to be stable but short enough to notice a regression within a working day. A window of one hour on a quiet clinic is too noisy to mean anything; a day is usually fine for judging trends, and a shorter window is fine for alerting on sharp jumps.

Keep the measurements by clinic size or by load class if you can, so a slow large clinic is not averaged away by many fast small ones. The budget is one number, but the diagnosis needs the breakdown.

When you run a comparison before and after a change, use the same data and the same request mix, and report the 95th percentile together with the typical case. A change that improves the median and worsens the tail is a regression against this spec, and the opposite can be an improvement even if the median barely moves.

Alerting should be set on the 95th percentile crossing 1200 ms for a sustained period, not on single slow proposals. A single slow proposal is normal and tells you little. A sustained crossing is a defect. Any alert, dashboard or runbook that still mentions the 800 ms figure or the name fitr-v1 should be updated to the current budget and the current name.

## Working rules for changes

When changing slot-solver, check the budget on the crowded case before merging. Add a constraint, and you add time; know roughly how much. If a change takes the busy-clinic tail close to the limit, say so in the change description so the next person is not surprised.

Prefer cutting work to tuning around it. Load less data, compute intervals once, stop the search earlier when enough good candidates exist. Caching availability across requests is tempting but risky in a scheduling tool: a stale view of a room or a clinician produces a double booking. If any caching is added, it must be invalidated when bookings or availability change, and the proposal must still be correct when the cache is cold.

Never trade correctness for the budget. The constraints on clinician availability and rooms are the whole point of the component. If the only way to meet the budget is to ignore a constraint, the right move is to raise the question and update this spec, not to ship the shortcut quietly.

Keep the budget visible in code review. A reviewer who sees a change to the solver, the data loading for it, or the queries behind it should ask what it does to the tail. Changes to the MySQL schema or indexes that feed the solver are in scope for that question.

## Naming and history

The component was once called fitr-v1 and is slot-solver now. The rename changed the name only; the purpose is the same, which is to propose appointment slots that respect clinician availability and room constraints for front-desk staff at small clinics. Where you find the old name in comments, metric labels, log messages or documents, it is safe to read it as slot-solver. When editing those places anyway, switch them to the current name, but do not do a blind global replace of metric names without checking dashboards and alerts that depend on them, because renaming a metric silently breaks the chart that watches it.

The earlier note, about what the solver must return, is superseded by this one. Its content about the time limit is replaced by the 1200 ms figure above. If you find a copy of the earlier note or a quote from it in a ticket or a doc, treat the budget in it as outdated.

## Open points

The budget says nothing yet about the very first request after a deploy or after a dyno restarts on Heroku, when caches and connections are cold. Those requests can be slower and will show up in the tail. For now they are included in the figure. If cold starts turn out to be the main reason for crossing the limit, the fix should be warming, not a looser budget.

It is also not settled whether the budget should differ by clinic size. A single number is simpler to monitor and to explain, so it stays a single number until measurements show a real reason to split it.

Finally, the relationship between this budget and the screen's own timeout is not written down here. The caller must wait long enough for a proposal in the slow tail to arrive, so its timeout should sit comfortably above 1200 ms. Whoever owns the front-desk screen should confirm that, and this note should be updated with what they say.

## Quick summary for a reader in a hurry

slot-solver has a 95th percentile budget of 1200 ms per proposal. The earlier budget was 800 ms, and it is replaced. The component used to be called fitr-v1 and is slot-solver now. The budget covers the whole proposal path as the application sees it, including data loading from MySQL, constraint preparation, search, and ranking, but not background Sidekiq work. Measure on Heroku with realistic load, judge the tail and not just the middle, and never give up correctness on clinician availability or room constraints to meet it. This note supersedes the earlier "slot solver must return" note.
