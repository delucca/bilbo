---
id: 01KVS4RH5Q3W5P5ZYJ6W35THEE
created: 2026-06-22T23:25-03:00
---

# exercise-index: edge_ngram title analysis for prefix search

We decided that exercise-index analyzes exercise titles with an edge_ngram filter with min_gram 3, so that teachers find exercises by typing a prefix. This note records the decision, the reasons, what we gave up, and what to check if someone wants to change it later. It is written for whoever touches the index mapping, the search endpoint on the Django side, or the Vue.js search box next.

Short version: titles in exercise-index are broken into leading fragments at index time. A teacher who types the start of a word gets hits without finishing it. The fragments start at a length set by min_gram 3, so nothing shorter than three characters is ever indexed as a prefix. The query side does not apply the same edge_ngram filter, which matters and is explained below.

## Context

ClassroomCompass tracks student progress against curriculum standards and suggests next exercises. Teachers in secondary schools use it. The suggestion side is driven by scikit-learn models that run in Celery jobs and write ranked candidates back for each student or group. The search side is separate: a teacher is planning a lesson or a homework set, wants a particular exercise, and types into a box in the Vue.js front end. That box talks to a Django view, which queries Elasticsearch, and the index it queries is exercise-index.

The pattern we kept seeing in feedback was that teachers do not remember full titles. They remember the first part of a name, or the topic word, or something that sounds like the beginning of it. They type a few letters and expect a list to narrow as they go. Plain whole-word matching fails this: typing the first letters of a word returns nothing until the word is complete. Teachers read that as the exercise not existing, and then they either build a duplicate or give up. Neither is good. Duplicates in particular pollute the progress data, because attempts get split across near-identical exercises and the standards coverage view then looks thinner than it is.

So the requirement was narrow: typing a prefix of a word in a title must find the exercise. Everything else about search relevance was secondary and is covered in other places.

## Decision

exercise-index analyzes titles with an edge_ngram filter with min_gram 3. Concretely:

- At index time the title is tokenized into words, lowercased, and each word is expanded into its leading fragments, starting at the length given by min_gram 3 and growing up to a configured upper bound.
- At search time the teacher's input is tokenized and lowercased with the same tokenizer and case handling, but it is not expanded into fragments. The typed text is matched as-is against the stored fragments.
- Words shorter than the minimum are kept whole by the analysis chain, so short words in a title are still findable once typed completely. They are not findable by a shorter prefix, since a prefix of a word that short is below the floor anyway.

The choice of min_gram 3 is the key parameter. The edge_ngram filter itself is the mechanism; the number is the policy. If someone says exercise-index uses n-grams, the precise statement is that it uses edge n-grams only, anchored at the start of each word, never n-grams from the middle of a word.

## Why edge_ngram and not the alternatives

We looked at four approaches before settling.

First, a prefix query at search time. It works without any special analysis, and the index stays small. The problem is cost and control. Prefix queries run against the term dictionary and get slower as the vocabulary grows, and they do not play well with the relevance scoring we want, where an exact word match should beat a prefix match. They also made it awkward to combine with the filters teachers use at the same time, such as year group and curriculum standard. It was workable for a prototype and we did not want it as the permanent answer.

Second, a completion suggester. It is fast and made for this kind of typing. We rejected it because it wants a separate field with its own input list, it is weak at matching a prefix of the second or third word in a title, and teachers routinely type a middle word because the title starts with a generic verb or a shared phrase. The suggester also does not share filtering and scoring with the main query, so the results would not agree with the full results page.

Third, a wildcard query with a leading fragment. Same cost problem as prefix, worse in practice, and easy to misuse from the Django layer with unescaped input.

Fourth, edge_ngram at index time. This moves the cost to indexing. Queries become ordinary term matches, which are fast and score the way every other match does. It also composes with filters and with the existing relevance tuning without special cases. The price is a larger index and slower reindexing, which we accepted because the exercise catalogue is small by search standards and changes mostly in batches.

That is why the decision landed on edge_ngram. The reasoning that carried it was that search-time simplicity and consistent scoring mattered more than index size for a catalogue of this scale.

## Why min_gram 3

The floor is a trade between noise and responsiveness.

A floor lower than three means one- and two-letter fragments are indexed for every word. Those fragments match a huge share of the catalogue. A teacher typing a single letter would get a long, mostly meaningless list, and the front end would spend effort rendering it. Worse, those short fragments bloat the index for no real benefit, since nobody expects a useful answer from one letter. They also distort scoring, because very common tiny terms interact badly with the scoring that rewards rare terms.

A floor higher than three would mean the teacher has to type more before anything appears. Titles in the catalogue often start with short, meaningful words and topic stems, and several of the common curriculum topic words are short. A higher floor would make some real exercises unreachable until the teacher typed most of a word. Teachers on a phone or in the middle of a lesson will not tolerate that.

Three characters is the point where a fragment starts to be specific enough to narrow the list and where typing feels immediate. We chose it by trying real teacher-style input against a sample of titles and watching when the lists became useful. We did not do a rigorous study; this was judgment backed by a handful of sessions with teachers and by our own testing. If someone wants to revisit it, they should bring comparable evidence, not a general preference.

