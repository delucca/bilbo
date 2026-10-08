---
id: 01KJE80PMAZZRH0RF3TZPXPRE4
created: 2026-02-26T21:29-03:00
sources:
  - "doc: GPS accuracy field study"
---

# geofence-verifier: GPS error research

A field study of GPS readings looked at what geofence-verifier actually gets as input from drivers' phones. In dense city streets the horizontal error reached up to 40 meters at the 95th percentile. So in tall-building areas, one in twenty readings can be worse than that, and any check in geofence-verifier that treats a raw fix as exact will misjudge some deliveries.

## Why this matters

geofence-verifier decides whether a courier was close enough to the drop-off point when they captured proof of delivery. If the reported position is off by tens of meters, a driver standing at the right door can look outside the fence. The reverse can also happen: a driver on the next street looks inside it.

## What the study found

- Error is worst in dense city streets, where buildings block and reflect satellite signals.
- The 95th percentile figure is the upper end, not the typical case. Most readings are better.
- The error is horizontal. Altitude was not the concern here.
- The study measured raw readings from phones, before any filtering on our side.

## Implications for the fence radius

A fence radius smaller than the worst-case error will reject honest drivers in cities. A radius much larger than it makes the check weak. The radius probably needs to depend on the area type, or on the accuracy value the device reports with each fix, rather than being one constant everywhere.

## Offline behavior

ParcelPin captures proof of delivery offline. That means the verification may run later, against a stored fix, with no way to retake the reading. The accuracy estimate for each fix should be stored with it, so the verdict can account for uncertainty when it is evaluated.

## Open questions

- Whether to reject fixes with poor reported accuracy or accept them with a wider fence.
- Whether a short window of several readings, averaged or filtered, beats a single fix.
- How to flag borderline results for review instead of failing them outright.
- How the accuracy field should be carried in the Protocol Buffers messages and in the Cloud Firestore records.

## Sketch of the rule

Not implemented. Just the shape of the idea:

```text
effective_radius = fence_radius + min(reported_accuracy, 40 meters)
inside = distance(fix, drop_off) <= effective_radius
```

The cap keeps a bad accuracy report from making the fence meaningless.

## Next steps

- Check how geofence-verifier currently uses the accuracy value from the Android location API.
- Pull a sample of real city deliveries and see how many would flip verdict under the rule above.
- Decide on the borderline handling before changing the radius.
