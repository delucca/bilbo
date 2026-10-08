---
id: 01JS4AVNW3JEJ6SXRXDE2WADKB
created: 2025-04-18T08:34-03:00
---

# standards-ingest spec

standards-ingest is the part of ClassroomCompass that pulls curriculum standards into the system so the rest of the product can track student progress against them and suggest next exercises. This note records what it has to do, what it must not do, and the one hard performance requirement we have agreed on. Details that are not settled are left general on purpose.

## Purpose

Teachers at secondary schools see progress per standard. For that to work, every national framework we support has to exist inside ClassroomCompass as a structured tree of standards: strands, topics, individual statements, and the links between them. standards-ingest reads a framework from its published source, normalises it, and stores it so that the Django app, the recommender and search can all use the same data.

It is not a general document importer. It handles curriculum frameworks only.

## Performance requirement

standards-ingest must complete a full import of one national framework within 20 minutes.

That is the budget for the whole run, from the moment the import is started to the moment the framework is searchable and usable by the recommender. It covers fetching, parsing, validation, writing to the database, and indexing. It is measured per framework, for a full import, not for incremental updates. Partial or incremental updates should be much faster, but no separate number is set for them yet.

If a run goes over the budget, treat it as a defect to investigate, not as something to retry quietly. Slow runs block teachers from seeing newly published standards, and they hold up Celery workers that other jobs need.

## Pipeline shape

The import runs as a chain of Celery tasks so that no single step has to hold the whole framework in memory.

1. Fetch the source material for the framework.
2. Parse it into a neutral internal structure, one record per standard.
3. Validate the records: required fields present, parent links resolve, no duplicate identifiers inside one framework.
4. Write the records to the Django models in batches, inside transactions that are small enough to roll back cleanly.
5. Index the standards in Elasticsearch so teachers can search them from the Vue.js front end.
6. Mark the framework version as active only after all of the above succeeded.

Steps 2 to 5 can be split across workers by strand. The 20 minute budget is the reason for that split: a single sequential worker is not expected to make it for the largest frameworks.

## Data handling rules

- Each import creates a new framework version. Older versions stay in the database so existing student progress still points at valid standards.
- Standards keep a stable identifier from the source where the source gives one. Where it does not, we derive one in a repeatable way, so that re-importing the same content gives the same identifiers.
- A failed import must leave the previously active version untouched. Nothing partial is ever visible to teachers.
- Validation problems are collected and reported together at the end of the validate step, not one at a time, so that a person can fix a source issue in one pass.

## Interaction with other components

The recommender uses scikit-learn features built from the standards text and the relations between standards. When a new framework version becomes active, those features need rebuilding. That rebuild is a separate job and is not counted in the import budget, but standards-ingest should trigger it once the version is active.

Elasticsearch indexing is part of the import budget. If indexing is the slow part, batch size and refresh settings are the first things to look at, before anything in the parsing code.

## Measuring and checking the budget

Time the run end to end and also per step, so that when the total creeps up we can see which step caused it. A rough way to record the requirement in a test or a config comment:

```python
# full import of one national framework: budget is 20 minutes
IMPORT_BUDGET = "20 minutes"
```

Timings should be taken on a framework of realistic size, not a toy sample. Run the check on hardware similar to production workers, since laptop numbers have misled us before.

## Open questions

- Which frameworks are in scope for the first release, and which one is the largest, so we can use it as the reference for timing.
- Whether incremental updates need their own time limit.
- How we alert someone when a run goes over the budget: a log line is the minimum, a notification to the team is better.
- How to handle a source that changes shape without warning. For now the validate step should fail loudly rather than guess.
