---
id: 01KP0QAXSFGJS5E924M77G6HF7
created: 2026-04-12T08:30-03:00
---

# weather-feed-client: general direction

The team settled on a direction for weather-feed-client: keep it a thin, boring client that pulls forecast data from outside providers, normalizes it into one internal shape, and hands it on. It does not try to be smart. The solar forecasting code in Julia does the thinking. The client only fetches, cleans, stamps and forwards. This note records why we went that way and what it means day to day. It gives no tuning values on purpose. Those live in config and change often.

## What we chose

weather-feed-client stays a single-purpose component. It talks to the weather providers, converts what they return into our own record layout, and publishes the result so the rest of GridHaven can use it. Everything else is somebody else's job.

Things we decided it should do:

- Treat every provider as unreliable. Any call can be slow, empty, partial or wrong, and the client has to cope without taking the scheduler down with it.
- Keep a single internal record shape. Provider quirks stop at the edge of the client. Nothing downstream should know which provider a number came from, except through a source tag kept for debugging.
- Carry the origin and the fetch time on every record, so a forecast run can say later what it was fed.
- Prefer stale-but-labelled data over no data, as long as the label is honest and the age is visible to the consumer.
- Publish through the existing messaging path rather than inventing a new one. Consumers subscribe and do not call the client directly.
- Write a copy of what it ingests into the time-series store, so we can replay and compare forecasts against what actually happened.

Things we decided it should not do:

- No solar physics. No panel tilt, shading, or inverter modelling in the client. That belongs with the forecasting code.
- No tariff logic and no battery decisions.
- No per-customer branching. The client fetches by location, and mapping a location to a household happens elsewhere.
- No hidden caching rules that only one person understands. If it caches, the policy is written next to the code and mentioned in the config docs.

## Why this shape

We went back and forth on a fatter client that would also do interpolation, bias correction and gap filling. It was tempting because the data is right there. We dropped it for a few reasons.

First, the failure modes multiply. If the client both fetches and corrects, then a bad forecast could be a provider problem, a parsing problem or a correction problem, and the logs would not tell us which. With a thin client, a bad number is either wrong at the source or wrong in the model, and we can check the stored raw record to see which.

Second, the people who touch the forecasting model change it often, and they should be able to do that without worrying about network behaviour. The reverse is also true. Whoever changes provider handling should not be able to shift model output by accident.

Third, installers occasionally ask why a forecast looked off on a given day. The answer is far easier if the raw input is stored untouched next to a record of what was done to it afterwards. A fat client blurs that.

Fourth, it keeps the component testable. A thin client can be tested with recorded provider responses and a fake clock. A client full of modelling needs real weather scenarios, and nobody wants to maintain those inside a networking component.

## How it fits with the rest of the system

The client sits upstream of the Julia forecasting code and downstream of the outside providers. It publishes over MQTT in the same style as the device telemetry, so consumers already know how to subscribe. Storage goes to InfluxDB, which is where the history lives for comparison with measured output. The installer-facing Svelte app never talks to the client. If the app shows weather, it reads what the backend already holds.

Azure IoT Hub is on the device side of the house. The client does not depend on it. We kept that boundary on purpose so that a problem with device connectivity cannot stop weather ingestion, and a problem with a provider cannot stop device messages.

Credentials for providers come from the normal secret handling in the deployment, never from files in the repo. The client reads them at start and on rotation. This note does not say where they are kept. Look at the deployment docs.

## Behaviour under trouble

The general rules, in rough order of importance:

1. A provider failure is normal, not exceptional. Log it once in a useful way, back off, try again later. Do not spam.
2. If one provider is down and another works, use the one that works and tag the record so downstream knows.
3. If everything is down, keep publishing the last good data with its true age, and let the consumer decide whether it is too old to use. The client should not decide on behalf of the scheduler how stale is too stale. That is a forecasting and scheduling concern.
4. Malformed responses are dropped and counted. We do not try to repair them. A repaired value looks like a real value later, and that is how bad data hides.
5. Clock problems matter. The client uses its own fetch time and the provider's stated valid time separately and never mixes them up. Mixing them is the easiest way to shift a forecast by hours without anyone noticing.
6. Rate limits and quotas from providers are respected. If we are close to a limit, we fetch less often rather than risk a block. Solar output does not change fast enough for this to hurt much.

## Open questions and things to watch

Some questions are still open and we are not deciding them here.

- Whether to add a second or third provider for redundancy, and how to compare them. For now the structure allows it, but nobody has argued for the cost yet.
- How much history the client itself should keep in memory versus relying on the store. Leaning toward very little, with the store as the single source of truth.
- Whether location handling should snap nearby households to a shared fetch point to save calls. It would cut load but loses some local accuracy, and we want evidence from stored forecasts before choosing.
- Whether the source tag should be exposed to installers in some form. Probably not, but support may ask.

Things to watch when changing the client later:

- If someone proposes putting a correction step into weather-feed-client, push it to the forecasting side and ask for a stored-before and stored-after comparison first.
- If the record shape changes, every consumer and the replay tooling need to know. Change it deliberately, not as a side effect of adding a provider.
- If a new provider returns something that does not fit the shape, extend the shape carefully or drop the field. Do not smuggle provider-specific fields through under generic names.
- Keep the client's logging readable. When a forecast looks wrong, the first place anyone looks is the client log, and it has to say which provider, which location and what happened without needing a debugger.

## Rule of thumb

If a change makes weather-feed-client know more about solar panels, batteries or tariffs, it is probably in the wrong place. If a change makes it better at getting honest, labelled, stored weather data from flaky providers, it is probably right.
