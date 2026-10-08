---
id: 01K0V0MY7X83MM0B00ZC67HEPZ
created: 2025-07-23T03:48-03:00
---

# installer-dashboard reference

installer-dashboard is the Svelte front end that home energy installers use to look at their customers' solar and battery setups in GridHaven. This note is the quick reference for running it locally and for the things that tend to trip people up. The one command to remember is the local start: run installer-dashboard locally with `npm run dev -- --port 5173`, which starts the Vite dev server on port 5173. The extra `--` matters, because it tells npm to pass the `--port 5173` flag through to Vite rather than swallow it. Open the address Vite prints in the terminal once it is up.

## Running it locally

The whole local workflow is one command, run from the installer-dashboard project folder after dependencies are installed:

```bash
npm run dev -- --port 5173
```

That starts the Vite dev server for installer-dashboard on port 5173. Vite serves the Svelte app with hot module replacement, so edits to components show up in the browser without a manual reload in most cases. If you leave off the `-- --port 5173` part, Vite falls back to its own default port, which may differ from what teammates, bookmarks and any allowed-origin settings on the backend expect. So use the explicit form every time, and say so when you hand someone instructions.

If the port is already taken, Vite may quietly pick another one, depending on how it is configured, and then the address you open will not match what you wrote down. When the page does not load or loads something stale, check the terminal output first to see which address Vite actually bound to. A leftover dev server from an earlier session is the usual cause. Stop the old process before starting a new one instead of letting two compete.

The dev server is only for development. It does not represent how the installer-dashboard is built and served for real installers, and performance in dev mode is not a guide to production behaviour. Dev mode ships unbundled modules and extra tooling, so it feels different from a built bundle. Do not judge load time or bundle size from it.

A few practical habits for the local loop:

- Keep the terminal running the dev server visible. Compile errors from Svelte and Vite show up there first and are more informative than the blank page you may see in the browser.
- If the browser shows an old version after a dependency change, stop the server and start it again with the same command. Hot replacement does not always pick up changes to the dependency set or to build configuration.
- Use a private window or clear site data when testing sign-in behaviour, because cached tokens from an earlier run can mask problems.
- Do not paste real customer data into screenshots or bug reports made from a local session. Treat anything the dashboard shows as customer information.

## What the dashboard is for

The audience is installers and, in a limited way, their customers. GridHaven as a whole forecasts household solar output and schedules battery charging against time-of-use tariffs. The installer-dashboard is the place where an installer sees whether that is working for a given site: what the forecast says, what the system actually produced, what the battery was told to do, and whether the devices at the home are reporting in.

The views fall into a few broad groups. There is a fleet or portfolio view that lets an installer scan across all the homes they look after and spot the ones that need attention. There is a per-site view with the forecast set against measured output, and the battery charging plan set against the tariff periods. There is a device health view that shows whether the on-site hardware is connected and sending data. And there is a settings area for the parameters that shape scheduling for a site, such as tariff information and battery limits.

The dashboard does not do the forecasting or the scheduling itself. That work happens in the Julia services. The dashboard displays results and sends configuration changes. When a number on screen looks wrong, the first question is whether the dashboard is misrendering a correct value or faithfully showing a wrong one. Those two cases have different owners, so settle which it is before changing any Svelte code.

## Where the data comes from

The pieces around the dashboard are worth knowing, because most debugging crosses a boundary.

- InfluxDB holds the time series: measured production, consumption, battery state, and the stored forecasts. Charts in the dashboard are ultimately built from queries against it, reached through a backend rather than directly from the browser.
- MQTT is the messaging layer used by the devices and services for telemetry and commands. The dashboard is not normally a raw MQTT consumer in the browser. Live-looking values reach it through the backend, so a delay in the display can come from anywhere along that path.
- Azure IoT Hub is the cloud entry point for the devices installed at homes. Device identity, connection state and cloud-to-device messages go through it. Device health in the dashboard reflects what the hub and the backend know, which can lag behind what is physically happening at the home.
- The Julia services produce the solar forecasts and the charging schedules, and write them where the rest of the system can read them.

Because of this chain, a stale chart does not by itself mean the dashboard is broken. Work from the bottom up: is the device connected, did its data arrive, was it stored, did the forecast run, and only then whether the dashboard queried and drew it correctly. Skipping to the front end first wastes time more often than not.

