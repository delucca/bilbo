---
id: 01JZDVDT3FZCSTY8ZRHPRXGGRT
created: 2025-07-05T14:51-03:00
---

# frontdesk-ui review themes

This is a general summary of what keeps coming up when people review frontdesk-ui. It is not a list of findings. It is the pattern across reviews, written so the next reviewer knows where to look first and what usually goes wrong. Nothing here is a requirement or a decision; those live elsewhere.

frontdesk-ui is the part of ClinicSlotter that front-desk staff actually touch all day. Most review comments are about whether a tired person at a busy desk can do the next thing without thinking. Code style comments exist but are a small share. The bigger share is about behavior under pressure: phone ringing, patient standing there, schedule changing underneath.

## Where reviews spend their time

Three areas take most of the attention. The first is the slot picker and the calendar views around it. The second is forms for creating and changing appointments. The third is the way the screen reacts when something changes on the server while someone is looking at it, for example another staff member booking the same slot or a background job updating availability.

Less attention goes to static pages, settings screens and anything an admin touches rarely. Reviewers tend to skim those. That has caused a few late surprises, so it is worth giving them one honest pass.

Reviews are usually done by someone who knows the Rails side better than the browser side. That shows. Comments on server rendering, params handling and query shape are confident and specific. Comments on keyboard behavior, focus and layout are fewer and softer. If you are reviewing, try to cover the second group on purpose.

## Slot picking and availability display

The most repeated theme is that what the screen shows as free can disagree with what the server will accept. Clinician availability and room constraints are both involved, and the UI often shows only one of them clearly. Reviewers keep asking the same questions:

- Does the picker show why a slot is unavailable, or does it just grey it out? Staff need the reason, because they will tell the patient something.
- If a slot becomes taken between display and submit, what does the person see? The good cases give a clear message and keep the rest of the form intact. The bad cases drop the form or show a generic failure.
- Are room and clinician treated as separate causes of unavailability in the display? When they are merged, staff cannot tell which to change.
- Does the view hold stale data after a long idle period? Desks leave pages open for hours.

A related theme is time zones and clinic-local time. Reviewers flag any place where the browser's idea of time leaks into what is shown or sent. The rule of thumb that keeps being restated is that the clinic's time is what matters, and the UI should not depend on the device clock being set right.

Another recurring point is that the picker should not do its own availability math. When some of the logic is copied into the front end, it drifts from the server. Reviews push toward asking the server and rendering the answer.

## Forms and data entry

Forms get a lot of small comments that add up. The common ones:

- Validation messages that say something is wrong without saying which field or what to do.
- Fields that lose their values after a failed submit, especially after a conflict with another booking.
- Patient lookup and selection that is slow to use with a keyboard, or that makes it easy to pick the wrong person with a similar name. Reviewers treat the wrong-patient case as the most serious of the form problems, because the harm is real and quiet.
- Free-text fields where a constrained choice would do, and the reverse.
- Double submit. A slow response plus an impatient click can produce two appointments. Reviews ask whether the button disables, whether the server tolerates a repeat, and whether the user can tell the first one worked.

Reviewers also ask how much of the form depends on data that arrives through the FHIR integration. Fields that come from outside can be missing, odd or out of date. The UI should show that state plainly instead of assuming the data is clean. Where it does not, the comment is almost always the same: handle absent values, and do not render a blank that looks like a real answer.

## Live updates and background work

Sidekiq jobs change data while the front desk is working. The UI has to cope. Review comments in this area are about what the person sees when a change lands mid-task. There are a few patterns that keep appearing.

Silent refresh is risky. If the schedule redraws under someone who is about to click, they can book the wrong slot. Reviewers prefer a visible hint that something changed, with the update applied in a way that does not move the target.

Status of background work is often invisible. When a booking triggers follow-up work, such as notifications or syncing to another system, the screen sometimes says "done" when only the first part is done. Reviews ask for wording that matches what is actually finished, and for a way to see when the later part failed.

