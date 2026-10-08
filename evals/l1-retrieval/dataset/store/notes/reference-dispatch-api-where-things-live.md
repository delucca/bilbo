---
id: 01JQT2ZZGNPKAKQBBNZVTYT21C
created: 2025-04-01T22:49-03:00
---

# dispatch-api reference: where things live

Quick map of dispatch-api for whoever picks it up next. It is the FastAPI service that dispatchers and other FreightWeave parts talk to. It does not do the heavy route math itself; it hands that to the planning code and to the rebalancing code, and it keeps short-lived state in Redis. This note only says where things sit, in general terms. Check the tree before trusting any of it.

## Service entry and routing

The app object is built in one entry module near the top of the service package. It creates the FastAPI app, registers routers, wires middleware and sets up startup and shutdown hooks. Routers are grouped by topic: route planning, load rebalancing, delay intake, and health or status. Each router module holds thin handlers. If a handler is getting long, the logic probably belongs in a service module instead.

Request and response models live in their own schema package, separate from the handlers. Pydantic models are shared between routers where the shape is the same, for example a leg, a stop, a load. When an API shape changes, start in the schema package and follow the usages out.

## Planning and rebalancing logic

The OR-Tools model building is kept out of the HTTP layer. There is a solver package that takes plain Python inputs (legs, capacities, time windows, modes such as truck or rail) and returns plain outputs. The handlers translate between schemas and these inputs. Constraints and cost terms are in separate small modules, so a new rule usually means adding one there and registering it, not editing the model builder.

Rebalancing after a delay reuses the same solver package with the current plan as a starting point. Look for the code that builds the starting assignment from an existing plan; that is where most surprises show up when a rebalance gives odd results.

Solver runs can be slow. Handlers that call it should be treated as potentially long, and the code around them decides whether to run it inline or push it to a worker. Look at how the existing handlers do it before adding another.

## Redis usage

Redis access goes through one client module created at startup and passed in through FastAPI dependencies. Typical uses: caching computed plans, holding the latest known state of a route, short locks so two rebalances do not collide on the same route, and idempotency markers for incoming events. Key naming helpers sit next to the client module. Use them instead of building key strings in handlers, so expiry and prefixes stay consistent.

If something looks stale, check the cache layer first, then check whether the writer to that key is the planning path or the event path.

## Pub/Sub integration

Delay and status events arrive from Google Cloud Pub/Sub. The subscriber code is in its own module group, separate from the routers, with a message parser, a handler that maps an event to a rebalance request, and acknowledgement logic. Outbound publishing of plan changes uses a small publisher wrapper in the same area. Topic and subscription names come from settings, not from code.

Duplicate delivery is expected. The idempotency markers in Redis are what protect against double rebalancing, so keep that in mind when touching the handler.

## Config, tests and tooling

Settings are loaded once from environment variables into a settings object in a config module. Anything environment specific (Redis location, Pub/Sub names, project identity, solver limits) should go through it. Do not read the environment directly elsewhere.

Tests sit in a top-level tests directory that mirrors the service package layout. Solver tests use small hand-built scenarios. API tests use the FastAPI test client with Redis and Pub/Sub replaced by fakes; the fakes are defined in shared fixtures. Look in the fixtures first when a test needs new setup.

Dependency and tooling files are at the project root: the Python dependency manifest, lint and type-check config, and the container build file for deployment. Deployment details are not here; ask the person who owns the infrastructure side.

## Where to start for common tasks

- New endpoint: schema package, then a router, then a service function.
- New routing rule: constraint modules in the solver package.
- New event type: subscriber parser and handler, plus the idempotency step.
- Odd cached data: Redis client module and key helpers.
- Config change: the settings object, then wherever it is consumed.
