---
id: 01K2K7NQSVWY2Q7295B65PHCFB
created: 2025-08-13T23:48-03:00
---

# tariff-scheduler: general direction chosen

We settled on a general direction for tariff-scheduler and I want it written down before it blurs. This note is about the shape of the approach, not about tuned values. Anything that needs a number lives in config or in the tariff data, and it should be looked up there, not copied from here. The sibling note [[battery-controller-watch-outs]] covers the device side. This one covers the planning side.

The short version: tariff-scheduler stays a planner that produces a charging plan from a forecast and a tariff, and it does not drive hardware directly. It prefers plans that are simple to explain to an installer over plans that squeeze out the last bit of savings. It is conservative when the inputs are weak. It is boring on purpose.

## Why we wrote this down

The scheduler has been argued about in the same few threads over and over. Each time somebody joins, they ask why it does not just solve one big optimisation and be done. The answer is spread across chat and heads. This note is the single place where the direction is stated, with the reasons, so a later session does not reopen it by accident.

It is a general note. If you need a threshold, a horizon length, a price band or a margin, do not look here. Look at the configuration the installer controls, or at the tariff record for the site.

## What tariff-scheduler is for

tariff-scheduler takes a solar output forecast, a picture of household demand, the state of the battery, and the tariff structure, and it decides when the battery should charge, hold, or be allowed to discharge. Its output is a plan that other parts of GridHaven consume. The plan is advice with a clear shape, not a stream of low-level commands.

Installers use it to set up a site once and then trust it. Customers see the result as a lower bill and a battery that behaves sensibly. Neither group wants to understand a solver. That shapes most of the choices below.

## The core direction

We chose a rule-led planner with a small amount of search on top, instead of a full optimisation over the whole horizon. The rules encode the tariff structure: cheap periods are for filling, expensive periods are for serving the house, shoulder periods are for waiting. The search only picks among a few sensible variants of that skeleton, based on the forecast.

The reason is that tariffs are structured and mostly predictable, so the skeleton is almost always right. The forecast is the uncertain part, and spending solver effort to be precise about an uncertain input is wasted. A plan that is roughly right and easy to read beats one that is exactly optimal against a forecast that will be wrong anyway.

## Why not a full optimiser

We looked at a proper mathematical optimiser seriously. It would find better plans on paper. We decided against it as the main path for several reasons.

First, the gain depends on forecast accuracy, and forecast error eats a large share of the theoretical benefit. Second, solver behaviour is hard to explain when a customer asks why the battery did not charge last night. Third, small input changes can flip the whole plan, which makes the system look jumpy. Fourth, it adds a heavier dependency and harder failure modes to something that has to run reliably on a schedule.

An optimiser can still be useful offline, as a yardstick to compare how far the rule-led plans are from the best case. We keep that door open for evaluation, not for production.

## Planning horizon and cadence

The plan covers the span of the current tariff cycle that matters for the next decisions, plus enough lookahead to catch the next cheap period. We deliberately do not plan far into the future, because the forecast degrades and the plan would be fiction.

The scheduler re-plans on a regular cadence and also when something meaningful changes, such as a clear forecast revision or a battery state that drifted from expectation. Re-planning is cheap by design. We would rather re-plan often with a short horizon than commit to a long plan and defend it.

## Handling forecast uncertainty

The direction here is to plan against a cautious reading of the forecast, not the central estimate. When the forecast is confident, the plan can lean on solar to fill the battery later in the day. When it is not, the plan fills more from the grid in the cheap period so the evening is covered.

We treat the forecast confidence as an input in its own right. A wide spread means more reserve and less reliance on a sunny afternoon. This is a policy stance, and the concrete margins are configuration. The stance is that running short in an expensive period is worse than having paid for a little grid energy we did not strictly need.

## Battery reserve and customer comfort

The plan always keeps some reserve for the household, and it never plans the battery to its absolute floor in the normal case. The reserve exists for outages, for forecast misses, and for the simple fact that people get annoyed when the battery is empty at the wrong time.

We agreed that comfort wins over savings when they conflict. An installer can tune the reserve per site, but the default stance is protective. Wear on the battery is also part of this: the planner avoids needlessly deep cycling when the saving is marginal.

## Tariff modelling

