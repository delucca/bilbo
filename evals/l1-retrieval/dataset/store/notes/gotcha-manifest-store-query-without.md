---
id: 01K0C8TB76Y3K99PH9CD6XZP19
created: 2025-07-17T10:22-03:00
---

# manifest-store: queries fail without their composite index

Any query against manifest-store that filters on one field and orders or filters on another needs a composite index in Cloud Firestore. If the index is missing, the call fails with `FAILED_PRECONDITION: The query requires an index`. It is easy to hit because single-field indexes are automatic and many queries work fine until someone adds a second condition. This note records how it shows up, why it bites us, and what to do.

## Symptom

The Kotlin call that reads from manifest-store throws, and the task returned by the Firestore client completes with a failure. The exception message starts with `FAILED_PRECONDITION: The query requires an index` and then adds a long console link that pre-fills the index definition. On a device, the driver sees an empty route list or a spinner that never ends, depending on which screen issued the query.

## Where it happens

It happens on the first run of a new query shape, not on the first run of the app. A developer adds a filter, say by delivery status together with a sort by stop order, tests against the emulator, and everything passes. Production or a shared Firebase project then rejects the same query. Staging usually catches it, but only if someone opens that screen.

## Why the emulator hides it

The local Firestore emulator does not enforce composite indexes the way the hosted service does. Queries that need an index run happily against it. So green local tests say nothing about index coverage. Do not treat a passing emulator run as proof that the query is safe to ship.

## Why it matters for couriers

Drivers work with bad signal and rely on the cached route. The failure is a server-side precondition check, so it appears when the query has to reach the backend. A driver who refreshes a route list after coming back online can suddenly see nothing, which looks like data loss even though nothing was deleted. Support tickets about missing stops often trace back to this.

## Offline behaviour is different

With the local cache, a query can be served from cached documents without the index check. That means the same code can work offline and fail online. If a bug report says the list was fine in the depot basement and broke on the street, suspect a missing index before suspecting sync.

## First thing to check

Read the full exception text. The message includes a link that opens the Firebase console with the exact fields and directions filled in. Copy the fields from that link, not from memory, because direction (ascending or descending) matters and a wrong guess produces an index that does not help.

## How to fix it

Define the index in the project's Firestore index configuration file, keep it in version control, and deploy it with the Firebase CLI. Avoid clicking the console link in production as the only fix, because the index then exists in one project and nowhere in the repository. Creating it by hand also leaves staging and production out of step.

## Build time

A new index is not usable at once. Firestore builds it in the background, and for a large collection that takes a while. Until the build completes, the query keeps failing with the same error. Deploy the index before the app release that depends on it, not together with it.

## Release ordering

The safe order is: add the index definition, deploy it, wait until the console shows it as ready, then publish the app build. Drivers update on their own schedule, so old builds and new builds will run side by side for some time. Indexes are additive, so leaving an old one in place does no harm.

## Review checklist

When reviewing a change that touches manifest-store queries, ask these questions. Does the query combine an equality filter with a range or an order on a different field? Does it use an array-contains clause along with other conditions? Is there a matching entry in the index configuration file? Did anyone run the query against a real project, not only the emulator?

## Common query shapes that need one

Filtering by status and ordering by time. Filtering by courier and ordering by stop sequence. Range filters on one field combined with ordering on another. Inequality filters on more than one field in a single query. Each distinct combination of fields and directions needs its own index, so a small change in sort direction can require a new one.

## Shapes that do not

A query on a single field, with or without ordering on that same field, is served by the automatic single-field indexes. Plain document reads by id never need anything. If you can reduce a query to that form without making the screen worse, that is often cheaper than maintaining another composite index.

## Protocol Buffers angle

manifest-store documents carry fields that are decoded from Protocol Buffers messages on the client. The index is defined on the stored Firestore fields, not on the message schema. If a proto field is renamed or moved into a nested structure, the stored path changes and the old index no longer applies. Check the index file whenever the mapping between message and stored fields changes.

## Testing approach

Add a small integration check that runs the real query shapes against a throwaway Firebase project in CI, not the emulator. It should fail the build when it sees `FAILED_PRECONDITION: The query requires an index`. This is slower than unit tests, so keep it to one call per query shape and run it on the main branch and on changes that touch the data layer.

## Handling the error in the app

Do not swallow the failure. Catch it at the repository layer, log it with the query name, and surface a clear error state rather than an empty list. An empty list tells the driver there is nothing to deliver. Report the failure to crash and error reporting so the missing index is noticed from the field, not from a complaint.

## Things that did not help

Retrying the query does nothing, because the condition is permanent until the index exists. Clearing the app cache does not help either. Changing the Firebase client library version is a waste of time. The only fix is the matching composite index being built and ready.

## Quick triage summary

See `FAILED_PRECONDITION: The query requires an index` on a manifest-store read: find the query shape, copy the fields from the console link into the index configuration file, deploy, wait for the build, and only then retest on a real project. If the error persists after the index shows as ready, compare field paths and directions against the query once more, since a near match is not a match.