## Local development against real or fake data

Running the dev server gives you the front end only. To see anything meaningful you need a backend to talk to. Use whichever non-production backend the team currently uses for development, and never point a local session at production data just to get realistic charts. If no backend is available, work on layout and component behaviour with fixed sample data and say clearly in the change description that it was checked that way. Sample data hides problems with empty states, gaps in time series and slow responses, so check those cases deliberately.

Time handling deserves care. Tariffs are defined in local time with periods that change through the day, while stored series are best treated as absolute timestamps. Most display bugs in this area come from mixing the two. When something looks off by a fixed number of hours, suspect a time zone conversion. When it looks off only around the days clocks change, suspect daylight saving handling. Test charts for sites in more than one time zone if the change touches anything about time.

Empty and partial data are normal, not exceptional. A newly commissioned site has little history and no meaningful forecast comparison yet. A site with a device offline has gaps. The dashboard should say what is missing rather than draw a flat line that looks like zero production. If a change makes a gap look like real zero output, that is a bug, because an installer may read it as a fault at the home and act on it.

## Things that go wrong

**The flag does not reach Vite.** Symptom: the server comes up on a different port than intended. Cause: the port flag was given without the separating `--`, so npm consumed it. Fix: use exactly `npm run dev -- --port 5173`.

**Port already in use.** Symptom: the command fails or the server starts elsewhere. Cause: an earlier dev server is still running, possibly in another terminal or a background session. Fix: stop it, then run the same command again.

**Page loads but shows no data.** Symptom: shell renders, panels stay empty or show errors. Cause is usually the backend being unreachable from the browser, or a cross-origin restriction because the dev origin is not what the backend allows. This is another reason to keep the port consistent with the documented one. Check the browser network panel before touching code.

**Values disagree with another tool.** Symptom: the dashboard shows a different figure than a query run straight against InfluxDB. Possible causes: different aggregation windows, different time ranges after time zone conversion, or rounding at display time. Compare like with like before concluding anything is wrong.

**Device shows connected but data is old.** Connection state and data freshness are separate facts. A device can hold a connection to Azure IoT Hub while its measurements stop arriving, or the reverse can briefly happen. Show both in the interface and do not merge them into one status.

**Forecast looks implausible.** Do not patch it in the front end. Report it to whoever owns the Julia forecasting side with the site, the time range and what you expected. Clamping or smoothing values in the display hides the real issue and misleads installers.

## Conventions for changes

Keep components small and keep data fetching out of presentational components, so views can be exercised with sample data. Prefer Svelte stores or the project's existing state mechanism for shared state rather than inventing a new one for each feature. Match the existing naming and file layout instead of reorganising as a side effect of a feature change.

For anything that sends changes to the backend, such as editing scheduling settings for a site, make the confirmation and the failure path obvious to the user. Battery scheduling affects real hardware and real energy bills, so a silent failure is worse than a loud one. Show what was changed, what the system currently has, and when a change is expected to take effect, since schedules apply at tariff period boundaries rather than instantly.

Accessibility and plain language matter here. Installers are not always at a desk, and some read the dashboard on a phone or tablet at the customer's home. Charts need text alternatives or readable summaries, colours should not be the only carrier of meaning, and labels should use words an installer would use, not internal service names.

## Before handing off a change

A short checklist that has saved trouble:

- Start it fresh with `npm run dev -- --port 5173` and confirm it comes up cleanly with no compile warnings you introduced.
- Walk the main views: fleet, a single site, device health, settings.
- Try an empty site, a site with gaps, and a site with a device offline.
- Check the time axis for a site in a different time zone than yours.
- Confirm failures from the backend produce a visible message, not a blank panel.
- Make sure no real customer data ended up in screenshots, test fixtures or commit messages.
- Say in the description what was checked against a live non-production backend and what was checked only against sample data.

## Open questions

These are not settled and should be confirmed with whoever owns the area before relying on them: how production builds are produced and hosted, which exact backend environment is the agreed default for local work, and how the dashboard authenticates installers. Add the answers here once known, in the same plain style, and correct any line above that turns out to be wrong. The one thing here that is settled and should not drift is the local start command: `npm run dev -- --port 5173`, which starts the Vite dev server for installer-dashboard on port 5173.
