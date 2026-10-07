# note-recall Specification

## Purpose
`bilbo recall` finds the notes in the store that match a query and points an agent at the best passage of each, so a note written in one session can be found in the next without knowing its filename.

## Requirements

### Requirement: Recall prints the best passages
`bilbo recall <query>...` SHALL print one block per matching note to stdout, best first, blocks separated by one blank line, and exit 0. A block is three lines: `<absolute path>:<line>`, a tab, the kind, a tab and the note's `created`; then the heading path of the best passage; then its snippet. The query is the words after the verb, joined by single spaces.

#### Scenario: One matching note
- **WHEN** the store holds `notes/decision-note-store.md`, created `2026-10-02T14:23-03:00`, whose `## Layout` section under `# Note store` starts at line 9 and mentions `flat`, and an agent runs `bilbo recall flat layout`
- **THEN** stdout is `<root>/notes/decision-note-store.md:9`, a tab, `decision`, a tab, `2026-10-02T14:23-03:00`, then `Note store > Layout`, then the snippet, and the exit code is 0

#### Scenario: Unquoted words form one query
- **WHEN** an agent runs `bilbo recall note store` and then `bilbo recall "note store"`
- **THEN** both print the same output

#### Scenario: A note without the words is not printed
- **WHEN** `plan-a.md` holds `rollback` and `plan-b.md` does not, and an agent runs `bilbo recall rollback`
- **THEN** stdout has a block for `plan-a.md` and none for `plan-b.md`

### Requirement: Passages
A note's body SHALL be read as passages: each heading outside fenced code blocks starts one, and it runs to the next such heading. A passage's heading path is the titles of its enclosing headings joined by ` > `, starting with the note's `#` title. A passage longer than 4,000 bytes SHALL be split at blank lines into parts of at most 4,000 bytes where possible; `<line>` is the line where the matching passage or part starts.

#### Scenario: A nested heading path
- **WHEN** the best match is under `### Two slots` inside `## Gotchas` of the note titled `# Embedder`
- **THEN** the heading path line is `Embedder > Gotchas > Two slots`

#### Scenario: A heading inside a code fence opens no passage
- **WHEN** a code fence in the `## Setup` passage holds the line `# install deps`
- **THEN** that line belongs to the `## Setup` passage and never appears in a heading path

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
Hits SHALL be ordered by how well their best passage matches. Without an embedder, a passage that holds more of the query words, holds rarer words or holds them more densely ranks higher. With an embedder, the keyword order and the meaning order are fused, so a passage near the top of either ranks high and one near the top of both ranks higher. Fusion SHALL consider the first 50 passages of each order; passages that hold a query word but rank past the keyword order's 50th SHALL follow the fused hits in keyword order. A note SHALL appear at most once, at its best passage. Equal matches are ordered by path, then line.

#### Scenario: More query words rank higher
- **WHEN** no embedder is configured, `plan-a.md` has a passage holding `embedder` and `timeout`, `plan-b.md` holds only `timeout`, and an agent runs `bilbo recall embedder timeout`
- **THEN** the block for `plan-a.md` comes first

#### Scenario: Agreement beats one signal
- **WHEN** an embedder is configured, `plan-a.md` is first by keywords and second by meaning, and `plan-b.md` is first by meaning and holds no query word
- **THEN** the block for `plan-a.md` comes first and `plan-b.md` is still printed

#### Scenario: Keyword matches past 50 still print
- **WHEN** an embedder is configured, 60 notes hold `rollback` and an agent runs `bilbo recall rollback --limit 60`
- **THEN** stdout has 60 blocks

#### Scenario: One block per note
- **WHEN** three passages of `gotcha-slots.md` hold the query word
- **THEN** stdout has one block for `gotcha-slots.md`

#### Scenario: Ties fall back to path order
- **WHEN** `plan-b.md` and `plan-a.md` hold the query word in passages that match equally well
- **THEN** the block for `plan-a.md` comes first

### Requirement: What recall searches
Without `--library` and `--corpus`, recall SHALL search every entry of `<root>/notes/` that is a regular file, not hidden, with a valid `<kind>-<topic>.md` name, skipping other entries and files it cannot read in silence, and SHALL NOT read `<root>/library/`. Frontmatter is not searched. A note that breaks other `note-store` rules SHALL still be searched; when its `created` is not valid, the block shows `-` in its place. A note with no `#` title uses its filename without `.md` as the title. With `--library` or `--corpus`, recall SHALL search the library instead, as the `library-recall` spec says, and of this spec only the Passages, Query words, Limit, Options and the query, Snippet and Recall is read-only requirements apply to it, besides what Kind filter and A missing store say about those options.

#### Scenario: Frontmatter is not searched
- **WHEN** the only occurrence of `github` in the store is a `sources` item
- **THEN** `bilbo recall github` matches nothing

#### Scenario: A note with a bad timestamp is still found
- **WHEN** `plan-release.md` has `created: 2026-10-02` and mentions `rollback`
- **THEN** `bilbo recall rollback` prints its block with `-` as the created field

#### Scenario: A badly named file is skipped
- **WHEN** `notes/idea-foo.md` mentions `rollback` and no note does
- **THEN** `bilbo recall rollback` matches nothing

#### Scenario: Sources are not notes
- **WHEN** `<root>/notes/` holds notes and only `library/go/effective-go.md` mentions `rollback`
- **THEN** `bilbo recall rollback` prints `bilbo: no notes match` to stderr and exits 1

### Requirement: Kind filter
`--kind <kind>` SHALL limit the hits to notes of that kind, and MAY be given several times to allow several kinds. An unknown kind SHALL be a usage error that lists the kinds. `--kind` given with `--library` or `--corpus` SHALL be a usage error that names `--kind` and `--library`, since the library has no note kinds.

