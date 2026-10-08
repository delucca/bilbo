---
id: 01M2CY4A457R1ABW6JWAQXFFJ7
created: 2026-09-13T05:28-03:00
---

# weather-feed-client fallback provider plan

This note replaces the earlier note about "weather feed second provider". The new value: the fallback weather provider for weather-feed-client is now due by the end of January 2027. The earlier date is dropped and should not be used anywhere. If a ticket, a calendar entry or a message still carries the old date, correct it to January 2027.

```
component: weather-feed-client
fallback_provider_due: end of January 2027
replaces: weather feed second provider (earlier date)
```

The rest of this note is the working plan as it stands today. It is written quickly and is meant to be edited as things get settled. Where something is not decided, it says so, and it does not guess.

## What changed and what did not

The only fact that moved is the due date. The fallback weather provider for weather-feed-client is due by the end of January 2027. Scope, purpose and the general approach are the same as in the earlier note. Anyone who read the earlier note and planned around its date has more time now, but the extra time is not a reason to leave the work idle. Several pieces below can start well before the end of January 2027, and the integration tests need real calendar time to run against live data.

Things that stay the same:

- weather-feed-client remains the single place in GridHaven where weather data enters the system. Nothing else should call a weather provider directly. If a forecasting job or a dashboard needs weather, it goes through weather-feed-client.
- The primary provider stays the primary provider. The fallback is only used when the primary cannot give us usable data. It is not a load-sharing arrangement and it is not a way to average two sources, unless a later decision says otherwise.
- The consumers are unchanged: the Julia forecasting code that estimates household solar output, the scheduler that plans battery charging against time-of-use tariffs, and the Svelte front end that installers and customers look at.

Things that changed:

- The due date: end of January 2027.
- Because the date is later, the order of work below can put design and measurement first, and defer the cutover rehearsal until the pieces are stable.

## Why a fallback is needed

The forecast of household solar output depends heavily on weather inputs: cloud cover, irradiance, temperature and similar. The battery schedule depends on the forecast. A bad or missing weather feed therefore does not just leave a gap on a chart; it changes when batteries charge and discharge against the tariff. A wrong schedule costs the customer money and costs the installer trust.

With a single provider, any outage, quota problem, contract change or quality drop on that provider's side becomes our outage. We have no control over those. A fallback gives weather-feed-client somewhere else to go. The goal is not perfect weather data; the goal is that the system degrades in a known, bounded way instead of failing in a surprising one.

The cases we care about, in rough order of how often they are likely to happen:

- The provider is slow or times out for a stretch of time.
- The provider returns errors, or returns a well-formed response that is empty or clearly incomplete.
- The provider returns data that parses but is stale, for example a forecast that has not been refreshed when it should have been.
- The provider changes its response shape or its terms without much notice.
- The provider is unavailable for a longer period, or access to it is lost for a commercial reason.

The first three are operational and the client should handle them automatically. The last two are slower and need a person to notice and decide. The fallback helps in all five, but the way it helps is different, and the plan should not pretend a single mechanism covers them.

## Plan of work

The work is split into phases. The phases are ordered by dependency, not by calendar. The end of January 2027 is the deadline for the whole thing being usable in production; individual phases do not have separate committed dates yet. When dates for phases are agreed, add them here and say who agreed.

### Phase one: pick the provider

Nothing else can be finished until the provider is chosen, so this comes first. The choice has to be made on evidence, not on the first thing that looks reasonable. What to look at:

- Coverage: does the provider cover every region where GridHaven has installations, including the less populated ones? Installers work in specific areas, and a provider that is strong in cities and weak elsewhere is not a fallback for those customers.
- The kinds of values it returns and whether they map onto what the forecasting code already consumes. Irradiance is the most important. If the fallback gives only cloud cover and not irradiance, the forecasting side has to derive irradiance, and that is a quality risk to be written down.
- Update frequency and how late its data can be compared to the primary.
- Historical data access. For backtesting and for comparing the two sources, we need to be able to ask what the fallback said in the past, or at least to record what it says from now on.
- Terms of use: whether storing the data in InfluxDB is allowed, whether redistributing derived values to customers through the Svelte interface is allowed, and what the usage limits are.
- Cost at the volume we expect, including growth in the number of sites.
- Reliability track record, and whether it shares infrastructure with the primary provider. A fallback that fails together with the primary is not a fallback.

The last point deserves emphasis. If both providers draw from the same upstream model or the same hosting, an upstream problem takes both down. Ask directly, and write the answer in this note.

Output of this phase: a short written choice with the reasons, kept in this note or in a linked decision note. Do not leave the reasons in a chat thread only.

### Phase two: define the common shape

weather-feed-client should present one internal shape to its callers, whichever provider produced the data. If the primary provider's shape leaks through today, this phase is where that gets cleaned up. Decisions needed:

