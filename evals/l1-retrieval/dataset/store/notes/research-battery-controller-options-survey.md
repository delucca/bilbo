---
id: 01K56P2H3BQPS18PA7J47QQRV7
created: 2025-09-15T09:37-03:00
---

# battery-controller: options survey

Notes on the general options we looked at for the battery-controller in GridHaven. This is a survey, not a decision. Nothing here fixes a threshold, interval or tariff value; those belong in a later note once someone has tried things against real installs. The aim is to keep the shape of the problem written down so the next session doesn't re-walk it.

The job of battery-controller: take a forecast of household solar output, a household load estimate, the current battery state, and the time-of-use tariff schedule, and produce a charge/discharge schedule that lowers the customer's bill without hurting the battery or leaving the house short. It then has to push that schedule to the device and notice when reality drifts from it.

## Problem shape

Three things make this harder than it looks.

First, the inputs are uncertain in different ways. Solar forecasts are wrong on cloudy or changeable days. Load is wrong whenever someone runs an oven, charges a car, or goes on holiday. The tariff is the only input that is known exactly, but it differs per customer and installers change it.

Second, the decision is sequential. Charging now from the grid only makes sense if there is a pricier window later that the battery can cover, and if the sun won't fill it for free anyway. So a greedy rule per time slot is usually wrong in the interesting cases.

Third, the controller sits at the edge of a messy system. Telemetry arrives over MQTT, history lives in InfluxDB, devices are managed through Azure IoT Hub, and the installer UI is Svelte. The controller must tolerate missing data, late data, duplicated messages and devices that are offline for hours.

Constraints that apply to every option below:

- It must degrade safely. If the forecast is missing or the broker is down, the battery should fall back to something boring and harmless, not to the last extreme command.
- It must respect device limits and installer-set reserve levels, regardless of what the optimiser wants.
- Installers need to understand why it did what it did. A black box that cannot explain a charge at an odd hour will generate support calls.
- The language for the numerical side is Julia, so options that need a mature Julia library are cheaper than ones that need us to port something.

## Rule-based scheduling

The simplest family: fixed rules keyed on the tariff. Charge in the cheap window, discharge in the expensive window, hold otherwise. Variants add a reserve level, a "skip grid charge if tomorrow looks sunny" check, or a seasonal profile.

Pros:

- Trivial to explain to an installer and to a customer.
- Easy to test; behavior is a small table.
- No solver, no model training, very small failure surface.
- Works with no forecast at all, which makes it a natural fallback for the other options.

Cons:

- Wastes value on days that differ from the typical day. Grid-charging when the sun would have done it for free is the classic loss.
- Rules multiply. Every new tariff shape (multiple tiers, weekend differences, export rates) adds branches, and the branches interact.
- Tuning is per site, and it silently goes stale as the household changes.

Verdict for now: keep it, but as the fallback path and as the baseline everything else is compared against. Any fancier option has to beat this on replayed history to justify itself.

## Rule-based with forecast gating

A middle step. Keep the rule table, but let the forecast switch rules on and off. For example, grid charging is allowed only when expected solar for the coming day is below what the household will use in the expensive window.

This captures a large share of the benefit of optimisation for little cost. It also keeps the explanation simple: "charged from the grid because tomorrow looked cloudy". The weakness is that the gating logic is itself hand-tuned and tends to grow into a poor man's optimiser. If the gating conditions start needing more than a few terms, that is the sign to move on to something principled.

It also needs a policy for forecast error. Options considered: trust the point forecast, use a pessimistic quantile, or blend with recent actual output. The quantile route is nicer but needs a forecaster that emits one.

## Optimisation over a horizon

Formulate the schedule as a mathematical program over the next day or so: decision variables are charge and discharge power per slot, state of charge follows from them, objective is cost under the tariff, constraints are device limits, reserve and efficiency losses.

### Linear / mixed-integer programming

With a simple efficiency model this is a linear program. Adding "cannot charge and discharge in the same slot", minimum run times, or discrete power levels pushes it to mixed-integer. In Julia, JuMP with an open-source solver is the obvious route, and it is pleasant to write and to read.

Pros:

- Optimal for the given forecast, with a clear objective that the installer can reason about.
- Constraints are explicit and auditable. The safety limits live in the model, not scattered through code.
- Solve times for a day-ahead horizon on one household are small, so running per site is feasible.
- Easy to add terms later, such as a battery wear penalty or an export price.

Cons:

- Optimal for the forecast, not for reality. Without re-planning it is brittle.
- Battery wear and efficiency curves are nonlinear in reality; the linear version is an approximation and we need to know where it lies.
- A solver dependency to ship, version and keep working on whatever hardware runs the controller. If the controller is meant to run on a small gateway at the site rather than in the cloud, this needs checking early.

### Model predictive control

Take the same program and re-solve it repeatedly on a rolling horizon, applying only the first step or first few steps each time. This is the usual fix for forecast error: each re-solve sees fresh battery state and a fresher forecast.