#### Scenario: Only the asked kind
- **WHEN** a `plan` and a `decision` both match and an agent runs `bilbo recall rollback --kind decision`
- **THEN** only the `decision` note is printed

#### Scenario: An unknown kind
- **WHEN** an agent runs `bilbo recall rollback --kind idea`
- **THEN** bilbo prints a message naming `idea` and listing the kinds to stderr and exits 2

#### Scenario: A kind with the library
- **WHEN** an agent runs `bilbo recall goroutine --library --kind reference` or `bilbo recall goroutine --corpus go --kind reference`
- **THEN** bilbo prints a message naming `--kind` and `--library` to stderr, exits 2 and prints nothing to stdout

### Requirement: Limit
`--limit <n>` SHALL print at most `n` blocks; without it, at most 10. `n` SHALL be a whole number of 1 or more, or the run is a usage error.

#### Scenario: The default limit
- **WHEN** 15 notes match
- **THEN** stdout has 10 blocks

#### Scenario: A zero limit is refused
- **WHEN** an agent runs `bilbo recall rollback --limit 0`
- **THEN** bilbo exits 2 and stdout is empty

### Requirement: Options and the query
Options SHALL be accepted before or after the query words. `--kind=<kind>`, `--limit=<n>` and `--corpus=<corpus>` SHALL be the same as `--kind <kind>`, `--limit <n>` and `--corpus <corpus>`. `--library` takes no value, and `--library=<anything>` SHALL be a usage error. An argument `--` SHALL end the options, so every argument after it is a query word, even one starting with `-`. Any other argument before `--` that starts with `-` followed by a character other than whitespace SHALL be a usage error.

#### Scenario: A word that looks like an option
- **WHEN** an agent runs `bilbo recall -- --title flag`
- **THEN** the query is `--title flag` and bilbo does not report an unknown option

#### Scenario: An unknown option
- **WHEN** an agent runs `bilbo recall rollback --json`
- **THEN** bilbo prints a message naming `--json` as unknown to stderr and exits 2

#### Scenario: An option with its value after `=`
- **WHEN** an agent runs `bilbo recall rollback --kind=decision --limit=1`
- **THEN** stdout is the same as for `bilbo recall rollback --kind decision --limit 1`

#### Scenario: Library options anywhere
- **WHEN** an agent runs `bilbo recall --library --corpus=go wrapping errors` and then `bilbo recall wrapping errors --corpus go`
- **THEN** both print the same output

#### Scenario: A value for --library
- **WHEN** an agent runs `bilbo recall wrapping --library=go`
- **THEN** bilbo prints a message naming `--library` to stderr and exits 2

### Requirement: Snippet
The snippet SHALL be the matching passage's text without its heading line, with every run of whitespace turned into one space, cut to its first 300 characters, never inside a character. An empty snippet SHALL be printed as `-`.

#### Scenario: A multi-line passage
- **WHEN** the matching passage is two paragraphs of 500 characters in all
- **THEN** the snippet is one line of 300 characters

#### Scenario: A short passage is not padded
- **WHEN** the matching passage is the single line `Use two slots.`
- **THEN** the snippet is `Use two slots.`

#### Scenario: A passage with no text
- **WHEN** the best match is a heading with no text under it
- **THEN** the snippet line is `-`

### Requirement: Nothing matches
When no note matches, `bilbo recall` SHALL print `bilbo: no notes match` to stderr, leave stdout empty and exit 1.

#### Scenario: An empty result
- **WHEN** no note holds any query word and an agent runs `bilbo recall wumpus`
- **THEN** stdout is empty, stderr is `bilbo: no notes match` and the exit code is 1

### Requirement: A missing store
Without `--library` and `--corpus`, when `<root>/notes/` does not exist, `bilbo recall` SHALL print `bilbo: no store at <root>` to stderr and exit 1. With either option, a missing `<root>/notes/` SHALL NOT stop recall, and the `library-recall` spec's Nothing in the library requirement applies instead.

#### Scenario: Wrong BILBO_HOME
- **WHEN** `BILBO_HOME` names a folder with no `notes/` inside it and an agent runs `bilbo recall rollback`
- **THEN** stderr is `bilbo: no store at <that folder>`, stdout is empty and the exit code is 1

#### Scenario: A library search without notes
- **WHEN** `<root>/notes/` does not exist, `<root>/library/go/` holds a source that mentions `rollback`, and an agent runs `bilbo recall rollback --library`
- **THEN** stderr does not say `no store`, and the source's block is printed

### Requirement: Recall is read-only
`bilbo recall` SHALL NOT create, change, rename or delete any file or folder.

#### Scenario: The store is left as found
- **WHEN** `bilbo recall` runs on a store
- **THEN** every entry under the root has the same bytes and modification time as before the run, and no new entry exists

### Requirement: Meaning ranking
With an embedder configured, `recall` SHALL embed `embedder.query_prefix` followed by the query, cut to 2,000 bytes, and rank the cached passages that hold text by cosine similarity to it. Only passages with a similarity of at least `embedder.min_similarity` SHALL enter the meaning order, so a passage can be a hit without sharing a word with the query.

#### Scenario: A paraphrase is found
- **WHEN** `decision-note-store.md` says `one flat folder` with no word of the query, its cached vector has similarity 0.6 to the query's, and an agent runs `bilbo recall where do notes live`
- **THEN** stdout has a block for `decision-note-store.md`

#### Scenario: A weak similarity is not a hit
- **WHEN** the only passage near the query has similarity 0.2, `embedder.min_similarity` is 0.35 and no passage holds a query word
- **THEN** nothing matches and the exit code is 1

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
