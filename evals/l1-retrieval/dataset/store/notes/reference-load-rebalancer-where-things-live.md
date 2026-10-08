---
id: 01K6CVA69C68SCCYT2MHM0MY81
created: 2025-09-30T05:20-03:00
---

# load-rebalancer reference

The load-rebalancer is the part of FreightWeave that reacts when a leg of a multi-leg truck or rail route slips. It takes the current plan plus a delay signal and produces a revised assignment of loads to legs. This note says where the pieces live in general terms. Check the repo tree before trusting any of it, since layout drifts.

## Where the solver lives

The core is an OR-Tools model written in Python. Look for a solver package under the main service source, separate from the HTTP layer. It usually splits into three parts: building the model from a route plan, running the search, and turning the solution back into plain assignments.

- Model building: reads legs, capacities, time windows and transfer points, and creates the variables and constraints.
- Objective and penalties: kept apart from the constraints so dispatch preferences can change without touching feasibility rules.
- Result mapping: converts solver output into the same route structures the planner uses, so the rest of the system never sees solver types.

When a rebalance gives odd results, start with the constraint builders, not the search settings.

## API surface

FastAPI exposes the rebalancer to dispatcher tools and to other FreightWeave services. Routers sit in the API package, request and response schemas in a models or schemas module. Handlers should stay thin: validate input, load state, call the solver package, return the result. If a handler contains routing logic, it probably belongs in the solver package.

Long solves should not block request handling. Check how the handlers hand work off before adding anything slow.

## Events and Pub/Sub

Delay information arrives through Google Cloud Pub/Sub. A subscriber module in the rebalancer area pulls delay and status messages, decides whether a rebalance is warranted, and triggers it. Rebalanced plans are published back out on a separate topic so dispatch views and other consumers can update.

Things to keep in mind:
- Messages can be redelivered, so handling a delay event must be safe to repeat.
- Ordering is not guaranteed; compare event timestamps with what is already stored rather than assuming arrival order.
- Topic and subscription names come from configuration, not code.

## State in Redis

Redis holds the live working state: current route plans, the latest known delay per leg, and short-lived locks that stop two rebalances from running on the same route at once. Key naming helpers live in one place in the code; use them rather than building key strings by hand. Most entries are meant to expire, so do not treat Redis as the system of record for plans.

## Config and tests

Settings such as solver time limits, penalty weights and broker names are read from environment-driven config, loaded in a single settings module. Do not hard-code them in solver or handler code.

Tests mirror the source layout. Solver tests use small hand-built route plans and check feasibility and rough shape of the answer rather than exact assignments, since OR-Tools may return different but equally good solutions. Pub/Sub and Redis are faked or run locally in tests. When adding a constraint, add a small plan that fails without it.

## Open spots

Where exactly the lock handling and the event deduplication sit has changed before. Confirm by searching for the lock helper and the subscriber entry point before editing either.
