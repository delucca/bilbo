---
id: 01K38SVK8QTGRCSPHHZZR7SHH5
created: 2025-08-22T08:50-03:00
---

# fhir-gateway: what it accepts

Quick notes on what the fhir-gateway takes in and what it turns away. Written from memory of how it behaves, not a full spec. Related: [[waitlist-promoter-options-survey]].

## Purpose

The fhir-gateway is the single door for HL7 FHIR resources coming into ClinicSlotter from outside systems. Front-desk staff never see it, but anything that lands in the schedule from an EHR passes through it first.

## Resource types it accepts

Only a small set of resource types is handled: appointment-ish things, patients, practitioners, and a few supporting ones for locations and schedules. Anything outside that set gets refused with a plain client error.

## Appointment resources

These are the main traffic. The gateway maps them onto our own appointment records and checks clinician availability and room constraints before anything is kept.

## Patient resources

Accepted so we can match or create the person an appointment belongs to. Matching is loose and leans on the usual demographic fields. Duplicates are a known annoyance.

## Practitioner and location resources

Used to line up clinicians and rooms with what the scheduler already knows. If the incoming reference does not match a known clinician or room, the resource is parked rather than guessed at.

## Content type

It expects JSON FHIR content with the proper media type header. XML is not supported. A wrong or missing header gets rejected early.

## Size limits

There is a configured body size limit. Bundles that go over it are refused, so senders should split large batches. Check the config for the actual value rather than trusting anyone's memory, including mine.

## Bundles

Transaction-style bundles are accepted, and each entry is validated on its own terms. Partial acceptance is possible, so the response needs reading entry by entry.

## Validation

Structural validation runs first, then our own rules about required fields. Failures come back as an operation outcome with the offending element named.

## Authentication

Callers need to present credentials the gateway knows about. Unauthenticated calls are refused before any parsing. Details live in the deployment settings on Heroku.

## Processing model

Accepted resources are not fully processed in the request. The gateway hands them to Sidekiq jobs and answers quickly. The caller gets an acknowledgement, not a final scheduling result.

## Retries and idempotency

Senders will retry. The gateway tries to recognise a resource it has already seen, using the identifiers on the resource, so repeats should not create a second appointment. This is the part I trust least.

## Storage

After a job succeeds the data ends up in MySQL in our own tables. The raw incoming resource may be kept for a while for debugging, per whatever retention is configured.

## Failure behaviour

If a job fails repeatedly it lands in the usual dead set and someone has to look. The sender usually does not know.

## Known gaps

- No clear story for updates that arrive out of order.
- Cancellations and reschedules are handled but thinly tested.
- Unknown extensions are dropped quietly.

## Open questions

- Should the gateway reject unknown resource types louder, or keep ignoring them?
- Is the size limit right for the busiest clinics?
- Who owns the parked resources queue?

## Where to look next

Start from the gateway controller and the job classes it enqueues, then the config for limits and credentials. Compare with the older note on the same subject before changing anything here.
