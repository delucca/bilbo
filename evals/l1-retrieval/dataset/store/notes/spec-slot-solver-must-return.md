---
id: 01KXX58TB4BQMG3Z1XZ8V5BHCW
created: 2026-07-19T09:23-03:00
---

# slot-solver latency spec

This note records the latency requirement for slot-solver, the component in ClinicSlotter that proposes appointment slots. Front-desk staff at small clinics wait on it while a patient is on the phone or standing at the desk, so it has to feel immediate. The requirement: slot-solver must return a slot proposal within 800 ms at the 95th percentile. That is the number to test against and the number to quote when someone asks what the budget is.

## Requirement

slot-solver must return a slot proposal within `800 ms` at the 95th percentile. In other words, at least nineteen of every twenty proposal requests must finish inside that time, measured from the moment the request reaches the Rails app to the moment the proposal is ready to be returned to the caller. The slowest requests may go over, but they should be rare and they should be understood, not shrugged off.

Things this requirement covers:

- A proposal request from the front-desk UI, where a staff member asks for a slot for a given patient, clinician and appointment type.
- The full solve: reading clinician availability, applying room constraints, choosing candidate slots and ranking them.
- Serialisation of the proposal into the response the UI or an API client consumes.

Things it does not cover:

- Work that happens after the proposal is shown, for example confirming the booking, writing the appointment, or sending reminders. Those run elsewhere, mostly in Sidekiq jobs, and have their own expectations.
- Cold-start time after a Heroku dyno restarts. A first request after a deploy may be slow. Record it, but do not count it against the budget when judging steady-state behaviour.
- Bulk or batch rescheduling. That is a different workload and should not be mixed into the same measurement.

## Why this budget

The budget comes from how the tool is used. A receptionist is usually talking to someone while the proposal loads. A wait that is noticeable makes people stop trusting the screen and start double-booking by hand, which defeats the point of the tool. The target is set at the 95th percentile and not at the average because averages hide the slow tail, and the slow tail is what staff remember. The tail also tends to be the busy mornings, which are the worst moment for it.

The small-clinic setting matters for the shape of the problem. Each clinic has a modest number of clinicians and rooms, so the search space per request is small. If the solver is slow, the cause is much more likely to be how data is loaded, how queries are issued, or how many times the same availability is recomputed than raw search complexity. Treat any proposal that needs heavy computation as a sign something upstream is wrong.

## How to measure it

Measure on the proposal request itself, not on a synthetic micro-benchmark of the solver function alone. The number that matters is what the user sees, so include database time, any FHIR lookups made on the request path, and serialisation.

Guidelines:

- Record request duration per proposal call and compute the 95th percentile over a rolling window long enough to include a busy period. A quiet window will flatter the result.
- Keep the measurement per clinic size if possible. A large clinic with many clinicians will sit near the top of the range, and mixing it with tiny clinics can hide a regression.
- Report the percentile and the sample count together. A percentile over a handful of requests is not evidence.
- When comparing before and after a change, use the same data shape and the same environment. Heroku dyno type and MySQL load both move the result.

A minimal way to state the target in a config or a test, so it lives in one place:

```yaml
slot_solver:
  latency_budget: 800 ms
  percentile: 95th
```

The exact key names are illustrative. The point is that the value is written once and referenced by both the monitoring check and any performance test, rather than copied into several places where it can drift.

## Where the time can go

These are the usual places to look first when the budget is at risk. None of this has been profiled in this note; it is a checklist, not a finding.

- **MySQL queries.** Availability and room data are read from MySQL. Repeated queries inside the solve loop, or missing indexes on the columns used to filter availability, are the most likely cause of a slow tail. Load what the solve needs up front, in as few queries as practical.
- **FHIR lookups.** ClinicSlotter speaks HL7 FHIR for some data. A synchronous FHIR call on the request path adds network time that the solver cannot control. If the proposal needs data from a FHIR source, prefer a local copy kept fresh by a background job, and keep the live call off the hot path.
- **Rails object overhead.** Building many ActiveRecord objects for rows that are only read once is wasteful. Plain values or selected columns are usually enough for the solve.
- **Sidekiq contention.** If background jobs share the same MySQL instance and run heavily at the same time as proposals, the database becomes the shared bottleneck. Watch for latency rising when queues are busy.
- **Heroku dyno limits.** A small dyno under concurrent requests will queue work. Slow proposals under load may be a capacity matter and not a code matter.

## Rules for changes

Any change that touches the solve path should be checked against the budget before it ships. If a change makes proposals more accurate but pushes the 95th percentile past `800 ms`, it needs either a second change that recovers the time or an explicit decision to relax the budget, written down in a note and not made quietly in code.

Do not buy speed by returning a worse proposal without saying so. If a fast path skips a constraint such as a room check, it must never return a slot that violates clinician availability or room constraints. Correctness of the proposal comes first; the budget limits how it is computed, not what counts as a valid answer.

If the solver cannot finish in time on a particular request, prefer a clear partial or fallback result that the UI can show over a long hang. The front desk can work with a shorter list of valid slots. They cannot work with a spinner that never ends.

## Open points

- Whether the budget should be enforced as a hard timeout on the request or only tracked as a monitored target is not settled. For now it is a target that we monitor and alert on.
- It is not decided yet whether the 95th percentile should be reported per clinic or across all clinics. Per clinic is more informative but noisier for small clinics.
- There is no agreed answer yet for how to treat requests that wait on a FHIR source that is down. The solver should not be blamed for that time, but the user still feels it.

## Related

Background job behaviour that interacts with this budget, especially work that shares the database with proposal requests, is covered in [[reminder-worker-experiment-showed]]. Read it before moving any work between the request path and Sidekiq, since that move changes what slot-solver has to finish inside the budget.
