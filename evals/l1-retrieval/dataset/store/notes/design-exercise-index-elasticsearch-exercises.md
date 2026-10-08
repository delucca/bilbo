---
id: 01K2ZNAJK8ABAKCHAZSK7AQYSD
created: 2025-08-18T19:38-03:00
---

# exercise-index design

exercise-index is the Elasticsearch index `exercises-v3`, created with `number_of_shards` set to 1. This note records how it is shaped and why, so nobody has to rediscover it. The index backs the "suggest next exercise" feature in ClassroomCompass, which secondary school teachers use to pick work for students based on progress against curriculum standards.

## Purpose

Teachers see a student's gaps against curriculum standards. The Django backend turns those gaps into a query against exercise-index and returns a ranked list of exercises. The Vue.js front end only shows the list; it does not talk to Elasticsearch directly. All queries go through Django so permissions and school scoping stay in one place.

## Index identity and shards

The index name is `exercises-v3`. The setting that matters most is `number_of_shards`, which is 1. The exercise catalogue is small compared with what Elasticsearch is built for, so a single primary shard is enough. One shard also keeps relevance scoring consistent: term statistics are not split across shards, so scores do not shift between runs depending on shard layout.

The shard count cannot be changed on an existing index without a split or a reindex. If the catalogue ever grows enough to need more shards, that means a new index version, not an edit to `exercises-v3`.

```json
{
  "settings": {
    "number_of_shards": 1
  }
}
```

## Document shape

Each document is one exercise. The fields we rely on are the exercise text, the curriculum standards it covers, a difficulty level, the subject and year group, and a few tags. Standards are stored as keyword values so they can be filtered exactly. Text fields use an analyzer suited to the teaching language of the school. Keep the mapping explicit; do not let dynamic mapping guess types for new fields.

## Ranking

Ranking is a mix of filtering and scoring. Standards the student has not yet met are the hard filter. Difficulty is used to prefer exercises just above the student's current level. The scikit-learn model that estimates student progress runs outside the query path and writes its output to the database. Django reads that output and builds the query. Elasticsearch does not run the model.

## Indexing and updates

Celery tasks keep exercise-index in sync with the database. When an exercise is created, edited or retired, a task updates the matching document. A periodic task checks for drift between the database and the index and repairs it. Tasks should be idempotent, since Celery may retry them. Indexing uses the exercise's database identifier as the document id, so a retry overwrites instead of duplicating.

## Versioning and reindex

The `-v3` suffix is the mapping version. Changing a mapping in a way Elasticsearch cannot apply in place means creating a new versioned index, reindexing into it, and switching over. Application code should reach the index through an alias or a single setting, not a hard-coded name scattered across modules, so the switch is one change.

## Gotchas

- A single shard means no parallelism inside the index. That is fine at current size, but watch query latency if the catalogue grows.
- Do not rely on dynamic mapping when adding fields; a wrong guessed type forces a reindex.
- Scores are only comparable within one query. Do not store them or compare across queries.
- If search results look stale, check the Celery sync tasks before suspecting Elasticsearch.

## Open questions

- Whether to add synonyms for curriculum terms that teachers phrase differently.
- Whether a second index version is worth it for multi-language exercise text, or whether per-language fields in the same index are enough.
