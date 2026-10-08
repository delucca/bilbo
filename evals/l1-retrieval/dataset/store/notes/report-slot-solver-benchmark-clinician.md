---
id: 01KSKFKSZSQ5GVSE02D02DFT34
created: 2026-05-26T22:08-03:00
---

# slot-solver benchmark: latency over 500 clinician-days

We ran a benchmark of slot-solver over 500 clinician-days. Median solve time came out at 140 ms and the 99th percentile at 610 ms. This note records what that means for ClinicSlotter and what to watch when touching the solver. It is a single benchmark result, not a full performance study.

## Result

Across 500 clinician-days, slot-solver had a median of 140 ms and a 99th percentile of 610 ms. The tail is a bit over four times the median. Most days solve quickly; a small share of days are much slower than typical. Each sample is one clinician-day, meaning one clinician's availability for one day matched against room constraints.

## What the numbers mean for front-desk use

Front-desk staff at small clinics wait on the screen while slots are proposed. A median of 140 ms feels instant in the Rails request. At 610 ms the 99th percentile is noticeable but still tolerable for an interactive page. Nothing here says the solver is too slow today. It does say we have little headroom if the tail grows, for example with more rooms per clinic or denser clinician schedules.

## Likely causes of the tail

I did not profile the slow days, so these are guesses to check, not findings:

- Days with many room constraints and tight clinician availability probably force more backtracking.
- Days with lots of existing appointments to respect may add work loading and checking them from MySQL.
- Availability coming in through HL7 FHIR resources might add parsing cost on some days.

Before blaming the solver itself, separate time spent in the search from time spent loading data. The benchmark number alone does not make that split.

## Where this should run

Interactive requests on Heroku web dynos should stay within the tail we measured. Anything bulk, such as re-solving a whole week for many clinicians, belongs in a Sidekiq job so a slow day never blocks a web request. Bulk work multiplies the per-day cost, so use the median for rough estimates and the 99th percentile for worst-case planning.

## Next steps

- Re-run the same benchmark after any change to slot-solver and compare median and 99th percentile against 140 ms and 610 ms.
- Profile the slowest days and record which constraint types they share.
- Keep the benchmark inputs fixed so runs stay comparable; note any change to the 500 clinician-days sample when it happens.
- Decide whether a time limit on a single solve is needed, and what the front desk sees if it trips.