- The canonical set of fields and their units. Units must be explicit and converted at the edge, inside weather-feed-client, not downstream.
- How time is represented: time zone handling, whether values are for an instant or for an interval, and how the interval is labelled. Mismatches here are a classic source of quiet errors in solar forecasts, because a shift of an hour moves the peak.
- How missing values are represented. A missing value must never be silently turned into zero. Zero irradiance is a valid value at night, and confusing it with missing data would make the forecast confidently wrong.
- A field that says which provider produced each record. This is required. Without it, nobody can later explain a strange forecast, and the comparison work in phase five is impossible.
- A field or flag for quality, so consumers can tell a normal record from one that came from the fallback or that was filled in.

Output of this phase: a description of the canonical shape, and the mapping from each provider into it, written next to the code.

### Phase three: build the fallback adapter

Write the adapter that talks to the fallback provider and converts its responses into the canonical shape. Keep it separate from the primary adapter so that either can be replaced without touching the other. Points to keep in mind:

- Authentication and secrets should be handled the same way as for the primary. Secrets do not go into the repository. If the deployment already keeps secrets in Azure-side configuration, use that; do not invent a second mechanism.
- Timeouts and retries must be bounded. A retry loop that is too patient defeats the purpose of having a fallback, because the switch happens too late.
- Parse defensively. A response that does not match the expected shape should be treated as a failure of that provider for that request, with a clear log message, not as a crash.
- Rate limits: respect them, and record when they are hit.
- Keep the adapter small. The less logic it has, the fewer places for provider-specific surprises.

### Phase four: selection logic

This is the part that decides when to use which provider. It is simple to describe and easy to get wrong, so it gets its own phase.

The rule of thumb: use the primary whenever it gives usable data; use the fallback when it does not. "Usable" needs a definition that the whole team agrees on. Candidate checks:

- The request succeeded within the allowed time.
- The response parsed and contains the fields the consumers need.
- The data is fresh enough: its issue time is recent relative to now.
- The values are within physically plausible ranges.

Switching back matters as much as switching over. If the client flips to the fallback at the first failure and flips back at the first success, it will flap during a partial outage, and the forecast will jump between two sources with different biases. Add some hysteresis: stay on the fallback until the primary has been healthy for a while. The exact length of that period is not decided and should be set after looking at how the two sources behave. Write the chosen values here when known.

Also decide what happens when both fail. Options are to serve the last good data marked as stale, to serve nothing and let the consumers decide, or to hand the consumers a climatological default. Our current leaning is to serve the last good data with a clear stale marker for a limited time, then stop, and let the scheduler fall back to a conservative charging plan. This is a leaning, not a decision. The scheduler side needs to be consulted because it is the one that bears the consequences.

### Phase five: compare and calibrate

Two providers will not agree. Their biases differ by region, season and weather type. If the fallback is systematically more optimistic about cloud clearing, then on days when the system switches, the forecast of solar output will be biased in a way that has nothing to do with the house.

Plan:

- Once the adapter works, run it alongside the primary in a passive mode: fetch from both, store both in InfluxDB with the provider field, and use only the primary for decisions.
- Collect enough time to cover different weather situations. A sunny week proves little. The more varied the period, the better.
- Compare the sources against each other and, where we have it, against measured output from real installations. Measured output is the ground truth that matters; two weather providers agreeing with each other is not evidence that either is right.
- Decide whether a correction is worth applying to the fallback values, or whether it is better to leave them uncorrected and widen the uncertainty the forecast reports when the fallback is in use. Widening uncertainty is the more honest option and is probably the starting point.

This phase is the one most constrained by calendar time, because data has to accumulate. That is why the adapter should be running passively as early as practical, well before the end of January 2027.

### Phase six: observability and alarms

A fallback that kicks in silently is almost as bad as no fallback, because nobody knows the system is running on its second choice. Needed:

- A metric or record each time the active provider changes, with the reason.
- A visible indicator in whatever dashboards the team uses, showing which provider is currently active.
- An alarm when the fallback has been active for longer than expected, since that suggests the primary has a real problem and someone has to look.
- An alarm when the fallback itself is failing while idle. The common failure of fallbacks is that they rot unnoticed and then fail exactly when they are needed. The passive mode from phase five helps here, because it keeps exercising the fallback path.
- Consider whether installers or customers should see anything. Probably the Svelte interface only needs a small, honest note when a forecast is based on reduced-quality data. Do not show it for every minor switch. This is a product question to settle with whoever owns that interface.

### Phase seven: rehearsal and cutover

Before relying on it, rehearse. Deliberately make the primary unavailable in a test environment, or simulate it, and watch what happens end to end: weather-feed-client switches, the data lands in InfluxDB with the right provider marking, the Julia forecast runs, the schedule is produced, and the front end shows sensible output. Then restore the primary and watch it switch back without flapping.

Also rehearse the case where the fallback is chosen but is itself degraded, since that is the realistic worst case.

Only after a clean rehearsal should the fallback be enabled in production. Leave time before the end of January 2027 for at least one repeat of the rehearsal after fixes. If the rehearsal turns up problems late, that is the first thing to threaten the date, so schedule it earlier rather than later.

