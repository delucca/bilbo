---
id: 01KGR4KGDEJJ0MMK1CRZBRRSWB
created: 2026-02-05T21:11-03:00
---

# pin-capture-app: options survey

Notes on the general options looked at for pin-capture-app, the courier-facing Android client that records proof of delivery and has to keep working with no signal. Nothing here is settled. It is a map of the choices so a later session does not redo the survey.

## Scope of the survey

pin-capture-app captures a photo, a signature, a location fix and a few status fields per stop. It must save locally first and sync later. The options below are grouped by concern: UI, local storage, sync, media, wire format, background work and testing.

## UI toolkit

Jetpack Compose is the obvious candidate for new screens: less boilerplate, easier state handling. The classic View system with XML is still viable and has more mature camera and signature widgets. A mixed approach, Compose screens hosting a few View-based widgets through interop, is likely the realistic path.

## Architecture pattern

Single-activity with a navigation graph and ViewModels holding screen state. Unidirectional data flow keeps capture state easy to restore after process death, which matters on low-memory devices. A heavier pattern with separate use-case layers was considered and looks like too much for a small app.

## Dependency injection

Hilt fits the Jetpack stack and cuts wiring code. Manual injection is possible given the small graph. Koin is lighter but moves errors to runtime.

## Local storage

Two real options: rely on the Firestore offline cache, or keep an own Room database as the source of truth for pending deliveries. The Firestore cache is the least code. Room gives explicit control over what is pending, retried or failed, and is easier to query and inspect when a driver reports a lost record.

## Offline cache behavior

The Firestore cache can be evicted and its write queue is opaque. For proof of delivery, losing a pending write is costly, so an explicit local queue is attractive even if Firestore persistence stays on as well.

## Sync strategy

Options: write straight to Firestore and trust its queue; or write to the local database and have a worker push records with retries and idempotent keys. The second is more work but gives clear status per record and safe replays after a flaky connection.

## Conflict handling

Delivery records are mostly append-only from one driver, so conflicts should be rare. Last-write-wins on status fields is probably enough. Dispatch-side edits to the same stop are the case to watch.

## Photo and signature capture

CameraX is the standard choice over the legacy camera APIs. Signatures can be drawn on a custom canvas and stored as a compressed image or as stroke data. Compressing before upload saves mobile data; the amount of compression is open.

## Media upload

Cloud Storage for Firebase handles resumable uploads, which suits bad networks. Uploads should be separate from the record write so a large photo does not block a small status update. Reference the media from the record by a stored path.

## Wire format

Protocol Buffers for the payloads sent to the backend, or for the local queue entries, gives compact and versioned messages. Firestore documents stay as native fields. The open question is whether protobuf is worth it for the queue or only for backend calls.

## Background work

WorkManager is the default for deferred sync with network constraints and backoff. Foreground services are only worth it for long uploads that the system would otherwise kill. Aggressive vendor battery managers are a known risk and need testing on real devices.

## Location

Fused location provider for a fix at the moment of capture. Continuous tracking was considered and set aside as a battery and privacy cost; a single fix plus accuracy value per delivery is the lighter route.

## Auth and security

Firebase Authentication for driver sign-in, with security rules scoping each driver to their own routes. Local data holds customer addresses and photos, so encryption at rest and a wipe on sign-out need a look.

## Testing

Unit tests for ViewModels and the sync queue. Instrumented tests for the database. Offline behavior is best checked with the Firebase emulator suite plus airplane-mode runs on a physical device. Screenshot tests are optional.

## Open questions

- Room as source of truth versus Firestore cache alone.
- Whether protobuf is used on the device side at all.
- How much to compress media before upload.
- Which low-end devices to include in test runs.