Pros:

- Self-correcting. Drift between plan and reality is handled by design instead of by special cases.
- Same model as the plain optimisation, so the work is shared.

Cons:

- More moving parts at runtime: a scheduler, state handling, and a rule for what happens when a solve fails or runs late.
- Can oscillate if the forecast jumps around between solves, which makes the battery look indecisive and can add cycling. Some damping or a minimum-change rule is usually needed.
- Harder to replay and debug than a single plan, because the history is a chain of plans.

This looks like the strongest candidate in general, but the open question is how often to re-solve versus how much load that puts on the data path. That depends on measurements we do not have yet.

### Stochastic or robust variants

Scenario-based stochastic programming or a robust formulation handles forecast uncertainty explicitly. It is attractive on paper and costs a lot in complexity: scenario generation, bigger models, harder explanations. Our view is that MPC with a conservative quantile gets most of the benefit. Parked unless replay shows persistent losses from uncertainty that re-planning cannot fix.

## Dynamic programming and learned policies

### Dynamic programming

Discretise state of charge and time, and solve by backward induction. It handles nonlinear efficiency and wear naturally and gives a policy that can be looked up quickly at runtime. The cost is the curse of dimensionality: adding a second state (say, a car charger or a hot-water tank) grows the table quickly. For a single battery it is feasible and can be written directly in Julia without a solver dependency. Worth keeping in mind as an alternative to MIP if solver packaging turns out to be painful.

### Reinforcement learning

Train a policy against simulated or replayed households. Appealing because it could learn household-specific patterns and handle nonlinearity without a hand-built model.

Concerns:

- Needs a trustworthy simulator, and we would be building that first.
- Safety guarantees are weak. We would need a hard rule layer wrapped around the policy anyway.
- Explanations are poor, which hurts the installer story.
- Per-household training or fine-tuning is an operational burden.

Verdict: not now. Revisit only if the optimisation approaches plateau and we have a decent simulator for other reasons.

## Where the forecast and data come from

The controller is only as good as its inputs, so the options for them matter as much as the scheduler.

- Solar forecast: either a physical model fed by weather data and the installed array description, a statistical model trained on the site's own history, or a blend. Site history sits in InfluxDB, so a per-site statistical correction on top of a physical baseline is cheap to try. Whatever is chosen should emit an uncertainty band, not just a point value, or the quantile idea above is dead.
- Load estimate: a profile by time of day and day type learned from recent history is the usual starting point. Anything fancier should be justified by replay.
- Battery state: read from the device over MQTT. Treat stale readings as unknown rather than reusing them silently.
- Tariff: store as data per customer, not in code. Installers will edit it, so validation matters more than cleverness.

Rough shape of the data path as it stands in our heads, using only the pieces already in the project:

```
device -> MQTT -> ingest -> InfluxDB
InfluxDB + tariff + forecast -> battery-controller -> schedule
schedule -> Azure IoT Hub -> device
```

## Where it runs

Two broad placements, and they interact with the choice of scheduler.

Cloud-side: one service plans for many sites. Easy to update and observe, solver packaging is not a problem, and it can share forecast work across sites. Downsides are dependence on connectivity and the need to push schedules down through Azure IoT Hub, so an offline site runs on a stale plan.

Site-side: the controller runs near the battery. Survives outages and reacts quickly, but updates are harder, hardware is limited, and every site is a place where things can go wrong without anyone noticing.

A hybrid is the likely answer: plan in the cloud, send a schedule with an expiry, and have the device or a small local agent fall back to the rule-based behavior when the schedule runs out. That makes the rule-based option load-bearing rather than just a baseline, and it is another reason to keep it simple and well tested.

## Evaluating the options

Whatever we pick should be compared on the same footing:

- Replay recorded days from InfluxDB through each candidate, with the forecast it would actually have had at the time, not hindsight values.
- Report cost against the rule-based baseline and against a perfect-foresight upper bound. The gap between them tells us how much any scheduler can possibly add.
- Track battery cycling as well as cost. A scheduler that saves a little by cycling hard may lose on wear.
- Include bad days on purpose: missing data, a stuck sensor, a tariff change, a device that drops off the network.

## Open questions

- How much does the battery wear model matter for the schedules we would produce? If little, the linear model is enough.
- Where does the controller run, and what hardware does that imply for solver choice?
- Do installers want to override or pin parts of a schedule, and if so how does that enter the model as a constraint?
- How are export or feed-in rates handled for customers who have them? It changes the objective noticeably.
- What is the right behavior when the forecast service is unavailable for a long stretch?

## Leaning

Not a decision. The current lean is: rule-based as the always-available fallback, a horizon optimisation in Julia re-solved on a rolling basis as the main path, and dynamic programming kept as the fallback if solver packaging is a problem. Reinforcement learning and stochastic formulations stay on the shelf until replay shows a reason to open them.
