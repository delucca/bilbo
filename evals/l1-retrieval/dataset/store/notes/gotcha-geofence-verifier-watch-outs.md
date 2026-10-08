---
id: 01JXPBT608CCX4W8EET07HKB66
created: 2025-06-14T01:40-03:00
---

# geofence-verifier: things to watch when changing it

Notes for anyone touching `geofence-verifier`. It decides whether a courier was actually at the drop-off place when they captured proof of delivery. It looks small, but a wrong change here either blocks honest drivers at the door or lets bad proofs through. Most of the traps come from the offline path and from the data it depends on, not from the distance math itself.

## Offline and ordering

The app has to work with no network, so `geofence-verifier` cannot assume it can reach the backend at check time. Whatever it does on the device must give a usable answer without a round trip. Keep the on-device check and the backend re-check clearly separate in your head, and in the code.

- Proofs get captured, queued, and synced later, sometimes much later. The verifier must judge against the location and time recorded at capture, not against whatever the phone reports at sync.
- Do not read "current location" inside a function that is also used by the backend re-check. The backend has no current location for the courier.
- Queued items can arrive out of order. Do not rely on the order of arrival to decide anything about a delivery.
- If you add a new input to the check, old queued proofs will not have it. Handle the missing case on purpose, and do not treat missing as zero or as a pass.

## Location quality

Raw GPS fixes are noisy, and indoors or in dense streets they are worse. The accuracy value that comes with a fix matters as much as the coordinates.

- Never compare only the point to the fence. A fix with a wide accuracy radius can sit inside the fence and still be meaningless.
- Cached or stale fixes from the platform look like normal fixes. Check the fix timestamp, not just that a fix exists.
- Mock-location and similar flags exist on Android. If you change how they are read, test on a real device, because emulators behave differently.
- Battery saving modes change how often fixes arrive. A change that works with a fast update rate can fail quietly with a slow one.

## Fence data and Firestore

Fence definitions come from Cloud Firestore and are cached locally for offline use. A change that touches how they are read or shaped has two sides, the cached copy and the live one.

- Cached fences can be old. If the verifier uses a cached fence, decide what happens when it is stale, and make that visible to whoever reads the result.
- Do not assume a fence document has every field. Older documents may lack fields you add now.
- Firestore listeners can fire from cache first and from the server later. Do not run a final verdict on the first callback only.
- Be careful with query changes that need new indexes, since they can fail only in a deployed environment and not in local tests.
- Security rules apply to these reads. A new field or collection may need a rule change, and the rules are not in the app code.

## Protobuf and compatibility

Proof payloads that carry verifier output are Protocol Buffers messages. Old app versions stay in the field for a long time because drivers do not update fast, and queued messages can outlive an app update.

- Only add fields. Never reuse or renumber an existing one, and never change its type.
- Adding a value to an enum is not safe by default. Older readers see an unknown value, so make sure the default and unknown cases map to something safe and not to "verified".
- Keep the proto default (unset) from meaning success. Use an explicit state for "not checked".
- If the result shape changes, check both the Kotlin client and the backend consumer before merging.

A small example of the kind of result handling to keep explicit:

```kotlin
when (result) {
    Verified -> accept()
    Outside -> flagForReview()
    NotChecked -> retryLater()
}
```

Do not add an `else` branch that falls through to accept.

## Testing and rollout

Unit tests with perfect coordinates prove very little here. The bugs show up at the edges.

- Test points near the fence boundary, on both sides, with poor accuracy values.
- Test with the network off, then on, and with the app killed between capture and sync.
- Test fences with odd shapes or tiny size, and drop-offs in buildings where the fix drifts.
- Replay a few real queued proofs from before your change, to see that they still decode and still get a sane verdict.
- Roll out behind a flag if you can. A stricter verifier hits drivers in the field right away, and they cannot easily retry at the same spot.
- When the verifier rejects someone, the driver needs a way forward. Check that the message and the fallback flow still work after your change.
