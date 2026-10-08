---
id: 01M1JNZ0HQ3CWFPBZX6VDV9XK2
created: 2026-09-03T00:45-03:00
sources:
  - "code: app/services/fhir_serializer.rb"
---

# fhir-gateway design

fhir-gateway is the part of ClinicSlotter that turns our internal bookings into HL7 FHIR resources so other systems at a clinic can read them. The core of it is one mapping: each internal booking becomes a FHIR Appointment with status booked, and the class that does this is `FhirSerializer`. Anything that wants to publish a booking to FHIR goes through that class, not through ad hoc hash building in controllers or jobs.

## Purpose

Front-desk staff book slots in the Rails app. The booking row in MySQL is the source of truth. fhir-gateway exists so that the same booking can be shown to outside systems in a standard shape. It does not decide availability, room fit or clinician conflicts; the scheduler already did that before a booking exists. fhir-gateway only describes the result.

## Mapping rule

One internal booking maps to one FHIR Appointment. The status is always booked, because a booking only reaches the gateway once the scheduler has accepted it. `FhirSerializer` is the single place that knows the field mapping. If a FHIR field needs to change, change it there and nowhere else.

Things the serializer has to carry over from the booking, in general terms:

- the start and end of the slot
- the clinician as a participant
- the room or location
- the patient reference

Keep the output free of anything that is not needed by the receiver. Do not leak internal ids or notes fields just because they are handy.

## Where it runs

Serialization is plain Ruby and has no side effects, so it is easy to call from a Rails request or from a Sidekiq job. Pushing the result to an external endpoint should be done in a Sidekiq job so a slow or down receiver never blocks the front desk. The app is deployed on Heroku, so the job needs to be safe to retry and to run on any dyno.

## Open points and gotchas

- Only booked is produced today. Cancellations and reschedules are not mapped yet, so a cancelled booking will not turn into a cancelled Appointment until someone adds that to `FhirSerializer`.
- Time zones are easy to get wrong. Use the clinic's zone when reading the slot, and emit FHIR instants with an explicit offset.
- Tests for the mapping should build a booking, run it through `FhirSerializer`, and check the resulting Appointment fields, including the status. Do not test it only through the HTTP layer.
- If a second consumer needs a different shape, add a second serializer rather than adding flags to the first one.

## Decisions so far

Keep one serializer class and one status for now. Revisit when cancellation support is needed. Any change to the mapping should be noted here so the next session does not have to reread the whole class to learn what the gateway promises.
