---
id: 01K0X069HV8JJQM3GH6S0VH0YH
created: 2025-07-23T22:19-03:00
---

# export-worker reference: upload prefix exports/v2/

This note replaces the earlier note about where export worker uploads finished files. The new value is `exports/v2/`: export-worker now uploads finished files to the object storage prefix `exports/v2/`, and the earlier prefix `exports/v1/` is no longer where new files go.

The internal codename of export-worker is `csvshipper`. You will see that name in older chat threads, in some dashboards, in a few log fields and in the names of some old tickets. It is the same component as `export-worker`. In this note I use `export-worker` everywhere. If you search the logs or the tracker and find nothing under one name, search the other.

## What export-worker is

export-worker is the background component of TownHall Pulse that turns the data from a live event into files that a producer or community manager can download after the fact, or during the event if they need a snapshot. The data it exports is mostly poll results, the full Q&A history, and the moderation trail. It exists because the live path (Phoenix channels over WebSockets) is tuned for low latency fan-out to a very large audience and is the wrong place to build large files. Building a file for a big event can take real time and memory, and we do not want that anywhere near the processes that hold sockets open.

The component is an Elixir application. It runs as its own deployable, separate from the Phoenix web tier that serves the live event traffic. It reads from CockroachDB, builds files, uploads them to object storage, and records in the database that the file is ready. The Next.js front end, which the producers use in the dashboard, asks the backend for the status of an export and shows a download link once the file is ready. The front end never talks to export-worker directly. It goes through the Phoenix API, which reads the export record.

The main kinds of output are tabular files for polls and for questions, plus a moderation log that records who hid, approved, answered or removed what and when. The tabular files are the reason for the old codename: the first version only shipped comma separated files, and the name stuck even after other formats were added. Do not read anything into the codename today. It does not mean the worker only does comma separated output.

## Where finished files go

This is the part that changed, and the reason for this note.

Finished files are uploaded under `exports/v2/` in the object storage bucket that export-worker is configured to use. The older prefix `exports/v1/` was used before. Anything that assumed the older prefix needs to be looked at. That includes:

- Any script or runbook step that lists or fetches files by prefix.
- Any lifecycle or retention rule on the bucket that is scoped to a prefix. A rule written for the older prefix will not cover files under `exports/v2/` unless it was also added there. Check this before assuming old cleanup behaviour still applies.
- Any access policy that grants read to a prefix. The same warning applies: a policy written for the older prefix does not automatically cover the new one.
- Any support habit of going to the bucket by hand to find a customer's file. Go to the new prefix first for recent exports.
- Any dashboard or alert that counts objects under a prefix. Counts under the older prefix will fall off, and counts under the new one will start from nothing. That is expected and not an outage.

Files that were already uploaded under `exports/v1/` were not described to me as moved. Treat them as still living where they are unless someone confirms they were migrated. The export record in CockroachDB holds the location of each file, so a download link for an old export should keep working as long as the record points at the old location and the old object has not been removed. Do not rewrite old records to point at the new prefix by hand. If you need old exports under the new layout, that is a migration job, and it should be agreed first.

The reason for the version in the prefix is simple: the layout and naming inside the prefix can change, and a version in the path lets old and new files live side by side without clashing. If the layout changes again, expect another version segment rather than a silent change under `exports/v2/`. Keep that in mind when writing any tooling: do not parse the part of the key after the prefix more than you must, and prefer reading the location from the export record.

## How an export runs

The lifecycle below is the general shape. I am keeping it free of exact names and numbers on purpose, since those move around and the code is the authority.

### Request and queueing

A producer or community manager asks for an export in the dashboard. The Phoenix API validates that they are allowed to export that event, creates an export record in CockroachDB with a pending status, and returns right away. The record carries who asked, which event, which kind of export, which format, and any filters such as a time window or only approved questions. The worker finds pending records and picks them up. Picking up is done in a way that two workers cannot take the same record, using the database as the coordination point rather than a separate queue service. This keeps the moving parts few. The cost is that the database sees polling traffic from the worker, which is light but not zero.

Because CockroachDB is a distributed database with serializable transactions, claiming a record can hit a transaction retry under contention. The worker code treats retry errors from the database as normal and retries the claim. If you see retry noise in the logs around claiming, that is usually contention and not a bug, unless it never settles.

### Building the file

Once a record is claimed, the worker reads the event data in pages rather than in one query. This matters for large events with a huge number of questions and votes. Reading in pages keeps memory flat and keeps each transaction short, which is friendlier to the database. The worker streams rows into the file writer as they arrive, so memory does not grow with the size of the event. The file is written to local temporary storage on the worker host first, and only uploaded when complete.

Consistency is worth knowing about. An export of a live event is not a single instant snapshot unless the worker reads at a fixed timestamp. CockroachDB supports reading as of a past time, and that is the natural tool for a consistent export. If an export is taken while the event is still running and the numbers look slightly off from what the dashboard showed a moment ago, the first thing to check is whether the read was at a fixed time or not. Votes keep arriving during a live poll, so two views a few seconds apart will differ.

Moderation state is part of what makes exports tricky. Questions can be hidden, restored, merged or removed by moderators in real time. Exports have to decide what to do with hidden and removed items. The default is that exports for the organiser include everything with its moderation status, while any export meant to be shared more widely leaves out removed content. When someone says the export contains a question they removed, check which export kind they chose and which filters were on before assuming a bug.

