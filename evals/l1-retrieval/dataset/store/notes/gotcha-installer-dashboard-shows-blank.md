---
id: 01KV3YW9BT4B06RWGTR3CXCNHR
created: 2026-06-14T17:58-03:00
---

# installer-dashboard blank chart on sites without a forecast

installer-dashboard shows a blank chart and the console error `Uncaught TypeError: Cannot read properties of undefined (reading 'forecast')` for a site that has no forecast yet. That is the whole trap: a new site, or one the Julia forecaster has not produced anything for, opens to an empty chart panel and nothing on the page says why. The only hint is the error in the browser console.

## Symptom

Open a site in installer-dashboard that has no forecast yet. The chart area stays blank. No spinner, no "no data" text. The browser console shows:

```
Uncaught TypeError: Cannot read properties of undefined (reading 'forecast')
```

Sites that already have a forecast render fine, so it looks like a per-site problem and is easy to mistake for a broken deploy or a bad browser.

## Cause

The Svelte chart component reads `.forecast` off an object that is undefined when the site has no forecast record. The lookup returns nothing, the component dereferences it anyway, and the render throws. Because the throw happens during render, the chart never draws and the rest of the panel is left empty.

This is a missing-data case being treated as if it could not happen. It is not an InfluxDB outage and not an MQTT or Azure IoT Hub connectivity problem; telemetry for the site can be arriving normally while the forecast is still absent.

## How to confirm

- Check that the affected site is new or has never had a forecast run.
- Open the browser console and look for the exact error above.
- Compare with a site that does have a forecast: same page, chart draws, no error.

If the site has a forecast and still shows the error, this note does not explain it; look at what the forecast lookup actually returned.

## Fix direction

Guard the access in the chart component. If the forecast object is undefined, render an explicit empty state such as "No forecast yet for this site" instead of reading `.forecast`. Optional chaining on the read plus an `{#if}` block around the chart is enough. Do not paper over it with a try/catch that hides the error; the installer needs to see that the site has no forecast.

## Related

See [[installer-dashboard-polls-forecast-revised-recap]] for how the dashboard polls for forecast updates, which matters if you change when the empty state should switch to a real chart once the first forecast arrives.

## Notes for later sessions

When testing a fix, use a site with no forecast, not only sites with data. The happy path never triggers this. A regression test that mounts the chart with an undefined forecast object would have caught it.