## Interfaces with the rest of GridHaven

weather-feed-client sits between external weather sources and the rest of the system. Notes on each neighbour:

- InfluxDB: weather records are stored here as time series. Adding the provider marking means a new tag or field on the records. Think about cardinality and about whether old records, which have no marking, need a backfill or can just be treated as coming from the primary. The likely answer is that old records are the primary's, but confirm before relying on it. Retention rules for weather data should be checked against the terms of the new provider.
- MQTT: if weather updates or provider-change events are announced to other components over MQTT, the message format has to carry the provider and quality information too, or consumers on that path will not see it. Check which components subscribe before changing any message shape, and make changes additive so existing subscribers keep working.
- Azure IoT Hub: device-side messages from the installations come through here. The weather feed does not depend on it directly, but the measured output used in the comparison work arrives this way. Make sure the comparison job reads measured output from the established path and does not create a new one.
- Julia forecasting code: it consumes weather inputs. It needs to accept the quality flag and ideally to widen its uncertainty when the flag says the input is from the fallback or stale. Agree with whoever maintains that code on how the flag is read, and test it with fallback-sourced data before cutover.
- Battery scheduler: it consumes the forecast, not the weather directly, but it must be told when the forecast is degraded so it can choose a more cautious plan. Define the conservative behaviour in writing.
- Svelte front end: only needs to display status honestly. Keep the change minimal.

## Risks and open questions

Risks, with what to do about each:

- The chosen provider turns out to lack a key field. Mitigation: check this in phase one before committing, not in phase three. Keep a short list of acceptable alternatives in case the first choice fails the checks.
- Terms of use block storing or redistributing the data. Mitigation: read the terms in phase one and get a clear answer, in writing if possible, before building.
- The two providers are not independent. Mitigation: ask, as described above. If it cannot be established, treat the fallback as weaker than it looks.
- Calibration work does not get enough data in time. Mitigation: start passive collection early. If time runs out, ship with widened uncertainty and no correction, and continue calibration after the date.
- Flapping between providers. Mitigation: hysteresis, and testing it explicitly in the rehearsal.
- The fallback rots while idle. Mitigation: passive mode and an alarm on idle failure.
- Scope creep into a general multi-provider framework. The date is firm enough that a clean, small implementation for one fallback is better than a general design. If a third provider is ever needed, refactor then.
- The date slips again. The date has already moved once. Do not treat the later date as slack. Keep a running view of what is done and what is not, and raise any slip early.

Open questions, to be answered and then removed from this list:

- Which provider is the fallback. Not decided yet in this note.
- The exact definition of usable data, and the hysteresis period.
- What to do when both providers fail, and whether the scheduler agrees with the leaning stated above.
- Whether to correct fallback values or only widen uncertainty.
- Whether customers should see any indication of degraded forecasts.
- Who owns the alarms once this is in production.
- Whether the earlier note's reasoning for its original date contained anything that still applies. The date was replaced, but the reasons behind the earlier plan may include constraints worth keeping. Check it before discarding it entirely.

## Working notes for whoever picks this up

Things that will save time:

- Treat January 2027 as the end of the month, and as the latest acceptable time for production use, not as the day to start the final testing. Work backwards from it: leave room for the rehearsal, for a repeat after fixes, and for the passive comparison period.
- Do the provider decision first. Several people can be blocked by it, and delay there passes straight through to the end.
- Write down decisions as they are made, in this note or in linked decision notes, with the reason. The next person should not have to reconstruct why a choice was made.
- Keep changes to weather-feed-client behind a switch until the rehearsal passes, so the fallback can be turned off quickly if it misbehaves in production.
- When talking about this work, call the component weather-feed-client and call the work the fallback weather provider. The earlier name, weather feed second provider, was used in the note this one replaces, and mixing the names makes searches miss things.
- When reporting status, say which phase is done, which is in progress, and which are not started, and say plainly whether the end of January 2027 still looks achievable. A vague status that hides a slip is worse than a bad status.

## Definition of done

The fallback weather provider counts as delivered when all of the following hold, by the end of January 2027:

- A provider has been chosen on written evidence, and its terms permit what we do with the data.
- weather-feed-client has a fallback adapter that converts the provider's responses into the canonical shape, with the provider recorded on every record.
- Selection logic switches to the fallback when the primary is not usable and switches back without flapping.
- The behaviour when both providers fail is defined and agreed with the scheduler side.
- The passive comparison has run long enough to give a sense of the differences, and the decision on correction versus widened uncertainty is recorded.
- Alarms exist for long fallback use and for an idle fallback that is failing, and someone owns them.
- The rehearsal has been done at least once cleanly, including switching back.
- This note is updated to say what was actually built, and anything left over is listed as follow-up, not forgotten.

If by the middle of January 2027 these are clearly not going to be met, say so then, and decide what to cut. The least harmful things to cut are the correction of fallback values and the customer-facing indication. The things not to cut are the provider marking, the alarms and the rehearsal.
