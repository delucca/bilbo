---
id: 01M24KEE9G0NS270NKNJAF369W
created: 2026-09-09T23:48-03:00
sources:
  - "code: freightweave/rebalancer/repair.py"
---

# load-rebalancer design

The load-rebalancer takes a delay on a planned multi-leg truck or rail route and produces a repaired assignment of loads to legs. It does this in two stages. First a greedy repair makes a quick feasible fix. Then a CP-SAT re-solve, restricted to the affected legs, improves that fix. The restriction is selected with the flag `--scope affected`. This note records the shape of that design and the reasons for it, so a later session does not have to rebuild the reasoning from the code.

## Name history

The component used to be called `reshuffler`. It is called `load-rebalancer` now. You will still find the old name in older commit messages, old dashboards, some log lines, stale branch names and a few comments. Treat them as the same component. If you grep for one name, grep for the other too. New code, docs, topic names and config keys should use `load-rebalancer` only. Do not reintroduce `reshuffler` in anything new, even as an alias, because that is how the two names ended up coexisting in the first place.

## Where it sits

FreightWeave plans routes and then keeps them alive while the world changes. The route planner builds the original multi-leg plan, mixing truck and rail legs. The load-rebalancer only starts working after a plan exists and something has gone wrong with it. It does not plan from scratch, and it should not become a second planner. If a change needs a full replan, that belongs to the planner, and the rebalancer should say so rather than quietly produce a poor patch.

The pieces around it are the ones in the stack:

- Delay events arrive over Google Cloud Pub/Sub. The rebalancer consumes them and does not poll for them.
- Current plan state and per-leg load assignments live in Redis. The rebalancer reads them at the start of a run and writes the repaired assignment back at the end.
- The FastAPI service exposes the dispatcher-facing surface. A dispatcher can see the proposed rebalance and accept it, and the service is also where a manual rebalance request enters.
- OR-Tools provides the CP-SAT solver used in the second stage.

Python is the glue for all of it. The solver model is built in plain Python from the data read out of Redis. There is no separate solver service.

## Trigger and inputs

A run is triggered by a delay event or by a dispatcher asking for a rebalance. Either way the input is the same in kind: the current plan, the set of legs that the delay touches directly, and the capacity and timing facts for each leg. From the touched legs the rebalancer derives the affected legs. These are the touched legs plus the downstream legs whose timing or load now cannot hold, for example a connection that would be missed or a rail slot that can no longer be reached from a late truck.

Computing the affected set is the cheap part and it is deliberately done before any solving. Everything later depends on it. The greedy stage works inside it, and the CP-SAT stage is limited to it. If the affected set is wrong, both stages are wrong in the same direction, so tests for the affected-set derivation matter more than tests for the solver.

A delay that does not break any downstream constraint produces an affected set of just the touched legs, and often the greedy stage alone resolves it. That is the common case and it should stay fast.

## Greedy repair

The greedy stage runs first. Its job is to get back to a feasible plan quickly, not a good one. It walks the affected legs in order of urgency and, for each load that no longer fits, tries to move it to the nearest alternative that satisfies capacity and timing. Preference goes to keeping the load on the same mode, and then to the smallest change from the existing plan. It does not backtrack.

Reasons for having it at all:

- Dispatchers need an answer in a short time. A feasible fix that arrives promptly beats an optimal one that arrives late.
- It gives the solver a starting point. A feasible hint makes the second stage start from something sensible instead of searching blind.
- It is the fallback. If the solver stage fails or runs out of time, the greedy result is what gets offered.

Greedy can leave real waste. It may fill an early alternative that a later load needed more, and it can move a load further than necessary. Those are the defects the second stage exists to clean up. Do not try to make the greedy stage smarter to compensate. Once it starts to look like a solver, there are two solvers to maintain.

## CP-SAT re-solve on the affected legs

After greedy, the rebalancer builds a CP-SAT model and re-solves. The model covers only the affected legs. Legs outside that set are fixed as constants in the model: their load assignments and timings are taken from the current plan and the solver cannot change them. This is what `--scope affected` selects, and it is the point of the design.

```
load-rebalancer ... --scope affected
  1. derive affected legs from the delay
  2. greedy repair inside the affected legs
  3. CP-SAT re-solve, affected legs only, hinted by step 2
  4. write the result back, offer it to the dispatcher
```

The reasons for restricting the scope:

- Model size stays tied to the size of the disruption instead of the size of the whole network. A small delay should not cost a full solve.
- Plan stability. Dispatchers have already told drivers and rail yards about the unaffected legs. Re-solving everything would move loads that had no reason to move, and that causes real phone calls.
- Solve time is more predictable, which matters because the solver has a time limit.

The objective is about minimizing disruption first and cost second: fewest changed assignments, then lower added delay, then cost. The exact weights are tuning values and are left out of this note on purpose. The greedy result is passed in as a hint, and the solver is only accepted if it is at least as good as greedy under the same objective. If it is not, greedy wins.

The flag is a scope choice, not a quality choice. Other scope values may exist or be added, but `--scope affected` is the one the production path uses. If you change the default, say so loudly in the commit and update this note.

## Failure modes and fallbacks

Things that have gone wrong or are likely to:

- **Solver hits its time limit.** Take the best feasible solution it has, if any, compared against greedy. Otherwise return greedy. Never return nothing when greedy found something feasible.
- **Greedy finds nothing feasible.** The affected set may be too tight, since fixing the outside legs can make the problem infeasible. In that case the rebalancer can widen the affected set once and retry. If it is still infeasible, it reports that to the dispatcher as needing a manual decision or a full replan. It does not loosen the fixed legs silently.
- **Stale state in Redis.** If the plan changed between reading state and writing the result, the write must be rejected and the run redone from fresh state. A repaired plan built on an old plan is worse than none.
- **Duplicate or out-of-order delay events.** Pub/Sub can deliver an event more than once and not in order. Runs need to be idempotent for the same delay, and a newer delay on the same leg should supersede an older one in flight.
- **Cascading delays.** Several delays arriving close together can overlap in the legs they affect. Prefer to merge them into one run over running them in parallel against the same legs.

When something fails, the log line should say which stage failed and what the affected set was. Many of the old `reshuffler` logs did not, which made them hard to read.

## Open questions and rules of thumb

Open questions:

- Whether the widening retry should be automatic or only on request from the dispatcher. For now it is automatic and limited to one retry.
- Whether rail legs with fixed slot times should be treated as harder constraints than truck legs by default, or tuned per region.
- How much of the solver's partial progress is worth showing to the dispatcher while it runs.

Rules of thumb for changes:

- Keep the stage order: affected set, greedy, CP-SAT. Do not skip greedy because the solver is fast on the test data.
- Keep the fixed-outside-legs rule. Anything that lets the solver move unaffected legs needs a deliberate decision, not a side effect.
- Test the affected-set derivation on its own, then each stage on its own, then the whole path.
- Use the name `load-rebalancer` everywhere, and when searching old material remember `reshuffler`.
