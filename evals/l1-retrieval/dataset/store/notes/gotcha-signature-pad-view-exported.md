---
id: 01M1EZMS55YZQP4X4FSWCBJ9E8
created: 2026-09-01T14:18-03:00
---

# signature-pad-view exported before layout throws on zero size

Exporting the signature from signature-pad-view before the view has been laid out fails. The view has zero height at that point, so there is nothing to draw into, and the export throws `IllegalStateException: Signature bitmap has zero size`. It looks like a random crash in the proof-of-delivery flow, but it is purely an ordering problem. Check the order first.

## Symptom

The courier opens the proof-of-delivery screen and the export is triggered very early, for example from a restored state, an auto-save, or a "done" action that fires right after the screen is created. The app crashes with `IllegalStateException: Signature bitmap has zero size`. The stack trace points at the bitmap creation inside the export path of signature-pad-view, not at the caller, so it is easy to blame the wrong code.

It is worse on slow or low-memory devices, where layout is delayed. On a fast phone the same code path may work most of the time, which hides the bug until drivers hit it in the field.

## Cause

signature-pad-view builds the exported bitmap from its own measured width and height. Before the first layout pass, the height is zero. A bitmap with a zero dimension cannot be created, so the view throws instead of returning an empty image. The view does not wait for layout and does not retry.

Being attached to a window or having had its constructor run is not enough. Only a completed layout pass gives it a real size.

## What to do

- Do not call export on signature-pad-view until it has been laid out. Check that the view has a non-zero width and height before exporting.
- If the export is triggered from screen creation code, post it to run after layout, or use a layout listener that fires once and removes itself.
- If the view is inside a container that is hidden or collapsed, it may still have zero size even after the screen is shown. Make sure the container is visible and measured before exporting.
- Treat "nothing drawn yet" as a separate case from "not laid out". An empty but laid-out pad should return an empty-signature result, or the caller should refuse to submit, depending on the product rule.

## Offline and queue concerns

ParcelPin captures proof of delivery offline and syncs later through Cloud Firestore. A crash during export means the capture is lost, and the courier has to ask the customer to sign again. Guard the export so that a failed or skipped export never leaves a half-written delivery record in the local queue. Only enqueue the proof after the bitmap exists and has been encoded.

If the signature is later stored in a Protocol Buffers message, build that message after the export succeeds. Do not create it with an empty bytes field as a placeholder, because that will pass validation on the backend and look like a valid but blank signature.

## Testing

- Add a UI test that creates the screen and triggers export immediately, with layout delayed. It should not crash, and it should not enqueue a proof.
- Add a test with a laid-out pad that has no strokes, to confirm the empty case is handled on its own path.
- When reproducing by hand, throttle the device or add an artificial delay to layout. Without that, the failure may not show up.

## Notes for later

If this keeps coming back, consider making signature-pad-view fail softly: return a clear "not ready" result instead of throwing. For now the exception is the only signal we get, so keep it visible in crash reports and do not catch and swallow it at the call site. Swallowing it would hide the ordering bug and produce blank proofs.
