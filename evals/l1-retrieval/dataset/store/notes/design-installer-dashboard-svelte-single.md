---
id: 01M1SHWY2MW7JNSM7SMBZJNY8W
created: 2026-09-05T16:49-03:00
sources:
  - "code: package.json"
---

# installer-dashboard design

installer-dashboard is a Svelte 5 single page app built with Vite and served as static files from Azure Blob Storage. There is no server process behind it that we run ourselves. The build produces a folder of static assets, those get uploaded to a blob container that is set up for static website hosting, and the browser does everything else. This note is the design as it stands: what the app is for, how it is put together, where the data comes from, and the traps we already know about. It is written fast, so expect gaps. When something here turns out wrong, fix it in place instead of adding a second note.

## Purpose and users

installer-dashboard is the screen that home energy installers use to look after the sites they have fitted. GridHaven forecasts household solar output and schedules battery charging against time-of-use tariffs. The customer sees a simpler view of that elsewhere. The installer view is for the person who has to answer the phone when a battery did not charge overnight, or when the forecast looks off for a roof that was just fitted.

The installer wants to answer a short list of questions without leaving the page. Is the site online? What did the forecast say for today, and what did the panels actually produce? What did the scheduler decide about charging, and which tariff window was it aiming at? Did the device accept the last schedule? Is anything in a bad state that needs a visit or a call?

It is not a configuration tool for the forecasting models and it is not an admin console for GridHaven staff. Those would need different permissions and a different audience. Keep the scope narrow: read-mostly, per-installer, per-site. The few write actions are small and listed further down.

Installers are not engineers. Labels should say what happened in plain words. Where a technical term is unavoidable, such as a tariff window or a state of charge, show it the same way everywhere in the app. If the same thing has two names on two screens, installers will think there are two things.

## Hosting model

The app is static. Vite builds it, and the output is copied to Azure Blob Storage, where it is served as plain files. That choice was made on purpose. It keeps hosting cheap, it removes a whole class of runtime failures, and it means a deploy is a file copy, not a rollout of a service.

Consequences that matter day to day:

- There is no server-side rendering. Everything the user sees is built in the browser after the assets load. The first paint is therefore a shell, and data fills in after.
- Routing is done in the browser. Because blob hosting has no rewrite rules of its own beyond a fallback document, deep links must resolve to the single entry page and let the client router take over. If a deep link returns a storage error page instead of the app, the fallback document setting on the storage account is the first thing to check.
- Caching needs care. Hashed asset files can be cached for a long time because their names change when content changes. The entry page must not be cached hard, or users keep loading an old shell that points at asset names that no longer exist. After a deploy, a stale entry page is the usual reason someone reports a blank screen.
- Secrets cannot live in the bundle. Anything shipped to the browser is public. Configuration baked in at build time must be limited to values that are fine for anyone to read, such as the address of the API and the public identifiers of the sign-in provider.
- Cross-origin rules apply. The static host and the API are different origins, so the API must allow the dashboard origin explicitly. When that breaks, the symptom is a working page with every data panel failing at once.

If we ever need headers or rewrites the storage service cannot do, the usual answer is a CDN or front door in front of the container, not a server. That has not been needed so far.

## Front-end structure

The app uses Svelte 5 and its rune-based reactivity. State that belongs to one component stays in that component. State shared across screens, such as the signed-in installer, the selected site and the chosen time range, lives in small shared modules that export reactive state and a few functions to change it. We avoided a heavy global store on purpose. Most screens only need the selected site and the time range, and passing those down is cheap.

Vite handles the dev server, the production build, environment-specific configuration and code splitting. Routes are split so that the heavy chart code only loads on screens that draw charts. The site list screen is light and should load fast on a phone with poor signal, because installers often open it standing in a driveway.

Components fall into three loose groups. First, layout and navigation: the shell, the site picker, the time range control. Second, data panels: each one fetches or subscribes to one kind of data and renders it, and each owns its own loading, empty and error states. Third, small shared pieces such as formatted energy values, tariff window badges and status chips. Keep formatting in the shared pieces. Energy and power units, rounding and time zone handling were inconsistent early on, and putting them in one place fixed most of the confusion.

Panels must never assume data exists. A new site has no history. A site that lost connectivity has a gap. A site on a tariff the scheduler does not understand yet has no schedule at all. Each of these needs a plain message, not a blank box and not a thrown error.

Accessibility is basic but not optional: keyboard reachable controls, sensible labels, and color not being the only signal for status. Charts have a text summary next to them.

## Data sources and flow

The dashboard does not talk to the devices or to the database directly. Everything goes through an API owned by GridHaven. Behind that API, the pieces are these.

Devices at the sites publish readings and receive schedules over MQTT, with Azure IoT Hub as the cloud endpoint for device connections and messaging. Time series data, meaning measured production, consumption, battery state and the stored forecasts, is kept in InfluxDB. The forecasting and charge scheduling logic is written in Julia and runs as backend jobs. The dashboard sees none of that as such. It sees the API's view: a site, its recent measurements, its forecasts, its schedule, and its device status.

The mental model for a panel is: ask the API for a window of time for one site, get back series and a few summary values, draw them. Two kinds of freshness exist. History and forecasts change slowly and are fetched on demand when the installer opens a screen or changes the range. Device status changes quickly and is refreshed on a timer or pushed, depending on the panel. Do not make every panel poll fast. It wastes battery on phones and load on the API, and most of the numbers do not move that quickly.

Time handling is a recurring source of bugs. The API returns timestamps in a single agreed form, and the dashboard converts for display using the site's own time zone, not the browser's. An installer in one place looking at a site in another would otherwise see the tariff windows shifted. Tariff windows are defined in site-local time, so charts that overlay them must use the same basis.