Tariffs are modelled as named periods with a price level attached, plus rules for which days and seasons they apply to. The scheduler reads this model and does not hardcode any provider. Adding a new tariff means describing it in the model, not changing the planner.

We also decided that the tariff model should be able to say "I am not sure" about a period, for example when a provider changes its structure and the data lags. In that case the scheduler falls back to a flatter, more cautious plan instead of trusting a stale schedule.

## Export and feed-in

When a site can export, the scheduler treats export as another price signal and not as a special mode. If exporting is poorly paid, the plan prefers self-use and storage. If it is well paid in some period, the plan may hold energy for that period, but only when the forecast and reserve allow it.

We kept this simple on purpose. Export rules vary a lot by region and change often, so the direction is to read them from the tariff model and avoid baking regional assumptions into the planner.

## Data flow and boundaries

tariff-scheduler reads forecasts and historical measurements from the time series store and receives live state from the device messaging layer. It writes its plan back so that other components, including the battery controller and the user interface, read the same thing. The cloud device hub is the path to remote sites.

The boundary we hold firm on is that the scheduler does not talk to hardware. It publishes a plan and the controller decides how to carry it out safely. That keeps safety logic in one place and lets us change planning without touching device behaviour. The device-side traps are in the related note named near the top.

## Implementation language and structure

The planner is written in Julia, which suits the numeric parts and lets the forecast and scheduling code share types. The structure we want is a pure core that takes inputs and returns a plan, wrapped by a thin layer that handles reading data and publishing results.

Keeping the core pure means we can test it with recorded inputs and compare plans across versions of the rules. It also means a bad day in production can be replayed offline. We consider this more important than any speed gain from tighter coupling to the data layer.

## Explainability

Every plan carries a short reason for each decision, in terms an installer would use: filling because the next period is expensive, holding because the forecast is weak, serving the house because the price is high. These reasons are generated by the rules themselves, not written afterwards.

This is why the rule-led approach won. If a customer or installer asks why the battery behaved a certain way, the answer is already in the plan. The interface in Svelte shows these reasons next to the schedule, and we want support questions to be answerable from that screen alone.

## Failure and fallback behaviour

If inputs are missing, late, or obviously wrong, the scheduler falls back to a safe default plan built from the tariff structure alone, without the forecast. That plan is plain: fill in the cheap period, serve the house otherwise. It is never clever, and it is always available.

If the scheduler itself fails to run, the last good plan stays in effect until it expires, after which the controller should revert to its own conservative behaviour. We prefer a stale but sane plan to no plan, and a plain plan to a wrong one. Silent failure is the worst case, so a missed run must be visible in monitoring.

## Testing approach

We test the core against a library of recorded situations: sunny and gloomy days, tariff changes, a battery that is already full, a battery that is nearly empty, a forecast that was badly wrong. Each case checks that the plan is sensible and explainable, not that it matches an exact expected schedule.

We avoid brittle tests that pin exact outputs, because small rule improvements would break them all. Instead we check properties: reserve is respected, no charging in the most expensive period unless forced, no plan that contradicts the tariff. Property-style checks age better than snapshots here.

## What we are not deciding here

This note does not fix any tuned parameter, any forecast model choice, or any tariff-specific rule. It does not decide how the controller carries out the plan. It does not cover billing or reporting. Those have their own places, and mixing them in would make this note go stale.

It also does not rule out smarter planning in the future. It records what we chose for now and why, so that a change is a deliberate decision and not drift.

## Open questions

Some things remain loose. How much to trust the forecast confidence signal across very different sites is not settled. How aggressively to use the offline optimiser as a benchmark, and whether to feed its findings back into the rules, is open. Handling households with unusual demand patterns, such as shift workers or home offices, needs more thought.

We also have not decided how much control installers should get over the planner's cautiousness before it becomes a support burden. The lean so far is to expose a small number of clear settings and keep the rest internal.

## When to revisit

Revisit this direction if forecast quality improves enough that the optimiser's theoretical gain becomes real, or if tariffs become so dynamic that fixed skeletons stop fitting. Also revisit if explainability stops being something installers value, though we do not expect that.

Until one of those happens, the working rule is: keep tariff-scheduler rule-led, cautious, explainable, and separate from the hardware. If a change makes the plan harder to explain, it needs a good reason, and that reason should be written here.
