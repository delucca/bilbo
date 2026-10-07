# library-browse Specification

## Purpose
`bilbo library` lets an agent see what the library holds without opening files: the corpora, a corpus's guide with live facts beside each entry, and a source's outline with line ranges and token counts.

## Requirements

### Requirement: List corpora
`bilbo library` with no other argument SHALL print one line per corpus to stdout, sorted by name, and exit 0. A line is `<corpus>`, a tab, `<n> source` or `<n> sources`, a tab, `<KB> KB`, a tab, `<tokens> tokens`, a tab, and the guide's title, or `-` when the guide is missing or has no title. Sizes follow the `library-store` spec's Derived sizes. Entries of `<root>/library/` that are not valid corpora SHALL be skipped in silence.

#### Scenario: Two corpora
- **WHEN** the library holds `go`, titled `# Go` with 2 sources of 1,000 and 1,500 bytes, and `rust`, titled `# Rust` with 1 source of 4,000 bytes
- **THEN** stdout is `go	2 sources	3 KB	1000 tokens	Go` and `rust	1 source	4 KB	1600 tokens	Rust`, and the exit code is 0

#### Scenario: An invalid folder is skipped
- **WHEN** the library holds `go` and `Go-Old`
- **THEN** stdout has a line for `go` and none for `Go-Old`

#### Scenario: No library
- **WHEN** `<root>/library/` does not exist or holds no corpus
- **THEN** stdout and stderr are empty and the exit code is 0

### Requirement: Show a corpus
`bilbo library <corpus>` SHALL print to stdout the absolute path of the corpus's `guide.md`, then the guide's lines after its frontmatter with a facts line inserted directly after each entry heading, then, for each source with no entry, the lines `## <name>`, its facts line and `(no entry in guide.md)`, in name order. It SHALL exit 0. A missing guide prints only the path and the sources with no entry.

#### Scenario: A guide with its facts
- **WHEN** `library/go/guide.md` holds `# Go`, a lead, and `## effective-go` followed by a blank line and two sentences
- **THEN** stdout is the guide's absolute path, `# Go`, the lead, `## effective-go`, the facts line of `effective-go.md`, the blank line and the two sentences

#### Scenario: A source the guide does not name
- **WHEN** `library/go/` holds `inspecting-errors.md` and the guide has no `## inspecting-errors`
- **THEN** stdout ends with `## inspecting-errors`, its facts line and `(no entry in guide.md)`

#### Scenario: Stub and stale lines are shown
- **WHEN** an entry holds a `TODO: describe this source.` or `stale: re-ingested ` line
- **THEN** that line is printed as it is in the guide

### Requirement: The facts line
A facts line SHALL be `` `<name>.md` ``, then ` · <id>`, ` · <KB> KB`, ` · <tokens> tokens`, ` · fetched <fetched>`, ` · <n> headings` (`1 heading` for one), where `<n>` counts the source's sections, then ` · catalog` when the source is a catalog and ` · capture <label>` when it has a `capture` key. A value the frontmatter lacks, or holds in an invalid form, SHALL be printed as `-`. An entry whose source does not exist SHALL get `` `<name>.md` · missing ``.

#### Scenario: A full facts line
- **WHEN** `effective-go.md` has id `01M3EZ8NVEC2KJQNGK5DTK349R`, a body of 96,211 bytes with 74 sections that is not a catalog, `fetched: 2026-08-23` and `capture: external`
- **THEN** its facts line is `` `effective-go.md` · 01M3EZ8NVEC2KJQNGK5DTK349R · 97 KB · 38485 tokens · fetched 2026-08-23 · 74 headings · capture external ``

#### Scenario: One heading
- **WHEN** `effective-go.md` has one section below its title
- **THEN** its facts line holds ` · 1 heading` and not ` · 1 headings`

#### Scenario: A catalog is marked
- **WHEN** `clippy-lints.md` is a catalog with no `capture` key
- **THEN** its facts line ends with ` · catalog`

#### Scenario: An entry with no source
- **WHEN** the guide has `## effective-go` and `effective-go.md` does not exist
- **THEN** the line after that heading is `` `effective-go.md` · missing ``

#### Scenario: A bad value
- **WHEN** `effective-go.md` has `fetched: yesterday`
- **THEN** its facts line holds ` · fetched -`

### Requirement: Corpus argument errors
`bilbo library <corpus>` SHALL exit 1 with `bilbo: no corpus '<corpus>' in <root>/library` on stderr when no such corpus folder exists. A corpus argument that breaks the name grammar SHALL be a usage error. The reserved corpus names of the `library-store` spec are the subcommands, so no corpus can take one, and each of them given with no other argument SHALL be a usage error naming what is missing.

