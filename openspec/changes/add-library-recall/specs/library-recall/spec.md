# Spec Delta

## Purpose

`bilbo recall --library` finds the passages of the library's sources and guides that hold the query's words, and points an agent at the section to read, so a lint, a flag or an API can be looked up without knowing which source holds it.

## ADDED Requirements

### Requirement: Search the library
`bilbo recall <query>... --library` SHALL search the library instead of the notes: the passages of every source and of every guide, read as the `note-recall` spec's Passages requirement reads a note. It SHALL match and rank by keywords alone, even when an embedder is configured: it SHALL NOT send anything to the embedder, read the vector cache, or print the embedder or not-indexed warnings. It SHALL NOT read `<root>/notes/`. A passage matches when it holds at least one query word, by the `note-recall` spec's Query words rule, in its text, its heading path or its file's title.

#### Scenario: A source passage is found
- **WHEN** `library/go/effective-go.md` holds `## Concurrency` and under it `### Goroutines`, whose text mentions `goroutine`, and an agent runs `bilbo recall goroutine --library`
- **THEN** stdout has a block for `effective-go.md` and the exit code is 0

#### Scenario: Notes are not searched
- **WHEN** only `notes/gotcha-goroutines.md` mentions `goroutine` and an agent runs `bilbo recall goroutine --library`
- **THEN** stdout is empty, stderr is `bilbo: no sources match` and the exit code is 1

#### Scenario: The embedder is never asked
- **WHEN** an embedder is configured and listening, the vector cache holds vectors for every note, and an agent runs `bilbo recall goroutine --library`
- **THEN** the embedder receives no request, stderr is empty, and the exit code is 0

#### Scenario: A paraphrase is not a hit
- **WHEN** an embedder is configured, the only source about goroutines never uses the word `thread`, and an agent runs `bilbo recall thread --library`
- **THEN** nothing matches and the exit code is 1

### Requirement: What library recall searches
Library recall SHALL search every corpus folder of `<root>/library/` whose name is a valid corpus name under the `library-store` spec, and in each corpus its `guide.md` and every regular file `<name>.md` whose `<name>` is a valid source name. It SHALL skip hidden entries, other entries and files it cannot read, in silence, and SHALL NOT read `<root>/.bilbo/`. Frontmatter is not searched. A source or a guide that breaks other `library-store` rules SHALL still be searched; one with no `# ` title uses its file name without `.md` as the title.

#### Scenario: Frontmatter is not searched
- **WHEN** the only occurrence of `golang` in the library is the `origin: "url: https://golang.org/doc/effective_go"` line of a source
- **THEN** `bilbo recall golang --library` matches nothing

#### Scenario: An edited source is still found
- **WHEN** `library/go/effective-go.md` mentions `goroutine` and its body no longer matches its `digest`
- **THEN** `bilbo recall goroutine --library` prints its block

#### Scenario: Invalid entries are skipped
- **WHEN** only `library/go/Effective_Go.md`, `library/Go-Old/errors.md` and `library/go/.draft.md` mention `goroutine`
- **THEN** `bilbo recall goroutine --library` matches nothing and stderr is `bilbo: no sources match`

#### Scenario: Captures are not searched
- **WHEN** only a `capture.md` under `<root>/.bilbo/captures/` mentions `goroutine`
- **THEN** `bilbo recall goroutine --library` matches nothing

### Requirement: Library hit blocks
Library recall SHALL print one block per matching file to stdout, best first, blocks separated by one blank line, and exit 0. A block is three lines. The first is `<absolute path>:<line>`, a tab, `source` or `guide`, a tab, the file's reference, a tab, and the lines of the hit's section as `<start>-<end>`. The reference is `<corpus>/<name>` for a source and `<corpus>` for a guide: the argument `bilbo library show` or `bilbo library` takes to show that file. The second line is the passage's heading path below the title, parts joined by ` > `, or `-` for a passage that sits under the title and no other heading. The third line is the snippet, by the `note-recall` spec's Snippet requirement. `<line>` is the physical line the passage or part starts on, by the `note-recall` spec's Passages requirement.

