---
id: 01K74FZBSJK3ND6SMJAD2HSJPW
created: 2025-10-09T09:43-03:00
---

# delivery-proto: deleting a field without reserving it fails the lint

Deleting a field from `delivery-proto` without leaving a reserved entry behind fails the schema lint. The error for the weight field on the parcel message is `Previously present field "4" with name "weight_g" on message "Parcel" was deleted.` If you see that line, someone removed the field outright. The fix is not to silence the check. The fix is to put the number and the name back as reserved entries. `wirefmt` is the internal codename of `delivery-proto`, so you will see both names in old tickets, chat threads and some build target names. They are the same component. This note uses `delivery-proto` throughout.

This is easy to hit because deleting an unused field looks like harmless cleanup. In ParcelPin it is not harmless, for reasons that come from offline support. The rest of this note explains the failure, the correct edit, and what to check before merging.

## What the failure looks like

The lint compares the current schema files against a previous baseline. When a field that existed before is no longer declared and is also not listed as reserved, the lint reports it. The message names three things: the field number, the field name, and the message that held it. In the case above, the number is in quotes as `"4"`, the name is `"weight_g"`, and the message is `"Parcel"`. The full text is `Previously present field "4" with name "weight_g" on message "Parcel" was deleted.`

A few things about how it shows up:

- It fails the check on the pull request, not the compile. The generated Kotlin code builds fine without the field. So a local build passing tells you nothing about whether the schema change is allowed.
- It fires once per deleted field. If a change removes several fields, expect several lines, and fix all of them in one go.
- The report is a single line per field. Reserving only the number and forgetting the name can leave you with a follow-up complaint, so reserve both.
- Renaming a field while keeping its number is a different case. That does not trigger this message, though it has its own risks, covered below.

If you are reading a CI log and cannot find the cause, search the log for the phrase "Previously present field". That is the stable part of the message.

## Why the rule exists

Protocol Buffers identifies a field on the wire by its number, not its name. The name only matters to generated code and to the text and JSON forms. So if a field is deleted and the number is later reused for something else, any old bytes carrying the old meaning will be read as the new field. Nothing errors out. The value is just wrong, or it is silently dropped, or it decodes to garbage of a compatible wire type.

For ParcelPin the old bytes are not hypothetical. The courier app is built to work offline. A driver can capture proof of delivery in a basement, a rural stretch or a depot with no signal, and the app keeps the captured record locally until it can sync. That local queue can hold serialized messages written by an older app version. Drivers also do not all update at the same time, so the backend sees a mix of versions on any given day. Anything that was written with the old schema can be read later by code built with the new schema, and the other way round.

The weight field is a good example. Suppose the field was dropped because the product team stopped asking drivers for parcel weight. An older app build still in the field keeps writing that number. If a later change reuses the same number for a different measurement or a flag, the backend would read a weight value as that new thing. A reserved entry makes the compiler refuse any attempt to reuse the number or the name, which closes that hole.

## The correct edit

To remove a field legitimately, do not just delete the line. Replace it with a reserved declaration in the same message that covers the field number and the field name. In prose, the shape is:

- Remove the field declaration from the message.
- In the same message, add a reserved statement for the number.
- Add a reserved statement for the name, as a quoted string.
- Keep a short comment saying why it was removed and roughly when, so the next person does not wonder.

For the failing example, the reserved entries must cover number 4 and the name weight_g on the parcel message. After that, the lint accepts the removal, because the baseline field is now accounted for.

Some notes on doing this well:

- Reserve the number and the name together. Number reservation protects the wire. Name reservation protects the JSON and text forms and any code that refers to the field by name.
- Put the reserved statement in the message that owned the field, not at the top of the file or in a neighbouring message. The lint matches by message.
- Do not renumber the other fields to fill the gap. Gaps in field numbers are normal and harmless. Renumbering is the most dangerous change you can make to a schema.
- If the field was inside a oneof, reserve it the same way, and check whether the oneof now has a single member, which you may want to leave alone rather than restructure in the same change.
- Update the baseline only through the normal process for it. Do not edit the baseline to make the error disappear. That removes the lint's memory of the field and defeats the purpose.

## Impact on offline clients and stored data

