---
id: 01JXQ8HRAHTTDT4143JV64RDFV
created: 2025-06-14T10:02-03:00
sources:
  - "code: config/routes.rb"
---

# fhir-gateway reference

fhir-gateway is the part of ClinicSlotter that talks HL7 FHIR to the outside world. It takes FHIR R4 resources from other systems (EHRs, booking portals, clinic tools) and turns them into appointments inside the Rails app. Front-desk staff never see it directly; they see the appointments it creates on the schedule. This note is a quick lookup for what it accepts and how it behaves, written from memory of how it is wired, so check the code before relying on fine detail.

## Endpoint

fhir-gateway accepts R4 resources at the endpoint `POST /fhir/r4/Appointment`. This is the one entry point for creating an appointment from a FHIR payload. The body is a FHIR R4 Appointment resource as JSON. Only R4 is supported; there is no other FHIR version served by fhir-gateway, so a caller sending STU3 or R5 shaped resources should expect them to be rejected or mis-read.

The request is handled by a normal Rails controller. It authenticates the caller, parses the resource, validates it, and then hands the real scheduling work off. The HTTP response comes back before slow work finishes where possible.

## What happens to a request

1. The controller checks the caller's credentials and that the content is a FHIR Appointment.
2. The resource is mapped into ClinicSlotter's own appointment model. Participants map to a clinician and a patient; the location or service maps to a room; start and end map to the requested slot.
3. The slot is checked against clinician availability and room constraints, the same rules the front-desk UI uses. fhir-gateway does not have its own copy of these rules and must not grow one.
4. If the slot fits, the appointment is stored in MySQL. If it does not fit, the caller gets an error response and nothing is stored.
5. Follow-up work, such as notifications or syncing back to the sender, runs in Sidekiq jobs so the request stays short.

## Mapping notes

- Clinician and room identifiers from the sender have to be matched to records we hold. Unknown references are a common cause of rejections.
- Time zones matter. FHIR timestamps carry offsets, clinics are stored with a local zone, and mixing them up shifts appointments by hours. Convert at the edge and store consistently.
- Status values from FHIR are only partly used. Treat anything we do not model as a plain booked appointment unless the code says otherwise.
- Fields we do not understand are ignored rather than stored, so a round trip will not return everything the sender gave us.

## Operations and gotchas

- Runs on Heroku with the rest of the Rails app, so request time limits apply. Keep the synchronous path small and push anything slow to Sidekiq.
- Retries from a sender can create duplicates if the payload has no stable identifier. Look at how identifiers are handled before changing the create path.
- Error responses should be FHIR-style OperationOutcome bodies so senders can read them. Keep that shape when adding new failure cases.
- When debugging a rejected appointment, reproduce it by posting the same resource to a local instance, then look at the availability and room checks first; most failures come from there, not from parsing.
- Any change to the mapping should be tested with real-looking sample resources from at least one partner, since spec-valid payloads from different systems differ a lot in which optional fields they fill in.

## Open points

Authentication details, exact validation messages and the full list of supported fields are not recorded here. Read the controller and mapper in the Rails app and update this note when they are confirmed.
