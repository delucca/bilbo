---
id: 01M3RV0DR6XBXGWVMVG0NRKV9S
created: 2026-09-30T06:40-03:00
---

# frontdesk-ui booking click limit

This note replaces the earlier note "frontdesk ui must staff". The new value: frontdesk-ui must let staff book an appointment in at most 2 clicks, down from the earlier limit of 3 clicks.

## Requirement

frontdesk-ui must let front-desk staff book an appointment in at most 2 clicks. The limit is a hard ceiling, not a target average. A booking that needs a third click counts as a defect against this spec.

## What changed

The old limit was 3 clicks. It is replaced, not supplemented. Anyone reading the old note should treat its number as void. Nothing else in the old note is changed by this one unless it conflicts with the 2 clicks ceiling.

## Why it matters

ClinicSlotter is used by front-desk staff at small clinics. They often book while a patient is standing at the desk or on the phone. Every extra click costs attention, and small clinics have few people to cover the desk. The tighter limit reflects that.

## What counts as a click

A click is one deliberate pointer or tap action by the user on the booking path. Typing into a field is not a click. Keyboard shortcuts are not clicks, but the 2 clicks limit must hold for a pointer-only user, so shortcuts cannot be the only way to meet it.

## Where counting starts

Counting starts from the screen the staff member sees after opening the schedule view for a clinician. It does not include navigating to the schedule. Counting ends when the appointment is confirmed and saved.

## Suggested flow

The likely shape is: click an open slot, then click confirm in the booking form. The form should arrive with clinician, time and room already filled from the slot that was clicked. Patient lookup is done by typing, so it adds no click.

```text
click 1: open slot
click 2: confirm booking
```

## Availability and rooms

Slots shown as open must already respect clinician availability and room constraints. If the UI only offers valid slots, staff never need an extra click to pick a room or fix a conflict. The scheduling rules stay on the server; frontdesk-ui only presents the result.

## Defaults

Defaults carry the budget. Room is chosen from the slot. Appointment length comes from the clinician's usual setting. Staff can change these, but changing them is an optional extra path and does not count against the ceiling for the default booking.

## Errors and conflicts

If a slot is taken between display and confirm, the UI must show the conflict and offer the nearest valid alternatives. Recovering from a conflict is a separate flow, not part of the 2 clicks measure, but it should not lose the data already entered.

## Background work

Anything slow, such as sending notifications or syncing with HL7 FHIR systems, must happen in Sidekiq jobs after the save. The confirm click must not wait on them, and they must not add a confirmation step for the user.

## How to verify

Walk the default booking path in a browser and count clicks. Add a UI test that books an appointment through the default path and fails if more than 2 clicks are used. Run the check on any change to the booking form or the schedule view.

## Out of scope

Rescheduling, cancelling and bulk booking are not covered by this limit. They may get their own limits in a later spec.

## Open questions

Whether a recurring appointment can meet the same ceiling is not decided. Whether accessibility needs, such as screen readers, change how clicks are counted is also not decided.

## Notes for later sessions

Do not raise the limit back without a decision recorded. If a design needs a third click, change the design. Keep the number in one place in the tests so a future change is a single edit.
