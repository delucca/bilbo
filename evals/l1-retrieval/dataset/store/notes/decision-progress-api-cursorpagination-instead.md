---
id: 01KXYR2PFXERV6AE72R963FH30
created: 2026-07-20T00:10-03:00
---

# progress-api uses CursorPagination instead of offset pagination

progress-api uses CursorPagination instead of offset pagination. The reason is performance: offset queries became slow beyond 100000 rows. This note records the decision, the reasoning, what it changes for callers, and what to watch for when touching the list endpoints. It is written for whoever next opens the pagination code in progress-api, human or agent, and assumes no memory of the discussion.

Short version: any list endpoint in progress-api that can grow large pages with a cursor. Do not add offset or page-number pagination back to those endpoints, even if a client asks for jump-to-page. If a screen needs jump-to-page, solve it on the client side or with a separate search path, not by reverting this.

## Context

progress-api is the Django service that serves student progress against curriculum standards. Teachers in secondary schools use it through the Vue.js front end to see where each student stands on each standard, and the exercise suggestion flow reads the same data. The progress records are the biggest table in the system. Every time a student attempts an exercise tied to a standard, a record is written, and Celery jobs add more rows when they recompute mastery estimates in batches. The table only grows. Nothing prunes it during a school year, and older years are kept for comparison.

The first version of the list endpoints used plain offset pagination, the default most Django projects start with. It worked fine in development and in the first months of use. As the table grew, deep pages got slower. Page one stayed fast. Pages far into the result set took noticeably longer, and the slowdown showed up in the front end as a spinner that never seemed to end when someone scrolled a long history.

The pain became visible once the table passed roughly 100000 rows in the larger deployments. Beyond that point the offset queries were slow enough that people complained, and the database load from a few teachers paging through history at once was enough to disturb other work on the same database, including the Celery batch jobs.

## Decision

progress-api list endpoints that return progress records use CursorPagination. A response carries an opaque pointer to the next page and one to the previous page, rather than a page number or an offset. Clients follow those pointers and never build them.

The ordering that the cursor relies on must be stable and unique in practice. For progress records that means ordering by a timestamp-like column, with a tiebreaker so that rows written in the same instant do not get skipped or repeated. If someone changes the ordering of an endpoint, they are also changing what the cursor encodes, so old cursors held by clients become invalid. That is acceptable, since cursors are short-lived, but it should be a deliberate change.

The decision applies to endpoints whose result sets can grow with usage. Small, bounded lists (for example the list of curriculum standards for one subject, or the list of classes a teacher owns) do not need it and can stay unpaginated or use whatever is simplest. Do not spend effort converting them.

## Why offset pagination was slow

Offset pagination asks the database to produce the first rows of an ordered result and throw them away until it reaches the requested page. The cost therefore grows with how deep the page is, even though the page itself is small. On a table with a lot of rows, page after page of discarded rows is real work: the database has to walk an index or sort, count past rows it will not return, and only then return the handful we wanted.

There is a second cost that is easy to miss. The usual page-number style also wants a total count, so the client can draw page numbers. A count over a large, filtered table is its own slow query, and it ran on every request. Dropping the total count was part of the win. Cursor pagination does not need it, so that query is gone.

Third, offset pagination is unstable under writes. While a teacher is paging through history, new progress rows may arrive, because students keep working and Celery keeps writing. With an offset, a new row at the front pushes everything down by one, so the next page repeats a row the teacher already saw. With deletions it skips rows. Cursor pagination anchors on a position in the ordering, so inserted rows do not shift what comes next.

Cursor pagination keeps the cost of each page about the same, whether it is the second page or the thousandth. The database seeks directly to the position in the index and reads forward. That is why the fix is structural and not just an index tweak. We looked at indexes first, and they helped the ordering but did not remove the cost of skipping.

## Alternatives considered

Keep offset and add an index. This was tried first. A good index on the ordering column made the early pages fine and made the worst case less bad, but deep pages still scaled with depth, and the count query stayed slow. It bought time, not a fix.

Keep offset but cap the depth. We could refuse pages beyond some limit and tell people to filter instead. This avoids the worst queries but breaks legitimate use, such as a teacher reviewing a whole term for one student. It also leaves the inconsistency under writes.

Keyset pagination written by hand. This is the same idea as the cursor approach, but we would own the encoding and the edge cases. The framework class already does what we need, so writing our own adds code with no benefit. If we ever need something it cannot do, such as multi-column ordering with mixed directions, revisit this.

Move listing to Elasticsearch. The project already uses Elasticsearch for searching standards and exercises, and one idea was to serve history from it as well. That would add a sync path between the database and the search index for data that is naturally relational, and deep paging in Elasticsearch has its own limits and its own cursor-like mechanism. Not worth it for plain history listing. Search stays where it is.

Precomputed pages or caching. Caching page results helps repeated reads but not the first read of a deep page, and invalidation gets messy when new progress rows arrive constantly. Rejected.

