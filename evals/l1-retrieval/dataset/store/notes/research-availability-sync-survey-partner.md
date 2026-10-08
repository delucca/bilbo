---
id: 01KRD0SME6XA1W72E6AMCC537T
created: 2026-05-11T23:38-03:00
sources:
  - "doc: FHIR Slot Survey Notes"
---

# availability-sync: deriving free time without Slot resources

A survey of partner clinics found that 38% publish no FHIR Slot resources, so availability-sync must derive free time from Schedule resources alone. That is the finding this note keeps. We cannot rely on the Slot resource being present for every clinic, and any design that reads Slot as the source of truth will show those clinics as having no availability at all.

## What the survey found

We asked partner clinics what their FHIR endpoints actually expose. 38% of them publish no Slot resources. They do publish Schedule resources, which describe the clinician or room and the planning horizon during which appointments can be booked. So for roughly four in ten clinics the only availability data we get is the Schedule.

The other clinics do publish Slot. For them the Slot data is more direct, but we should not build two unrelated code paths if we can avoid it. The safer plan is one derivation from Schedule that works everywhere, with Slot used only as a cross-check where it exists.

## What this means for availability-sync

availability-sync cannot treat a missing Slot as "nothing free". It has to compute free time itself:

- Read the Schedule for each clinician and room, and take its planning horizon as the window that can be booked.
- Subtract appointments already booked in ClinicSlotter, and subtract room conflicts, from that window.
- What is left is the free time offered to front-desk staff.

The clinician and room constraints stay the same as before. Only the source of the raw open time changes.

```ruby
# sketch only: derive free time from Schedule, no Slot needed
free = schedule.window - booked_appointments - room_conflicts
```

## Open questions and next steps

- Schedule resources can be coarse. Some clinics may give a wide horizon with no breaks, so the derived free time may be too generous. We need to check this with a few partner clinics.
- Decide how Sidekiq jobs should handle clinics where Slot appears later, for example after an upgrade of their system. The sync should notice and not double count.
- Add a test fixture for a clinic with Schedule only, so this case is covered in the MySQL-backed test suite.
- Confirm which clinics are in the 38% before rollout on Heroku, so support knows who is affected.
