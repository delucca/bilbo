---
id: 01KV77WPTBMVFN78CRH62DTFNJ
created: 2026-06-16T00:34-03:00
sources:
  - "code: config/tariffs/uk-eco7.toml"
---

# tariff-scheduler: where tariff definitions live and how they are read

This note is a pointer sheet for how `tariff-scheduler` finds and uses tariff definitions. The short version: every tariff is one TOML file under the config directory, and the file for the UK off-peak tariff is `config/tariffs/uk-eco7.toml`. Read this before adding a tariff or debugging a schedule that looks wrong.

## Where tariff definitions live

Tariff definitions read by `tariff-scheduler` live in `config/tariffs/uk-eco7.toml`, one TOML file per tariff. The directory is `config/tariffs/`. Each tariff gets its own file, named after the tariff. There is no single combined file and no database table for tariffs; the files are the source of truth.

```
config/tariffs/uk-eco7.toml
```

## One file per tariff

Do not put two tariffs in one file. The scheduler treats a file as a single tariff, so a second tariff pasted into the same file will either be ignored or will overwrite the first, depending on how the parser handles repeated tables. If an installer needs a variant, make a new file next to the existing one and give it its own name.

## What a definition describes

A tariff file describes the time-of-use structure: which periods of the day are cheap and which are expensive, and the unit price that applies in each. It is the only place the scheduler learns about price bands. Nothing about prices should be hard-coded in the Julia code.

## Naming convention

The file name carries the market and the tariff family, lowercase, with hyphens. `uk-eco7` is the example: country prefix first, then the tariff name. Follow the same pattern for new files so a directory listing sorts by market and is easy to scan.

## Who reads the files

Only `tariff-scheduler` reads these files. The forecasting side of GridHaven does not need them, because forecasting solar output does not depend on price. The Svelte front end shows tariff information to customers but gets it through the scheduler, not by opening the TOML itself.

## When they are loaded

The scheduler loads definitions when it starts and when it builds a plan. If a file is edited while the service is running, assume the change is not picked up until the next load. When in doubt, restart the scheduler after editing and look at the next plan it produces.

## Relationship to battery scheduling

The point of the tariff data is to decide when the battery should charge. Cheap periods are candidates for charging from the grid, and expensive periods are candidates for discharging or for staying idle. The scheduler combines the tariff bands with the solar forecast and the battery limits to pick charge windows.

## Relationship to the solar forecast

Solar forecasts come from the Julia forecasting code and are stored in InfluxDB. The scheduler reads the forecast and the tariff separately and joins them only at planning time. A wrong tariff file therefore changes charging decisions but leaves the forecast untouched, which helps when deciding where a bug sits.

## Relationship to device commands

Once a plan exists, charge and discharge commands go out to the home battery over MQTT, through Azure IoT Hub for fleet-managed installs. The tariff file has no direct role in that path. If commands arrive at the wrong time, check the plan first and the tariff file second, before suspecting the messaging layer.

## Time zones and clock changes

Time-of-use bands are local-time concepts. Be careful about how the file expresses times and which zone the scheduler assumes when it reads them. Clock changes in spring and autumn are the usual source of a band that appears shifted by an hour for a few weeks. When a customer reports charging at the wrong hour around those dates, look at zone handling before editing prices.

## Adding a new tariff

Copy an existing file in `config/tariffs/`, rename it following the convention, and edit the bands and prices. Keep the structure of the keys the same as the file you copied. Then make sure the installer's site configuration points at the new tariff by name. Review the diff with someone who knows the real tariff sheet from the supplier.

## Changing an existing tariff

Suppliers change prices and sometimes band boundaries. Edit the existing file rather than creating a copy, so history in version control shows how the tariff moved. If the change takes effect on a future date, note that in the commit message, since the file itself may only describe the current rates.

## Validation

A malformed TOML file will stop that tariff from loading. Keep edits small and check that the file still parses before committing. Typical mistakes are a missing quote, a duplicated key, and a band that overlaps another one. Overlaps and gaps in the day are worth checking by eye, because a parser will not always catch them.

## Common mistakes

- Editing the wrong file because two tariffs have similar names.
- Putting a price in the wrong unit, for example mixing pence and pounds.
- Forgetting that the scheduler may need a restart to see an edit.
- Treating `uk-eco7` as a template for tariffs from other countries without checking the band structure.

## Debugging a bad schedule

Start from the plan: which windows did the scheduler choose, and why. Then open the tariff file and compare the bands against what the supplier publishes. Next compare the forecast for the same day. Only after those three agree should you look at the device side. Most surprises have turned out to be the tariff file.

## Ownership and review

The scheduler owner is responsible for the loader code. The tariff values themselves are a data concern, and installers sometimes ask for changes. Treat any edit to `config/tariffs/uk-eco7.toml` or its siblings as a customer-facing change, because it alters when real batteries charge in real homes.

## Open questions

Still unclear to the author of this note: whether the loader reloads files on change without a restart, and how it behaves when two files declare the same tariff name. Confirm both in the loader source before relying on either behaviour, then update this note.
