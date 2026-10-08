---
id: 01KTT29BE8X38VWHPRJHCRS9HX
created: 2026-06-10T21:46-03:00
---

# signature-pad-view spec

This note replaces the earlier note about "signature pad view accepts". The current rule: signature-pad-view accepts a signature only when it holds at least 35 touch points. The earlier minimum was 20 touch points, and that value is no longer valid.

## Purpose

signature-pad-view is the custom Android view where a recipient signs on the courier's phone as part of proof of delivery. The courier hands the phone over, the recipient draws a signature, and the app keeps the result with the rest of the delivery record. The view is written in Kotlin and sits inside the Jetpack-based app. It has to work with no network, because drivers often sign in stairwells, basements and rural spots.

## Current minimum

The minimum is 35 touch points. A stroke set with fewer than 35 touch points counts as too short and is not accepted as a signature. The old minimum of 20 touch points let through scribbles and accidental taps that did not look like a signature, which is why it was raised. Anyone reading older notes, tickets or test names that mention the lower figure should treat them as out of date.

## What counts as a touch point

A touch point is one sampled position reported by the touch system while the finger or stylus is down on the view. Move events add points. The initial down event adds a point too. Points are counted across all strokes of the current signature, not per stroke. A signature made of several short strokes can pass if the combined total reaches the minimum.

## What does not count

Hover events, events outside the bounds of the view, and events delivered after the view has been cleared do not count. Points that come from a cancelled gesture are discarded together with that stroke. If the system cancels a stroke midway, the count drops back to what it was before that stroke began.

## Where the check runs

The check lives with the view, so every screen that embeds signature-pad-view gets the same rule. Screens must not keep their own copy of the threshold. If a screen needs to know whether the signature is acceptable, it asks the view. This keeps the value in one place and avoids two screens disagreeing about what a valid signature is.

## Behavior below the minimum

While the count is under the minimum, the view reports the signature as not ready. The confirm action stays disabled in the screens that use it. The view does not throw and does not show a modal. It simply stays in the not-ready state and lets the courier or the recipient keep drawing until the count reaches the minimum.

## Behavior at or above the minimum

Once the count reaches 35 touch points, the view reports the signature as ready and the confirm action becomes available. Drawing more after that is fine and does not change the state. Clearing the pad resets the count and returns the view to not-ready.

## Feedback to the recipient

The recipient should understand why confirm is disabled. The surrounding screen shows a short hint that the signature is too short. The wording of that hint belongs to the screen and its string resources, not to the view. Do not hard-code the minimum into user-facing text, so a later change to the threshold does not leave stale copy behind.

## Offline behavior

The rule is evaluated locally on the device, with no backend call. This matters because proof of delivery must be capturable offline. The captured signature is stored locally first and synced later through the normal offline path to Cloud Firestore, along with the other delivery data.

## Stored data

What is stored is the signature itself, in the same form as before this change. The threshold affects only whether a signature is accepted at capture time. It does not add a field to the stored record, and the Protocol Buffers message shapes used for sync are unchanged by this rule.

## Backend considerations

The backend does not re-count touch points. It trusts the app to have enforced the minimum at capture. Firebase rules and any server-side checks do not know about this threshold. If a stricter server-side check is wanted later, it needs its own spec, since the stored form may not keep the raw point count.

## Older app versions

Devices running an older build of the app can still produce signatures that passed the lower minimum. Those records are already stored and stay valid. Nothing migrates or rejects them. Only captures made with a build that includes this change are held to 35 touch points.

## Accessibility

Some recipients cannot draw a long signature easily, for example people with limited hand control. The minimum applies to everyone, so couriers need a fallback process for those cases, handled outside this view. The view itself has no bypass, and it should not gain a hidden one without a decision recorded in a separate note.

## Testing

Unit tests should cover the boundary: one point below the minimum is not ready, exactly the minimum is ready. They should also cover multi-stroke totals, cancelled strokes, and clearing the pad. Any existing test that encodes the old lower value needs to be updated, not loosened. Instrumented tests can simulate touch input and confirm the confirm action toggles correctly.

## Known gotchas

Touch sampling rate differs between devices, so a quick short signature may produce fewer points on a slow-sampling screen than on a fast one. The count is a rough proxy for effort, not a measure of signature quality. Watch for reports from drivers with older phones who say a normal signature will not confirm.

## Open questions

Should the minimum depend on the physical size of the pad or on stroke length instead of raw point count? Should the threshold be remotely configurable through Firebase so it can change without an app release? Neither is decided. For now the value is fixed at 35 touch points inside signature-pad-view.
