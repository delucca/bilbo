---
id: 01JZZJKQJPJ9VND8ZQRQSS4J06
created: 2025-07-12T12:03-03:00
---

# battery-controller milestone recap

This is a recap of the stretch of work that took battery-controller from a prototype that mostly worked on a bench to something an installer could leave running in a customer's house. It is written quickly, from memory of how the work went, and it is meant for whoever picks the component up next. It does not record settings or thresholds. Those live in the configuration and in the code, and they change more often than this note would.

battery-controller sits between two things. On one side is the solar forecast, which says roughly how much the panels will produce over the coming hours. On the other is the tariff schedule, which says when grid power is cheap and when it is dear. The controller turns those inputs, plus the live state of the battery and the house, into a charging and discharging plan, and then sends commands to the inverter. Most of the milestone was about making that chain dependable, not about making the plan cleverer.

## Where it started, and getting data in and out

The first working version was a Julia program that read a forecast, read a tariff table, solved a small optimisation, and printed the schedule. It ran on a developer machine and was fed recorded data. That version was useful for arguing about the shape of the problem, but it had several habits that would not survive a real site.

It assumed every input was present and fresh. It assumed the battery did what it was told. It assumed the clock on the machine was right. It kept its state in memory, so a restart meant it forgot what it had been doing, including whether it was halfway through a forced charge. And it treated the schedule as a single answer, computed once, rather than something that has to be revised as the day goes differently from the forecast. None of these are exotic. Every one of them showed up quickly against real hardware. The milestone is basically the list of those assumptions being replaced with something that fails gracefully.

The controller talks to the site over MQTT. Telemetry from the inverter and meter arrives as messages, and commands go back the same way. Early on we subscribed broadly and parsed whatever came in. That caused trouble of two kinds. Some devices publish at irregular intervals, so the controller would act on a reading that was much older than it looked. Others publish the same quantity under slightly different names depending on firmware, so a field would silently be absent.

The fix was to put a thin layer in front of the planner. Every incoming reading is stamped with when the controller received it, not just the device's own timestamp, and the planner asks for the latest value of a quantity together with its age. If the age is too old, the planner treats the quantity as unknown rather than as its last value. What counts as too old is a per-quantity setting, and I deliberately kept it out of this note.

The naming differences are handled by a small mapping from device-reported names to the names the controller uses internally. When a name is not in the mapping, the controller logs it once and carries on without it. That is much better than the earlier behaviour of either crashing or quietly using zero.

Commands go out through the same layer in reverse. A command is not considered applied until the telemetry shows the effect. The controller sends, waits for the readback, and retries a limited number of times before it marks the device as not responding and falls back to a safe mode. Safe mode is deliberately dull: it stops issuing grid-charging commands, leaves the battery to follow the house load, and keeps trying to talk to the device in the background. We argued about whether safe mode should try to hold the last plan instead. Holding a plan the controller can no longer verify seemed worse than doing nothing clever, so it does nothing clever.

On the cloud side, devices are registered through Azure IoT Hub, and the controller uses the hub for the parts that need to be reachable from outside the house: pushing configuration changes, receiving a small heartbeat, and letting installers see whether a site is alive. The local MQTT broker remains the path for control. This split matters. A site must keep managing its battery if the internet is down, so nothing in the control loop waits on the cloud. The hub connection is allowed to be late or absent, and the controller only reports that fact.

One thing that took longer than expected was device identity. During bring-up, it was easy to end up with two physical sites reporting under the same logical name, because a test device had been cloned from another. The controller now refuses to start its loop if the identity it was given does not match what the local broker reports about the site. It is a blunt check, but it has already caught a copy-paste mistake during installer testing.

## History, state and the time series store

InfluxDB holds the history the controller needs: past production, past consumption, past battery behaviour, and the plans it produced. Originally the controller wrote to it as an afterthought and read from it only for the forecast model. During this milestone it became a real dependency, in a limited sense.

Two things changed. The controller now writes each plan and each decision it took, with the inputs that led to it, as records alongside the telemetry. When a customer asks why the battery charged from the grid last week, there is something to look at. That sounds like a nice-to-have, but support questions of that sort were the biggest source of lost time, and they were hard to answer when we only had the raw telemetry and had to reconstruct the plan by hand.

