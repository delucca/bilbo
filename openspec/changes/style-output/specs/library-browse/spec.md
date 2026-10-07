## MODIFIED Requirements

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

## ADDED Requirements

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
