---
id: 01JRJPH8Q60N8DYMXW31YTASFK
created: 2025-04-11T12:12-03:00
sources:
  - "code: firestore.indexes.json"
---

# manifest-store: composite index for the daily stop list

We decided that manifest-store carries a composite Cloud Firestore index on `driverId` ascending and `scheduledAt` ascending. The index is declared in `firestore.indexes.json`. The reason is simple: the daily stop list for one driver is the hottest query in the whole system, so it gets an index built for it.

## Decision

manifest-store defines one composite index over two fields, in this order: `driverId` ascending, then `scheduledAt` ascending. It lives in `firestore.indexes.json` and is deployed with the rest of the Firebase config. Anyone changing the stop list query should look at that file first.

## Why this query

Every driver opens the app at the start of a shift and loads the list of stops for the day. That read happens on every device, many times a day, and again on every refresh. It is the most frequent read manifest-store serves. Other queries exist, but none comes close in volume.

## What the query does

It filters stops by one driver and orders them by scheduled time. Equality on `driverId` plus a range or order on `scheduledAt` is exactly the shape a composite index serves. Without the index, Firestore rejects the query, so this is not only a speed matter.

## Why ascending on both

Drivers work the route from the earliest stop to the latest. Ascending `scheduledAt` returns stops in the order the driver will visit them, with no sorting on the client. Keeping `driverId` ascending is the default and there is no reason to flip it, since we only ever match one value.

## Field order matters

`driverId` goes first because it is an equality match. `scheduledAt` goes second because it is the sort and range field. Swapping them would give an index that does not fit the query. If someone adds a third field, it goes after these two and needs its own discussion.

## Offline behavior

The app supports offline use, so the stop list is read from the local Firestore cache when the device has no signal. The index definition does not change what the cache does, but the same query shape is used online and offline. Keep the query identical in both cases so the driver sees the same order.

## Alternatives considered

We thought about sorting on the device after fetching all stops for a driver. That moves work to low-end phones and returns more data than needed. We also thought about separate collections per driver. That complicates the data model and the rules for little gain. The composite index is the smallest change that fits.

## Cost and trade-offs

An index costs extra storage and adds a small write cost for each stop document written or updated. Stops are written far less often than the list is read, so the trade is good. We accept it.

## How to check it

After editing the index file, deploy it with the Firebase CLI and wait for the build to finish before shipping an app version that relies on it. Building can take a while on large collections.

```json
{
  "collectionGroup": "stops",
  "queryScope": "COLLECTION",
  "fields": [
    { "fieldPath": "driverId", "order": "ASCENDING" },
    { "fieldPath": "scheduledAt", "order": "ASCENDING" }
  ]
}
```

The collection name above is illustrative; match whatever manifest-store actually uses.

## Risks

If the index is missing in an environment, the stop list fails to load for drivers, which blocks their day. Make sure every Firebase project, including staging, gets the same index file.

## Revisit when

Revisit this if the daily stop list query changes shape, for example by adding a status filter, or if a different query overtakes it in volume. Until then, this index stays as is.

## Open points

Nothing is open right now. If the stop list ever needs paging, note it here and check that the same index still serves the paged query.
