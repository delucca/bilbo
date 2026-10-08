---
id: 01JYT9WSPMT09KKQ4R5B1DV2KZ
created: 2025-06-28T00:39-03:00
---

# pv-forecast-engine: where things live

This is a map of `pv-forecast-engine`, not a spec. It says where to look first for each piece. It gives no values on purpose. Open the code and config for those, and check this note against them before you trust it, because layouts drift.

`pv-forecast-engine` is the part of GridHaven that turns weather inputs and recent household generation into a predicted solar output curve. The battery scheduler reads that curve and plans charging against the time-of-use tariff. The engine is written in Julia. Telemetry comes in over MQTT, passes through Azure IoT Hub, and is stored in InfluxDB. The Svelte front end only shows what the engine produces.

## Rough layout of the Julia side

The engine is a normal Julia package with a project file at its root, a source directory, and a test directory. Start at the main module file in the source directory. It includes the other files and decides what gets exported. If you are hunting for a function, read the include list there first, then grep.

The source is split by job, roughly:

- Ingestion and cleaning of raw readings: parsing messages, dropping bad samples, filling short gaps.
- Feature building: sun position, clear-sky estimates, lagged generation, weather fields lined up to the same time grid.
- Models: the forecasting logic itself, plus anything that fits or calibrates per-site parameters.
- Output: shaping the forecast into the form the scheduler and the API expect, and writing it back.
- Configuration loading: site settings, panel geometry, and which tariff a site is on.

Names of files change more often than the split does. Trust the split, grep for the rest.

The test directory mirrors the source layout loosely. Small unit tests use synthetic series. Anything using real recorded days lives in a data folder next to the tests. Do not add large recorded files to that folder without asking.

## Telemetry path: MQTT, IoT Hub, InfluxDB

Inverters and meters at a home publish readings over MQTT. In deployments, devices are registered in Azure IoT Hub and messages are routed from there. The engine does not talk to devices directly. It consumes what has already been routed and written.

For local work there is usually a plain MQTT broker standing in for the hub. Topic naming is set in config, not hard-coded in several places, so find the topic template in the config loader rather than guessing. If readings do not arrive in local runs, check the broker and the topic template first, and only then suspect the engine.

The routing rules in the hub (which message types go where) are not in the Julia package. They live with the deployment definitions. If a new field from a device never shows up, the cause is often there or in the device firmware, not in the parser.

## InfluxDB: what is stored and how it is read

InfluxDB holds both the measured series and the forecasts the engine writes. Measurements are separated from forecasts by measurement name, and sites are separated by tags. Read the query helpers in the engine's storage code to see the exact names; do not copy them from dashboards, which may use older ones.

Things to keep in mind when touching queries:

- Raw readings and downsampled readings sit in different buckets with different retention. Check which bucket a query targets before concluding data is missing.
- Timestamps are stored in UTC. Local time and tariff windows are applied later, in the scheduler and in the display.
- Forecast writes should be repeatable for the same run: writing the same forecast twice must overwrite, not duplicate. Look at how the writer builds its points before changing it.
- Credentials and URLs come from the environment or a config file kept out of version control. Never put them in the note, a test, or a commit.

## Handoff to the battery scheduler

The scheduler is a separate component. It consumes the forecast and the site tariff and produces a charging plan. The engine's job ends at the forecast. If a charging plan looks wrong, first decide whether the forecast was wrong or the scheduler misused a good one. Pull the stored forecast for the time in question and compare it to what was measured afterwards. Only change the engine if the forecast itself is off.

The shape of the forecast (time grid, units, whether there is an uncertainty band) is a contract with the scheduler. Find it in the output code and in whatever shared types or schema the two sides use. Change it on both sides in the same piece of work, and say so in the commit message.

## Front end and API surface

The Svelte app shows forecast against actual generation for a site, and is used by installers and by customers. It reads through an API layer, not from InfluxDB directly. When a chart looks wrong, check three places in order: the raw stored series, the API response, then the component that draws it. Most bugs seen so far in this area are unit or time-zone mismatches at the API or display layer, not model errors. Verify that before you start retuning models.

Installer views expose more settings than customer views. Panel geometry, orientation and shading entries are edited there and end up in the site config that the engine loads. If a forecast is consistently biased for one site, look at those site entries before touching shared model code.

## Running and checking things

Work from the package root and use the Julia project environment, not the global one. Instantiate dependencies once, then run the test suite from the package. Keep long model-fitting runs out of the default tests; they are separated so the quick suite stays quick. Look at the test setup file to see how that split is made.

For a manual end-to-end check, the usual path is: start a local broker and InfluxDB, replay a recorded day through the ingestion code, run the forecast, then look at the written series. A replay script or helper exists for this in the repo tooling. Look in the scripts or tools folder rather than writing a new one.

Notes on habits that save time:

- When a forecast is flat or empty, check for missing weather input before anything else.
- When a forecast is shifted in time, check time zone and daylight-saving handling in feature building.
- When the first day of a new site is poor, that is expected: there is no history for the lagged features yet.
- Julia start-up and compile time is large. Keep a session open while iterating instead of restarting for each change.

## Open questions and things to confirm

I have not confirmed where the deployment definitions for the hub routing live relative to this repo. Ask or search before assuming they are in the same tree. I also have not confirmed how model parameters are versioned per site. If you find out, update this note and replace the guess with a pointer.

If something in this map is wrong, fix the note in place instead of adding a second one on the same component.
