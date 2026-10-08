---
id: 01M1NTKNHWG7A799R0TQ9ZPP5Q
created: 2026-09-04T06:04-03:00
---

# leg-cost-matrix build fails on leg rows without a yard code

The leg-cost-matrix build fails with `KeyError: 'yard_code'` when a carrier file contains a leg row without a yard code. The whole build stops at that point. It does not skip the bad row and it does not build a partial matrix. If you see this error, look at the carrier file that was being loaded first, not at the OR-Tools side.

This note is about leg-cost-matrix only. It records what triggers the failure, how to recognise it, what to do right now, and what a proper fix should look like. Nothing here has been fixed yet. Treat it as a trap that is still open.

## What happens

leg-cost-matrix reads carrier files and turns each leg row into a cost entry between two points of a route. The yard code is used as a key when the rows are read. A row that has no yard code has no such key, so the lookup raises `KeyError: 'yard_code'` and the build aborts.

The error comes from a plain dictionary lookup, not from a validation step. That is why the message is so short. It names the missing key but does not say which carrier file, which row or which leg caused it. You have to find that yourself.

Things that make it more confusing:

- The failure shows up when the matrix is built, not when the carrier file is uploaded or received. A bad file can sit around for a while and only break the next rebuild.
- A delay event that triggers a rebalance can be the thing that forces a rebuild. The dispatcher sees a rebalance that never finishes, and the real cause is a carrier file that was fine to receive but is bad for the build.
- Because the build is all or nothing, one bad row in one carrier file blocks cost data for every other carrier too. The planner then has no fresh matrix to work from.
- The row may look complete to a human. A blank cell, a column that is present but empty, and a column that is missing entirely can all end up as a missing yard code, depending on how the file was parsed. Do not assume that a filled-looking row is fine.

## How to recognise it

The signature is the exact text `KeyError: 'yard_code'` in the log of the process that builds the matrix. In a FastAPI request path it will usually appear as a server error on whatever endpoint asked for the build or a refresh. In a background path it appears in the worker log, and the matrix just stops updating.

Symptoms a dispatcher might report instead of the error text:

- Routes are not being replanned after a delay.
- Costs look stale compared with what a carrier just sent.
- A new carrier works nowhere, while existing carriers keep their old numbers.

If the matrix is cached in Redis, the old cached matrix may keep being served after a failed build. That hides the problem for a while and makes the symptom look like stale data rather than a crash. When you see stale costs, check the build log for this error before you start looking at the cache.

## What to do right now

Find the carrier file that caused it and fix or remove the rows without a yard code, then rebuild. In practice:

- Check the log around the failure for the carrier file the build was reading when it died. If the log does not say, bisect: build with one carrier file at a time until the failing one is found.
- Open that file and look for leg rows where the yard code is empty or absent. Compare against rows from the same carrier that work.
- Ask the carrier, or the person who owns the integration with them, for a corrected file. Do not invent a yard code to make the build pass. A guessed code gives a wrong cost on a wrong leg, and that is worse than a failed build because nobody notices it.
- If you must unblock dispatch quickly, take the bad rows out of the file in a copy, build from the copy, and keep the original untouched so the evidence remains. Write down which rows you removed and tell the dispatchers that those legs are missing from planning.

Do not catch the `KeyError` and carry on silently as a quick patch. That turns a loud failure into legs that quietly vanish from the matrix, and the planner will then route around them or call them impossible without anyone knowing why.

## Proper fix, not done yet

The build should validate each leg row before it uses it. The shape of a good fix:

- Check every row for a yard code at load time, before any matrix work starts.
- Collect all offending rows in one pass instead of stopping at the first one, so a carrier can be sent the full list in one message.
- Report the carrier file, the row and the leg in the error, so the message says more than a bare key name.
- Decide, as a product choice and not only a code choice, whether a file with bad rows is rejected as a whole or whether the good rows are used and the bad ones are flagged. Rejecting the whole carrier file is the safer default. Using the good rows is friendlier but means the matrix is incomplete, and dispatchers need to be told.
- Either way, one bad carrier file should not stop the other carriers from being built. Keep the last good data for the failing carrier if there is any, mark it as stale, and say so in whatever the dispatcher sees.

Also worth doing: reject such files where they come in, so the carrier gets feedback at once and not at the next rebuild. If carrier files arrive through Google Cloud Pub/Sub, that check belongs in the consumer, with the bad file set aside and an alert raised, rather than the message being acknowledged and the problem found later.

## Tests to add with the fix

- A carrier file with one leg row lacking a yard code, expecting a clear validation error that names the file and the row and not a raw `KeyError: 'yard_code'`.
- A file where the column exists but the value is blank, as a separate case from the column being absent.
- A build with several carriers where only one is bad, expecting the others to still produce a matrix.
- A check that a failed build does not replace a good cached matrix with partial data, and that the cache is marked stale instead.

## Open questions

Nobody has yet confirmed whether any real carrier regularly sends rows without a yard code, or whether it only happens with new or hand-edited files. Find that out from the carrier file history before deciding how forgiving the loader should be. Also still open is who tells the carrier when their file is rejected. Until that is settled, the first person to hit this error is probably the one who has to chase the carrier.