Polling versus push comes up from time to time. The recurring view is that either is fine if the failure mode is shown. A page that quietly stops updating is worse than one that updates slowly and says so.

## Accessibility and keyboard use

Staff who book many appointments a day rely on the keyboard and on predictable tab order. Reviews often find that new components break this: focus gets lost after a modal closes, a dropdown cannot be reached without a mouse, or the order of fields does not match the order people think in.

Contrast and text size also come up, since screens at small clinics are often old, shared or at an awkward angle. Color alone is used to mean unavailable or conflicting in a few places, and reviewers keep asking for a second cue.

Screen reader support is mentioned less often than it should be. When it is mentioned, it is usually about the calendar grid, which is hard to make sensible. Nobody has treated this as solved. A reviewer who has time can add real value here.

## Error handling and wording

Error text is a steady source of comments. The patterns: messages written for developers, messages that blame the user, messages that hide the cause, and messages that disagree with each other for the same underlying problem. Reviewers want short plain sentences that say what happened and what the person can try.

Network trouble at a clinic is normal, not exceptional. Reviews ask whether the UI distinguishes a failed save from a slow one, and whether anything entered is kept while the person retries. Losing a half-entered appointment while the patient waits is the sort of thing that gets remembered by staff.

There is also a consistent ask to log enough on the server side that a support person can match a complaint from the desk to a request, without exposing patient details in the log. Reviewers are careful here, and any change that adds data to logs or to page markup gets a second look for sensitive content.

## Privacy and what is on screen

Front desks are semi-public. Patients can see monitors. Reviews check how much patient detail is shown by default in lists and calendar cells, and whether there is a way to reduce it. The usual comment is that the minimum needed to recognize the appointment should be visible, and more should need a deliberate action.

Related: page titles, tooltips, URLs and browser history can leak details. Reviewers watch for identifying information in places that are not obviously part of the visible page. Print views and exports are another spot that gets less attention than the main screens and should get more.

## Performance and rendering

Complaints here are about perceived speed more than measured speed. The calendar with many clinicians and rooms is the heavy case. Reviews ask whether the page renders something useful early, whether large lists are cut down, and whether repeated queries are hiding behind a pretty view. On the Rails side, N+1 style queries in view rendering are a recurring catch, and so is loading more related data than the screen shows.

Front-end weight is mentioned occasionally. The consensus is that the audience has modest hardware and uneven connections, so additions should justify their cost. New libraries get questioned more than new code.

## Testing and review habits

Reviewers often ask for test coverage of the awkward paths rather than the happy path: conflict on submit, repeated submit, missing outside data, long idle sessions. Browser-level tests exist for the main flows, and reviewers are inclined to ask for one more whenever a flow changes. They also note when those tests are slow or flaky, since people start ignoring failures. This has not been fixed and tends to come back.

A habit worth keeping: when a change touches the picker or the forms, the reviewer tries it with the keyboard only and with a deliberately slow connection before approving. Several useful catches came from nothing more than that.

Another habit that helps is asking for a short description of what the desk user sees before and after the change. Pull requests that include it get better reviews, because the discussion stays on behavior.

## Things that keep recurring and are not settled

- How much logic about availability the UI may hold, versus always asking the server.
- How to show live changes without moving the target under the user's hand.
- A real answer for assistive technology on the calendar.
- Consistent wording for errors across screens.
- How to keep tests for the main flows fast enough to be trusted.
- What the default level of patient detail on shared screens should be.

None of these has a clean answer in the reviews so far. They are open threads, and a new reviewer should expect to meet each of them again.

## Practical advice for the next reviewer

Start from the desk user's task, not the diff. Ask what happens if the data changed a moment ago, if the network is slow, if the outside system sent something incomplete, and if the person is using only a keyboard. Check the less glamorous screens once. Look for places where the UI repeats a rule the server already owns. Keep comments short and name the behavior you are worried about, because that is what the author can act on. If something touches patient identity or visible patient detail, slow down and read it twice.
