---
id: 01M3K5WB74HKQ3NSASY8WKJ08V
created: 2026-09-28T01:55-03:00
---

# waitlist-promoter daily offer cap plan

This note replaces the earlier note about "waitlist promoter offers entry". The new value for the daily offer cap of waitlist-promoter is `WAITLIST_MAX_OFFERS=2`. The earlier planned value was 3 and no longer holds.

## Goal

Stop waitlist-promoter from flooding front-desk staff and patients with slot offers in one day. Two offers per day is the limit we settled on. Keep the change small and easy to undo.

## Decision

The cap is `WAITLIST_MAX_OFFERS=2`. It is a daily cap on offers that waitlist-promoter sends out. Anything that mentions 3 as the cap is out of date.

## Why lower than before

Three offers per day was a guess made before we looked at how small clinics work. Front-desk staff have little time to chase replies. Fewer open offers means fewer double-booking races on the same slot. Two is enough to cover a cancelled slot and one fallback.

## Config

Set it as an environment variable on Heroku for the app. The Rails code reads it at boot and falls back to a safe default if it is missing, so a missing variable never means unlimited offers.

```
WAITLIST_MAX_OFFERS=2
```

## Steps

- Update the config var in each Heroku environment.
- Make the promoter read the value from the environment, not from a constant in code.
- Remove any hardcoded copy of the old planned value.
- Restart the dynos so the new value is picked up.
- Watch the first day of offers to confirm the cap holds.

## Where it is enforced

The check belongs in the Sidekiq job that builds offers, before anything is sent. Count offers already made that day, compare to the cap, and stop when it is reached. Do not rely on the UI to enforce it.

## Counting rules

Count each offer made, whether or not the patient answers. An expired or declined offer still counts toward the day. This keeps the cap predictable. If we later want declined offers to free up the count, that is a separate decision.

## Concurrency

Several Sidekiq workers may run at once. Two workers could both see one offer left and both send. Use a database-level guard in MySQL, such as a locked row or a unique constraint, so the cap cannot be exceeded by a race.

## Rooms and clinicians

The cap does not replace availability checks. An offer must still respect clinician availability and room limits. See [[room-constraints-must-define]] for how room constraints are meant to be defined.

## FHIR

Offers that touch HL7 FHIR resources should not change shape because of this cap. The cap only decides whether an offer goes out. Slot and appointment resources stay as they are.

## Tests

- A test that sends offers up to the cap and checks the next one is refused.
- A test that two concurrent jobs cannot both take the last offer.
- A test that a missing variable uses the default and not unlimited.

## Rollout

Change the variable in staging first, run the tests, then production. Do it outside clinic opening hours if possible, since the restart interrupts jobs briefly.

## Rollback

If clinics complain that slots go unfilled, raise the variable and restart. No migration is needed for that. Record any new value in this note.

## Open questions

- Should the cap be per clinic rather than global? Not decided.
- Should staff be able to override it by hand for one day? Not decided.

## Things to avoid

- Do not copy the old planned value of 3 back into docs or code.
- Do not add a second cap name for the same limit.

## Status

The value is decided. The code and config changes are still to do. Update this note when they land.
