---
id: 01M2Q7HHAHDP2V1JQ9FYYJ5Z5X
created: 2026-09-17T05:25-03:00
---

# producer-console hydration failure on live poll widget

producer-console throws `Hydration failed because the initial UI does not match` when the live poll widget renders Date.now() on the server. The server HTML carries one timestamp, the browser computes another during hydration, and React gives up on reusing the markup. It looks like a random Next.js glitch, but it is deterministic and comes from the widget reading the clock during render.

## Symptom

Open producer-console while a poll is live. The page flashes, the console logs `Hydration failed because the initial UI does not match`, and React falls back to client rendering for the affected tree. Sometimes the poll widget looks fine afterwards, so it is easy to dismiss. Do not dismiss it. The fallback throws away the server-rendered markup, slows first paint for producers, and can leave the widget showing a stale or flickering time value.

It shows up most for producers who open the console late in an event, when the page has been server-rendered a moment before the browser hydrates it. The gap between the two clocks is what differs.

## Cause

The live poll widget calls Date.now() while rendering. In Next.js the same component runs twice: once on the server to produce HTML, and once in the browser to hydrate. Date.now() returns a different value each time, so the text or attribute built from it differs between the two passes. React compares the two trees, sees the mismatch, and raises the hydration error.

The widget uses the value for things like "closes in N seconds" or "last updated" labels. Any such label built from the current time during render has the same problem.

## Why it is easy to miss

- In local dev the gap between server render and hydration is small, and sometimes the rendered text happens to match, so the error comes and goes.
- A poll with no countdown or timestamp visible may not trigger it.
- The error text is generic. It does not name the widget or the Date.now() call, so you have to find the clock read yourself.
- Moderation and Q&A panels in the same console do not show it, which sends people looking at the wrong component.

## Fix

Keep the clock out of the server render. Pick one of these:

1. Render a stable placeholder on the server and first client pass, then set the real time inside an effect after mount.
2. Pass the timestamp down from the server as a prop, so both passes use the same value. The server value is then used for the first paint and the client takes over for ticking.
3. Load the widget with dynamic import and server rendering turned off, if the whole widget is time-driven and has no useful server HTML.

I prefer the first option for small labels and the second when the poll's close time comes from the Phoenix backend anyway. The third is a last resort because producers see an empty slot until the bundle loads.

```tsx
const [now, setNow] = useState<number | null>(null);

useEffect(() => {
  setNow(Date.now());
  const t = setInterval(() => setNow(Date.now()), 1000);
  return () => clearInterval(t);
}, []);

// first render: now is null on server and client, so markup matches
```

## What not to do

- Do not add suppressHydrationWarning across the widget to hide the message. It silences the warning on one element only and leaves other mismatches in place.
- Do not compute the time in the Phoenix channel message and mix it with a client clock. Clock skew between the server and the producer's machine produces odd countdowns.
- Do not round Date.now() to the second and hope the passes agree. They will not always.

## Checking the fix

Load producer-console with a live poll, open the browser console, and do a hard reload several times. The hydration message should not appear. Also test with the network throttled, since a slow hydration widens the gap between the two renders and exposes any remaining clock read. Check the poll widget once with no active poll and once with one running, because the two states render different branches.

## Related notes for later

When adding any new widget to producer-console, grep for clock reads, random ids, and locale-dependent formatting in render code. All three cause the same class of hydration error. Locale and timezone formatting are the next most likely culprits, since the server and the producer's browser can differ on both.
