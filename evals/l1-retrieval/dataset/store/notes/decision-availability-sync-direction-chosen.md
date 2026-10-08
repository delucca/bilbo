---
id: 01JS8SSKR6M9Y7MWWNWSXC2F0S
created: 2025-04-20T02:12-03:00
---

# availability-sync: general direction chosen

This note records the general direction the team chose for availability-sync, the part of ClinicSlotter that keeps clinician availability current so the scheduler can place outpatient appointments without double-booking people or rooms. It is deliberately free of exact values. Intervals, limits, and counts live in config and in the code, and they change more often than the direction does. If a later session needs a number, read it from the running configuration instead of from here.

The short version: availability-sync treats the external calendar and scheduling systems as the place where clinicians' availability is authored. ClinicSlotter keeps a local, derived copy that the slotting logic reads. The copy is refreshed by background jobs, never inline in a front-desk request. When the copy is stale or unsure, we prefer to offer fewer slots, not more.

## Direction in plain terms

availability-sync pulls availability from outside, normalizes it into our own shape, and stores it locally in MySQL. The scheduler only ever reads the local copy. It does not call out to another system while a front-desk person is waiting on a screen. All outbound and inbound work goes through Sidekiq jobs, so a slow or broken upstream cannot freeze the booking page.

We chose a pull-first model with push as an accelerator. Pulling on a regular rhythm is the baseline that always works. Where an upstream system can tell us that something changed, we use that signal only to trigger an earlier pull for the affected clinician. We do not trust the content of a change notification as the data itself. The notification says look again, and the pull says what is true.

We also chose to keep availability-sync narrow. It answers one question: when is this clinician, or this room, open to be booked. It does not decide which patient goes where, and it does not hold appointment state. Keeping that boundary means the sync code can be reasoned about, tested, and replaced without touching the slotting rules.

## Why this direction

Small clinics do not have an IT person watching dashboards. The front desk needs the schedule screen to load and to be right. Anything that makes the screen depend on a remote system being healthy in that moment turns an upstream hiccup into a front-desk outage. That was the main reason for local reads and background refresh.

The second reason is trust. A wrong open slot is worse than a missing one. A missing slot costs the desk a phone call or a short wait. A wrong open slot sends a patient to a clinic when the clinician is not there, and that is the failure people remember. So wherever the design had to choose between more availability shown and less, we chose less.

The third reason is that the upstream systems differ. Some speak FHIR cleanly, some expose something close to it, and some give us little more than a feed of busy blocks. A local normalized shape lets the scheduler stay the same while the adapters vary. The cost is a translation layer we have to maintain, and we accepted that.

## Source of truth and ownership

Availability is owned upstream, not by ClinicSlotter. If the local copy and the upstream disagree, upstream wins after the next successful pull. We do not write availability back upstream from availability-sync. Anything that looks like a write-back is a separate feature with its own decision and its own review.

What ClinicSlotter does own is the set of local constraints layered on top: room assignments, room closures, and the rules that say which clinician can use which room. Those are entered in ClinicSlotter, they are not overwritten by a sync, and availability-sync must never delete or alter them while refreshing clinician data. When a sync run finishes, the scheduler combines the two sources at read time.

We keep the raw upstream payload for a limited period next to the normalized rows. This is for debugging a disputed slot, not for product features. Nothing should read the raw payload at request time. If a bug report says a slot was wrong, the raw copy is how we find out whether the fault was upstream, in normalization, or in the scheduler.

## How refreshes are triggered

There are three kinds of trigger, and they all end up in the same job path. First, a regular scheduled refresh per clinician or per calendar, which is the safety net. Second, an event-driven refresh when an upstream change signal arrives or when the desk books or cancels something that touches that clinician. Third, a manual refresh the front desk can ask for when they suspect the screen is behind.

All three go through the same entry point so they share deduplication. If many triggers arrive for the same clinician close together, we want one effective refresh, not a pile. The direction here is to collapse duplicates by clinician and let the latest request win, rather than to queue every request faithfully.

Manual refresh is intentionally limited in what it promises. It queues the work and tells the person that a refresh is under way. It does not hold the page open until the upstream answers. If the refresh fails, the screen keeps showing the last good copy with a visible note that it is out of date.

## Staleness policy

Every availability record carries the time it was last confirmed from upstream. The scheduler reads that and applies a general rule: fresh data is used normally, somewhat old data is used with a visible warning, and data that is too old is treated as unknown for that clinician. Unknown means the slotter offers nothing automatically and the desk has to confirm by hand.

The thresholds are configuration, not code constants, and they are allowed to differ by clinic because clinics differ in how often their calendars change. We did not pick a single universal freshness window. We did decide that the same three-state shape applies everywhere, so the front desk learns one behavior.

We considered silently extending the old window during upstream outages so the screens would stay busy. We rejected that. A quiet outage that looks like normal operation is exactly how wrong slots get booked. The warning state is there to be seen.

## Conflict and overlap handling