Reserving is the minimum. Before removing a field, think about who still holds data containing it.

On the device, the offline queue may contain records written before the removal. When the new app version reads them, an unknown or removed field is skipped by the parser, which is fine, but only if the number is not reused. This is the main reason the lint is strict.

In the backend, delivery records are stored in Cloud Firestore and travel through Firebase services. Where a message is stored as raw bytes or as a base64 string inside a document, old documents keep the old field forever. Where the message is mapped to document fields by name, a removed field may leave a stale property on old documents. Neither is a failure by itself, but both mean the old name can show up in data for a long time after the schema stops mentioning it. That is another reason to reserve the name, not only the number.

For older app versions still in use, the question is what they do when the field stops being filled in. If the old app treats the weight as required in its own validation, it may reject records or show an error after the server stops supplying it. Check the minimum supported app version before dropping anything that the app reads. A field that only the server ever read is much safer to remove than one the app displays.

If a replacement field is needed, give it a new number. Never move the meaning of an old number onto something new, even when the types look compatible.

## Review checklist for schema changes

Use this when reviewing a change to `delivery-proto`, or when you are the one making it.

- Does the diff remove any field line? If yes, is there a matching reserved number and reserved name in the same message?
- Does the diff change a field's type? Changing a type, even to something that looks wire compatible, can corrupt old data. Prefer a new field with a new number.
- Does the diff change a field's number? Treat that as a removal plus an addition. It breaks old data.
- Does the diff rename a field but keep its number? The wire stays compatible, but generated Kotlin names change, and JSON consumers break. Check every place that uses the name, including any Firestore mapping and any analytics export.
- Does the diff change an enum? Removing an enum value needs the same reservation treatment as removing a field. Also keep the zero value stable, because unset fields decode to it.
- Did the lint pass on the branch, and was it run against the right baseline rather than a stale one?
- Is there a note in the change description about which app versions are affected, so release planning can take it into account?

The lint catches the deletion case, but it does not know about your data. The questions about stored data and old app builds are on the reviewer.

## Related traps that look similar

A few nearby problems get confused with this one.

The first is reusing a number after a reserved entry was removed. If someone later deletes the reserved statement because it looks like clutter, the lint may or may not notice, depending on how the baseline is maintained. Leave reserved entries in place permanently. They cost nothing.

The second is a message deletion rather than a field deletion. Removing a whole message type is reported differently, and the same logic applies: other messages may still embed it, and old stored bytes may still carry it. Prefer to deprecate and stop using it before removing it, and reserve where the language allows.

The third is moving a field between messages. To the wire this is a deletion in one message and an addition in another. The lint will complain about the deletion on the original message. Reserve there, and give the new message a fresh number unless it is a brand new message.

The fourth is the codename confusion. People search for `wirefmt` and find nothing under that name in the newer docs, or search for `delivery-proto` and miss older threads. They are one component, so search for both.

## What to do when the lint fires

Start by reading the message. It tells you the message, the number and the name. Open the schema file for that message and look at the current declaration. Then do the following in order:

- Confirm the removal was intended. If it was accidental, for instance a bad merge or a bulk edit, restore the field and move on.
- If it was intended, add the reserved number and the reserved name in the same message, with a short comment.
- Push and let the lint run again. It should now accept the change.
- If it still fires for the same field, check that the reserved statement is inside the right message and that the name is spelled exactly as in the old declaration. A near-miss spelling is the usual cause.
- If the lint fires for other fields you did not touch, the baseline may be stale or the branch may be missing a recent merge. Rebase or merge the main line and try again before changing anything.

Do not respond by disabling the check for the branch, and do not edit the baseline by hand to hide the field. Both approaches get the build green and leave the real risk in place.

## Open questions

These are things not settled in this note, and worth confirming with the schema owners.

- How long old app builds stay supported, since that decides how long a removed field can still appear in queued offline records.
- Whether any stored Firestore documents carry the removed weight under its old name, and whether a cleanup is wanted.
- Whether the lint baseline is refreshed automatically after merge or by hand, since that affects how stale it can become on long-lived branches.
- Whether to add a short section in the schema contributor guide that points at this failure, because it keeps recurring for people who treat deletion as cleanup.