#### Scenario: An unknown corpus
- **WHEN** an agent runs `bilbo library haskell` and `<root>/library/haskell/` does not exist
- **THEN** stderr is `bilbo: no corpus 'haskell' in <root>/library`, stdout is empty and the exit code is 1

#### Scenario: A bad corpus name
- **WHEN** an agent runs `bilbo library Go`
- **THEN** bilbo prints a message naming `Go` to stderr and exits 2

#### Scenario: A reserved word
- **WHEN** an agent runs `bilbo library plan`
- **THEN** bilbo prints a message naming the missing reference to stderr, exits 2, and writes no plan

### Requirement: Source references
A source reference SHALL be `<corpus>/<name>` or a source's id, optionally followed by `#<anchor>`. An id SHALL resolve among the ids of every source in the library. A reference in neither form SHALL be a usage error. A reference that names no source SHALL exit 1 with a message saying so; when the id is a note's, the message SHALL name that note's absolute path. When two sources share the id, the run SHALL exit 1 naming both.

#### Scenario: By name or by id
- **WHEN** `library/go/effective-go.md` has id `01M3EZ8NVEC2KJQNGK5DTK349R`
- **THEN** `bilbo library show go/effective-go` and `bilbo library show 01M3EZ8NVEC2KJQNGK5DTK349R` print the same output

#### Scenario: A note's id
- **WHEN** `01M3YJ7R6HK6NQ30DCDB1P4DYB` is the id of `notes/decision-note-store.md` and of no source
- **THEN** `bilbo library show 01M3YJ7R6HK6NQ30DCDB1P4DYB` exits 1 and stderr names `<root>/notes/decision-note-store.md`

#### Scenario: No such source
- **WHEN** an agent runs `bilbo library show go/effective-rust` and that file does not exist
- **THEN** stderr names `go/effective-rust`, stdout is empty and the exit code is 1

#### Scenario: A malformed reference
- **WHEN** an agent runs `bilbo library show effective-go` or `bilbo library show go/Effective_Go`
- **THEN** bilbo exits 2

### Requirement: Show a source
`bilbo library show <ref>` SHALL print header lines to stdout, then a blank line, then one row per section in file order, and exit 0. The header lines are `path: <absolute path>`, `id:`, `title:`, `origin:` (the item without its quotes), `fetched:`, then `kept:` and `capture:` when the source has them, `lines: <a>-<b>` (the title line to the last line), `tokens:`, `headings:`, `catalog: yes` or `catalog: no`, and `capture folder: <absolute path>` when a capture records this source (`library-ingest` spec). A row is `<start>-<end>`, a tab, `<tokens> tokens`, a tab, and the heading path.

#### Scenario: A small source
- **WHEN** `library/go/errors.md` is 20 lines long, its title on line 7, with `## Wrapping` on line 9 and `### Is and As` on line 14
- **THEN** after the header and a blank line, stdout holds `9-20`, a tab, the section's tokens, a tab and `Wrapping`, then `14-20`, a tab, its tokens, a tab and `Wrapping > Is and As`

#### Scenario: A headingless source
- **WHEN** the source has no heading below its title
- **THEN** stdout holds the header lines, `headings: 0`, a blank line and no row

#### Scenario: A source whose capture is not held
- **WHEN** the source has `capture: external` and no capture records it, as on a fresh copy of the library
- **THEN** the header holds `capture: external` and no `capture folder:` line

### Requirement: Narrow the outline
With `#<anchor>` in the reference, `library show` SHALL print only the row of the section the anchor resolves to and the rows of the sections inside it. An ambiguous anchor SHALL exit 1 and list each matching heading path with its start line on stderr. A missing anchor SHALL exit 1 naming it. `--depth <n>` SHALL print only the rows whose heading path has at most `n` parts; `n` is a whole number of 1 or more, or the run is a usage error.

#### Scenario: One section and its children
- **WHEN** an agent runs `bilbo library show go/errors#Wrapping`
- **THEN** the rows are those of `Wrapping` and `Wrapping > Is and As`

#### Scenario: An ambiguous anchor
- **WHEN** an agent runs `bilbo library show rust/clippy-lints#What it does`, and many lints have that heading
- **THEN** stdout is empty, stderr lists each matching heading path with its start line, and the exit code is 1

#### Scenario: Top sections only
- **WHEN** an agent runs `bilbo library show go/errors --depth 1`
- **THEN** the rows are only those whose heading path has one part, such as `Wrapping`

#### Scenario: A zero depth
- **WHEN** an agent runs `bilbo library show go/errors --depth 0`
- **THEN** bilbo exits 2 and stdout is empty