## What changes for callers

The response shape of the list endpoints changes. Instead of a page count and numbered links, the body gives a pointer to the next results and a pointer to the previous results, either of which can be empty at the ends. Clients must treat these pointers as opaque strings and pass them back unchanged. They must not parse them, store them long-term, or try to construct one.

There is no total count in these responses. Any front end element that showed a total or a page number list has to go, or has to get its number from a separate, cheaper source. For the teacher dashboard, summary figures such as how many standards a student has mastered come from aggregate endpoints that are computed separately and do not depend on the list being paged.

There is no jump to an arbitrary page. Navigation is forward and backward only. In the Vue.js front end this fits infinite scroll and a simple load-more button well, and those are what the history views use. A numbered pager would not fit and was removed.

Page size is still controllable within limits set on the server side. A client may ask for a smaller or larger page, but the server clamps it. Do not raise the clamp casually, since the point is bounded work per request.

Filters keep working and are applied before the cursor position. A cursor is only meaningful together with the same filters that produced it. If a client changes the filter (a different student, a different standard) it must start again from the first page and drop the old cursor.

## Implementation notes

The pagination class is set per view or through a shared base class for the progress list views, not globally. A global default would silently change small endpoints that do not need it, and would force the ordering rules on views that have none. When adding a new large list endpoint, use the same shared base so the page size clamp and the ordering rules stay in one place.

The ordering column must be indexed, and ideally the index covers the filters most often combined with it, such as the student and the standard. Without a matching index the cursor query still avoids skipping rows but may sort a large set. Check the query plan when adding a new filter combination.

Because the cursor encodes the position of the last row seen, ordering on a nullable column is a trap. Null values sort in a database-specific way and make the position ambiguous. Order only by columns that are never null for progress records. If a new endpoint needs to sort by something nullable, use a different approach for that endpoint and write down why.

Serializers should not do per-row queries. With cursor pagination the page is small, so an N+1 problem is bounded, but it is still slow and easy to avoid with the usual prefetching of related students, standards and exercises.

Celery tasks that need to walk the whole table, such as the batch recomputation of mastery estimates, should not go through the HTTP list endpoints at all. They iterate over the queryset directly in chunks using the same kind of keyset idea. Keeping them off the API keeps batch load away from teacher-facing latency.

## Testing

Tests for the list endpoints cover these cases: the first page has no previous pointer; the last page has no next pointer; following next pointers visits every row exactly once; rows inserted during paging do not cause repeats; two rows with the same timestamp are neither dropped nor duplicated across a page boundary; and a tampered or stale cursor yields a clean client error and not a server error.

The tie case matters most. It is the one that quietly loses data when the tiebreaker is missing, and it will not show up with small hand-made fixtures unless the fixtures deliberately share timestamps. Keep that fixture.

For performance, there is no strict pass or fail threshold in the test suite, since timing in CI is noisy. Instead, when changing a list query, run it against a large local dataset and compare the first page with a page deep into the data. They should cost about the same. If the deep page is clearly slower, something is wrong with the ordering or the index, and the cursor is not doing its job.

## Gotchas

Changing the ordering of an endpoint invalidates cursors in flight. A teacher with a tab open may get an error on load-more after a deploy that changes ordering. The front end should handle that by restarting from the first page rather than showing a failure.

Cursors are not a security boundary. They are opaque but not secret, and permissions must be checked on every request as usual. A cursor from one teacher's view must not unlock anything for another. Because filters and permissions are applied before the cursor, this holds as long as the queryset is built from the requesting user's permitted data first.

Anyone who reads the old docs or old front end code may expect a count and page numbers. Those are gone. If a request comes in for a total, point to the aggregate endpoints.

Export features (for example a teacher downloading a student's whole history) should not page through the API. They run as a background job and produce a file. Trying to build export on top of paged list calls reintroduces the load problem from another direction.

If a new feature seems to need random access into a big list, stop and ask whether a search or a filter would serve the user better. A teacher looking for a specific attempt almost always knows a student, a standard or a date range, and filtering by those is both faster and clearer than paging.

## When to revisit

Revisit if the ordering needs become more complex than a single stable column with a tiebreaker, if the framework class stops fitting, or if product requirements insist on page numbers for a bounded dataset. Revisit also if the table moves to a different storage engine with different tradeoffs.

Do not revisit just because someone misses page numbers. The tradeoff was made knowingly: we gave up random page access and a total count in exchange for steady response time as progress data grows past 100000 rows and far beyond.

## Summary of what to remember

progress-api pages large lists with CursorPagination. Offset pagination got slow beyond 100000 rows, the count query was a further cost, and offsets misbehave while rows are being inserted. Callers follow opaque next and previous pointers, get no total count, and cannot jump to a page. Keep ordering stable, unique in practice, non-null and indexed. Keep batch jobs off the HTTP list endpoints. Small bounded lists do not need any of this.