The controller also recovers its working state from the store at start-up. Before, a restart in the middle of a forced charge would leave the battery charging with nobody in charge of stopping it, until the next planning cycle noticed. Now on start-up the controller reads the most recent decision records, compares them with the live battery state, and either resumes or cancels the action explicitly. The rule I settled on is that it only resumes if everything it needs to check is available and consistent. Otherwise it cancels and replans from scratch. Being conservative here costs a little money on rare occasions and avoids the failure that is hard to explain to a customer.

There is a trap worth writing down. Queries to the store are not free, and early versions of the planner asked for more history than they used, which made planning slow on small hardware. The planner now asks for aggregated windows instead of raw points, and the aggregation is done in the store. If planning feels slow after a change, check first whether a new query has gone back to pulling raw data.

Another trap: the store can be unreachable or slow without the rest of the site being in any trouble. The controller treats a failed write as something to buffer and retry, not as a reason to stop controlling. The buffer is bounded; when it fills, the oldest decision records are dropped and a counter notes that. Losing history is acceptable. Blocking the control loop is not.

## Planning against forecasts and tariffs

The planner itself changed less than the surroundings, but a few things about it are worth keeping.

The forecast is treated as uncertain. Earlier the planner took the central estimate and optimised against it, which produced schedules that looked excellent on paper and were fragile in practice. On a cloudy day that the forecast had missed, the battery would have been emptied into the house in the morning on the assumption of a sunny afternoon that never came. The planner now looks at a pessimistic and an optimistic version of the forecast and prefers actions that are acceptable under both. It is not a full stochastic treatment. It is a cheap hedge, and it behaves sensibly in the cases we tried.

The plan is revisited regularly rather than computed once. Each cycle, the planner takes the actual battery state and the newest forecast, throws away the old future, and builds a new one, committing only to the immediate step. That made the system much more forgiving of forecast error, and it also made the plan history in the store much more informative, because each record shows what the planner believed at that moment.

Tariffs are handled as a schedule with a clear notion of what period the controller is in and what comes next. The awkward parts were not the prices but the boundaries: changes of season, public holidays that use a different pattern, and the hour when clocks change. The controller works in a single consistent time reference internally and converts only at the edges, where it reads the tariff and where it displays things. Before that rule, a handful of off-by-an-hour bugs had appeared around local time changes, and they were hard to reproduce because they only showed up twice a year. The rule is cheap to follow and I would hold the line on it.

Battery wear is accounted for in a simple way. The planner assigns a small cost to cycling the battery, so that it does not chase tiny price differences by charging and discharging constantly. How that cost should be set is an open question and depends on the battery chemistry and warranty terms; installers have opinions. For now it is a configurable parameter with a conservative default, and it is the first thing to look at if a customer complains that the battery seems to do very little.

There is also a set of hard limits that the planner cannot override: the minimum reserve the customer wants kept for outages, the maximum power the inverter and the grid connection can take, and the allowed operating range of the battery itself. These are applied after planning as a clamp on the commands, not only as constraints inside the optimisation. That is redundant on purpose. If the solver ever returns something odd, the clamp keeps it from reaching the hardware.

## The operator view and what went wrong along the way

The installer-facing interface is a Svelte application. During this milestone it stopped being a status page and became something installers could use during commissioning, which was the actual goal.

The views that mattered were these: the current state of a site (battery level, flows between panels, house, battery and grid, and whether each device is responding); the plan the controller currently intends to follow, next to the forecast it was based on; and the recent decisions with the reasons recorded for them. The last is the one installers asked about most, because it turns a mysterious battery into an explainable one.

We kept the interface read-mostly. The few things an installer can change, such as the reserve level and the choice of tariff, go through the hub as configuration, and the controller acknowledges them. The interface shows a change as pending until the controller confirms it. Early on it showed the new value immediately, and installers reasonably assumed it had taken effect even when the site was offline. Showing pending state honestly removed a whole class of confused calls. Installers want to know first whether the site is healthy, so the top of the page is a single plain statement about that, with detail underneath. They do not want to read charts to find that a device has stopped talking; that should be a sentence. And they work on phones, often with poor reception on site, so the application has to cope with partial loading and show what it has.

Some of the more instructive problems, in no particular order.

A feedback loop between the controller and the inverter's own built-in behaviour. Some inverters have their own self-consumption mode, and if it is left enabled it fights the commands from battery-controller. The symptom was a battery that oscillated between charging and discharging in quick succession. The cure was to detect the inverter's mode during commissioning and refuse to proceed until it is set to accept external control. This is now part of the commissioning checklist and the controller checks it too, rather than trusting the checklist.

