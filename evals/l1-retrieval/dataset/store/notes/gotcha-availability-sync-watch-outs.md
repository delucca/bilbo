---
id: 01KZ6HCZGVNT7VJ8FDTBTBF1ZS
created: 2026-08-04T11:03-03:00
---

# availability-sync: things to watch when changing it

Quick list of traps in availability-sync, written so the next person does not have to rediscover them. Nothing here is a spec. It is what tends to go wrong when someone touches this component without reading the surrounding code first.

The component sits between the clinician calendars that clinics keep elsewhere and the slot generator that front-desk staff actually see. If it drifts, nobody gets an error. Staff just see slots that look open and are not, or slots that are missing. Both are noticed late, usually by a patient at the desk.

## Source data is not as clean as it looks

Availability comes in through FHIR resources, and different clinics fill them in differently. Treat every field as possibly missing, possibly in a different shape, and possibly contradicting another field.

- Time zones are the biggest source of pain. A clinic's local time, the server's time and what the upstream system sends can all differ. Daylight saving changes shift weekly patterns by an hour for part of the year. Check how a change handles the days around a clock change before calling it done.
- Recurring availability and one-off exceptions both exist. If you change how recurrences expand, re-check that exceptions (leave, closures, extra sessions) still override the base pattern and not the other way around.
- Resources can be updated or deleted upstream without any hint that a related slot already has a booking. Do not assume a removed block means the appointments inside it are gone.
- Partial payloads happen. A page of results that stops early looks the same as a clinician with no availability. Never treat "nothing came back" as "nothing is available" without checking that the fetch finished cleanly.
- Duplicate or overlapping blocks for one clinician are common. Merge logic needs to be idempotent, so running it twice on the same input gives the same result.

## Room constraints and clinician availability must move together

A slot is only real when both a clinician and a room are free. availability-sync touches the clinician side, but the room side lives close by and the two are easy to get out of step.

- Changing how clinician blocks are written without thinking about rooms can leave slots that have a clinician and no usable room. The reverse also happens.
- Some clinicians work in more than one room, or share rooms with others. Anything that assumes one clinician maps to one room will quietly break these clinics.
- If you add a new kind of block (a break, a admin period, a telehealth session), decide explicitly whether it holds a room. Defaults here have caused surprises before.
- Do not delete and recreate availability rows to refresh them. Other records may point at those rows, and the gap between delete and recreate is visible to anyone who is booking at that moment.

## Background jobs and timing

The sync runs through Sidekiq, so everything about it has to survive retries, duplicates and running out of order.

- Jobs get retried. Any step that is not safe to run twice will eventually run twice. Prefer upserts keyed on something stable from the source over inserts.
- Two jobs for the same clinician can overlap, especially after a deploy or a queue backlog. Without some kind of per-clinician guard, the older data can overwrite the newer data. Compare source timestamps or versions, not the time the job happened to run.
- Large clinics produce long jobs. If you add work inside a job, think about what happens when it is killed halfway, which happens on every Heroku dyno restart. Half-applied state needs to be either impossible or repairable on the next run.
- Do not hold database transactions open across calls to the upstream system. MySQL lock waits there show up as slow booking screens, not as sync failures.
- Be careful with job arguments. Pass identifiers and reload, do not pass whole objects, since a queued job can sit for a while and the data will be stale by the time it runs.
- Scheduled runs and on-demand runs triggered by staff share code. A change that is fine for a nightly batch can be too slow or too chatty for a button press at the front desk.

## Upstream limits and failure handling

The upstream systems are not under our control. Rate limits, slow responses and odd outages all happen.

- Back off on failures and keep the backoff bounded. A tight retry loop against a struggling system makes things worse for the clinic and for us.
- Separate errors that deserve a retry from ones that never will. Bad data for one clinician should not block every other clinician in the same run.
- Log enough to tell which clinic and which clinician a failure belongs to, but keep patient details out of logs. Availability is not clinical data, but the resources around it sometimes carry names and other identifying fields.
- When the sync fails, staff should still see the last good availability, ideally with some sign that it is old. Silently showing stale data is bad. Silently showing nothing is worse.
- Credentials and endpoints differ per clinic. Do not hardcode assumptions that hold for one integration, and check what happens for a clinic with no integration at all.

## Before you ship a change

- Test with realistic messy input, not just a tidy single-clinician case. Include a clinician with exceptions, one with several rooms and one with an empty schedule.
- Check what happens to existing bookings when availability shrinks. The sync should not cancel or move them on its own. Surface the conflict instead and let staff decide.
- Look at the migration path. Data already stored in the old shape has to keep working while the new code rolls out, since dynos and workers do not all switch at the same moment.
- Run the change against a copy of real-looking data if you can, and compare the slots before and after for a few clinicians by hand. Diffs in slot output are the quickest way to see an unintended change.
- If you touch the mapping from FHIR fields to internal fields, search for every consumer of those internal fields. Several reports and the slot generator read them directly.
- Leave a short note here if you find another trap. This list is only useful if it keeps growing.
