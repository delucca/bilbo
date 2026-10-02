# Spec Delta

## MODIFIED Requirements

### Requirement: Query words
A word SHALL be a run of Unicode letters and digits, compared without case and with Latin accents removed; words shorter than 2 characters are ignored. A passage matches when it holds at least one query word, in its text or its heading path, or, with an embedder, when the Meaning ranking requirement admits it. A query with no word of 2 or more characters SHALL be a usage error.

#### Scenario: Case and accents are ignored
- **WHEN** a note says `Decisão tomada` and an agent runs `bilbo recall DECISAO`
- **THEN** that note is a hit

#### Scenario: Words are whole words
- **WHEN** no embedder is configured, the only note mentions `notes` and never `note`, and an agent runs `bilbo recall note`
- **THEN** nothing matches

#### Scenario: A query without words is a usage error
- **WHEN** an agent runs `bilbo recall "- ?"`
- **THEN** bilbo prints a message saying the query has no words to stderr and exits 2

### Requirement: Ranking
Hits SHALL be ordered by how well their best passage matches. Without an embedder, a passage that holds more of the query words, holds rarer words or holds them more densely ranks higher. With an embedder, the keyword order and the meaning order are fused, so a passage near the top of either ranks high and one near the top of both ranks higher. A note SHALL appear at most once, at its best passage. Equal matches are ordered by path, then line.

#### Scenario: More query words rank higher
- **WHEN** no embedder is configured, `plan-a.md` has a passage holding `embedder` and `timeout`, `plan-b.md` holds only `timeout`, and an agent runs `bilbo recall embedder timeout`
- **THEN** the block for `plan-a.md` comes first

#### Scenario: Agreement beats one signal
- **WHEN** an embedder is configured, `plan-a.md` is first by keywords and second by meaning, and `plan-b.md` is first by meaning and holds no query word
- **THEN** the block for `plan-a.md` comes first and `plan-b.md` is still printed

#### Scenario: One block per note
- **WHEN** three passages of `gotcha-slots.md` hold the query word
- **THEN** stdout has one block for `gotcha-slots.md`

#### Scenario: Ties fall back to path order
- **WHEN** `plan-b.md` and `plan-a.md` hold the query word in passages that match equally well
- **THEN** the block for `plan-a.md` comes first

## ADDED Requirements

### Requirement: Meaning ranking
With an embedder configured, `recall` SHALL embed `embedder.query_prefix` followed by the query, cut to 2,000 bytes, and rank the cached passages by cosine similarity to it. Only passages with a similarity of at least `embedder.min_similarity` SHALL enter the meaning order, so a passage can be a hit without sharing a word with the query.

#### Scenario: A paraphrase is found
- **WHEN** `decision-note-store.md` says `one flat folder` with no word of the query, its cached vector has similarity 0.6 to the query's, and an agent runs `bilbo recall where do notes live`
- **THEN** stdout has a block for `decision-note-store.md`

#### Scenario: A weak similarity is not a hit
- **WHEN** the only passage near the query has similarity 0.2, `embedder.min_similarity` is 0.35 and no passage holds a query word
- **THEN** nothing matches and the exit code is 1

### Requirement: Keyword fallback
When the embedder cannot embed the query within 5 seconds, or answers with an error or a malformed body, `recall` SHALL rank by keywords alone, print one line `bilbo: embedder unavailable (<reason>); keyword results only` to stderr, and otherwise behave as without an embedder. Passages with no cached vector for the configured model SHALL rank by keywords alone, and `recall` SHALL then print `bilbo: <n> passages not indexed; run bilbo index` to stderr.

#### Scenario: The embedder is down
- **WHEN** an embedder is configured but nothing listens at its URL, and a note holds `rollback`
- **THEN** `bilbo recall rollback` prints that note's block, stderr holds one line starting with `bilbo: embedder unavailable`, and the exit code is 0

#### Scenario: A note written after the last index
- **WHEN** `bilbo new` created a note with one passage after the last `bilbo index`, and it holds `rollback`
- **THEN** `bilbo recall rollback` prints its block and stderr holds `bilbo: 1 passages not indexed; run bilbo index`

#### Scenario: No embedder, no warnings
- **WHEN** no embedder is configured and a note holds `rollback`
- **THEN** `bilbo recall rollback` leaves stderr empty
