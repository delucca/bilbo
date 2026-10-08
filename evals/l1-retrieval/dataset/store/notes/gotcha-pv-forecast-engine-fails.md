---
id: 01KR4R1W3R0CP8ZT56H5GVCFET
created: 2026-05-08T18:31-03:00
---

# pv-forecast-engine fails on site records with missing azimuth

pv-forecast-engine crashes when a site record has a missing azimuth value. The error is `DimensionMismatch: arrays could not be broadcast to a common size`. It does not say which site or which field caused it, so the first time you see it you will probably look in the wrong place (array shapes, weather grid, time axis) when the real cause is a gap in the site metadata.

## Symptom

A forecast run for one or more sites aborts in the Julia process with `DimensionMismatch: arrays could not be broadcast to a common size`. The stack trace points into broadcasting code in the plane-of-array or irradiance step, not at the site loader. Nothing in the message mentions azimuth. Other sites in the same batch may have forecast fine before the failure, so the output looks partly complete, which makes it look like a data volume or timing problem.

## Cause

Each site record carries panel orientation: tilt and azimuth, among other things. When azimuth is missing for a site, the value that reaches the forecast math is empty or has a different length than the other per-site vectors. The engine then broadcasts it against the tilt, capacity and time-series arrays, and the shapes no longer line up. Julia refuses and raises the DimensionMismatch.

The missing azimuth is not caught earlier. There is no validation on the site record before the forecast starts, so the failure shows up deep in the computation.

## How to confirm

- Look at the site records the engine was fed for the failing run and check that every one has an azimuth. Compare against the sites that succeeded.
- Records often come from installer onboarding. A site that was registered without panel orientation, or where the field was cleared during an edit, is the usual suspect.
- If the site data flows through Azure IoT Hub device twins or is stored alongside the InfluxDB measurements, check the source of the record too. A field that is present in one place and absent in another can produce the gap during sync.
- Remove or fix the suspect site and rerun. If the error disappears, that was it.

## Workaround

Fill in the azimuth for the affected site, with the real value from the installer or the installation survey. Do not guess a default such as south facing, since that quietly produces a wrong forecast, and the battery charging schedule is built on that forecast against the time-of-use tariffs. A wrong schedule costs the customer money without any error being raised, which is worse than a crash.

If you need the batch to continue while the data is being fixed, skip the incomplete site explicitly and log it, so the gap is visible and the site gets no forecast rather than a bad one.

## Follow-up ideas

- Validate site records at load time in pv-forecast-engine and fail with a message that names the site and the missing field.
- Reject or flag site records without azimuth at the point where installers enter them, in the Svelte front end, so the problem never reaches the engine.
- Add a test with a site record that lacks azimuth, asserting a clear error instead of the broadcast failure.
- Make one bad site not abort the whole batch.
