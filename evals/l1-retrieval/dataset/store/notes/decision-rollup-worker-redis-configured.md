---
id: 01KPTTXQ5MNW8XN6WK5A9V4GZR
created: 2026-04-22T11:53-03:00
---

# rollup-worker broker: Redis instead of RabbitMQ

rollup-worker uses Redis as its Celery broker, configured through `CELERY_BROKER_URL`, instead of RabbitMQ. The reason is simple: the team already operates Redis for caching, so there is no second broker to run, patch, monitor or be on call for. This note records that choice so nobody reopens it without a new reason. The internal codename of rollup-worker is `nocturne-q`, so you will see that name in older chat threads, dashboards and branch names. It is the same component as `rollup-worker`.

## Decision

The broker for `rollup-worker` is Redis. The connection is set only through the `CELERY_BROKER_URL` setting, read from the environment by the Django settings module. RabbitMQ was the alternative and it is not used for this component. If someone asks "why not RabbitMQ", the answer is: the team already runs Redis for caching, and one fewer service to operate outweighs what RabbitMQ would add for our load.

## Why Redis and not RabbitMQ

- The team already operates Redis for caching. Backups, upgrades, alerting and access rules for it already exist.
- Adding RabbitMQ would mean a new service, a new failure mode and new runbooks for a small team.
- The rollup jobs are not exotic. They aggregate student progress against curriculum standards, and a delayed or retried task is acceptable. We do not need the stronger routing or delivery features that RabbitMQ is known for.
- Celery supports Redis as a broker out of the box, so no custom code is needed in the Django app.

## Configuration

Only the broker URL changes between environments. Keep it in the environment, not in the repository.

```
CELERY_BROKER_URL=redis://<host>/<db>
```

Fill in host and database from the deployment config for each environment. Do not hard-code them in settings. Anything that starts a `rollup-worker` process needs this variable set, or Celery will fall back to its default broker and tasks will not be picked up the way you expect.

## Consequences and risks

- Redis is shared infrastructure with the cache. A heavy cache workload can affect task latency, and a large task backlog can use memory the cache wants. Watch memory use on the instance.
- Redis as a broker gives weaker delivery guarantees than RabbitMQ. Tasks that run long can be redelivered if the visibility timeout is shorter than the task. Rollup tasks should therefore be idempotent, so a repeat run gives the same result.
- If the cache and broker ever need to be isolated, use a separate logical database or a separate Redis instance and change only `CELERY_BROKER_URL`. No code change is needed.
- Scikit-learn exercise suggestions and Elasticsearch indexing are not part of this decision. They keep their own configuration.

## When to revisit

Reopen this only if one of these happens: we lose tasks in a way that the idempotent design cannot absorb, we need routing or priority features Redis cannot give, or the cache and broker workloads start hurting each other and splitting the instance is not enough. Until then, `rollup-worker` stays on Redis, and `nocturne-q` in any old document means this same component.