When upstream availability changes under an appointment that is already booked, availability-sync does not cancel or move that appointment. It flags the appointment as needing attention and leaves the decision to a person. The sync's job is to report that the world changed, not to rewrite bookings.

When two upstream sources describe the same clinician, for example a main calendar and a leave calendar, we merge them conservatively: a time is open only if no source says it is blocked. Overlapping blocks are unioned. Overlapping open periods are intersected with any other restriction that applies. When sources contradict each other in a way that cannot be resolved by that rule, the time is treated as unavailable and the contradiction is logged for later review.

Time zones and daylight changes are handled by storing the instant and the clinic's zone and never relying on local clock strings. Recurring availability is expanded into concrete periods within a bounded look-ahead window and re-expanded on refresh. We do not store an open-ended recurrence and expand it at read time, because that made the scheduler's queries slow and hard to test.

## Failure and retry behavior

Upstream failures are expected. The direction is to retry with growing delays, give up on a single run after a reasonable effort, and try again on the next scheduled trigger. A permanently failing source should not keep consuming workers. After repeated failures a source is marked as unhealthy, its data goes to the stale or unknown state by the staleness policy, and a person who administers the clinic is told once, not on every failed attempt.

Jobs are written to be safe to run twice. Each refresh replaces the clinician's availability for the window it covers in a single transaction, so a half-finished run never leaves a clinician with partial data. If a run dies midway, the old data stays, which is the preferred failure.

Authentication problems are treated differently from transient network trouble. An expired or revoked credential will not fix itself on retry, so those failures stop retrying early and raise a clear task for the clinic's administrator. Piling retries onto a bad credential risks lockouts upstream and hides the real cause.

## Room constraints and availability-sync

Rooms are not synced from clinician calendars. Some upstream systems do expose room or resource calendars, and where they do, availability-sync can read them as an additional source of blocked time. They are treated the same way as clinician blocks: they can only reduce what is open.

The linking between a clinician and the rooms they may use stays local. That mapping changes rarely and is edited by clinic staff, and we did not want an upstream rename or reshuffle to silently change it. If an upstream resource disappears, availability-sync marks the matching local room as needing review instead of deleting the link.

## Observability and support

Each sync run records what it did in plain terms: which source, what it found, how many records it changed in broad terms, and whether it succeeded. Support staff should be able to answer "why does this clinician look empty" from those records without reading code. The goal is that the answer is one of a short list: upstream is down, credentials need attention, the data is genuinely empty, or a local constraint is blocking it.

We avoid putting patient information in sync logs. Availability data is about clinicians and rooms, and the logs should stay that way. Where a booked appointment is involved in a flag, the log carries an internal reference and nothing about the patient.

Front-desk visibility matters as much as logs. The schedule screen shows when a clinician's data was last confirmed, and shows the three freshness states in a consistent way. We would rather add one small indicator than a settings page nobody opens.

## Alternatives we did not pick

Live lookups at booking time were the simplest idea and the first one dropped. They tie booking speed and correctness to the slowest upstream, and they make every outage a front-desk outage.

A fully push-driven design, where we trust change notifications as the data, was rejected because notifications get lost, arrive out of order, or arrive for changes we cannot see in full. Pull remains the baseline so a lost message costs freshness for a while, not correctness forever.

Two-way sync, where ClinicSlotter writes availability back, was rejected for now. It brings conflict rules, ownership disputes, and a much larger blast radius if we have a bug. If a clinic really needs it, that is a separate decision.

A single global freshness policy was rejected in favor of per-clinic configuration with one shared shape of behavior. A hand-built queue instead of Sidekiq was never seriously considered, since Sidekiq is already how background work runs on Heroku for this app.

## Consequences to keep in mind

There will always be a window where the local copy is slightly behind. We accept it and make it visible instead of trying to remove it. Anyone adding a feature that assumes availability is exactly current should stop and reconsider, or go through the staleness states.

Adding a new upstream system means writing an adapter that produces our normalized shape and nothing else. It should not reach into scheduler code or add special cases there. If a new source needs a special case in the scheduler, that is a sign the normalized shape is missing something and should be extended deliberately.

Heroku's dyno behavior matters: workers can restart at any time, which is another reason jobs must be idempotent and why a refresh replaces data in one transaction. Do not rely on in-memory state surviving between runs.

## Open questions

We have not settled how aggressively to prefetch further ahead for clinics that book far in advance versus those that book same day. Right now the look-ahead is one shared setting, and it may need to differ by clinic.

We have not decided how to present contradictory upstream sources to a clinic administrator in a way that is useful. For now they are logged and treated as unavailable, which is safe but not friendly.

We also have not decided whether a write-back path is ever worth building. Revisit it only if a clinic's workflow cannot be served by reading alone.

## If you change this

Changes that alter who owns availability, that let the sync modify bookings, or that make the scheduler call upstream during a request are changes to this decision, not implementation details. Update this note in the same piece of work, and say what replaced what and why. Tuning intervals, thresholds, and retry spacing is not a change to the decision and does not need a note update.