### Requirement: Library options
The `library` verb's options SHALL be accepted before or after its positional arguments, `--<option>=<value>` SHALL be the same as `--<option> <value>`, and an argument `--` SHALL end the options. Any other argument before `--` that starts with `-` followed by a character other than whitespace SHALL be a usage error. An option that a subcommand does not take, a missing argument, or an extra argument SHALL be a usage error.

#### Scenario: An option after its value
- **WHEN** an agent runs `bilbo library show --depth=1 go/errors`
- **THEN** stdout is the same as for `bilbo library show go/errors --depth 1`

#### Scenario: An unknown option
- **WHEN** an agent runs `bilbo library go --json`
- **THEN** bilbo prints a message naming `--json` as unknown to stderr and exits 2

#### Scenario: A missing reference
- **WHEN** an agent runs `bilbo library show`
- **THEN** bilbo exits 2

### Requirement: Browsing is read-only
`bilbo library`, `bilbo library <corpus>` and `bilbo library show` SHALL NOT create, change, rename or delete any file or folder.

#### Scenario: The store is left as found
- **WHEN** an agent runs the three forms on a store
- **THEN** every entry under the root has the same bytes and modification time as before, and no new entry exists

### Requirement: Human view of the corpora
When stdout gets the `cli` spec's human view, `bilbo library` SHALL print a table with the dim header `CORPUS`, `SOURCES`, `SIZE` and `TOKENS`, and `TITLE` only when some title differs from its corpus name, one row per corpus with its name in bold, numbers right-aligned and grouped by thousands, and sizes in `KB` or `MB`. An empty library SHALL print one `○` line saying there are no corpora yet.

#### Scenario: Two corpora on a terminal
- **WHEN** the library holds `rust`, 117 sources and 779,594 tokens, and `writing`, each titled with its name, and a user runs `bilbo library` in a terminal
- **THEN** stdout is the header without `TITLE` and two rows, the `rust` row holding `117` and `779,594`

#### Scenario: An empty library on a terminal
- **WHEN** `<root>/library/` holds no corpus and a user runs `bilbo library` in a terminal
- **THEN** stdout is one line starting with `○` and the exit code is 0

#### Scenario: An empty library down a pipe
- **WHEN** an agent runs `bilbo library` with stdout piped and no corpus
- **THEN** stdout is empty

### Requirement: Human view of a guide
When stdout gets the `cli` spec's human view, `bilbo library <corpus>` SHALL print the corpus name in bold and the guide's `~` path dim, the guide's lead wrapped to the width, then per entry its name in bold, its text indented two columns and its facts dim, with tokens grouped by thousands and the id last. An entry whose text is a TODO stub SHALL show `▲` and `TODO` in place of its text, and a source with no entry, or an entry with no source, SHALL show `▲` and the reason in place of its text or facts.

#### Scenario: A guide on a terminal
- **WHEN** the `writing` guide has an entry for `minto-pyramid-untools` and a user runs `bilbo library writing` in a terminal
- **THEN** a line holds only `minto-pyramid-untools` in bold, the entry's text follows indented, and a dim line holds its tokens and `fetched` date

#### Scenario: A stub entry on a terminal
- **WHEN** the `writing` guide's entry `minto-pyramid-untools` still holds its TODO line and a user runs `bilbo library writing` in a terminal
- **THEN** the line under `minto-pyramid-untools` starts with `▲` and holds `TODO`

#### Scenario: A pipe keeps the guide's text
- **WHEN** an agent runs `bilbo library writing` with stdout piped
- **THEN** stdout is the guide path, the guide's lines and the facts lines of Show a corpus

### Requirement: Human view of a source
When stdout gets the `cli` spec's human view, `bilbo library show <ref>` SHALL print the source's title in bold beside its reference, dim lines of its origin, fetch date, capture, id, line range, tokens, headings and `~` path, and, for a catalog, a dim line saying to pick a section as `<ref>#<anchor>`; then its outline: each section's lines dim, its tokens right-aligned and grouped, and its last heading indented by its depth below the shallowest shown, with escaped Markdown read as written.

#### Scenario: An outline on a terminal
- **WHEN** `writing/communication-the-mckinsey-way` has a top section `Business COMMINICATION` with five subsections and a user runs `bilbo library show writing/communication-the-mckinsey-way` in a terminal
- **THEN** `Business COMMINICATION` appears once in the outline, and each subsection appears on its own row, indented, without the top heading before it

#### Scenario: A pipe keeps the rows
- **WHEN** an agent runs `bilbo library show writing/communication-the-mckinsey-way` with stdout piped
- **THEN** stdout is the header lines and tab-separated rows of Show a source
