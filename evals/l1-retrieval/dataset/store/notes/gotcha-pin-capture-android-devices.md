---
id: 01M0JENC10JPFVAM1S6FCX57BC
created: 2026-08-21T12:22-03:00
---

# pin-capture-app crashes on Android 13 gallery reads without READ_MEDIA_IMAGES

On Android 13 devices pin-capture-app crashes the moment it reads gallery images if the READ_MEDIA_IMAGES permission has not been granted. The crash is a hard process death, not a handled error, so the driver loses the screen they were on and any unsaved proof-of-delivery state with it.

## Symptom

The app dies with `java.lang.SecurityException: Permission Denial: reading com.android.providers.media.MediaProvider` in the stack trace. It shows up when a courier taps the option to attach an existing photo from the gallery instead of taking a new one with the camera.

## Who is affected

Only drivers on Android 13 handsets. Older Android versions still work with the legacy storage read permission, which is why this slipped past testing on older test phones. Camera capture is not affected, because it does not go through the media provider.

## Cause

Android 13 split the old broad storage read permission into per-media-type permissions. Images now need READ_MEDIA_IMAGES. Our code still assumed the old storage permission was enough, so the content resolver query against the media provider was rejected by the system and threw the SecurityException.

## What was wrong in the code

The gallery picker path queried the media store directly without checking for the new permission first. There was no try/catch around the query either, so the exception went all the way up and killed the process.

## Fix direction

Declare READ_MEDIA_IMAGES in the manifest for Android 13 and above, and keep the legacy permission limited to older versions. Request it at runtime before the gallery picker opens. If the driver denies it, show a short message and leave the camera flow available.

## Guard rails

Check the permission state every time before querying, not only on first launch, since users can revoke it in system settings while the app is in the background. Wrap the media query so a SecurityException turns into a denied state instead of a crash.

## Offline considerations

Because this app is built for offline delivery capture, a crash mid-capture is worse than usual: a driver without signal cannot easily retry. Make sure pending captures already queued for Cloud Firestore sync are persisted before the gallery step starts.

## Testing

Test on a real or emulated Android 13 device with the permission freshly denied, then revoked after being granted. Both paths need to land on the denied state with no crash.

## Things to watch

Do not assume a permission model that works on one Android version carries over to the next. Any new place that reads media should go through the same permission check helper rather than querying directly.

## Status

Known trap, documented here so nobody reintroduces a direct media read without the permission check.
