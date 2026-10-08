---
id: 01JW2TXXA9NFPYT0B2H7XQA69A
created: 2025-05-25T01:24-03:00
---

# room-constraints spec

This note specifies how room-constraints works in ClinicSlotter: what a row means, what the scheduler does with it, and what is still undecided. The one hard rule is that every row in room-constraints must define `max_concurrent`. If a row does not set it, the value defaults to 1, so the room hosts one appointment at a time. Everything below follows from that rule. Written quickly, so check it against the code before relying on any detail.

## Purpose

Front-desk staff at small clinics book outpatient visits against two limits: the clinician must be free, and a room must be free. Clinician availability is handled elsewhere. room-constraints covers the room side. It says which rooms exist for scheduling purposes, what each can hold, and which visit types may use it.

Small clinics usually have a handful of rooms, and some rooms are shared. A procedure room may take only one patient. A group session room may take several at once. A row with no explicit capacity must never be read as unlimited, because that would double-book a small exam room and the front desk would find out only when two patients turned up. That is why the default is the conservative one.

## The max_concurrent field

Each row in room-constraints must define `max_concurrent`. It is the number of appointments that may overlap in that room at any moment. The default is 1, which means one appointment at a time.

Rules for the field:

- It is required on every row. Never store it as null. A null here would have to mean something, and we do not want to guess.
- The default of 1 applies when the creator of the row says nothing. It is applied when the row is created, not looked up at read time, so the stored value is always explicit.
- It must be a whole number and must be at least 1. A value of zero would mean a room nobody can book. To take a room out of service, use the availability mechanism, not this field.
- Raising it is a deliberate act, for example for a group room. Lowering it on a row that already has future bookings needs the conflict handling described below.

A reader who only wants the answer to what happens when nothing is set: the room hosts one appointment at a time.

## How the scheduler uses a row

When the scheduler proposes a slot, it asks whether the room named by the candidate slot has spare capacity for the full time span of the visit. Spare capacity means that, at every instant of the span, the count of existing active appointments in that room is below `max_concurrent`. Counting is by overlap on the time axis, not by identical start times. Two visits that only touch at the boundary do not overlap.

Only active appointments count. Cancelled and no-show appointments free the room. Tentative holds made while a booking is in progress do count, until they expire or are confirmed, so two staff members cannot take the last place at the same moment.

The room check is combined with the clinician check using logical and. Both must pass. When the room fails, the scheduler moves on to the next candidate room if the visit type allows more than one room, and only then to the next time.

## Interaction with visit types

Not every room suits every visit. The row can list the visit types it supports, and the scheduler filters rooms by that list before it looks at capacity. A room that does not support the visit type is skipped even if it is empty.

Capacity and visit type are separate checks. A group room with a higher `max_concurrent` still only accepts the visit types listed for it. Do not use a high capacity as a way to say a room is generic.

A visit that needs a whole room to itself, such as a procedure with setup and cleanup time, should block the room regardless of its capacity. Treat this as an exclusive flag on the visit type, not as a change to the room row. This part is still open, see the questions at the end.

## Data and background work

The rows live in MySQL and are managed through the Rails app. Validation of the capacity field is in the model, and there should also be a database level constraint so that a bad value cannot get in through a console session or a migration script.

Capacity checks run in the request path when staff pick a slot. They must be cheap: read the room row, read the overlapping appointments for that room and window, compare. Do not move this to Sidekiq, because the answer is needed while the user waits and a queued check could return after the slot was taken.

Sidekiq does handle the slower follow-up work. When a row changes, a job re-checks future appointments in that room and reports any that now exceed the new capacity. It does not move or cancel anything by itself. It produces a list for the front desk to resolve.

On Heroku, the web dynos and the worker dynos share the same database, so the lock described next has to live in MySQL rather than in process memory.

## Concurrency and booking races

Two staff members at the same desk, or at two desks, can try to book the last place in a room at the same moment. The read then compare then write sequence is not safe on its own. The booking write must take a lock on the room row inside a transaction, recheck the overlap count under that lock, and only then insert the appointment. If the recheck fails, the user gets a clear message that the room was just taken and is offered the next options.

Locking the room row serializes bookings per room, which is fine at the scale of a small clinic. Do not widen the lock to the whole table. Do not rely on the earlier check made when the slot was displayed, since it may be stale by the time the user confirms.

## FHIR mapping

The app exchanges appointments with other systems over HL7 FHIR. A room maps to a location resource, and an appointment refers to it as a participant. Capacity is a scheduling rule of ours and has no standard field on the location, so it is not exported. Inbound appointments that name a room are checked against the same capacity rule before they are accepted. If an inbound appointment would exceed the room capacity, it is rejected with a reason that the sender can read, not silently dropped, and it is logged for the front desk.

When an inbound location does not match any known room, treat it as unknown and do not guess. Create no room row automatically, because an automatic row would take the default capacity and might hide a configuration mistake.

## Changing capacity on live rows

Raising capacity is safe: nothing existing can become invalid. Lowering it is the risky case. The change is allowed, but the follow-up job lists every future appointment that now overlaps beyond the new limit. Staff decide which to move. Until they do, the existing appointments stay as they are, and new bookings into the over-full windows are refused.

An audit entry should record who changed the value, the old value, the new value and when. Front-desk staff ask about this after a double booking, and the answer needs to be in the app, not in a developer's memory.

## Open questions

- Exclusive visit types: whether the flag lives on the visit type or on the booking is not settled. Leaning toward the visit type.
- Cleanup time between visits: whether it extends the span counted against capacity, and whether it is per room or per visit type.
- Rooms shared between two clinics under one account: whether capacity is per room or per clinic. Currently assumed per room.
- Whether to show remaining capacity in the slot picker for group rooms, or only show full or not full.
- Whether the default should ever differ by room type. Current answer is no: the default stays at 1 everywhere, and group rooms are set up explicitly.

## Testing notes

Cases worth keeping in the suite: a row created without an explicit value gets the default and refuses a second overlapping appointment; a group room accepts overlapping appointments up to its limit and refuses the next one; boundary touching appointments do not conflict; cancelled appointments free capacity; two concurrent bookings for the last place result in exactly one success; lowering capacity with future bookings produces the report and leaves the appointments untouched; an inbound FHIR appointment over capacity is rejected with a readable reason.

Test the race with real transactions against MySQL, not with stubs, since the behavior depends on the locking.
