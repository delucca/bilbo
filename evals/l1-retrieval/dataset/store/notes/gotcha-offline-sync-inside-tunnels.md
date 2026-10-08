---
id: 01KKH8MYGBCG3S807QQ1K9T3M7
created: 2026-03-12T11:54-03:00
sources:
  - "code: sync/src/main/kotlin/com/parcelpin/sync/ErrorClassifier.kt"
---

# offline-sync-engine: tunnel errors are retryable

When a courier drives through a tunnel, offline-sync-engine gets this error from Firestore: `FirebaseFirestoreException: UNAVAILABLE: Unable to resolve host firestore.googleapis.com`. It is a connectivity failure, not a problem with the data. offline-sync-engine must treat it as retryable, not as a permanent failure. The pending proof-of-delivery record stays queued and is tried again once the network is back. It must never be marked failed, dropped, or shown to the driver as a delivery that did not go through.

This has bitten us because the message looks like a hard error. It names a host and says "Unable to resolve", which reads like a config or DNS mistake on our side. It is not. The phone had no route out, so the lookup of the Firestore host failed. The same call works a few seconds later when signal returns.

## What the failure looks like

- The exception is a FirebaseFirestoreException with code UNAVAILABLE.
- The message text is "Unable to resolve host firestore.googleapis.com".
- It shows up on writes (proof-of-delivery uploads) and on listener reconnects.
- It comes in bursts while the device is in the tunnel, often several per minute, one per attempt.
- It stops by itself when the device regains coverage. No user action is needed.

## What offline-sync-engine must do

Classify UNAVAILABLE with the unresolved-host message as transient. Keep the item in the pending queue with its state unchanged. Schedule another attempt with backoff, and keep the delay capped so a long tunnel or a dead zone does not push the next try far out once signal returns. Reset the backoff after one success.

Do not count these attempts toward any give-up limit. A tunnel can last a long time, and a retry budget meant for real server errors would run out and turn a normal drive into lost uploads. If a limit exists for other error types, this error should not touch it.

Photos, signatures and scan data captured offline are still on the device. Local capture must keep working the whole time. The sync layer only retries the upload and does not block the courier from finishing the next stop.

## What is still permanent

Do not widen this rule to every Firestore error. Errors such as permission denied, invalid argument, or a record the server rejects as malformed will not get better by waiting. Those stay permanent failures and should surface for review. Only the connectivity class is retryable. If you add a new error to the retryable list, check that it really depends on the network and not on the payload or the auth state.

A Protocol Buffers decode problem on our side is also not a network error, even if it happens near an unavailable event. Check the exception type before deciding.

## Testing notes

- Simulate the tunnel by turning off the network on a test device or emulator while a delivery is queued, then turning it back on. Do not rely on only toggling airplane mode once, as the second reconnect path differs.
- Check that the item is still pending after the error, not failed.
- Check that it uploads once, without duplicates, after reconnect.
- Add a unit test that feeds the exact message above into the error classifier and expects retryable.

## Open questions

- Whether the backoff cap should differ between foreground use and background sync.
- Whether we should tell the driver quietly that uploads are waiting for signal, without alarming wording.