Personal data is the other sensitive point. Attendee names or identifiers may appear in the data. Exports follow the same visibility rules as the dashboard for the person asking. The worker should not widen what a requester can see. If a rule changes in the web tier, the worker has to follow it, and this is one of the places where the two sides can drift apart. When changing visibility rules, test the export path as well as the live path.

### Upload and completion

When the file is fully built, the worker uploads it to object storage under `exports/v2/`. Uploads of large files use the multipart style of upload provided by the storage service, so a failed part can be retried without starting over. After the upload succeeds, the worker updates the export record to ready and stores the location. Only then does the front end show a download link.

The order matters and should not be changed: upload first, record second. If the record said ready before the object existed, a producer could click a link that fails. If the worker dies between the upload and the record update, the result is an orphan object and a record that is still in progress. The next attempt will build again, and the orphan is harmless apart from storage. Leftover objects are the reason lifecycle rules on the prefix matter.

Download links given to users are time-limited signed links created by the Phoenix API at the moment of click, not stored links. A link that has expired does not mean the file is gone. Ask the user to reload the page and click again.

### Failure and retry

A failed export is marked failed with a short reason that the dashboard can show, and the full error goes to the logs. Some failures are retried automatically with a pause between attempts that grows each time, up to a limit, and then the record is marked failed for good. Typical retryable failures are transient network errors to object storage and database retries. Typical permanent ones are an event that no longer exists, a requester who no longer has permission, or a filter that cannot be satisfied.

A record that stays in progress for a long time with no worker activity is stale. The worker has a way to reclaim such records after a timeout so a crashed worker does not leave exports stuck forever. If you restart workers during an incident, expect some in-progress exports to be rebuilt from the start rather than resumed.

## Operating notes

These are working habits, not a formal runbook.

When a user says an export is missing, go in this order. First look at the export record for the status and the stored location. If the status is ready, check that the object exists at the stored location, and note which prefix it is under. If it is under `exports/v2/`, it was produced after the change. If it is under the older prefix, it was produced before. If the status is failed, read the reason, then the logs, searching for the export identifier. If the status is pending for a long time, check that workers are running and are connected to the database. Only after those, suspect the front end.

When searching logs or dashboards, remember the two names. Older log lines and some metric labels may use `csvshipper`, newer ones may use `export-worker`. Build searches that match either. Do not conclude that the worker stopped logging just because one name went quiet.

Storage permissions are a common cause of sudden upload failures after a configuration change. Since the prefix changed, any credentials used by the worker must be allowed to write under `exports/v2/`. If the worker's role was scoped narrowly to the older prefix, uploads will fail with an access denied style error from the storage service while everything else looks healthy. The same goes for the role used by the Phoenix API to sign download links: it needs read access to the new prefix.

Disk on the worker host is the second most common cause of trouble. Files are built locally before upload, so many large exports at once can fill the disk. Temporary files should be removed after a successful upload and after a permanent failure. If a worker is killed mid-build, its temporary files can stay behind, so the host needs a periodic sweep or a restart policy that clears the temporary area. Keep the number of exports built at the same time on one host limited by configuration. Raising it without watching memory and disk is how you get an incident during a big event.

Load shape: exports cluster right after an event ends, when every producer wants their results. This is predictable. The worker should have enough capacity to drain the queue in a reasonable time after a big event, and the database load from exports should not compete with live traffic for other events running at the same time. Reading from a past timestamp, and running exports at lower priority, are the levers we have. If live events are suffering while exports run, throttle the exports, not the live path.

Deploys: the worker can be deployed without touching the web tier. A deploy stops the old instances, and in-progress exports on them go stale and get reclaimed. Avoid deploying the worker in the middle of a large event's closing minutes if you can, since that is when exports are requested most.

Configuration: the target bucket and the prefix are configuration, not hard-coded constants, as far as I know. If you need to point a staging environment somewhere else, change configuration rather than code, and use a separate bucket or at least a separate prefix so staging files never mix with real customer files. Whether the version segment itself is configurable or baked into the code was not stated to me, so check the code before relying on either.

## Things to check when touching this component

A short list for anyone changing export-worker or something that depends on it:

- Anything that builds or parses a storage location should use the stored location from the export record, with `exports/v2/` as the prefix for new files. Do not hard-code the older prefix anywhere new.
- Bucket lifecycle rules, access policies and monitoring that mention the older prefix need a matching entry for `exports/v2/`. Decide separately what to do with the old prefix: keep it readable until old exports expire, then retire it.
- Docs, support macros and runbooks that mention where exports live should be updated. The earlier note on this subject said the older prefix and is superseded by this one.
- Tests that check upload keys should assert the new prefix. A test that still asserts the older one is either stale or covering a path that should no longer be used.
- Adding a new export format means thinking about moderation visibility, personal data, and memory use for large events, in that order. Streaming and paging are not optional.
- Renaming things: the codename `csvshipper` still shows up in places. Renaming those places is cosmetic and low priority, but do it in one pass with a note in the change description, so people searching for either name can follow the trail.

## Open questions

Things I do not know and did not want to guess:

- Whether files under `exports/v1/` will be migrated to the new prefix, or left to expire in place. Until someone decides, leave them alone.
- How long exports are kept in storage before removal, and whether the retention rule has been set up for the new prefix. Check the bucket rules directly.
- Whether the version segment will be bumped again for the next layout change, or whether the layout will now stay stable. Assume it can change and read locations from records.
- Whether every environment, including staging and any customer-specific deployments, has moved to the new prefix at the same time as production. Check each environment's configuration rather than assuming.

If any of these get answered, update this note in place rather than writing another one. The subject is the same: where export-worker puts finished files, and how the component behaves around that.
