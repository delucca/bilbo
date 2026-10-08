---
id: 01KAG9DTTEQ5ABQ8DSG4TDE519
created: 2025-11-20T06:27-03:00
sources:
  - "doc: Front Desk Usability Review"
---

# frontdesk-ui usability review: drag-and-drop rescheduling

A usability review with 6 front-desk staff concluded that drag-and-drop rescheduling in frontdesk-ui was confusing, and it recommended adding a confirmation dialog before a moved appointment is saved. This note keeps the outcome of that review so the next person touching the calendar view in frontdesk-ui does not have to rerun the sessions or guess why the dialog is wanted.

The review was about one interaction only: picking up an existing appointment in the day or week grid and dropping it on another slot, clinician column or room row. Other parts of frontdesk-ui (booking a new visit, searching patients, the waiting list view) were not the focus, and the notes below should not be read as findings about them.

## What the review found

The staff who took part work at small clinics, which is the normal user of ClinicSlotter. They are not power users of scheduling software and they often have a patient on the phone or at the desk while they move things around. That context shaped most of the findings.

The main finding is that a drop takes effect at once and the result is not clear enough to the person who did it. Several participants were unsure whether a drop had actually saved, whether it had been refused, or whether it had landed in a different place than they meant. Because the grid is dense, a drop that is a little off lands on a neighbouring clinician or the next slot without any obvious sign.

Specific patterns that came up in the sessions:

- Accidental moves. Staff picked up an appointment while trying to click it to open details, and dropped it somewhere by mistake. Some did not notice until later.
- Unclear target. When the pointer was between two columns, people were not sure which clinician or room the appointment would end up under.
- Silent constraint handling. ClinicSlotter respects clinician availability and room constraints, so some drops are not allowed. Participants could not always tell the difference between a drop that was rejected for a rule and a drop that simply failed to register.
- No obvious way back. After a move there was no clear undo, so people tried to drag the appointment back to where it came from. They were not always sure of the original slot.
- Patient impact. Moving an appointment can mean the patient gets told about a new time. Staff were nervous that a wrong drop would lead to a wrong message or a confused patient.
- Touch and trackpad use. People using a trackpad or a touch screen found the drag gesture harder to control than with a mouse, and more prone to dropping early.

Not everything was negative. Participants liked that drag-and-drop is quick for the simple case of shifting a visit a little later in the same day with the same clinician. Nobody asked for the gesture to be removed. The ask was for a safety step, not a different interaction model.

## Recommendation

Add a confirmation dialog after a drop. The dialog should show the appointment, the old slot and the new slot side by side, name the clinician and room for both, and offer a clear confirm and cancel. Cancelling should put the appointment back exactly where it was.

Points to keep when this gets built:

- The dialog should say what will change in plain words, not just ask "are you sure". Staff said a vague prompt would be clicked through without reading.
- If the move breaks a clinician availability or room rule, say which rule and do not offer confirm. Show the reason where the person is already looking.
- If the patient will be notified of the change, the dialog should say so, so the front desk knows before confirming.
- Keep the keyboard path working. Confirm and cancel should be reachable without the mouse.
- Think about whether a dialog on every move gets tiresome for people doing many moves in a row. The review did not settle this. A possible compromise is to skip the dialog for small same-day, same-clinician shifts, but that was our idea and was not tested with the staff. Treat it as open.

The review did not recommend a specific visual design, and none of the options above were prototyped with participants. Any dialog design is still to be tried.

## Where this touches the rest of the system

Rescheduling in frontdesk-ui goes through the Rails backend, which checks availability and room constraints against MySQL before saving. Any follow-up work such as patient notifications or syncing changes outward over HL7 FHIR is handled by Sidekiq jobs, so a confirmed move can trigger background work. That is a reason to put the confirmation before the request is sent, not after. An undo after the fact would have to reverse jobs that may already have run or been queued.

A server-side check is still needed. The dialog is a usability guard, not a replacement for the constraint checks in the backend. If the dialog is built so that it previews the result of a proposed move, the preview should use the same rules as the save, or it will show something the server later refuses. A dry-run style request from frontdesk-ui to the backend would keep them in step, but that is a design choice for whoever builds it.

The app runs on Heroku, so any change to how the move request behaves should be thought about with request time limits in mind. A preview call should be cheap.

## Open questions and next steps

- Decide whether the confirmation applies to every move or only to some. See the compromise above.
- Decide whether to add a visible undo after confirming, in addition to the dialog. Participants wanted a way back, and the dialog only helps before the save.
- Show a clearer drop target highlight while dragging, so the clinician, room and time are obvious before release. This was not part of the formal recommendation but follows from the target-confusion findings.
- Run a short follow-up with front-desk staff once a prototype dialog exists, ideally including people who use a trackpad or touch screen, since they had the most trouble.
- Record the final behaviour in a decision note once it is chosen, and link back to this review.

## Caveats

This was a small study with a handful of participants, all from the kind of small clinic ClinicSlotter targets. It shows a clear pattern of confusion but is not a measurement of how often wrong moves happen in real use. If error numbers are needed to prioritise the work, they will have to come from production data, not from this review.

The review text is a summary of the conclusion and recommendation. Session recordings and raw observations are not reproduced here.
