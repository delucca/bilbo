---
id: 01K40GZVQ3MPHWCN235YPJ0BS8
created: 2025-08-31T13:57-03:00
---

# slot-solver gotcha: mixing Time and TimeWithZone raises ArgumentError

slot-solver blows up when clinician hours and requested appointment times are a mix of plain Time and zoned values. The error is `ArgumentError: comparison of Time with ActiveSupport::TimeWithZone failed`. It is not a solver logic bug. It is a type mismatch at the point where the solver compares one instant against another, and it shows up as soon as one side of a comparison was built a different way from the other side. This note is for whoever sees that error again and wants the short version of why it happens and what to check first.

Internally some people still call slot-solver by its old codename, `slotris`. If you grep old tickets, chat logs, branch names or Heroku log lines for `slotris`, you are looking at the same component. In this note and in new code the name is `slot-solver`. Both names point at the same thing; there is no second solver hiding behind the codename.

## Symptom

A front-desk user asks for a slot, or a background job tries to fill a day, and the request fails. In the web path the user sees a generic error page or a spinner that never finishes. In the Sidekiq path the job goes to the retry set and keeps failing with the same message each time. The message is always the same:

```
ArgumentError: comparison of Time with ActiveSupport::TimeWithZone failed
```

The backtrace points into slot-solver, at a comparison or a sort, not at the place where the bad value was created. That is what makes it annoying: the line that raises is innocent, and the line that made the odd value is somewhere upstream.

## What is actually going on

Rails has two time-like classes in play. One is the plain Ruby Time. The other is ActiveSupport::TimeWithZone, which is what you get from most Rails time helpers, from ActiveRecord datetime columns when the app time zone is configured, and from parsing strings through the Rails zone-aware helpers. They look alike when printed and they often compare fine. But in some orderings, and in some places such as sorting a mixed array or checking a range, Ruby raises instead of coercing. slot-solver does a lot of comparing: is this start inside the clinician's hours, does this interval overlap another, which candidate is earliest. Any one of those can hit the mismatch.

So the rule: inside slot-solver, every time value should be the same kind. If the inputs are a blend, the solver will work for some days and fail for others, depending on which pair meets first.

## Where the mix usually comes from

Clinician hours are the first suspect. They are stored as times of day on the clinician's schedule and expanded into concrete instants for a given date. How that expansion is done decides whether the result is plain Time or zoned. If someone builds the expansion with a bare Time constructor, you get plain Time. If someone builds it through the Rails zone helpers, you get a zoned value.

The requested time is the second suspect. A request from the web form is parsed by Rails and tends to be zoned. A request that arrives through the HL7 FHIR side, from an appointment resource or a slot search with a start parameter, is often parsed by a generic library and can come out as plain Time. Same for anything a Sidekiq job rebuilds from serialized arguments, since serialization can drop the zone wrapper.

## Why it hides in testing

Tests that build all their times in one style pass. Developers usually write fixtures in one style, so the mix never appears locally. In production the clinician hours come from one path and the requests from another, so the combination only appears with real traffic. It also depends on the clinic: a clinic whose time zone matches the server default can behave differently from one that does not, because some conversions are no-ops in one case and not in the other.

Another reason: the error depends on comparison order. Comparing a zoned value to a plain one can succeed while the reverse raises, or the other way around depending on the Ruby and Rails versions. Do not conclude that a pair is safe because one order worked in a console.

## First checks when it happens

Do these in order, they are cheap.

- Read the backtrace and find which comparison in slot-solver raised. Note which two values were being compared.
- Print the class of each value. One will be Time and the other ActiveSupport::TimeWithZone. Finding which one is the odd one tells you whether the bug is on the hours side or the request side.
- Check whether the failing clinic is special, for example has a zone different from the app default.
- Check whether the failure is only in the Sidekiq path. If so, suspect argument serialization.
- Check whether the request came in through the FHIR endpoint. If so, suspect the parsing there.

## Fix pattern

Normalize at the boundary, not inside the comparison. Whatever enters slot-solver, convert it once to the zoned form in the clinic's time zone, and make everything inside assume that. Do not sprinkle conversions around each comparison; that hides the real source and leaves the next new caller free to reintroduce the mix.

Concretely, the entry point of slot-solver should accept hours and requested times, convert each to the zoned type using the clinic's zone, and only then build intervals. If a value has no zone information at all, treat that as a caller bug and raise a clear error of your own that names the field, instead of guessing a zone.

