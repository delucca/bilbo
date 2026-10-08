---
id: 01JRM0S42VEQ882Q1XWZKZAM76
created: 2025-04-12T00:30-03:00
---

# pv-forecast-engine spec

This note is the working spec for pv-forecast-engine, the Julia service in GridHaven that turns weather and site data into solar output forecasts. The battery scheduler reads those forecasts to plan charging against time-of-use tariffs. It is written in a hurry from what the team has settled so far. Where something is still open, it says so.

The core requirement: pv-forecast-engine must produce a 48-hour forecast at 15-minute resolution for every registered site. That means each registered site gets a forecast that looks forward over a 48-hour horizon. The horizon is divided into 15-minute intervals, and each interval has an expected output value. There are no exceptions for small sites, new sites or sites with poor data. If a site is registered, it gets a forecast on every cycle. When the engine cannot do its best for a site, it still produces a forecast of the full length and marks it as degraded. It does not return a shorter one or skip the site. Everything below follows from this requirement.

## Scope and the core requirement

The engine answers one question for each site: how much energy will the array produce in each upcoming 15-minute interval across the next 48-hour window. It does not decide when to charge the battery, and it does not talk to customers. It produces numbers and quality information, and the scheduler and the Svelte front end consume them.

A reader who only has this note should be able to answer these questions:

- How long is the forecast? It covers a 48-hour horizon.
- How fine is it? The resolution is 15-minute intervals.
- Which sites get one? Every registered site, with no opt-in list and no minimum data requirement.
- What happens when data is poor? The forecast is still produced and is flagged as degraded.

The reasons for the shape are practical. A 48-hour horizon lets the scheduler see through the whole of the next day, including the morning after tomorrow's overnight charging decision. Tariffs often change at times that do not line up with whole hours, and a 15-minute grid lines up with the settlement periods and tariff boundaries that installers actually deal with. A coarser grid would blur the moment a cheap rate ends. A finer grid would claim precision that the weather inputs cannot support.

The forecast must be complete as well as present. Every interval in the horizon has a value, there are no gaps, and the intervals are contiguous and aligned to the same grid for all sites. Alignment matters because the scheduler compares and sums across sites, and because the front end draws charts from the raw series. If the grid differed per site, every consumer would need to resample.

What counts as a registered site is defined by the site registry, not by the engine. The engine reads the registry at the start of each cycle and treats whatever is there as the list to serve. A site that is registered mid-cycle is picked up on the next cycle. A site that is deregistered stops being forecast on the next cycle, and its stored forecasts follow the normal retention rules rather than being deleted at once.

### Non-goals

The engine does not do battery dispatch. It does not do tariff optimisation. It does not calibrate inverters or diagnose hardware faults, though it will notice when measured output is far from what it expected and will report that as a signal. It does not provide a public API for customers. It does not forecast household consumption, which belongs to a separate concern. It also does not try to be a general weather service. It uses weather data as an input and stores what it needs, nothing more.

## Inputs and data flow

The engine works from three families of input: site description, recent measurements from the site, and weather forecast data. Each has its own handling and its own failure modes.

### Site description

For each registered site the engine needs the physical description of the array: location, orientation of each array section, tilt, nominal capacity, and any known shading notes the installer entered. It also needs the inverter limits, because the inverter clips output and the forecast must not exceed what the hardware can deliver. This data comes from the site registry and changes rarely. The engine caches it in memory for the duration of a cycle and rereads it at the start of the next. Orientation and tilt are what the installer typed, and installers make mistakes. The engine does not silently correct them. It can flag a site when long-run measurements consistently disagree with the geometry, and that flag is reported through the quality information so a human can check.

### Measurements

Sites publish telemetry over MQTT, relayed through Azure IoT Hub. The engine does not subscribe to device topics directly in production. A separate ingestion path takes the device messages, validates them and writes them into InfluxDB. The engine reads recent production, and where available the inverter's reported state, from InfluxDB. This keeps the forecast engine decoupled from device connectivity: if a site's gateway drops off for a while, the engine sees a gap in the stored series and handles it, rather than blocking on a live connection.

Measurements serve three purposes. They anchor the near-term part of the forecast, since the next few intervals are best predicted by what the array is doing now. They feed the per-site correction that learns how a particular site differs from the generic physical model, such as persistent shading or soiling. And they let the engine score its own past forecasts. Timestamps are treated as UTC everywhere inside the engine. Local time only appears at the edges, where tariff windows are interpreted and where the front end renders. Daylight saving changes therefore do not alter the grid.

Measurement data is messy. Expected problems include duplicate points, late points that arrive out of order, counters that reset, negative values at night from inverter standby draw, and flat lines where a gateway repeated its last value. The engine cleans these before use. Night values are clamped to zero for the forecast target. Flat runs during daylight are treated as missing rather than real. The cleaning rules are deliberately conservative. When in doubt, a point is dropped, because the engine can bridge a short gap and cannot recover from a trusted bad value.

### Weather data

