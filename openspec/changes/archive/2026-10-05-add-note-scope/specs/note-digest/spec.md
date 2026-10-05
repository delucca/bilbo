# Spec Delta

## MODIFIED Requirements

### Requirement: The gate
With an embedder answering within the budget, a note SHALL pass the gate when one of its passages has a similarity of at least `digest.min_similarity` to the query, or when one of its passages that `bilbo index` withholds, under the `note-index` spec's Withheld passages, passes the keyword gate below. Sharing words alone SHALL NOT admit any other passage, and a passage `bilbo index` has not embedded for another reason cannot pass. Otherwise, a note SHALL pass only when one passage holds at least 3 distinct query words of 4 or more letters or digits, in its text or heading path. Words are `recall`'s words. A configured embedder that fails, misses the budget, or finds no passage indexed SHALL leave the keyword gate in charge. When `bilbo index` withholds every passage, the digest SHALL send no query and leave the keyword gate in charge without an error.

#### Scenario: Close in meaning
- **WHEN** an embedder answers, the best passage of `decision-note-store.md` has similarity 0.62 to the query and `digest.min_similarity` is 0.55
- **THEN** `decision-note-store.md` passes the gate

#### Scenario: Shared words do not override a weak similarity
- **WHEN** an embedder answers, a passage of `plan-specs.md` holds the query words `work` and `specs`, and its similarity to the query is 0.41
- **THEN** `plan-specs.md` does not pass the gate

#### Scenario: Keyword gate without an embedder
- **WHEN** no embedder is configured, the prompt is `why does the embedder livelock on long chunks`, and one passage holds `embedder`, `livelock` and `chunks`
- **THEN** that note passes the gate

#### Scenario: Two words are not enough without an embedder
- **WHEN** no embedder is configured and the best passage holds only two of the query's 4-letter words
- **THEN** that note does not pass the gate

#### Scenario: Nothing indexed yet
- **WHEN** an embedder is configured, the vector cache is empty, and one passage holds 3 of the query's 4-letter words
- **THEN** that note passes the gate, and no request reaches the embedder

#### Scenario: A withheld note passes on keywords
- **WHEN** `embedder.url = http://bagend:8081` answers, `scope.work.embedder = local`, `scope.personal.embedder = any`, a note with `scope: personal` is embedded, `bilbo index` ran after the last change, the prompt is `why does the deploy pipeline stall on staging`, and a note with `scope: work` has a passage holding `deploy`, `pipeline` and `staging`
- **THEN** that note passes the gate

#### Scenario: A withheld note needs three words
- **WHEN** the same setup holds, and the work note's best passage holds only `deploy` and `staging`
- **THEN** that note does not pass the gate

#### Scenario: Every passage withheld
- **WHEN** `embedder.url = http://bagend:8081`, `scope.work.embedder = local`, every note has `scope: work`, and one passage holds 3 of the query's 4-letter words
- **THEN** that note passes the gate, no request reaches the embedder, and stderr is empty

#### Scenario: An unembedded note in no local scope still needs meaning
- **WHEN** `embedder.url = http://bagend:8081` answers, no scope sets `embedder = local`, some passages are embedded, and a note written after the last `bilbo index` has a passage holding 3 of the query's 4-letter words
- **THEN** that note does not pass the gate