Readings that were technically valid but physically implausible, for example a battery level that jumped sharply and then came back. These come from glitches in the device or the meter. Treating them as real led the planner to make a wasteful decision. The input layer now applies a basic plausibility filter, comparing each reading with what the previous ones imply, and discards what cannot be right. The filter is intentionally loose, because a strict one throws away real events such as a large appliance switching on.

Slow drift between the forecast model and a particular roof. Shading from a tree, a different panel orientation than the installer recorded, a dirty array: all produce a consistent bias that the generic forecast does not know about. The controller now compares the forecast with what the panels actually produced and keeps a slowly adapting correction per site. It is a modest piece of code and it made forecasts at individual sites noticeably more believable. It also needs care, because if the correction adapts too fast it learns the weather rather than the roof.

Configuration sprawl. Over the course of the work the number of settings grew, and some of them interacted in ways nobody remembered. We pulled settings into one structured place, gave each a description and a safe default, and made the controller report on start-up which values differ from the defaults. That report has been surprisingly useful in support: it is the quickest way to see what is unusual about a given site.

Dependence on a single process. For a while the controller was one long-running Julia process doing input, planning and output together. Planning occasionally took long enough that input handling fell behind, and the readings it then acted on were stale. Splitting the work so that input and output keep running while a plan is being computed fixed this. It also made start-up time a concern, since Julia compiles on first use. Keeping the process alive and warming the planner at launch handled most of it, and the control loop does not wait for the warm-up to finish before it starts watching the inputs.

## Testing, and where this leaves battery-controller

The testing approach changed from checking that the planner gives the right answer on recorded days to checking that the whole controller behaves sensibly when things go wrong. The recorded-day tests still exist and are useful for catching regressions in the planning logic. Alongside them there is now a simulated site that speaks MQTT like a real one, with a battery model, a house load, and knobs to make things misbehave: drop messages, delay them, report nonsense, refuse commands, or restart in the middle of an action.

That simulator found more real bugs in a short time than anything before it. Most were in the seams: what happens when a command is acknowledged but not applied, what happens when the broker restarts, what happens when the store comes back after being away. Each of these now has a scenario that is run before changes are accepted.

We also began running the controller in an observe-only mode at real sites before letting it act. In that mode it reads, plans and records, but does not send commands, and the installer can compare what it would have done with what the site actually did. This was the best way to build trust with installers and it caught a few tariff-configuration mistakes before they cost anyone money. The things we do not test well yet are long-horizon behaviour, such as seasonal changes in the forecast correction, and unusual hardware combinations. Both depend on real sites running for a long time, and there is no shortcut.

The component now has a defensible shape. Inputs are checked and aged. Commands are verified. The plan is rebuilt regularly, hedged against forecast error, and clamped by hard limits. State survives a restart. Decisions are recorded with reasons and shown to installers. The cloud is optional for control and used for configuration and visibility.

What I would not claim is that the economics are tuned. The planner makes reasonable choices, but the cycling cost, the reserve policy and the forecast hedge have been set cautiously, and there is room to be less cautious once more data from real sites is in. That tuning should be driven by the recorded decisions and outcomes, not by intuition, and it should be done per kind of site rather than globally.

Things still open, roughly in the order I would pick them up:

- A cleaner story for multiple batteries or multiple inverters at one site. Right now the controller assumes a single battery and inverter and the assumption leaks into several places.
- Better handling of customers who have dynamic tariffs rather than fixed time-of-use periods. The planner's structure can take it, but the tariff input layer assumes a schedule known in advance.
- A graceful way to roll out controller changes to sites in stages, with an automatic fall back to the previous behaviour if a site starts reporting problems.
- More explicit treatment of outage preparation, for example raising the reserve when bad weather is forecast. At the moment the reserve is a static customer preference.
- Reducing the amount of site-specific knowledge that lives in installers' heads. Much of the commissioning checklist could become checks the controller runs itself and reports on.

If you are about to change something here, the places most likely to bite are the input aging logic, the clamp on outgoing commands, and the start-up recovery. Those carry most of the safety of the component. The planner can be wrong and cost a little money; those can be wrong and do something the customer notices. Read the decision records from a real site before and after any change to them, and run the simulator scenarios that break things on purpose before trusting a green result from the ordinary tests.