The physical model needs irradiance, cloud cover, temperature and wind at least. These come from an external weather forecast provider. The engine fetches data for the geographic area around each site, and nearby sites share fetches to avoid redundant calls. The raw provider data is coarser in time than the output grid in some cases, so the engine interpolates it onto the 15-minute grid. Interpolating irradiance linearly through sunrise and sunset gives wrong shapes. The engine therefore interpolates a clear-sky-normalised quantity and multiplies it back by the computed clear-sky curve for each interval. This keeps the morning ramp and evening drop shaped correctly even when the provider only gives coarse steps.

Weather forecasts are issued on the provider's schedule, not ours. The engine keeps the most recent successful pull and records its issue time. If a new pull fails, the previous one is used and the forecast is marked as relying on older weather. The far end of the horizon is the least reliable part, and the engine widens its uncertainty there instead of pretending to be sure.

### Flow of a cycle

A cycle goes in this order. Read the registry for the list of sites. Pull or refresh weather for the needed areas. For each site, read recent measurements from InfluxDB, clean them, compute clear-sky and plane-of-array irradiance for each interval, apply the site model and correction, clip to inverter limits, attach uncertainty and quality flags, and write the result. The per-site work is independent, so it runs in parallel across sites using Julia's threading. One slow or failing site must never delay or prevent the others. A failure in one site's computation is caught at the site boundary, logged with the site identifier, and handled by the fallback path described below, so that site still gets its forecast.

The cycle runs on a fixed cadence and also on demand for a single site, for example when an installer has just commissioned a system and wants to see a forecast immediately. The on-demand path uses the same code as the scheduled path, with the list of sites narrowed to one. There must not be a separate implementation, because two paths drift apart.

## Output contract

The output for a site is a time series. Each element has the interval start time, an expected energy or average power value for that interval, a lower and an upper bound for the uncertainty band, and a quality flag. The series covers the full 48-hour horizon and every element is on the 15-minute grid. The first interval is the one containing or immediately following the cycle's issue time, and the series runs forward from there. Alongside the series, the engine records metadata: the issue time, the weather issue time used, the model version, and an overall site-level quality summary.

### Units and conventions

Values are for the interval, not for an instant. The unit and its meaning must be identical across all sites and all consumers. The scheduler and the front end must not guess. The convention is fixed in the shared schema definition and not restated differently in each component. If anyone proposes changing it, treat that as a breaking change for every consumer. Negative values are not allowed in the output. Values never exceed the inverter limit for the site. Bounds satisfy lower at most equal to the expected value, which is at most equal to upper.

### Storage

Forecasts are written to InfluxDB so that the scheduler, the front end and the accuracy reports all read from one place. Each forecast is tagged with the site and the issue time, so several issues for the same target interval can coexist. This is what makes later scoring possible: for any past interval we can find what was predicted, and when. The engine never overwrites a previous issue. It writes a new one. The latest issue for a site is what consumers normally read, and the history is for evaluation and debugging. Retention for old issues is a storage policy and is set outside the engine.

The write must be effectively atomic from the reader's point of view. A reader must never see half of a new issue and half of an old one as if they were one forecast. The engine achieves this by writing all points of an issue and only then writing a marker that makes the issue visible as the latest. Readers use the marker. If the engine dies mid-write, the incomplete issue is invisible and the previous complete one remains current.

### Notification

When a new issue is complete for a site, the engine publishes a small message on MQTT saying that a new forecast is available for that site and giving the issue time. It does not put the series in the message. The scheduler reacts by reading from InfluxDB. This keeps messages small and keeps one source of truth for the data. Messages are not relied on for correctness. The scheduler also polls on its own schedule, so a lost message delays reaction and nothing worse.

### Quality flags

Each interval and each series carries a quality state. The states are roughly: normal, degraded and fallback. Normal means fresh weather, adequate recent measurements and a healthy site model. Degraded means one or more inputs were stale or thin but the standard model still ran, for example weather older than expected or a gap in recent measurements. Fallback means the standard path could not run and a simpler method produced the values. The flag travels with the data so the scheduler can choose to be more cautious, for example by charging from the grid with more margin when forecasts are degraded. The front end shows the state to installers in plain words.

## Fallbacks, failure handling and the requirement

Because every registered site must get a full forecast, failure handling is part of the spec and not an afterthought. The rule is simple: there is always a result, and the quality flag tells the truth about how it was made.

### Degradation ladder

The engine tries the best method first and steps down only as far as it must.

First, the full path: fresh weather, cleaned recent measurements, site correction applied. This is the normal case.

Second, stale weather: the last successful weather pull is used. Uncertainty bands widen according to how old it is. The flag is degraded.

Third, thin or missing recent measurements: the near-term anchoring is skipped and the correction from long-run history is used without a recent adjustment. The flag is degraded.

Fourth, no usable history for the site, such as a newly registered site: the engine uses the physical model from the site description alone, with a conservative loss factor, and wide bands. A brand-new site therefore still gets a forecast on its first cycle. The flag is fallback until enough measurements accumulate.

Fifth, no usable weather at all and no recent weather pull: the engine uses a climatological profile for the location and time of year, shaped by the clear-sky curve. This is the lowest rung. It is crude, but it is full length, on the grid, and honestly flagged as fallback.

