---
id: 01K4T7T9NF0W786Y80GE3HRX1W
created: 2025-09-10T13:37-03:00
---

# waitlist-promoter: options survey

Notes on the general ways to build waitlist-promoter, the part of ClinicSlotter that fills a freed appointment slot from the waitlist. Nothing here is settled. It is a map of what we looked at and what each option costs.

## Problem shape
When a slot opens (cancellation, clinician change, room freed), waitlist-promoter must pick a waiting patient who fits the clinician, room and time, and offer or book it. Front-desk staff at small clinics need to see and override what it does.

## Trigger options
### Event on cancel
Fire a Sidekiq job from the cancellation code path. Fast, simple, but easy to miss other ways a slot frees up.

### Periodic sweep
A scheduled job scans for open slots and waiting patients. Catches everything, but promotion lags and the scan costs more as data grows.

### Hybrid
Event for the quick path, sweep as a safety net. More moving parts, but the sweep covers dropped events.

## Candidate selection
### First come first served
Order by time joined. Easy to explain to front desk. Ignores urgency.

### Priority score
Weighted by clinical urgency, wait time, patient flexibility. Fairer in theory, harder to explain and tune.

### Staff-picked
The system only suggests; staff choose. Safest, slowest.

## Promotion mode
### Auto-book
Slot is booked at once and the patient is told. Risk: patient never sees the message and no-shows.

### Offer with hold
Slot is held briefly while the patient confirms. Needs expiry handling and a fallback to the next candidate.

### Notify and race
Notify several patients, first reply wins. Fast fill, but disappointed patients and messy cleanup.

## Concurrency
Two promotions can grab the same slot. Options are row locks in MySQL, optimistic locking with a version column, or serializing per clinician through a single queue. Row locks are the most direct; per-clinician queues are cleaner but add Sidekiq setup.

## Constraints check
Reuse the existing availability and room rules rather than copying them. A duplicated rule set will drift.

## Notification channel
SMS, email, or a phone call task for staff. Depends on what clinics already have consent for. Needs checking before building anything.

## FHIR angle
Slot and Appointment resources could represent holds and bookings, which helps interoperability. Mapping a waitlist entry is less clear, so keep the internal model primary and map outward.

## Heroku fit
Dynos restart often, so jobs must be idempotent and safe to retry. Scheduler add-ons are coarse, so sweeps should not depend on exact timing.

## Open questions
- What urgency data do we actually have?
- Which fairness rule do clinics expect?
- How are patient messaging consents stored?
- Who audits automatic bookings?
