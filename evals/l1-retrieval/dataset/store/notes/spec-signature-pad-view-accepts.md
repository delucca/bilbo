---
id: 01KRMH4G3P5YEF3VRF4G2RFYYY
created: 2026-05-14T21:38-03:00
---

# signature-pad-view spec

This note specifies how signature-pad-view behaves in the ParcelPin courier app. It is the Android view where a recipient signs on the driver's phone as part of proof of delivery. It is written fast, so treat it as the working contract, not polished documentation.

## Purpose

signature-pad-view captures a handwritten signature from the recipient and hands it to the delivery flow as part of the proof of delivery record. It has to work with no network, because drivers are often in stairwells, basements and rural stops. It does no upload itself. It only produces a signature result that the caller stores.

## Acceptance rule

signature-pad-view accepts a signature only when it contains at least 20 touch points. Anything below that is not a signature as far as the view is concerned, and the view reports it as not accepted. The rule is about the count of recorded points, not about how long the finger was down and not about how large the drawing is on screen. A reader of this note alone should take away: the threshold is 20 touch points, and the check is a minimum, so exactly 20 passes.

## Why the threshold exists

Before the rule, accidental taps, a palm brushing the screen and a single dash all produced a signature that looked valid in the record. Support then had deliveries with a blank or near blank signature and no way to defend them. A minimum point count is cheap to check on the device and works offline. It does not prove the signature is genuine, and nobody should claim it does.

## What counts as a touch point

A touch point is one sampled position of the pointer recorded while it is down on the pad. Down, move and up events each contribute their sampled positions. Historical samples that Android batches into a single move event are counted as points too, so a fast stroke is not penalized for the batching. Points are counted across all strokes in the current signature, not per stroke.

## Duplicate and near duplicate points

The view drops a point that is identical to the previous one in the same stroke, since a resting finger would otherwise inflate the count. Dropped duplicates do not count toward the minimum. We do not smooth or thin points before counting beyond that. If someone adds thinning for rendering, the count must still be taken from the recorded points, not from the thinned ones.

## Multiple strokes

A signature can have several strokes, for example a name and then an underline or a dot over a letter. All strokes are summed for the acceptance check. A single long stroke and many short ones are treated the same way. The order of strokes is kept, because the stored form preserves it.

## Rejection behavior

When the count is too low, the view does not return a signature result. It reports a rejection to the caller and keeps the drawn marks on screen so the recipient can keep drawing and try again. The confirm action in the surrounding screen stays disabled or refuses while the pad is below the minimum. The user facing wording of the message belongs to the screen and the string resources, not to this view.

## Clearing

The clear action removes all strokes and resets the point count to nothing. After a clear the view is back to the empty state and is not accepted. Clearing must not leave stale points that would let a later small mark pass the check by adding to old ones.

## Rendering

Strokes are drawn on a custom View canvas with a round cap and a fixed pen width that follows screen density. The line is drawn through the recorded points, with simple curve smoothing between them. Rendering is cosmetic. It must not change which points are recorded or counted. A bitmap export, if the caller asks for one, is drawn from the same stroke data.

## Lifecycle and rotation

Strokes survive configuration changes such as rotation. The point data is saved through the usual Android Jetpack state mechanism, not by keeping the bitmap. After restore, the acceptance check gives the same answer as before the rotation. If the process is killed mid signature, the partial signature is lost and the recipient signs again; we accept that.

## Offline behavior

Nothing in the acceptance check touches the network. The view validates locally and the result is queued with the rest of the proof of delivery for later sync. A signature accepted offline stays accepted when it is uploaded; the backend does not run a different rule against it in a way that could reverse the decision silently.

## Serialization

When the signature is accepted, the caller turns the strokes into the Protocol Buffers message used by the delivery record. Each stroke is a list of points in the pad's own coordinate space, with the pad size kept alongside so the signature can be redrawn at another size. The view exposes the stroke data and leaves message building to the data layer. Keep the field layout backward compatible when changing it, since old app versions are still out with drivers.

## Storage in Firestore

The serialized signature is attached to the delivery document in Cloud Firestore, or referenced from it if it is stored elsewhere, using the same sync path as the other proof of delivery parts. The view knows nothing about collections or document ids. Firestore offline persistence plus the app's own queue handle delivery of the write when connectivity returns.

## Accessibility

The pad has a content description that says it is a signature area. Drawing is inherently touch based, so there is no keyboard alternative inside the view. If a recipient cannot sign, the flow has a separate path, such as a driver note with a reason, and that path does not go through this view or its minimum.

## Testing

Unit tests cover the counting logic with synthetic point lists: just under the minimum fails, exactly at the minimum passes, duplicates do not count, and a clear resets everything. Instrumented tests drive real touch events for a simple signature and check that rotation keeps the result. Keep the threshold in a single named constant so tests and the view cannot drift apart.

## Gotchas

- Counting batched historical samples is easy to forget, and forgetting it makes fast signatures fail on slow devices.
- A multi touch gesture must not add points from a second pointer into the stroke of the first.
- Do not count points from a cancelled gesture; the stroke is discarded.
- Do not compute the count from the smoothed path.
- Changing the threshold needs a product decision, not just a code change, because it alters what counts as valid proof.

## Open questions

Whether to add a minimum drawn extent in addition to the point count, so a tiny scribble in one corner with enough points still gets rejected. Whether the backend should record the point count for audits. Neither is decided; until then the only acceptance rule is the touch point minimum above.