#### Scenario: A source hit
- **WHEN** in `library/go/effective-go.md` the line `## Concurrency` is line 300, `### Goroutines` is line 340, the next level-2 or level-3 heading is on line 381, and the `Goroutines` passage is the best match for `bilbo recall goroutine --library`
- **THEN** the block is `<root>/library/go/effective-go.md:340`, a tab, `source`, a tab, `go/effective-go`, a tab, `340-380`; then `Concurrency > Goroutines`; then the snippet

#### Scenario: A guide hit
- **WHEN** `library/go/guide.md` has `## effective-go` on line 12, the next `## ` heading on line 16, and the entry's prose is the best match for `bilbo recall idioms --library`
- **THEN** the block is `<root>/library/go/guide.md:12`, a tab, `guide`, a tab, `go`, a tab, `12-15`; then `effective-go`; then the snippet

#### Scenario: A hit under the title
- **WHEN** the best passage of `library/go/errors.md` is the text between its title on line 7 and its first heading below the title on line 15
- **THEN** the first line ends with `7-14` and the second line is `-`

#### Scenario: A note block is not printed
- **WHEN** `bilbo recall goroutine --library` prints blocks
- **THEN** no first line holds a note kind or a `created` timestamp, and none names a path under `<root>/notes/`

### Requirement: The section of a hit
The section of a hit SHALL be the section, under the `library-store` spec's Outline requirement, of the last heading below the title at or before `<line>`: it starts at that heading's line and ends on the line before the next heading of the same or a higher level, or on the file's last line. When no heading below the title sits at or before `<line>`, the section SHALL start at the title's line and end on the line before the first heading below the title, or on the file's last line. Guides follow the same rule. Reading lines `<line>` to `<end>` SHALL give the rest of the section from where the passage starts.

#### Scenario: A section holds its subsections
- **WHEN** the best passage of `effective-go.md` is the `Concurrency` heading's own text, `## Concurrency` is on line 300, and the next level-2 heading is on line 520
- **THEN** the first line ends with `300-519`, which spans `### Goroutines` and every other subsection of `Concurrency`

#### Scenario: A part of a long section
- **WHEN** the `Goroutines` passage is over 4,000 bytes, so it is split into parts, and its second part, starting on line 361, is the best match
- **THEN** the block's `<line>` is 361 and its section is still `340-380`

#### Scenario: The same lines as library show
- **WHEN** a hit's second line is `Concurrency > Goroutines` and its first line ends with `340-380`
- **THEN** `bilbo library show go/effective-go#Concurrency > Goroutines` prints a row starting with `340-380`

#### Scenario: A heading in a code fence does not end a section
- **WHEN** a fenced block inside the `Goroutines` section holds the line `## not a heading` on line 360, and the next real level-2 or level-3 heading is on line 381
- **THEN** a hit in that section still ends with `340-380`

#### Scenario: A source with no heading below the title
- **WHEN** a 40-line source has its title on line 7 and no other heading, and it is a hit
- **THEN** the first line ends with `7-40` and the second line is `-`

### Requirement: Library ranking
Library hits SHALL be ordered by how well their best passage matches: a passage that holds more of the query words, holds rarer words or holds them more densely ranks higher, with rarity measured over the passages searched. A file SHALL appear at most once, at its best passage. Equal matches are ordered by path, then line. `--limit <n>` SHALL cap the blocks as the `note-recall` spec's Limit requirement says, 10 by default.

#### Scenario: More query words rank higher
- **WHEN** a passage of `go/effective-go.md` holds `goroutine` and `leak`, `go/errors.md` holds only `leak`, and an agent runs `bilbo recall goroutine leak --library`
- **THEN** the block for `effective-go.md` comes first

#### Scenario: One block per source
- **WHEN** 30 sections of `rust/clippy-lints.md` hold `return`
- **THEN** `bilbo recall return --library` prints one block for `clippy-lints.md`

