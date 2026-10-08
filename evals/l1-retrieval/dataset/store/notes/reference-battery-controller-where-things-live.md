---
id: 01K3QR7FQT75WB3KCN1XCN1JYD
created: 2025-08-28T04:10-03:00
---

# battery-controller: where things live

Quick map of the battery-controller component, written from memory of how the project is laid out. It points to areas, not exact locations. Check the repo tree before trusting any of it.

## What it does

battery-controller decides when a home battery charges or discharges. It takes the solar forecast and the time-of-use tariff and produces a schedule. Installers see the result in the UI; the device gets commands through the cloud.

## Main code

The scheduling logic is Julia. Look in the Julia package for the controller, not in the forecasting package. The two are separate, and the controller only consumes forecast output. If something feels like it belongs to forecasting, it probably does.

## Optimisation core

The part that picks charge windows is the core of the Julia code. It takes forecast series, tariff bands and battery limits, and returns a plan. Keep it free of I/O so it can be tested with plain inputs.

## Tariff handling

Tariff data is read and normalised in its own module. Time zones and band boundaries cause most of the bugs here. Look at this module first when a schedule looks shifted.

## Forecast input

Forecast values are read from InfluxDB. The controller queries the bucket the forecasting side writes to. It does not write forecasts back. Controller outputs go to a separate measurement.

## Telemetry in

Battery state and household readings arrive over MQTT from the devices, relayed through Azure IoT Hub. The subscriber code is thin. It parses messages and hands state to the core.

## Commands out

Charge and discharge commands go back toward the device via Azure IoT Hub, either as device messages or direct calls. The sending code sits next to the subscriber, in the messaging area of the package.

## Device limits and config

Battery capacity, power limits and reserve settings come from per-site configuration. Defaults live with the package config; per-customer values come from the installer's setup. Do not hardcode limits in the core.

## Storage

InfluxDB holds time series: telemetry, forecasts, and the planned versus actual schedule. Retention and bucket setup are handled in deployment config, not in the Julia code.

## UI

The Svelte front end shows the schedule and lets installers change preferences. It talks to a backend API rather than to the controller directly. Look in the front end for the schedule view components.

## Tests

Julia tests sit beside the package and cover the core and tariff code with synthetic inputs. Messaging has fewer tests. The UI has its own test setup in the front end.

## Deployment

Deployment and IoT Hub settings live in the infrastructure config, separate from the application code. Secrets are not in the repo.

## Gotchas

- Forecast and controller packages are easy to confuse when searching.
- Clock and time zone handling is split across tariff and telemetry code.
- Stale telemetry should make the controller fall back to a safe plan; check that path when touching the subscriber.

## When something is missing

If a piece is not where this note says, search for the MQTT topic handling or the tariff module name first. Update this note when you find the real place.
