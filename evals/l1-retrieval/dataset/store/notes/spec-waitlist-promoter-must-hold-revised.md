---
id: 01KMD7YEN1J30H1SZ9X7WXT6ST
created: 2026-03-23T08:40-03:00
---

# waitlist-promoter spec

This note replaces the earlier note "waitlist promoter must hold". The new value is that waitlist-promoter must hold an offered slot for 10 minutes, and that replaces the earlier 15 minute hold.

The rest of this spec describes what waitlist-promoter does around that hold, so a reader does not need the old note. Where a detail is not pinned down yet it is described in general terms on purpose.

## Purpose

When an appointment is cancelled or a clinician opens new availability, front-desk staff at a small clinic want the freed slot to go to someone on the waitlist without a phone call for every candidate. waitlist-promoter is the part of ClinicSlotter that does this. It picks the next eligible waitlist entry, offers the slot, holds it while the patient decides, and then either books it or releases it to the next entry.

It does not create availability and it does not override room constraints. It only fills slots that the scheduler already considers valid.

## Hold duration

An offered slot is held for 10 minutes. The hold starts when the offer is recorded as sent, not when the patient opens it. The hold is the full window in which the slot is unavailable to other offers and to manual booking by front-desk staff.

The value of 10 minutes replaces the earlier 15 minute hold. Anything that still assumes the longer hold (copy shown to staff, patient-facing text, tests, dashboards, alert thresholds) is stale and should be changed to match.

The duration should live in one configuration place, not be repeated in several jobs. Patient-facing text should be built from that same value so the message and the behaviour cannot drift apart.

```yaml
waitlist-promoter:
  hold: 10 minutes
```

## Eligibility and ordering

A waitlist entry is eligible for a freed slot when all of these hold:

- The appointment type matches what the patient is waiting for.
- The clinician is one the patient accepts, or the patient accepted any clinician.
- The room and equipment needed for the visit are free for the slot.
- The slot falls inside the time preferences on the entry.
- The entry has no other live offer at the moment.

Among eligible entries the order is by how long the entry has waited, with clinical priority set by the clinic taking precedence when the clinic uses it. Ties are broken by entry creation order so the result is stable and easy to explain to a patient who asks why someone else was offered the slot first.

## Offer lifecycle

An offer moves through a small set of states: pending, held, accepted, declined, expired. Pending means the entry was chosen but the message has not gone out. Held means the message went out and the hold window is running. Accepted, declined and expired are terminal for that offer.

Only one offer may be held for a given slot at any time. Moving from pending to held and taking the slot lock happen together in one database transaction in MySQL, so two workers cannot both hold the same slot.

If the patient accepts inside the hold, the slot is booked and the offer becomes accepted. If the patient declines, the hold ends at once and the slot goes to the next eligible entry without waiting out the rest of the window. If nothing comes back before the hold ends, the offer becomes expired and the slot is released and offered onward.

## Background jobs

The work runs in Sidekiq. There are three kinds of job in outline:

- A promotion job, triggered when a slot is freed, that finds the next eligible entry and creates the offer.
- A delivery job that sends the offer to the patient and marks it held.
- An expiry job scheduled for the end of the hold when the offer becomes held.

The expiry job must be safe to run late or twice. It checks the current state of the offer first and does nothing if the offer is already accepted or declined. A late expiry is a nuisance for the next patient but must never undo a booking.

Because expiry is scheduled work, a backed-up queue stretches the real hold past 10 minutes. The slot should not be offered to someone else during that stretch, so the check that matters is the state of the offer, not the clock alone.

## Interaction with staff booking

Front-desk staff can still see a held slot. The calendar shows it as held for a waitlist offer, with the time the hold ends. Staff may cancel the offer by hand, which behaves like a decline. Staff may not double-book over a live hold; the booking screen should say who the slot is held for only to the extent the clinic's privacy settings allow.

When staff book a slot directly before the promotion job reaches it, the promotion job finds the slot taken and stops quietly. That is a normal outcome and should not raise an error.

## FHIR and data exchange

Bookings made through waitlist-promoter are written out the same way as any other booking, as HL7 FHIR Appointment resources, and a held slot is represented with the FHIR Slot status that marks it as busy while the hold runs. When a hold ends without a booking, the slot returns to free. External systems that read slots through FHIR should therefore never see a slot as free while an offer is held.

The hold itself is internal. It is not exposed as a separate FHIR resource.

## Deployment notes

The app runs on Heroku. Sidekiq workers run as their own process type so a surge of promotion work does not starve web requests. The hold length is read from configuration at the time the offer is created, so a deploy that changes it affects new offers only; offers already held keep the window they were given.

After changing the value, check that the expiry job schedule in a staging environment matches it, since that is where a mismatch would show first.

## Open points

- Whether a patient who declines should drop to the back of the waitlist or keep their place. Today they keep it.
- Whether clinics should be able to set their own hold length. For now it is one value for every clinic.
- How to show the remaining hold time to patients on slow connections without implying a precision the system does not have.

Until these are decided, keep behaviour as described above and keep the hold at 10 minutes everywhere.
