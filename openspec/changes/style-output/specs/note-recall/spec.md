## MODIFIED Requirements

### Requirement: Keyword fallback
When the embedder cannot embed the query within 5 seconds, or answers with an error or a malformed body, `recall` SHALL rank by keywords alone, print one line `bilbo: embedder unavailable (<reason>); keyword results only` to stderr, and otherwise behave as without an embedder. Passages that hold text and have no cached vector for the configured model SHALL rank by keywords alone, and `recall` SHALL then print `bilbo: <n> passages not indexed; run bilbo index` to stderr, with `passage` when `<n>` is 1, where `<n>` counts the distinct embedder inputs of the whole store. Passages that `bilbo index` withholds under the `note-index` spec's Withheld passages SHALL rank by keywords alone and SHALL NOT count in `<n>`. When no passage has a vector, `recall` SHALL NOT send the query to the embedder.

#### Scenario: The embedder is down
- **WHEN** an embedder is configured but nothing listens at its URL, and a note holds `rollback`
- **THEN** `bilbo recall rollback` prints that note's block, stderr holds one line starting with `bilbo: embedder unavailable`, and the exit code is 0

#### Scenario: A note written after the last index
- **WHEN** `bilbo new` created a note with one passage after the last `bilbo index`, and it holds `rollback`
- **THEN** `bilbo recall rollback` prints its block and stderr holds `bilbo: 1 passage not indexed; run bilbo index`

#### Scenario: A store never indexed
- **WHEN** an embedder is configured, `bilbo index` never ran and the store holds two passages, one of them holding `rollback`
- **THEN** `bilbo recall rollback` sends no request to the embedder, prints that note's block, and stderr is `bilbo: 2 passages not indexed; run bilbo index`

#### Scenario: No embedder, no warnings
- **WHEN** no embedder is configured and a note holds `rollback`
- **THEN** `bilbo recall rollback` leaves stderr empty

#### Scenario: Withheld passages are not reported
- **WHEN** `embedder.url = http://embedder.example:8081`, `scope.work.embedder = local`, `bilbo index` ran after the last change, and a note with `scope: work` holds `rollback`
- **THEN** `bilbo recall rollback` prints that note's block and stderr holds no `not indexed` line

## ADDED Requirements

### Requirement: Human view of note hits
When stdout gets the `cli` spec's human view, `bilbo recall` SHALL print per hit, best first, a blank line between hits: a title line of the rank, right-aligned in at least two columns, then two spaces, the note's title in bold and each heading below it after `›`; up to three lines of the passage, two when the width is under 60, with Markdown markup removed, starting at the sentence of the first query word when the lines would not reach it, and the query words in bold; and a dim line of the kind, the scope when the config declares two or more scopes, the age, `by meaning` when no query word is in the passage's heading path or text, and the note's path from `~/` with its line. The passage and dim lines SHALL be indented to the title. A blank line and a dim count line SHALL close the list: `<n> notes, best first` (`1 note` for one), or, when `--limit` left hits out, `<shown> of <total> notes, best first · --limit <total> shows all`. When nothing matches, stderr SHALL say `no notes match '<query>'` after `○`, and a hint line SHALL follow.

#### Scenario: Two hits on a terminal
- **WHEN** two notes hold `rollback`, created 5 days ago and 17 minutes ago, and a user runs `bilbo recall rollback` in a terminal
- **THEN** the first line starts with ` 1` and holds the best note's title, each hit's last line holds its kind, an age such as `5 days ago` or `17 min ago` and a path starting with `~/`, and the last stdout line is `2 notes, best first`

#### Scenario: A limit that cut the list
- **WHEN** five notes hold `rollback` and a user runs `bilbo recall --limit 2 rollback` in a terminal
- **THEN** the last stdout line is `2 of 5 notes, best first · --limit 5 shows all`

#### Scenario: A match in a heading only
- **WHEN** a note's heading `Rollback plan` sits over a passage without the word, an embedder is configured, and a user runs `bilbo recall rollback` in a terminal
- **THEN** that hit's dim line does not hold `by meaning`

#### Scenario: A long passage starts near the match
- **WHEN** a note's passage holds 900 characters before its first `rollback` and a user runs `bilbo recall rollback` in a terminal 100 columns wide
- **THEN** the hit's first passage line starts with `… ` and the passage lines hold `rollback`

#### Scenario: Nothing matches on a terminal
- **WHEN** no note holds `wumpus` and a user runs `bilbo recall wumpus` in a terminal
- **THEN** stdout is empty, stderr's first line is `○`, two spaces and `no notes match 'wumpus'`, a hint line follows indented three columns, and the exit code is 1

#### Scenario: A pipe keeps the blocks
- **WHEN** a user runs `bilbo recall rollback | cat` in a terminal
- **THEN** stdout is the three-line blocks of Recall prints the best passages, with no rank, count line or escape byte
