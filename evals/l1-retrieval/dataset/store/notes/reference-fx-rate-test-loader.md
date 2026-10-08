---
id: 01M2X2G16ZMJ38XMHP3TCYNQ3F
created: 2026-09-19T11:52-03:00
---

# fx-rate-loader reference

To test fx-rate-loader without writing rows, run `fx-rate-loader --dry-run --date 2026-03-01`. That is the safe way to check what the loader would do for a given day: it goes through the normal fetch and validation path for the date you pass, reports what it would have stored, and leaves the database tables untouched. Use it before any backfill, after any change to the loader, and whenever someone asks "would this day load cleanly?". The same command, with the same date, is the one to paste into a ticket when you want someone else to reproduce what you saw.

This note is a quick reference for fx-rate-loader in Ledgerlark. It is written for someone who has to run it, debug it, or change it and does not want to reread the whole codebase first. It does not try to be a design document. Where a detail is likely to drift (exact schedules, provider endpoints, table column lists), the note says so and points you to the place where the current value lives instead of copying a value that will go stale.

```
fx-rate-loader --dry-run --date 2026-03-01
```

## What fx-rate-loader does and why it exists

Ledgerlark reconciles card-processor settlement files against internal ledger entries. Marketplaces that use it sell in many currencies, but a processor often settles in a different currency from the one the buyer paid in, and the internal ledger may book in yet another one. To compare a settlement line with a ledger entry, finance operations need a rate that converts one amount into the other, and they need it to be the same rate every time the same comparison is rerun. fx-rate-loader is the component that brings those rates into PostgreSQL so the matching logic can read them locally instead of calling an outside service during reconciliation.

The loader is a small Go program. It fetches published daily rates from the configured rate source, checks them, and writes one set of rows per business date. It does not compute anything about settlements itself. It knows nothing about which marketplace owns which ledger entry. Its only contract with the rest of the system is the rates table in PostgreSQL and, where downstream consumers want to know that new rates are available, a message on a Kafka topic. Keep it that narrow. Every time someone has proposed adding "just a little" matching logic to the loader, the result has been rates that depend on ledger state, which makes reruns unpredictable.

Reconciliation reads rates by date and currency pair. If a rate for a date is missing, the matcher does not guess or fall back to a neighbouring day on its own. It flags the affected lines for review with a reason that says the rate was unavailable. That means a gap in loaded rates shows up to finance operations as a pile of review items, which is a noisy way to find out the loader did not run. Check the loader first when you see a sudden cluster of "rate unavailable" flags.

### Where it sits in the system

- Input: a published daily rate table from the rate source. The source, credentials and base URL come from configuration, not from the code. Do not hard-code any of them.
- Output: rows in PostgreSQL, keyed by business date and currency pair, plus a notification that the date is now loaded.
- Consumers: the reconciliation matcher reads the rows directly. Other services that care about freshness listen for the notification on Kafka rather than polling the table.
- Deployment: the loader is provisioned through Terraform along with its scheduler, its database role and its secrets. Changes to when or where it runs go through the Terraform configuration, not through manual edits on a host.

The loader is stateless between runs. Everything it needs to decide whether a date is already loaded comes from the database. It can be killed at any point and started again without a cleanup step, because writes for a date happen inside one transaction and either all land or none do.

## Running it

The binary takes flags, and a few of them matter far more than the rest.

The `--date` flag selects the business date to load. When you leave it out, the loader works on the current business date as it understands it from configuration. For anything other than the normal scheduled run, pass the date explicitly so there is no ambiguity about which day you meant. Dates are written year, month, day, with dashes, exactly as in the example at the top of this note.

The `--dry-run` flag turns off all writes. With it, the loader still contacts the rate source, still parses the response, still applies every validation rule, and still compares the result with what is already stored for that date. What it skips is the database write and the Kafka notification. This is what makes it useful: if a dry run fails, a real run would have failed the same way, and you learned that without changing any data. If a dry run succeeds but reports that it would replace existing rows, you now know that a real run is not a no-op, and you can decide whether that is what you want.

