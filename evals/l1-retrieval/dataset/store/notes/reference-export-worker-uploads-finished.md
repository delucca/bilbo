---
id: 01JWN34PQKM7YA5CX6KX0ZZP3G
created: 2025-06-01T03:33-03:00
---

# export-worker reference

export-worker is the part of TownHall Pulse that turns the data of a finished or running event into files people can download, and puts those files in object storage. This note records where it writes and how the pieces around it fit. Keep it short; check the code before trusting any detail that is not stated here.

## Where files go

export-worker uploads finished files to the object storage prefix `exports/v1/` under the event identifier. So a file for one event lives below `exports/v1/` in a folder named for that event's identifier. Anything that wants to find, list or clean up the exports of one event should start from that prefix plus the event identifier.

## Why the prefix has a version

The `v1` part of the prefix is a layout version. If the file layout or naming changes in a way that would break readers, the new layout should go under a new version prefix instead of mixing with the old one. Do not rewrite files already under `exports/v1/` to fit a new layout.

## What it exports

The exports cover the things event producers and community managers ask for after or during an event: poll results, Q&A questions with their status, and moderation actions. The exact column sets are defined in the worker code, not here.

## Inputs

The worker reads from CockroachDB, the main store for the Phoenix app. It does not read from live WebSocket channels. Whatever was committed to the database when the job ran is what ends up in the file.

## Who starts a job

A job is requested from the Next.js dashboard by a producer or manager. The Phoenix side records the request, and the worker picks it up. The dashboard then shows the job as pending, running or done.

## Running it

The worker is an Elixir process under the same release as the rest of the backend, supervised so a crash restarts it. It should not run inside web request handlers, because large events produce big exports that would block them.

## Large events

Big virtual events can hold a lot of rows. The worker reads in pages instead of loading everything into memory, and writes the file as it goes. Keep that behavior when changing queries.

## Consistency

Because exports are built from the database at a point in time, a file made during a live event is a snapshot. A later export may contain more votes and questions. Do not treat two exports of the same event as identical.

## Retries

Uploads can fail on network or storage errors. The worker retries a limited number of times and then marks the job failed. A retried job overwrites its own earlier partial file rather than creating a second one, so readers never see two files for one job.

## Failures

When a job fails, the dashboard shows it as failed and the reason goes to the logs. Look at the worker logs first, then storage access, then database connectivity.

## Access and permissions

The worker needs write access to the bucket under the prefix `exports/v1/`. Read access for downloads is handled separately and should be time limited. Do not make the prefix public.

## Download links

Users do not get the raw storage location. The backend creates a short lived link when someone asks for a finished export. Links should not be stored in the database.

## Retention

Old exports pile up. Clean up by event identifier under `exports/v1/`, using whatever lifecycle rule the storage provider offers rather than a custom sweep, unless the code already has one.

## Testing

Test the worker against a local storage stand in and a local CockroachDB. Check that the uploaded key starts with `exports/v1/` and contains the event identifier. A wrong prefix is the easiest mistake and breaks every reader quietly.

## Things to watch

Moderation data can include removed content. Decide per export type whether removed items are included, and keep that consistent. Personal data in exports should follow the event's privacy settings.

## Open questions

Whether exports should be split into several files for very large events is not settled. Neither is a standard retention period.