The name to remember is min_gram 3. It must stay in the mapping as written and the note about the decision should keep the same wording, so a text search for it finds both.

## Consequences and trade-offs

The good side: prefix typing works, results are scored like any other match, filters combine without special handling, and the Vue.js box can call the same endpoint for as-you-type and for submitted searches.

The costs we accepted:

- Index size grows. Every word in every title produces several stored fragments. For this catalogue that is fine. If the catalogue grows by a large factor, or if other fields start using the same analysis, revisit.
- Reindexing takes longer than before. Anyone changing the mapping has to rebuild exercise-index into a new index and switch over, because analysis settings cannot be changed in place on a live index. This is not specific to our choice, but it makes any later change to min_gram 3 a rebuild, not a tweak.
- Fewer than three typed characters returns nothing useful from the fragment match. The front end should not fire a search until the input reaches the floor, and it should say nothing rather than show an empty result that looks like failure. This coupling between the Vue.js box and the index setting is easy to forget. If the floor changes, the front end threshold must change with it.
- Matching is anchored at word starts. A teacher who types the end or the middle of a word will not find it. That was a deliberate limit, because the requirement was prefix search. Teachers have not asked for more so far.
- Fragments can produce unexpected matches across unrelated words that share a start. That is inherent to prefix matching and is tolerated. Ranking puts whole-word matches ahead of fragment-only matches where the query is built to do so.

## Gotchas

The most common mistake with edge_ngram is applying the same filter at search time. If the query text is also expanded into fragments, a short typed word matches almost everything that shares any fragment with it, and relevance collapses. The search analyzer for the title field must differ from the index analyzer: the index side includes edge_ngram, the search side does not. When reading the mapping, check that a separate search analyzer is set. If someone simplifies the mapping by removing it, results get noisy immediately, and the symptom is that every query seems to return a lot of loosely related exercises.

Second, test data. Local development against a tiny sample hides most of these issues. A mapping that looks right can behave differently once the vocabulary is realistic. Test with a realistic title set when changing anything about analysis.

Third, language. Titles are mostly in the teaching language of the school, and some contain accented characters. The analysis chain must fold or preserve accents consistently on both the index and search sides. A mismatch there shows up as prefixes that work for plain words and fail for words with accents. This is a separate concern from the edge_ngram decision, but the two are tangled in practice because both sit in the same analyzer definition.

Fourth, the Celery side. Reindexing jobs that run in Celery must write to the same index alias the search view reads from. During a rebuild, the new index is filled first and the alias is moved when it is complete. If a worker still writes to the old concrete index after the switch, new exercises will not appear in search. When changing the mapping, restart or drain the workers that touch exercise-index before declaring it done.

Fifth, the scikit-learn suggestion code does not use this analysis and should not. Suggested exercises come from the models and are looked up by identifier. Do not route suggestions through the title search to resolve them. Mixing the two would make suggestions depend on a tuning that exists for human typing.

## How to change it

If you need to change the floor or the analysis chain, treat it as a rebuild.

- Write the new analysis settings and mapping on a new index, leaving the live one alone.
- Populate the new index from the source of truth in the database, through the existing reindex task, not by copying documents from the old index. Copying would carry over any drift.
- Compare behaviour on a set of real teacher queries between old and new before switching. Pay attention to the shortest inputs and to titles that begin with short words.
- Move the alias to the new index once it is complete, then drain workers, then remove the old index after a safe interval.
- Update the front end threshold in the Vue.js search box so it matches the new floor.
- Update this note: change the decision wording, and keep the old value stated as superseded so a later reader sees the history.

If someone wants to drop edge_ngram entirely, the realistic fallbacks are the prefix query or the completion suggester described above. Weigh them against the reasons given there before proposing it, and expect the filter and scoring concerns to come up again.

## Open questions

A few things are not settled and should not be treated as decided by this note.

Whether the upper bound of the fragment length should be tied to the longest word we expect in a title. For now it is set generously enough that long topic words are covered. A very long compound word may be cut off, and a teacher typing most of such a word could miss it. We have not seen a complaint, but nobody has looked for one systematically.

Whether other fields, such as the curriculum standard names or tags, should get the same treatment. Teachers sometimes search by standard. That probably wants a different design, since standard names are structured and often searched by code, and a code behaves differently from natural words under prefix analysis. Keep that out of this decision.

Whether to expose a minimum-length message in the interface instead of silent waiting. This is a front end question and belongs with whoever owns the search box.

Whether the cost of the larger index will matter once the catalogue grows. Keep an eye on index size and reindex time as exercises are added. If either becomes a problem, the first thing to examine is the upper bound of the fragments, not the floor.

## Where to look

The index settings and mapping for exercise-index live in the Django project, in the search app, next to the reindex task used by Celery. The search view that builds the query is in the same app. The Vue.js search box has the input threshold, and it should be read together with the mapping whenever either is changed. This note does not give exact file paths because they were not re-read when writing it; open the search app and follow the reindex task to find the settings.

When reviewing a change that touches exercise-index, ask three things: does the index analyzer still include edge_ngram with min_gram 3, does the search analyzer still differ from it, and does the front end still wait for the same number of characters. If all three agree, the decision still holds.
