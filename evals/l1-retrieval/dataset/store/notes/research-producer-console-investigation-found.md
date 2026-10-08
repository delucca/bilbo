---
id: 01KJ5G5GQM894RBZ5Z1A0DVP76
created: 2026-02-23T11:58-03:00
sources:
  - "doc: console bundle analysis"
---

# producer-console bundle size investigation

The producer-console client bundle was 1.8 MB when we looked at it. The charting library was the largest single part of that bundle, and lazy loading it removed most of that weight. This note records what the investigation found, how we read it, and what to watch for next time. It is written for whoever touches producer-console after us, human or agent.

## Headline finding

The producer-console client bundle measured 1.8 MB. The charting library was the biggest contributor, bigger than the framework code, the WebSocket client code and the moderation widgets. Loading the charting library lazily, only when a chart is about to render, removed most of the weight from the initial download. That is the whole story in one paragraph; the sections below are the detail.

## Why we looked

Producers complained that the console felt slow to open, mostly on venue networks and on laptops that are busy running the stream software. The console is the first thing a producer opens before an event, and a slow first load is felt at the worst time. Nobody had measured the bundle since the console was first built, so we measured it.

## What the console is

producer-console is the Next.js front end that event producers use to run live polls, watch the Q&A queue and moderate in real time. It talks to the Phoenix backend over WebSockets and gets pushed updates. Most screens are dashboards: counts, trends, vote distributions. That is why a charting library is in there at all, and why it grew to be the largest piece.

## How the size was measured

We built the production bundle and used the bundle analyzer that ships with the Next.js tooling to produce a treemap of the client output. The total came to 1.8 MB for the client side. We read the treemap by package, not by file, so that one library spread over many modules showed up as one block. The charting library was clearly the largest block on the map.

## Reading the treemap

The treemap showed the charting library as a big block, with its own dependencies attached to it. The next blocks down were the framework runtime, the UI component code and the real-time client. Application code for producer-console itself was small by comparison. The takeaway is that the weight was in a third-party dependency and not in our own screens.

## Where the charting weight came from

The library was imported at the top of shared dashboard modules, so it landed in the main client chunk. Every page paid for it, including pages that draw no chart, such as the login step and the moderation queue. The library also pulls in helper packages for scales, shapes and animation. We were shipping all chart types to render only a few.

## What we changed

We made the charting library load lazily. The chart components are now wrapped so the library is fetched only when a chart is about to render on screen. Pages without charts no longer download it. This removed most of the weight of the 1.8 MB bundle from the initial load, which was the goal. We did not replace the library or rewrite the charts.

## Effect on first load

After the change the initial download is much smaller, since the charting code moved into a separate chunk fetched on demand. The console opens faster, most noticeably on slow connections. Pages with charts fetch the chart chunk right after they mount, so those pages show a short placeholder before the chart appears. We judged that a fair trade.

## Placeholder behavior

A chart that has not loaded yet shows a plain placeholder with the same footprint as the finished chart. Keeping the footprint stops the layout from jumping when the chunk arrives. Producers watching a live poll should not see the page shift under their cursor, since a mis-click during a live event can approve the wrong question.

## Real-time updates and lazy charts

Updates keep arriving over the WebSocket while the chart chunk is still loading. The data layer holds the latest state independent of the chart, so when the chart mounts it draws the current values and not stale ones. We checked that no updates were lost during the window between page mount and chart mount. The chart is a view of state, never the owner of it.

## What we did not change

The backend, the Phoenix channels and the CockroachDB queries were not touched. The bundle problem was purely a client concern. We also did not alter the moderation widgets or the poll controls, which stay in the main chunk because producers need them immediately and they are small.

## Things we ruled out

We considered whether the real-time client or the UI component set was the main problem. Neither was. We also considered whether duplicate copies of a package were bundled twice, and the treemap showed no such duplication worth acting on. We considered dropping the charting library for hand-drawn graphics, and decided it was too much work for a gain that lazy loading already delivered.

## Risks

Lazy loading adds a network request at the moment a chart is needed. If that request fails, the chart area stays on its placeholder. We should make sure the failure state says something useful and offers a retry, since a producer mid-event will not know why a chart is empty. Check this whenever chart code changes.

## How to re-measure

Build the production client, run the analyzer, and compare the total with the 1.8 MB figure recorded here as the starting point. Look first at the largest block by package. If the charting library is back in the main chunk, someone has probably imported it at the top of a shared module again. Imports of that library should only live inside the lazily loaded chart components.

## Guardrails to consider

A size budget in the build would catch regressions early. Failing or warning the build when the main client chunk grows past an agreed limit is cheap to add. We have not set one yet; the limit should be chosen from a fresh measurement after the lazy loading change, not from the old 1.8 MB figure.

## Open questions

We have not measured real-world load times on actual venue networks, only the bundle composition. We have not checked whether other heavy dependencies deserve the same treatment now that the charting library is out of the way. The next largest blocks on the treemap are the first candidates.

## Takeaway

The producer-console client bundle was 1.8 MB, the charting library was its largest part, and lazy loading that library removed most of the weight. Keep chart imports inside the lazy components, re-run the analyzer after dependency changes, and treat any return of the charting library to the main chunk as a regression.