Run the test command from above for the date you care about, read the output, and only then decide on a real run. A typical sequence when something looks wrong with one day of rates is:

1. Run the dry run for that date and read what it says about the fetch, the validation and the comparison with stored data.
2. If the fetch fails, the problem is on the source side or in credentials or network access. Fix that before anything else.
3. If validation fails, look at which rule fired. Do not loosen the rule to make a run pass. Rates that fail validation are usually wrong at the source for that day, and the correct response is to wait for a corrected publication or to escalate to whoever owns the source relationship.
4. If the dry run is clean, run the real command for the same date, without `--dry-run`, and confirm afterwards with a query on the rates table.

A dry run is cheap, but it is not free from the source's point of view. It makes a real request to the rate source. If you are looping over many dates, space the requests out and respect whatever limits the source imposes. Do not script a dry run for every historical day in a tight loop.

### Output you should expect

A dry run prints a summary of what it fetched and what it would do. It says how many currency pairs came back, whether any expected pairs were missing, whether the date is already present in the database, and whether the values it fetched differ from the stored ones. Read the part about differences carefully. A rate source can revise a published table, and the stored rows may be from before the revision. The loader reports this rather than silently overwriting. Whether a real run actually replaces existing rows is governed by configuration and by how the run was invoked, so read the dry-run output and the current configuration before assuming either way.

Exit status is nonzero when the run could not complete its checks. A dry run that finds differences but completes still exits cleanly. Do not use the exit status alone to decide whether stored data is correct; use it to decide whether the checks ran.

### Logs

The loader logs in a structured form. Each line carries the business date it is working on and the stage it is in, such as fetch, parse, validate, compare or write. When asking someone for help, give them the date and the stage where it stopped. That is nearly always enough to tell which of the above categories of problem you are in. Logs from a dry run say clearly that writes were skipped, so you cannot mistake them for a real load when reading them later.

## How rates are validated and stored

Validation is deliberately strict, because a bad rate does silent damage: a settlement line gets compared against a wrongly converted ledger amount and either a real mismatch is hidden or a false one is raised. Finance operations lose time either way, and a false "all matched" is worse than a false alarm.

What the loader checks, in general terms:

- The response parses completely. A partially readable table is treated as a failed fetch, not as a smaller table.
- Every currency pair the deployment is configured to expect is present. A missing pair fails the run for that date. The loader does not fill gaps from earlier days.
- Each rate is a positive, finite number. Zero, negative, missing and not-a-number values are rejected outright.
- The table is for the date that was asked for. If the source returns a table labelled with a different date, the run fails instead of storing it under the requested date.
- Rates are not wildly out of line with the previous stored day for the same pair. This is a sanity band, not a market model. Its width is configuration. When it trips, the right move is almost always to look at the source, not to widen the band.
- Duplicate pairs within one response are rejected, since there is no safe way to choose between them.

If any rule fails, nothing is written for that date. That all-or-nothing behaviour is intentional. A date with half its pairs loaded is much harder to reason about than a date with none.

Storage uses one transaction per date. Inside it the loader writes all pairs for the date, then commits. Rows are keyed so that rerunning the same date cannot create duplicates. When a rerun finds stored rows that match what was fetched, it changes nothing. When it finds rows that differ, behaviour depends on configuration, and the dry run tells you in advance which case you are in.

Rates are stored with enough precision that conversion in the matcher does not lose meaningful digits. Do not round at load time to make display nicer. Presentation rounding belongs in the review interface. If you are tempted to reduce precision in the loader to fix a mismatch someone complained about, stop and look at how the matcher converts amounts instead.

### Notification after a load