#### Scenario: Sources and guides share one order
- **WHEN** `go/guide.md` and `go/effective-go.md` both hold `idioms`
- **THEN** each has one block, ordered by how well its best passage matches, and stdout has no other block

#### Scenario: The default limit
- **WHEN** 15 sources match
- **THEN** stdout has 10 blocks

### Requirement: Narrow to corpora
`--corpus <corpus>` SHALL limit library recall to that corpus and SHALL imply `--library`. It MAY be given several times to search several corpora. A corpus with no folder in `<root>/library/` SHALL make recall print `bilbo: no corpus '<corpus>' in <root>/library` to stderr and exit 1. A corpus name that breaks the `library-store` spec's name grammar, or is reserved, SHALL be a usage error. `--corpus` without a value SHALL be a usage error.

#### Scenario: One corpus
- **WHEN** `go/errors.md` and `rust/errors.md` both hold `wrapping`, and an agent runs `bilbo recall wrapping --corpus go`
- **THEN** stdout has a block for `go/errors.md` and none for `rust/errors.md`

#### Scenario: Two corpora
- **WHEN** `go`, `rust` and `haskell` each hold a source that mentions `errors`, and an agent runs `bilbo recall errors --corpus go --corpus rust`
- **THEN** stdout has blocks from `go` and `rust` and none from `haskell`

#### Scenario: An unknown corpus
- **WHEN** an agent runs `bilbo recall errors --corpus lisp` and `<root>/library/lisp/` does not exist
- **THEN** stderr is `bilbo: no corpus 'lisp' in <root>/library`, stdout is empty and the exit code is 1

#### Scenario: A bad corpus name
- **WHEN** an agent runs `bilbo recall errors --corpus Go` or `bilbo recall errors --corpus plan`
- **THEN** bilbo prints a message naming that corpus to stderr and exits 2

### Requirement: Nothing in the library
With `--library` or `--corpus`, when no passage matches, recall SHALL print `bilbo: no sources match` to stderr, leave stdout empty and exit 1. When `<root>/library/` does not exist or holds no valid corpus folder, recall SHALL print `bilbo: no library at <root>` to stderr and exit 1, whether or not `<root>/notes/` exists. A missing `<root>/notes/` SHALL NOT stop library recall.

#### Scenario: Nothing matches
- **WHEN** no source or guide holds any query word and an agent runs `bilbo recall wumpus --library`
- **THEN** stdout is empty, stderr is `bilbo: no sources match` and the exit code is 1

#### Scenario: No library
- **WHEN** `<root>/notes/` holds notes, `<root>/library/` does not exist, and an agent runs `bilbo recall goroutine --library`
- **THEN** stderr is `bilbo: no library at <root>`, stdout is empty and the exit code is 1

#### Scenario: A library without notes
- **WHEN** `<root>/library/go/` holds a source that mentions `goroutine` and `<root>/notes/` does not exist
- **THEN** `bilbo recall goroutine --library` prints its block and exits 0

### Requirement: Plain search leaves the library alone
`bilbo recall` without `--library` and `--corpus`, `bilbo index` and `bilbo digest` SHALL NOT read `<root>/library/`. Their output SHALL NOT name a source, a guide or a library match, and `bilbo index` SHALL send no library text to the embedder.

#### Scenario: Plain recall ignores sources
- **WHEN** a source and the note `notes/gotcha-goroutines.md` both mention `goroutine`, and an agent runs `bilbo recall goroutine`
- **THEN** stdout has the note's block only, and stderr is empty

#### Scenario: No hint when only the library matches
- **WHEN** only a source mentions `goroutine` and an agent runs `bilbo recall goroutine`
- **THEN** stderr is exactly `bilbo: no notes match`, stdout is empty and the exit code is 1

#### Scenario: Index embeds no library text
- **WHEN** an embedder is configured, the store holds one note with one passage and a library of 20 sources, and an agent runs `bilbo index`
- **THEN** the embedder receives the note's passage and no source or guide text

#### Scenario: The digest shows no source
- **WHEN** a source holds every word of the prompt and no note passes the gate
- **THEN** `bilbo digest` prints nothing to stdout
