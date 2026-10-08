---
id: 01KEK73QHCSVDYK7GNC4HJ3RXN
created: 2026-01-10T02:47-03:00
---

# Deleting a field in delivery-proto

Notes from being bitten by removing a field from the delivery-proto schema. Written quickly, not a full writeup. The short version: deleting a field from a message in delivery-proto looks like a harmless cleanup and it is not, because old app builds in drivers' hands keep talking in the old shape for a long time, and because the same messages end up stored in places we do not control.

## What goes wrong

The obvious move is to delete the line from the message, regenerate the Kotlin classes, fix the compile errors and ship. The compile errors are the easy part. The hard part is that nothing fails loudly afterwards.

Three things happen quietly:

- The tag the field used to have is now free. If anyone adds a new field later and it gets that tag, old data and old clients will read the new field as if it were the old one. A string can turn into a nested message, or a number can turn into a flag. Sometimes the parse fails, and sometimes it succeeds with garbage, which is worse.
- Old clients still send the removed field. The new side silently treats it as an unknown field. Depending on the runtime settings, unknown fields are either kept and re-serialized or dropped. If a service in the middle re-serializes and drops them, data a newer client wanted to keep is lost on the round trip.
- Anything persisted in the old shape is still sitting there. For ParcelPin that means queued proof-of-delivery records on the device waiting to sync, and whatever we put in Cloud Firestore.

The offline support is what makes this nastier than in a normal backend. A courier can capture a delivery with no signal, the app stores it locally, and it uploads hours or days later, possibly after the app was updated in between. So the writer and the reader are not on the same schema version, and we cannot assume they are.

## The rule we should follow

Do not delete a field outright. Mark it deprecated first, stop writing it, wait until the old builds are gone, and only then remove it, and when you remove it, reserve both the tag and the name so neither can be reused.

A reserved block looks like this, with the placeholders replaced by the real old tag and name from the removed field:

```proto
message Delivery {
  reserved <old field number>;
  reserved "<old field name>";
}
```

The reserved name matters too, not just the number. The name matters for any JSON or text form of the message, and for people who read the schema later and wonder whether they can reuse a nice-sounding name. The compiler will refuse a reuse of either once it is reserved, which is the whole point: it turns a silent data bug into a build failure.

## Order of operations that I think is right

This is what I would do next time, in order. It is not tested end to end, it is just the sequence that avoids the problems above.

1. Stop reading the field first. Change the consuming code, both in the Kotlin app and on the backend side, so that nothing depends on the field being present. Treat it as optional everywhere, even if it was always filled before.
2. Stop writing the field. Once readers tolerate absence, writers can stop setting it. This is a separate release from step one, because if the order is flipped, an old reader that expects the value will get the default value instead, and the default is often a plausible-looking empty string or zero.
3. Mark the field as deprecated in the schema with a comment saying why and when it can go. The comment should name the release that stopped writing it, so whoever comes later does not have to guess.
4. Wait. The wait is not a fixed number of days. It is until the minimum supported app build no longer reads or writes the field, and until the offline queue on any device that has been dormant has had a chance to drain. Drivers who share a device or who only work some weeks of the year are the tail here.
5. Remove the field and add the reserved lines in the same change. Do not remove it and promise to add the reservation later. The later never comes.
6. Regenerate and check that nothing in the repo, including tests and fixtures, still mentions the removed name.

## Where old data hides

This is the part that is easy to forget, so listing it.

### Local queue on the device

The app keeps pending deliveries on the device until they are uploaded. If those are stored as serialized protocol buffer bytes, then an app update that changes the schema has to be able to parse bytes written by the previous schema. Removing a field is fine for parsing, since unknown fields are skipped, but the thing to check is whether any code path then relies on the field to decide what to do with the record, such as deciding whether a photo was captured or whether a signature is required. If that code now sees the default, it may decide the record is incomplete and either block the upload or drop it. A dropped proof of delivery is the worst outcome in this product, because a driver can lose the evidence that they delivered.

### Cloud Firestore documents

If any messages are converted to documents field by field, the removed field stays in documents that were already written. Firestore does not care that the schema changed. Later reads that map documents back into the message will either ignore the extra key or choke on it, depending on how the mapping is written. Check the mapping code for strictness. Also check any query or index that refers to the old field, because queries on a field that is no longer written will quietly return fewer results, and no error is raised.

If the stored form is the serialized bytes in a blob field, then the situation is the same as the local queue: parsing is fine, but anything that depends on the content is suspect.

### Security rules and validation

We have validation that looks at incoming data. If a rule requires the removed field to be present, then every old client that is still out there will start being rejected the moment the rule is deployed, and since the app is offline-first it will retry quietly and the failure will show up as drivers saying their deliveries are stuck. The rule should be loosened before the field is removed from the schema, not after.

### Tests and fixtures

Recorded fixtures that include the old field are actually useful here. Keep at least one fixture of the old shape and a test that parses it with the new classes. That test is the cheapest protection against the next person making the same change carelessly.

## Things that looked safe but were not

- Renaming a field. In the binary format a rename is invisible, because only the tag is on the wire, so it feels safe. It is not safe for the text or JSON form, and it is not safe for anything that maps by name, which includes the Firestore conversion if it is name based. Treat a rename as a delete plus an add.
- Changing a field's type to something that seems compatible. Some changes between numeric types do parse, but the meaning can change, such as signed versus unsigned. I would not rely on this and just add a new field instead.
- Reusing the freed tag after a long time. Long enough is not a real thing here, since the device that has the old bytes might be one that nobody has opened in a while.
- Assuming the generated Kotlin classes protect us. They do not know about old data. Compiling cleanly says nothing about stored records.
- Making a required-style assumption in app code, such as calling a getter and trusting it is non-empty. With the field gone, or never set by a newer writer, that assumption is wrong at runtime, with no compile error.

## How I noticed

Not going to include the specifics. The symptom was that a subset of deliveries synced with a missing piece of information, and only for records created before a certain app update and uploaded after it. The cause was exactly the mix described above: written by the older build, read by the newer one, which had had the field removed and a default assumed in its place. It only reproduced when the device had been offline across the update, which is why local testing with a fresh install never showed it. If you are trying to reproduce something like this, install the old build, capture a delivery with the network off, update the app in place, then go online.

## Open questions

- What is the policy for the oldest app build we still accept? Without that, step four above has no end date, and fields will linger in the schema forever. Someone should write the policy down in the repo next to the schema.
- Do we want a lint or a CI check that fails when a tag disappears from the schema without a matching reserved entry? Protocol buffer tooling has breaking change detection that can do this, and I think it is worth wiring in, but I have not looked at what it would take for our setup.
- Is the Firestore mapping strict or lenient about unknown keys? I believe lenient, but I did not confirm in this pass. Check it before relying on it.
- Should the local queue carry its own schema version marker so that the app can migrate records explicitly instead of relying on the wire format being forgiving? Probably yes, but that is a bigger change than this note is about.

## Quick checklist before touching delivery-proto

- Is the field read anywhere that would misbehave on the default value?
- Is it written by any build we still support?
- Is it present in stored documents or queued records, and does anything query it?
- Does any validation or rule require it?
- Will the removal come with reserved lines for both the tag and the name?
- Is there a fixture of the old shape that still parses?

If any answer is unknown, do not delete the field yet. Deprecate it and come back later; an extra unused field costs almost nothing, and a lost proof of delivery costs a lot.