## What not to do

Do not rescue the ArgumentError and retry with converted values. That turns a loud failure into a silent guess, and in scheduling a guess about zone means a patient booked at the wrong hour. Do not convert everything to UTC plain Time as a shortcut either; clinician hours are local wall-clock concepts, and daylight saving transitions will then shift them in ways front-desk staff will notice only when a patient shows up early or late.

Do not monkey-patch comparison on either class. It would make the error go away everywhere, including in unrelated code, and it would hide real bugs.

## Daylight saving interaction

The mismatch error and daylight saving problems are different bugs that look related. Normalizing to the zoned type is a precondition for handling daylight saving properly, because plain Time carries no rule about the zone. Once everything is zoned, a day with a clock change expands its clinician hours correctly, with the missing or repeated hour handled by the zone rules. If you fix the type mismatch and still see off-by-an-hour slots around a clock change, that is a separate problem in how the hours are expanded; look there and not at the comparison.

## Sidekiq specifics

Jobs that call slot-solver should take simple arguments, such as ids and ISO strings, and rebuild the zoned values inside the job using the clinic's zone. Passing time objects as job arguments is fragile: what comes back after serialization is not guaranteed to be the type that went in. Retries make this worse, because a job that failed on a mismatch will fail identically on every retry, and the retry set fills up. When you fix the bug, clear or replay the failed jobs after deploy, and replay only after confirming the fix on one of them.

## FHIR specifics

Slot and appointment resources use an instant or dateTime string with an offset. Parse these in one place, convert to the clinic zone, and pass the result on. Do not hand the raw parsed value to slot-solver. Some FHIR dateTime values are date-only or lack an offset; those are not safe to treat as instants and should be rejected or completed from clinic context before they get near the solver.

## Database side

MySQL stores datetimes without a zone. What you read back is interpreted by Rails according to the configured zone settings. If the app is configured to be zone-aware, the model attributes come back zoned. Raw queries and anything that bypasses the model layer can return plain values. If clinician hours were ever loaded through a raw query or a pluck-style call, that can be the entire source of the mix. Check how the failing values were loaded before blaming the parsing code.

## Heroku and environment

The dyno runs with its own default zone, normally UTC. Code that works on a developer laptop set to a local zone can behave differently there, because a plain Time built from local wall-clock values means something different on each machine. If an error appears only after deploy and never locally, suspect this first. Check the app zone setting and the process environment zone on the dyno before chasing anything else.

## Logging that would help

The error message does not say which values were compared. When you touch this area, add a log line at slot-solver entry that records the class of each incoming time value and the clinic. Keep it at debug or at a level that can be enabled temporarily, and do not log patient details; times and clinic identifiers are enough. With that line, the next occurrence takes minutes instead of an afternoon.

## Tests worth having

Write a test that feeds slot-solver clinician hours in one style and a request in the other, in both orders, and expects it to produce a normal result, not an error. Write a second test with a clinic whose zone differs from the app default. Write a third that goes through a serialized job argument round trip. These three cover the three ways the mix has appeared. A test that only uses one style everywhere proves nothing about this bug.

## Related note

The waitlist logic also depends on slot times being comparable, so it can hit the same problem when it asks the solver for freed slots. See [[waitlist-promoter-must-hold-revised]] for the hold rules; that note is about hold behavior, not about types, but a mismatch error from there likely traces back to the same source.

## Quick reference

- Error text: `ArgumentError: comparison of Time with ActiveSupport::TimeWithZone failed`
- Component: `slot-solver`, codename `slotris`.
- Cause: plain Time mixed with zoned values in clinician hours and requested times.
- Fix: convert once at the entry of slot-solver, to the clinic's zone, and fail loudly on values with no zone.
- Check first: class of each value in the failing comparison, then where each was built.
- Do not: rescue and retry, convert everything to UTC plain Time, or patch comparison globally.

## Open questions

It is not settled whether the entry-point conversion should live inside slot-solver or in a thin wrapper that all callers must use. Inside is safer because nobody can skip it; a wrapper is easier to test on its own. Lean toward inside, with the log line, unless a caller is found that legitimately needs the raw behavior. Also not checked: whether any reporting code outside the solver compares the same values and would fail the same way. If someone has time, grep for those after the fix lands.
