---
id: 01KQDWZZDKWXAYEEN2CR5V2JG6
created: 2026-04-29T21:35-03:00
sources:
  - "code: freightweave/api/main.py"
---

# dispatch-api reference

The dispatch-api is a FastAPI service listening on port 8080, and dispatchers request a plan with `POST /v1/routes`. That is the one entry point most people need. Everything else in this note is background around it, kept short on purpose.

## What it is

dispatch-api is the HTTP front of FreightWeave. It is a Python FastAPI app. Regional freight dispatchers, or the tools they use, call it to get a multi-leg truck and rail route plan. It does not solve routes itself in the request handler's own code; it hands the problem to the OR-Tools based planner and returns what comes back.

## Port and main endpoint

Port: `8080`. Plan request: `POST /v1/routes`. A dispatcher sends the shipment details, the available trucks and rail legs, and any time windows, and gets a plan back. If you are checking whether the service is up, hit that port first, before looking at anything else.

## Request flow

1. The request arrives on port 8080 and FastAPI validates the body.
2. The handler builds the routing problem and calls the OR-Tools solver.
3. The solver result is turned into legs, truck legs and rail legs, in order.
4. The plan is stored in Redis so later rebalancing can find it.
5. The response goes back to the dispatcher.

## Rebalancing on delays

When a delay happens, the loads on the affected legs are rebalanced. Delay events reach the service through Google Cloud Pub/Sub. The service reads the stored plan from Redis, re-solves the affected part, and writes the new plan back. Dispatchers see the updated plan the next time they fetch it.

## Dependencies

- Redis: holds current plans and short-lived state. If Redis is down, new plans may still be solved but rebalancing will not work.
- Google Cloud Pub/Sub: carries delay events in.
- OR-Tools: does the actual route optimization.

## Gotchas

- Large problems can make the solver slow, so a plan request can take a while. Do not assume it is hung.
- Stale plans in Redis can make a rebalance start from old data. Check what is stored before blaming the solver.
- Delay events may arrive more than once; handling should tolerate repeats.

## Where to look next

Start with the FastAPI app entry point and the router that serves the routes endpoint. Then follow the call into the planner module. For delay handling, look at the Pub/Sub subscriber code. Update this note when the port or the plan endpoint changes.
