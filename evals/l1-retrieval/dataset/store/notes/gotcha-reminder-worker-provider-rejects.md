---
id: 01KZ2X9MJ1D1BJQ9R13WZCYCZ5
created: 2026-08-03T01:14-03:00
---

# reminder-worker: SMS rejects numbers without country prefix

reminder-worker sends appointment reminders by SMS through the provider's API. When a patient phone number is stored without a country prefix, the provider refuses the message and the reminder never reaches the patient. The error text is `Invalid 'To' Phone Number`. It looks like a provider outage or a bad credential at first glance, but it is neither. It is just the number format. Written in a hurry, so check details against the code before relying on them.

## Symptom

Reminders for some patients silently do not go out. In the Sidekiq dashboard the reminder-worker jobs for those patients show failures, and the provider response carries `Invalid 'To' Phone Number`. Other patients at the same clinic, in the same batch, get their messages fine. That split is the clue: it follows the data, not the provider or the deploy.

## Cause

The provider wants the destination number with a country prefix. Front-desk staff at small clinics type numbers the way the local patients say them, without a prefix. Those numbers get stored as typed. reminder-worker passes the stored value through to the provider, so the provider sees a number it cannot route and rejects it.

## What it is not

It is not rate limiting. It is not a Heroku dyno problem. It is not a Sidekiq connection or MySQL issue. It is not a problem with the FHIR side, although patient contact data can arrive from HL7 FHIR sources too, and those may also lack a prefix. Do not spend time restarting workers; the same input fails again.

## How to confirm

Pick one failing job and look at the number it was given. If there is no country prefix, that is the cause. Then compare with a succeeding job: its number has the prefix. No need for anything cleverer than that.

## Why retries do not help

Sidekiq retries failed jobs. A rejected number is a permanent failure, not a transient one, so each retry gets the same rejection. Retries only fill the retry set and add noise to the dashboard. Worth remembering when reading the queue: a pile of retrying reminder-worker jobs with this message is one data problem repeated, not many problems.

## Where the bad numbers come from

Three sources, as far as I know. Manual entry at the front desk. Imports from older clinic systems. Patient records pulled in over HL7 FHIR, where the telecom value is free text and often has no prefix. Any of these can put an unprefixed number in the patient table.

## Fix direction

Normalize the number to include the country prefix before it goes to the provider, ideally when the patient record is saved, and again in reminder-worker as a safety net. Where the prefix is missing, the clinic's default country is the sensible guess. If the number cannot be normalized with confidence, do not send; mark the reminder as skipped with a clear reason so staff can fix the record.

## Do not guess blindly

Adding a default prefix to every number is wrong for patients with foreign numbers that already carry a prefix in another form. Check for an existing prefix first. Also strip spaces, dashes and brackets before judging.

## Visibility for staff

Front-desk staff currently get no sign that a reminder failed. A skipped or failed reminder should be visible on the appointment, so someone can call the patient or correct the number. Otherwise the failure shows up only as a no-show.

## Logging

Log the patient id and the reason when a number is rejected, not the full phone number. Phone numbers are personal data and the logs on Heroku are shipped elsewhere. Keep the provider error text in the log line so it can be searched.

## Testing

Add a spec with a number lacking the prefix and check that reminder-worker either normalizes it or skips with a reason. Add another with a prefixed number to check nothing changes. Stub the provider; do not call it from tests.

## Related

Appointment data layout questions that touch reminders are in [[appointments-table-direction-chosen]]. Read it before changing where reminder state is stored, since a skipped-reminder flag has to live somewhere on the appointment side.

## Cleanup of existing data

Existing patients with unprefixed numbers need a one-off pass. Do it as a reviewed script, run first against a copy, and keep a list of the records changed so it can be reverted. Do not run it blind against production MySQL.

## Open questions

Whether each clinic should have its own default country setting, or one global value is enough. Whether FHIR-sourced numbers should be normalized on import or only at send time. Nobody has decided yet.

## If it shows up again

Search the worker logs and the Sidekiq dead set for `Invalid 'To' Phone Number`. Count affected patients by clinic. If one clinic accounts for most of them, the cause is probably an import or a front-desk habit there, and fixing that source is better than fixing records one by one.