After a successful real run, the loader publishes a message to Kafka saying that a business date now has rates. The message carries the date and little else. Consumers should treat it as a hint to go and read the table, not as the data itself. If the publish fails after the database commit, the loader reports the failure and exits nonzero, but the rates are already stored. Rerunning the same date is safe: it will find the rows already present, change nothing, and attempt the notification again. A dry run never publishes.

Because the notification is sent after commit and not inside the transaction, there is a short window where rates exist and no message has gone out. Consumers that must not miss a date should reconcile against the table on a schedule as well as reacting to messages. That is a consumer concern and not something to fix inside the loader.

## Operating notes, gotchas and who to ask

### Scheduling and gaps

The loader is meant to run once per business day, after the source has published. Running it too early gives you either a fetch failure or, worse, a table for the previous day. The date check in validation catches the second case, but it is still wasted effort. If scheduled runs start failing in a consistent way at the same stage every day, suspect a change in source publication time before suspecting the code.

Weekends and holidays are a source-side concept. The source may publish nothing, or republish the last table, on non-business days. How the deployment is configured to treat those dates determines whether the loader skips them or expects a table. When the matcher needs a rate for a date the loader never loaded, it flags lines for review. If that happens for a non-business day, check configuration for how such days are meant to be handled before assuming the loader is broken.

Backfills are done by running the loader once per missing date with `--date`. Do a dry run for each date first. Do not backfill from a script that swallows errors; a date that fails validation should stop you and make you look.

### Gotchas

- A dry run proves the fetch and validation work for that date at that moment. It does not prove the scheduled job is healthy. The scheduler, the role and the secrets are separate things, all managed in Terraform.
- A clean dry run can still be followed by a failing real run if the database is unreachable or the role lacks permission, because the dry run skips the write path. If a real run fails right after a clean dry run, look at database connectivity and permissions first.
- Forgetting `--date` on a manual run loads the current business date, which may not be the day you were investigating. Always pass it.
- Do not edit stored rates by hand to fix a one-off. The next rerun or comparison will disagree with your edit, and nobody will know why. Fix the source problem and rerun, or raise it with the owners of the source relationship.
- Do not point a development copy of the loader at the production database to "just see" what happens. Use the dry run against the real source for that, and a non-production database for anything that writes.
- Time zones are a classic trap. The business date is a configured notion and may not match the calendar date on the machine where you happen to run the command. If a date looks off by one, read the configuration before changing code.
- The matcher caches nothing about rates between reconciliation runs, but any downstream report that did cache them will not notice a corrected rerun. After correcting a date, tell whoever owns reports for that period.

### Changing the loader

When you change validation, add a test that feeds the bad input and asserts that nothing is written. Most past trouble has come from a rule that was too lenient, not too strict. When you change storage, think about reruns first: a change that is correct for a first load but wrong for a rerun will show up as duplicated or silently replaced data. When you change the notification, remember consumers on Kafka may be on older versions of the message shape, so additive changes only.

Keep the dependency list short. The loader is a single-purpose Go program and should stay easy to build and read. If a feature needs gRPC calls into other Ledgerlark services, that is a sign it is taking on matcher or ledger concerns it should not have; talk to the team before building it.

### Checking after a real run

After a real load, confirm three things: the rates table has rows for the date and for every expected pair; the loader's final log line for that date reports success at the write stage; and the Kafka notification went out, which you can see in the loader's log or by checking the topic. If any of the three is missing, do not declare the date done. Rerun it, since reruns are safe, and read the stage where it stops.

### Who to ask

Source behaviour, publication times and credentials belong to whoever manages the rate source relationship for finance operations. Infrastructure, scheduling and secrets belong to the team that owns the Terraform configuration. Matching and review behaviour belongs to the reconciliation team. When filing an issue, include the date, the stage that failed, whether the failing run was a dry run, and the exact command you ran, so the person reading it can reproduce it with `fx-rate-loader --dry-run --date 2026-03-01` or the same command with the date you were using.
