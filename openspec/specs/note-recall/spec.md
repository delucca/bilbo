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
A word SHALL be a run of Unicode letters and digits, compared without case and with Latin accents removed; words shorter than 2 characters are ignored. A passage matches when it holds at least one query word, in its text or its heading path. A query with no word of 2 or more characters SHALL be a usage error.

#### Scenario: Case and accents are ignored
- **WHEN** a note says `Decisão tomada` and an agent runs `bilbo recall DECISAO`
- **THEN** that note is a hit

#### Scenario: Words are whole words
- **WHEN** the only note mentions `notes` and never `note`, and an agent runs `bilbo recall note`
- **THEN** nothing matches

#### Scenario: A query without words is a usage error
- **WHEN** an agent runs `bilbo recall "- ?"`
- **THEN** bilbo prints a message saying the query has no words to stderr and exits 2

### Requirement: Ranking
Hits SHALL be ordered by how well their best passage matches: a passage that holds more of the query words, holds rarer words or holds them more densely ranks higher. A note SHALL appear at most once, at its best passage. Equal matches are ordered by path, then line.

#### Scenario: More query words rank higher
- **WHEN** `plan-a.md` has a passage holding `embedder` and `timeout`, `plan-b.md` holds only `timeout`, and an agent runs `bilbo recall embedder timeout`
- **THEN** the block for `plan-a.md` comes first

#### Scenario: One block per note
- **WHEN** three passages of `gotcha-slots.md` hold the query word
- **THEN** stdout has one block for `gotcha-slots.md`

#### Scenario: Ties fall back to path order
- **WHEN** `plan-b.md` and `plan-a.md` hold the query word in passages that match equally well
- **THEN** the block for `plan-a.md` comes first

### Requirement: What recall searches
Recall SHALL search every entry of `<root>/notes/` that is a regular file, not hidden, with a valid `<kind>-<topic>.md` name, skipping other entries and files it cannot read in silence. Frontmatter is not searched. A note that breaks other `note-store` rules SHALL still be searched; when its `created` is not valid, the block shows `-` in its place. A note with no `#` title uses its filename without `.md` as the title.

#### Scenario: Frontmatter is not searched
- **WHEN** the only occurrence of `github` in the store is a `sources` item
- **THEN** `bilbo recall github` matches nothing

#### Scenario: A note with a bad timestamp is still found
- **WHEN** `plan-release.md` has `created: 2026-10-02` and mentions `rollback`
- **THEN** `bilbo recall rollback` prints its block with `-` as the created field

#### Scenario: A badly named file is skipped
- **WHEN** `notes/idea-foo.md` mentions `rollback` and no note does
- **THEN** `bilbo recall rollback` matches nothing

### Requirement: Kind filter
`--kind <kind>` SHALL limit the hits to notes of that kind, and MAY be given several times to allow several kinds. An unknown kind SHALL be a usage error that lists the kinds.

#### Scenario: Only the asked kind
- **WHEN** a `plan` and a `decision` both match and an agent runs `bilbo recall rollback --kind decision`
- **THEN** only the `decision` note is printed

#### Scenario: An unknown kind
- **WHEN** an agent runs `bilbo recall rollback --kind idea`
- **THEN** bilbo prints a message naming `idea` and listing the kinds to stderr and exits 2

### Requirement: Limit
`--limit <n>` SHALL print at most `n` blocks; without it, at most 10. `n` SHALL be a whole number of 1 or more, or the run is a usage error.

#### Scenario: The default limit
- **WHEN** 15 notes match
- **THEN** stdout has 10 blocks

#### Scenario: A zero limit is refused
- **WHEN** an agent runs `bilbo recall rollback --limit 0`
- **THEN** bilbo exits 2 and stdout is empty

### Requirement: Options and the query
Options SHALL be accepted before or after the query words. `--kind=<kind>` and `--limit=<n>` SHALL be the same as `--kind <kind>` and `--limit <n>`. An argument `--` SHALL end the options, so every argument after it is a query word, even one starting with `-`. Any other argument before `--` that starts with `-` followed by a character other than whitespace SHALL be a usage error.

#### Scenario: A word that looks like an option
- **WHEN** an agent runs `bilbo recall -- --title flag`
- **THEN** the query is `--title flag` and bilbo does not report an unknown option

#### Scenario: An unknown option
- **WHEN** an agent runs `bilbo recall rollback --json`
- **THEN** bilbo prints a message naming `--json` as unknown to stderr and exits 2

#### Scenario: An option with its value after `=`
- **WHEN** an agent runs `bilbo recall rollback --kind=decision --limit=1`
- **THEN** stdout is the same as for `bilbo recall rollback --kind decision --limit 1`

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
When `<root>/notes/` does not exist, `bilbo recall` SHALL print `bilbo: no store at <root>` to stderr and exit 1.

#### Scenario: Wrong BILBO_HOME
- **WHEN** `BILBO_HOME` names a folder with no `notes/` inside it and an agent runs `bilbo recall rollback`
- **THEN** stderr is `bilbo: no store at <that folder>`, stdout is empty and the exit code is 1

### Requirement: Recall is read-only
`bilbo recall` SHALL NOT create, change, rename or delete any file or folder.

#### Scenario: The store is left as found
- **WHEN** `bilbo recall` runs on a store
- **THEN** every entry under the root has the same bytes and modification time as before the run, and no new entry exists
