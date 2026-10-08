---
id: 01M18M00KKXGBQ7K3BK77HW1MJ
created: 2026-08-30T02:59-03:00
---

# pv-forecast-engine holdout accuracy report

pv-forecast-engine reached a mean absolute error of 0.21 kWh per day on the March 2026 holdout set. That is the headline result of this report. The error is measured per household per day, on predicted versus metered solar energy, and averaged over every household and every day in the holdout. This note records what that number means, what it does not cover, how it was produced, and what to check before anyone quotes it to an installer or a customer.

## Result in one place

The figure is a daily energy error, not an instantaneous power error. A forecast for a day is a sum over the day's production curve, and the metered value is the sum of the inverter's reported output for the same day. The absolute difference is taken per household-day, and then the mean is taken over all household-days in the holdout.

```
metric:   mean absolute error, daily energy
value:    0.21 kWh per day
holdout:  March 2026
engine:   pv-forecast-engine
```

If someone asks "how good is pv-forecast-engine", the honest answer is: 0.21 kWh per day mean absolute error on the March 2026 holdout set, and nothing in this note says it holds for other months.

## What the holdout is

The holdout is the set of household-days from March 2026 that were kept out of training and out of any tuning. The engine never saw those days when its parameters were fitted. Weather inputs for those days were the forecasts that would have been available the evening before, not the observed weather afterwards, so the number reflects the real operating condition: forecast weather in, forecast energy out.

March is a transitional month in the northern hemisphere. Days swing between clear and heavily clouded, and sun elevation climbs quickly through the month. That makes it a reasonably hard test, but it is still one month. It does not include midsummer, midwinter, or snow cover on panels.

## How the metric is computed

For each household and each day in the holdout, take the forecast daily energy and the metered daily energy, subtract, take the absolute value. Average across all household-days with equal weight. No household is weighted by array size, so a small array and a large array count the same. This matters when reading the result: a large array contributes larger absolute errors in kWh for the same relative miss, so the mean is sensitive to the size mix of the households in the set.

Days with missing meter data were excluded rather than filled. Days where the inverter reported a fault state were also left out of the metric, since the forecast cannot be blamed for a unit that was not producing.

## Data path

Metered production arrives from the household gateways over MQTT, goes through Azure IoT Hub, and lands in InfluxDB as time series per household. The forecast engine, written in Julia, reads its history from InfluxDB and writes its forecasts back to InfluxDB so that the scheduler and the Svelte dashboard read from one place.

The evaluation pulled both series from InfluxDB for the March 2026 window and joined them on household and calendar day. The join key is the local calendar day of the household, not UTC, because the daily total of a household should follow its own sunrise and sunset. A UTC join would split an evening across two days for some regions and add noise that is not the engine's fault.

## Inputs the engine used

The engine takes the array description for each household (tilt, orientation, nominal capacity as recorded at install), recent metered production, and a weather forecast covering cloud cover and irradiance estimates. It combines a clear-sky model with a correction learned from past production, so that shading, soiling and local horizon effects are folded into the correction rather than modelled separately.

For the holdout, the learned correction was frozen from data before March 2026. It was not refit on March days, even though refitting would likely lower the error. The reported number is therefore the frozen-model number.

## Why this number matters for scheduling

The reason to care about daily error is the battery scheduler. It charges against time-of-use tariffs and decides how much to pull from the grid in cheap hours based on how much solar is expected the next day. An over-forecast leaves the battery undercharged when the sun does not show, and the household buys at the expensive rate. An under-forecast charges from the grid when the sun would have done it for free.

A mean absolute error of 0.21 kWh per day is small next to typical household battery capacity, so on average the scheduler's decision is only mildly off. The mean hides the bad days, though, and the bad days are what cost money.

## What the mean hides

Mean absolute error says nothing about the tail. A few badly missed days, such as an unforecast storm front or a day of fog, can sit inside a small average. This report does not give the distribution of errors, a worst-day figure, or an error split by weather type. Anyone who needs to size a safety margin in the scheduler should compute those from the same holdout rather than reuse the mean.

Bias is also not reported here. Absolute error cannot tell a model that is often too high from one that is often too low. If the engine leans in one direction, the scheduler would want to know, because the two directions cost different amounts under a time-of-use tariff.

## Limits of the claim

The claim is narrow on purpose.

- It covers March 2026 only. Other seasons are untested in this report.
- It covers the households present in that holdout. New installs with little history behave differently, since the learned correction has less to work with.
- It uses day-ahead forecast weather. If the weather provider changes or degrades, the number can move without any change in the engine.
- It is daily energy. Intraday shape, which matters for when the battery should charge, is not scored here.

Do not present 0.21 kWh as a guarantee to customers. It is an average on a past month.

## Comparison to a baseline

This report does not include a baseline comparison, and I did not want to invent one. A fair reading of the result needs a naive reference, such as predicting today's production from yesterday's, run over the same holdout and scored the same way. Without it, the figure shows the engine is in a useful range but not how much better it is than something trivial.

That comparison is cheap to run, since the data is already in InfluxDB and the metric code is the same. It is the first follow-up on the list below.

## Reproducing the evaluation

To reproduce, fix the holdout to the March 2026 household-days, freeze the learned correction at its pre-March state, load the day-ahead weather forecasts that were archived for those days, run the forecast for each household-day, and score against metered daily energy from InfluxDB. Keep the exclusions for missing meter data and inverter faults, otherwise the result will not match.

The usual traps: using observed weather instead of archived forecasts makes the error look better than it is in production; joining on UTC days makes it look worse; refitting on March makes it look better in a way that does not carry forward. Check these three before suspecting the engine.

## Open questions

- How does the error behave in other months, especially midsummer and midwinter?
- What is the error distribution and the worst-day behavior?
- Is there a systematic bias, and does it differ between clear and cloudy days?
- Does the error depend on array size or on how long the household has been on the platform?
- How much of the error comes from the weather forecast and how much from the engine itself?

None of these is answered by the single number in this report.

## Follow-ups

1. Run a naive baseline over the same March 2026 holdout and record it next to the engine's figure.
2. Report bias and an error distribution, not only the mean.
3. Repeat the evaluation on a holdout from another season, with the same freezing rules.
4. Score intraday shape, since the scheduler cares about timing as well as total energy.
5. Add the evaluation as a repeatable job so the number can be regenerated after any change to the Julia code or the weather source.

## Status

As of this note, the settled fact is: pv-forecast-engine reached a mean absolute error of 0.21 kWh per day on the March 2026 holdout set. Everything else above is context, caveats, or work not yet done. When a later run produces a different figure, update this note rather than adding a second one, and say which holdout and which frozen model it refers to.
