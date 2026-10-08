---
id: 01JWZEKJE6RBMGZHH2WEPRNV5W
created: 2025-06-05T04:06-03:00
sources:
  - "doc: progress-api review notes"
---

# Code review of progress-api: N+1 in ProgressSerializer

Review of progress-api, done in a hurry, with one main finding: `ProgressSerializer` issues one query per standard. That is a classic N+1 pattern. The conclusion of the review is that the queryset behind it must call `prefetch_related('standard')`. Everything else below is context and follow-up, kept general on purpose.

## Names used in this note

The component is called `progress-api` in full. The short form `pgapi` is used in chat, in branch names and in review comments, and it means the same thing as `progress-api`. A related short form is `pevents`, which stands for `progress_events`. Do not mix the two up: `pgapi` is the API, `pevents` is the event stream or table of progress events that feeds it.

## Scope of the review

The review covered the read path of progress-api: the views that list a student's progress against curriculum standards, and the serializer that shapes the response. Write paths, Celery tasks and the scikit-learn exercise suggestions were only glanced at. The Vue.js client was not reviewed at all.

## Main finding: N+1 in ProgressSerializer

When a teacher opens a class view, the endpoint returns a list of progress records. Each record points to a standard. `ProgressSerializer` reads fields from that related standard while serializing, and the queryset did not load it ahead of time. So Django ran the main query once, then one extra query for every standard touched. With a full class and a long list of standards this adds up fast, and the response time grows with the amount of data rather than staying flat.

## Why it matters here

Secondary school teachers open these views during lessons, often on slow school networks. A slow list view is felt directly. The database load also lands at the same moments across many classes, usually at the start of a period.

## Conclusion and fix

The queryset used by the view must call `prefetch_related('standard')`, so the standards are loaded in one extra query instead of one per row. Short example of the shape of the change:

```python
queryset = Progress.objects.prefetch_related('standard')
```

The serializer itself needs no change in logic. The fix belongs on the queryset, not in `ProgressSerializer`, so that every caller of the serializer benefits once the view is fixed.

## How to confirm

Count queries for the list endpoint before and after the change, using a test with a query-count assertion. The count should stay constant as the number of standards grows. Add that test next to the existing serializer tests so the regression does not come back quietly.

## Things not checked

I did not check whether other serializers in progress-api have the same problem. It is likely, since they share the same style. I also did not look at whether Elasticsearch-backed endpoints bypass the serializer. Both are worth a quick pass.

## Interaction with pevents

`pevents` (the `progress_events` data) may be read from the same view in some cases. If so, the same N+1 risk applies there, and it should get the same treatment. This was not confirmed in this review.

## Follow-ups

- Apply `prefetch_related('standard')` in the list view.
- Add a query-count test.
- Scan other serializers in `pgapi` for per-row related lookups.
- Check whether `pevents` reads have the same pattern.

## Status

Finding is clear and the fix is small. Nothing has been measured yet, so no numbers are claimed here. Update this note once the fix lands and the query count has been checked.
