---
id: 01JVANQK9W9WCVFVNWTAHP58Z1
created: 2025-05-15T16:11-03:00
---

# progress-api: survey of options considered

Rough survey of the options for progress-api, the service that exposes student progress against curriculum standards and feeds the next-exercise suggestions to the Vue.js front end. Nothing here is settled. It is a list of what was looked at and what each option costs, so we don't redo the thinking later.

## What progress-api has to do

It reads progress records per student and per standard, returns them in shapes the teacher dashboard needs (class view, student view, standard view), and triggers or serves suggestions. Teachers open it in bursts, mostly at the start and end of lessons, so reads dominate and writes arrive in batches when marks are entered.

## API style

Options: plain Django views with serializers, Django REST framework viewsets, or a GraphQL layer on top of the Django models.

- DRF is the least surprising. Pagination, filtering and permissions are already solved, and the team knows it.
- GraphQL would suit the dashboard, which asks for odd combinations of fields. The cost is query depth control and N+1 problems on the ORM side, plus one more thing to learn.
- Hand-written views give full control but we would rebuild what DRF gives for free.

Leaning toward DRF, with a few purpose-built endpoints for the heavy dashboard views.

## Where the data is read from

Options: query the relational database directly, precompute summaries, or serve search-style aggregations from Elasticsearch.

- Direct queries are simple and always fresh, but class-wide roll-ups across many standards can get slow.
- Precomputed summaries refreshed by Celery tasks make reads cheap. The downside is staleness, and teachers will notice if a mark they just entered does not show up.
- Elasticsearch is good for filtering and aggregating over standards text and tags. It adds a second copy of the data that must be kept in sync, so it is worth it only for the search-heavy views.

## Suggestions and the scikit-learn model

Options: call the model inside the request, have Celery compute suggestions ahead of time and store them, or run the model in a separate service.

- Inline calls tie request latency to model load and prediction time, and a worker process has to hold the model in memory.
- Precomputing through Celery keeps progress-api fast and makes failures non-fatal: stale suggestions are better than an error page.
- A separate service is cleaner for versioning the model, but it is probably too much for the current team size.

Precompute looks like the natural fit, with progress-api only reading stored results.

## Caching and consistency

Short-lived caching of dashboard responses per class is an option. Invalidation on write is the hard part; time-based expiry is easier but shows old data. Worth deciding together with the summary approach above, since the two overlap.

## Auth and data protection

Students are minors, so access has to be scoped by school and class. Options are checking permissions in each view or enforcing it in a shared queryset layer. The shared layer is safer because one forgotten check in a view should not leak data. Logging should avoid personal data.

## Open questions

- How fresh must a dashboard number be after marks are entered?
- Is Elasticsearch needed in progress-api at all, or only in the exercise search?
- Who owns model retraining and how does progress-api learn a new model is live?
