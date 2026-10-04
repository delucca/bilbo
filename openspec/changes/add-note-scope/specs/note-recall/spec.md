# Spec Delta

## MODIFIED Requirements

### Requirement: Keyword fallback
When the embedder cannot embed the query within 5 seconds, or answers with an error or a malformed body, `recall` SHALL rank by keywords alone, print one line `bilbo: embedder unavailable (<reason>); keyword results only` to stderr, and otherwise behave as without an embedder. Passages that hold text and have no cached vector for the configured model SHALL rank by keywords alone, and `recall` SHALL then print `bilbo: <n> passages not indexed; run bilbo index` to stderr, where `<n>` counts the distinct embedder inputs of the whole store. Passages that `bilbo index` withholds under the `note-index` spec's Withheld passages SHALL rank by keywords alone and SHALL NOT count in `<n>`. When no passage has a vector, `recall` SHALL NOT send the query to the embedder.

#### Scenario: The embedder is down
- **WHEN** an embedder is configured but nothing listens at its URL, and a note holds `rollback`
- **THEN** `bilbo recall rollback` prints that note's block, stderr holds one line starting with `bilbo: embedder unavailable`, and the exit code is 0

#### Scenario: A note written after the last index
- **WHEN** `bilbo new` created a note with one passage after the last `bilbo index`, and it holds `rollback`
- **THEN** `bilbo recall rollback` prints its block and stderr holds `bilbo: 1 passages not indexed; run bilbo index`

#### Scenario: A store never indexed
- **WHEN** an embedder is configured, `bilbo index` never ran and the store holds two passages, one of them holding `rollback`
- **THEN** `bilbo recall rollback` sends no request to the embedder, prints that note's block, and stderr is `bilbo: 2 passages not indexed; run bilbo index`

#### Scenario: No embedder, no warnings
- **WHEN** no embedder is configured and a note holds `rollback`
- **THEN** `bilbo recall rollback` leaves stderr empty

#### Scenario: Withheld passages are not reported
- **WHEN** `embedder.url = http://bagend:8081`, `scope.work.embedder = local`, `bilbo index` ran after the last change, and a note with `scope: work` holds `rollback`
- **THEN** `bilbo recall rollback` prints that note's block and stderr holds no `not indexed` line