Large ranges should be downsampled by the API, not in the browser. If a panel receives far more points than pixels, that is an API request bug, and the fix is to ask for coarser resolution.

## Authentication and access

Installers sign in through the organization's identity provider using a browser-based flow suitable for single page apps. The app holds a short-lived token in memory and attaches it to API calls. It should not put tokens in places that survive a page close unless there is a deliberate reason. When the token expires, the app tries a quiet renewal first and only sends the user back to sign in if that fails.

Access is scoped by installer. An installer sees only the sites that belong to their company. This is enforced by the API, not by the dashboard. The dashboard hides what it is not given, but hiding is a courtesy, not security. Never rely on the front end to keep one installer from another installer's data. If a bug report says an installer saw another company's site, it is an API authorization bug first, and the dashboard second only if it cached something across sign-ins.

Switching accounts on a shared device must clear cached data. Anything cached in memory for the previous user is dropped at sign-out. If we add local persistence for offline convenience, it has to be keyed by user and wiped at sign-out too.

Roles inside an installer company are minimal for now: people who can view and people who can also perform the small write actions. If roles grow, put the permission check in the API and have the dashboard read the capabilities it was given, instead of guessing from role names.

## Screens and behavior

The site list is the landing screen. It shows each site with a status chip, the last time it was heard from, and a one-line hint of today's forecast against actual production. Sorting puts sites needing attention first. Search by customer name or address is client-side filtering over what was loaded, which is fine at the current size. If the list becomes too large for that, move search to the API.

The site detail screen has a header with identity and status, then panels. The production panel overlays measured output on the forecast for the chosen range, so an installer can see at a glance whether the forecast is consistently high or low for that roof. The battery panel shows state of charge over time together with the scheduled charge and discharge periods. The tariff panel shows the time-of-use windows and which ones the scheduler targeted. The device panel shows connection state, last message time and the acknowledgment state of the last schedule.

The few write actions are deliberately small: add a note to a site, flag a site for follow-up, and request that a schedule be recomputed. Each is a call to the API and shows its result. The recompute request does not change anything on the device immediately; it queues work for the backend. The panel must say so, and show the schedule as pending until the backend reports a new one. Early versions showed success the instant the request was accepted, and installers assumed the battery was already following the new plan.

Empty, loading and error states are part of every screen. A loading state should keep the layout stable so the page does not jump. An error state should say which panel failed and offer a retry for that panel alone, not reload the whole page.

## Build, deploy and release

The build is a standard Vite production build run in CI. The output is the static folder. The deploy step copies that folder to the blob container used for static hosting. Order matters: upload hashed assets first, then the entry page last. That way, no visitor can load a new entry page that references assets not yet uploaded. Old hashed assets should stay for a while after a deploy, so users with a tab open on the previous version can still lazy-load route chunks without a failure.

Environment configuration is chosen at build time. There is one build per environment, so a build made for staging is never promoted to production as is. This is a cost of the static model. If it becomes annoying, the alternative is loading a small runtime config file at startup, with the rule that it contains only public values and is never cached hard.

Rollback is a re-deploy of the previous build output, kept as a CI artifact. Because the files are static and hashed, rollback is quick, but remember the entry page caching issue: after a rollback, clients that already fetched the newer entry page may keep it until it expires.

Before shipping, check the basics by hand: sign in, open a site with full history, open a brand new site with none, open a site that is offline, change the time range, and try a deep link in a fresh tab. Those cases have caught most regressions.

## Testing and debugging notes

Unit tests cover formatting, time zone conversion and the shared state modules, since that is where the quiet bugs live. Component tests cover each panel's loading, empty and error states with a faked API. A small number of end-to-end runs hit a staging API with seeded sites that represent the awkward cases: no history, a gap, an unknown tariff, an offline device.

When a panel is wrong, work from the outside in. First, look at the API response in the browser network tools and decide whether the data is wrong or only the drawing. If the data is wrong, the problem is behind the API: the stored series, the forecast job, or the device messages. If the data is right and the picture is wrong, it is almost always time zone handling, unit formatting, or a missing-data case that the panel did not handle.

When the whole page is blank after a deploy, suspect the cached entry page first, then a missing asset, then a configuration value that was wrong at build time. When every panel fails together with a cross-origin message, suspect the API's allowed origins. When one panel fails alone, suspect that endpoint or that panel's parsing.

Keep logging in the browser quiet. A small client-side error reporter sends unexpected failures with the site and panel involved, but never tokens and never customer personal details. Installers do not read consoles, so the report is how we learn something broke.

## Known gaps and open questions

Offline behavior is minimal. Installers on poor connections get a failed panel and a retry button. A cached last-known view would help, but it brings the sign-out and per-user cache rules described above, so it has been put off.

Live device status is currently closer to polling than true push. If the backend exposes a push channel suitable for browsers, the status chip and the schedule acknowledgment state are the first things worth moving to it.

Search is client-side and will stop being enough as installer fleets grow. Moving it to the API is the plan when that happens.

Comparing forecast against actual across many sites at once, for an installer who wants to find roofs where the model is weak, is not built. It would need an API endpoint that aggregates, not many calls from the browser. Do not build it by fanning out per-site requests.

The static hosting setup gives us little control over headers. If security headers or finer caching rules become a requirement, plan on a CDN or front door in front of the container instead of working around storage limits.

Last, the language and unit settings are fixed per installer company at the moment. Per-user preferences are a likely request. Keep all formatting going through the shared pieces so that change stays small when it comes.
