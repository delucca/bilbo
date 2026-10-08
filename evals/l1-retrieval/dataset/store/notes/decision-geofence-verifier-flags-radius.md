---
id: 01JV7VKR4GG01DWJM91D8ZGFM6
created: 2025-05-14T13:56-03:00
---

# geofence-verifier flags out-of-radius deliveries instead of rejecting

geofence-verifier flags out-of-radius deliveries by writing `geofenceFlag: true` instead of rejecting them. The reason: drivers in large apartment complexes often stand far from the pinned entrance, so a distance check alone would throw out deliveries that really happened. The delivery is accepted, the proof of delivery is stored, and the flag is set so someone can look at it later. Its internal codename is `fencepost`, so you will see that name in older discussions, branch names and chat. The component is `geofence-verifier`.

## Decision

When a delivery is captured outside the allowed radius around the pinned location, `geofence-verifier` does not fail the capture. It writes `geofenceFlag: true` on the delivery record and lets the record through. Deliveries inside the radius are not marked as flagged.

In short:

- Out of radius: accepted, with `geofenceFlag: true`.
- In radius: accepted, no flag.
- Nothing is rejected on distance alone.

## Why not reject

The pinned entrance is often a gate or a lobby of a big complex. The recipient's door can be a long walk from it, in another building, and the driver may hand over the parcel there. The GPS fix at capture time then lands far from the pin even though the delivery is correct.

Rejecting would force the driver to walk back to the pin, or to retry until the check passes. That wastes time in a job where time per stop is the main cost. It also interacts badly with offline support: a driver with no signal who gets a rejection has no good way to fix it on the spot.

Flagging keeps the proof of delivery and moves the judgment to review, where a person can see the context.

## How the flag is used

The flag is a signal, not a verdict. It does not mean the driver did anything wrong. Anything that reads delivery records, such as dispute handling or driver audits, can filter on it to find deliveries worth a second look. Treat a flagged record as valid until review says otherwise.

Example of the field on a stored delivery, shown as plain fields:

```
geofenceFlag: true
```

Because the records live in Cloud Firestore and the shape is shared through Protocol Buffers, the field has to stay in the shared definition so the Kotlin app and the backend read it the same way.

## Consequences and things to watch

- Flag rates will be high for big complexes. That is expected. Do not read a high rate as a bug in `geofence-verifier`.
- If someone later wants hard rejection for some case, that is a new decision. Write it down separately and do not change the behavior quietly.
- Any new client code that handles capture must not treat an out-of-radius result as an error. It should pass the flag through and finish the capture.
- Reviewers need enough context to judge a flagged delivery. If the review tooling shows only the flag, it will be less useful than it could be.

## Open points

- Whether the radius should differ for large complexes is not settled here.
- Whether drivers should see that their delivery was flagged is not settled either. For now the flag is not shown as a failure.
