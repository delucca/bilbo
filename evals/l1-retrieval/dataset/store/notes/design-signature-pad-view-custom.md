---
id: 01K97QNMY6B2HW8AJEPT9ZVE8Q
created: 2025-11-04T12:27-03:00
sources:
  - "code: signature/src/main/kotlin/com/parcelpin/signature/SignaturePadView.kt"
---

# signature-pad-view design

signature-pad-view is a custom Android View in the ParcelPin courier app. It draws Bezier-smoothed strokes on a Canvas while the driver signs, and it exports the finished signature as a PNG of 480x240 pixels. The export is what gets attached to a proof of delivery record.

## Scope

The view only handles capture and export of the signature image. It does not upload anything, and it does not decide where the PNG goes. The surrounding screen owns that, including the offline queue.

## Where it lives

It is a plain View subclass used inside a Jetpack-based screen. It can be placed in a layout or hosted from a Compose wrapper. The host screen holds the reference and asks it for the image when the driver confirms.

## Input handling

Touch events from the driver's finger or stylus are collected as points. Each move event adds a point to the current stroke. Lifting the finger ends the stroke. A new touch starts another stroke, so a signature can have several strokes.

## Stroke smoothing

Raw touch points look jagged, so consecutive points are joined with Bezier curves instead of straight lines. The curve control points are derived from neighbouring points. The result is a smooth line even when the device samples touch input slowly.

## Drawing

Drawing happens in onDraw on the Canvas. Finished strokes and the stroke in progress are both painted each frame. Paint settings are kept in fields so no allocation happens per draw call.

## Export

Export renders the strokes to an offscreen bitmap and compresses it to PNG. The output size is always 480x240 pixels, whatever the size of the on-screen view. Strokes are scaled to fit that size.

```kotlin
// export contract
val png: ByteArray = signaturePadView.exportPng() // 480x240
```

## Empty state

If nothing has been drawn, the view reports that it is empty. The host screen should check this before asking for an export, so a blank image is not saved as a signature.

## Clearing

The host can clear all strokes and reset the view. Clearing also resets the empty flag.

## Configuration changes

Stroke data should survive rotation and process recreation if the host screen saves it. The view can hand its points back out and take them in again. The PNG alone is not enough to restore editing.

## Offline behavior

Nothing in this view needs the network. The exported PNG is handed to the app's offline storage and synced later with the rest of the delivery data.

## Testing

Check the exported image dimensions in an instrumented test. Draw a few strokes with simulated touch events, export, decode, and compare width and height. Also check the empty case.

## Known rough spots

Very fast strokes can still show small corners. Very short taps leave a dot or nothing, depending on the paint cap. Both need a device check.

## Open items

- Decide whether the host or the view owns the undo of the last stroke.
- Check behavior on small screens with large font scaling.
- Confirm stroke width looks right on the exported image at several screen densities.
