---
id: 01K7YZCQXVM48E4F3GJ3S7H2XY
created: 2025-10-19T16:33-03:00
---

# standards-ingest weekly recap

Most of this week went into standards-ingest, mainly on making the import of curriculum standards less fragile and easier to reason about. Nothing here is finished in the sense of being shipped everywhere, but the direction is clearer than it was at the start of the week. These are my notes written quickly, so some parts are rough.

## Where things stood at the start

standards-ingest takes curriculum standard documents supplied by exam boards and education departments, turns them into structured records, and hands them to the rest of ClassroomCompass. The progress tracking and the exercise suggestions both depend on those records being clean. When the ingest misbehaves, teachers see odd gaps in their class views, or exercises that point at a standard that no longer exists.

The Django side holds the models and the admin screens. Celery runs the actual import jobs. Elasticsearch gets the indexed standards so teachers can search them. The Vue.js front end only reads the result, but it is where people notice problems first.

Going into the week, the known pain points were these: imports that stop halfway and leave partial data, inconsistent handling of how standards are numbered across sources, and an index that sometimes lagged behind the database without anyone being told.

## Import jobs and partial failures

The biggest chunk of time went on what happens when an import job fails in the middle. Before, a failure could leave some standards written and others not, and the next run would sometimes duplicate or skip records. I spent a good while reading the task code to see exactly where the writes happen relative to the parsing.

The change I am leaning toward is to parse and validate the whole source first, collect the problems, and only then write inside a single database transaction. If validation turns up problems, nothing is written and the job reports what it found. I started restructuring the task along those lines. The parsing part is separated out now and can be called without touching the database, which also makes it far easier to test.

Still open: very large sources may make a single transaction uncomfortable. I have not measured this and I do not want to guess. Next week I want to try it against the largest source we have and see how it behaves before deciding whether batching is needed.

## Identifiers and numbering across sources

Different sources number their standards in different ways. Some use nested dotted codes, some use flat codes, and a few reuse the same code in different year groups. The existing matching logic leans on the code alone, which is why re-imports occasionally merge two standards that are not the same thing.

I wrote down the cases I found and started on a matching approach that uses the code together with the source and the level it belongs to. This is not wired in yet. The thing I keep worrying about is the migration of existing records: if the matching key changes, old records have to be mapped onto the new key without orphaning the student progress that points at them. That is the part that could hurt teachers, so I am treating it as the careful step and not rushing it.

## Handling changes between versions of a source

Sources get revised. A standard can be reworded, split, merged, or withdrawn. The ingest currently treats a revised source mostly as a fresh import with updates, and it has no good way to say that a standard was withdrawn. I sketched how withdrawal should look: the record stays, it is marked as retired, and progress already recorded against it is kept and still visible, but it stops being offered in new suggestions.

Splits and merges are harder. I do not have a clean rule for them yet. For now the plan is to flag them for a human to review in the admin screens instead of trying to infer the mapping automatically. I would rather have a short review queue than silently wrong progress data.

## Celery behaviour

I looked at how the import tasks are retried. Some failures are transient, such as the search cluster being briefly unavailable, and some are permanent, such as a malformed source. They were being treated alike, so a bad file could be retried pointlessly while a transient problem could give up too early.

I started separating the two kinds of failure. Transient ones get retried with a delay; permanent ones fail straight away with a readable message stored on the job record. The message part matters because teachers and support staff cannot read worker logs. I have the structure in place and still need to go through the individual error paths and sort each one into the right bucket.

## Elasticsearch indexing

The index lagging behind the database was the other recurring complaint. The cause seems to be that indexing happens as a separate step after the import and its failures are not tied back to the import job. So the job shows as successful while the search results are stale.

What I did this week was small: the indexing step now records its outcome on the same job record, so a stale index shows up as a visible state and not a silent one. I have not yet added a way to rebuild the index for a single source, which is what support will want. I noted it as the next piece here. A full rebuild exists but is heavy and should not be the answer to a small problem.

## Tests and fixtures

Because parsing is now separate from writing, I could add tests that feed in small sample sources and check the parsed output directly. I built a handful of fixtures covering the awkward numbering cases from earlier. I kept them small and made up, not copied from real documents, partly to avoid licensing questions and partly so they are readable.

Gaps in testing: there is nothing yet that exercises the full path through Celery and Elasticsearch together. Those tests would be slower and need more setup. I want at least one end-to-end check before I trust the transaction change, since that is where surprises tend to hide.

## Interaction with the recommender

The scikit-learn part that suggests next exercises reads standards as features. I did not change it, but I checked what it assumes about standards. It assumes that a standard keeps its identity between imports. The numbering work above is therefore not only about tidiness; if identities shift, the model inputs shift with them. I should talk to whoever owns the recommender before the key change goes in, so they are not surprised and can say whether any retraining is needed.

## Loose ends and risks

- The matching key migration is the riskiest item and has no rollback plan written down yet.
- The review queue for splits and merges needs a minimal screen in the admin; nothing exists yet.
- Error messages for permanent failures are still inconsistent in tone and need a pass so they make sense to non-engineers.
- Documentation for how to add a new source is out of date and partly wrong now. I noticed this while working and have not touched it.
- I have not checked how the front end behaves when a standard is retired. It may need a small label so teachers understand why something is no longer suggested.

## Plan for next week

First, run the restructured import against the largest source and see whether the single transaction is acceptable. Second, finish sorting the failure paths into transient and permanent. Third, write the migration plan for the new matching key, including how to check that no progress records were orphaned afterwards, and get a second pair of eyes on it. Fourth, add the single-source reindex. If time is left, the end-to-end test and the documentation fix.

I will keep this note updated as these land; anything that turns out wrong goes in here as a correction and not as a new note.