The ladder is evaluated per site, so one site's problem does not drag others down the ladder. Missing data in a shared weather area affects all sites in that area, which is expected and visible in the flags.

### Things that must not happen

The engine must not return an empty series. It must not return a series shorter than the horizon. It must not return values off the grid. It must not emit not-a-number or infinite values: these are checked right before writing, and any such value triggers the next rung of the ladder for that site. It must not block the whole cycle on one slow external call, so every external call has a timeout, and the timeout path leads into the ladder. It must not crash the process because of a single site's bad data, since the site boundary catches errors.

A post-computation validation step runs for every site before anything is written. It checks completeness over the whole horizon, grid alignment, value ranges, ordering of bounds, and absence of non-finite values. If validation fails, the site is recomputed one rung lower. If even the lowest rung fails validation, which indicates a bug, the engine logs loudly, raises an alert, and writes the last good forecast shifted forward on the grid and flagged as fallback, so consumers still have something. This last resort should never be seen in practice, and any occurrence is a defect to fix.

### Timing

A forecast is only useful if it arrives before the scheduler needs it. The cycle must finish for all sites well before the scheduler's planning moments, which sit ahead of tariff boundaries. The exact budget is an operational setting and is not fixed in this note. The design intent is that the cycle scales with the number of sites by adding threads or instances, not by lengthening the cycle. If the cycle overruns its cadence, the engine does not start a second overlapping cycle on the same sites. It skips or delays the next start, logs the overrun, and reports it as a metric. Sustained overruns mean capacity is short and need action, because they break the requirement in practice even if the code is correct.

### Scaling and registration growth

The requirement says every registered site, and the number of sites will grow as installers onboard customers. The engine therefore must not hold per-site state in a way that grows without bound in memory, and should process sites in batches with bounded concurrency. Weather fetches are grouped by area so that growth in sites within an area does not grow calls to the provider. Writes to InfluxDB are batched. If a single process cannot meet the timing, sites can be partitioned across instances by a stable assignment from the site identifier, and each instance serves its partition. Partitioning must cover all sites with no gaps and no overlaps, and a test should check this against the registry. A site left out by a partitioning bug would silently violate the core requirement, which is exactly the kind of failure that is hard to see from outside.

## Accuracy, evaluation and open questions

### How we judge it

For every past interval, the stored issues let us compare predicted with measured output. The engine, or a companion job, computes error summaries by site, by lead time within the horizon, by time of day and by weather regime such as clear, mixed and overcast. Error should grow with lead time, and we expect the far end of the 48-hour window to be much worse than the first hours. That is acceptable as long as the uncertainty bands say so. The bands are judged separately by coverage: over a long period, measured values should fall inside the stated band about as often as the band claims. If the bands are too narrow, the scheduler will trust forecasts it should not. If they are too wide, the scheduler will be overly cautious and waste money for customers.

A naive baseline is kept for comparison: persistence of the previous day shaped by clear-sky. Any model change must beat this baseline and the previous model version on a held-out period before it ships. Scoring uses energy over scheduler-relevant windows as well as per-interval error, because the scheduler cares about whether a block of cheap-rate hours will fill the battery, not whether one interval was off.

### Testing expectations

Tests should cover the requirement directly. A test registers a set of sites with varied conditions and asserts that each has a series covering the whole 48-hour horizon on the 15-minute grid, with no missing intervals. Further tests force each rung of the degradation ladder, by cutting weather, by emptying measurements, and by registering a site with no history, and assert that the series is still complete and that the flag matches the rung. Property-style tests feed odd inputs, such as polar-like latitudes, sites near the date line, daylight saving transitions, leap days, and arrays with zero capacity, and check that validation passes. Another test confirms that one failing site does not stop others and that the failing site still receives a flagged forecast. A test for partitioning confirms that every registered site is covered exactly once.

### Open questions

- Whether the uncertainty band should be a fixed-confidence interval or a small set of quantiles. The schema should leave room for quantiles even if we start with a single band.
- How aggressively the per-site correction should adapt. Fast adaptation tracks soiling and new shading but chases noise. Slow adaptation is stable but lags. This needs data from real sites before deciding.
- Whether to blend more than one weather provider. It would help in the far part of the horizon but adds cost, complexity and another failure source for the ladder.
- How snow cover on panels should be handled. At the moment it appears only as unexplained error. A separate snow signal may be needed for cold-climate installers.
- Whether installers should be able to override geometry or add shading notes from the front end and see the forecast change at once. This touches the on-demand path and the registry and needs agreement with the Svelte side.
- What the right retention is for old issues, balancing evaluation needs against storage cost in InfluxDB.

### Decisions already settled

The horizon and resolution are fixed by the requirement: a 48-hour forecast at 15-minute resolution for every registered site. The engine is in Julia. It reads measurements from InfluxDB, gets device data via the MQTT and Azure IoT Hub path owned by ingestion, and writes forecasts back to InfluxDB. A forecast is always produced and always flagged honestly. Notifications are small MQTT messages and not data carriers. Any change to these points should be written up and agreed with the scheduler and front end owners before code changes, since both depend on the contract described above.
