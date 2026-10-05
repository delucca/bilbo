# note-store Specification

## Purpose
Where bilbo keeps notes and the exact shape of a note file. Agents writing files directly, `bilbo new` and `bilbo check` all rely on this contract.

## Requirements

### Requirement: Store root
On every supported OS, macOS included, bilbo SHALL resolve the store root in this order: `BILBO_HOME` when it is set and not empty, then `$XDG_DATA_HOME/bilbo` when `XDG_DATA_HOME` is an absolute path, then `$HOME/.local/share/bilbo`. A relative `BILBO_HOME` SHALL be a usage error.

#### Scenario: BILBO_HOME wins
- **WHEN** `BILBO_HOME` is `/tmp/store` and `XDG_DATA_HOME` is `/data`
- **THEN** the store root is `/tmp/store`

#### Scenario: XDG data home is used next
- **WHEN** `BILBO_HOME` is unset and `XDG_DATA_HOME` is `/data`
- **THEN** the store root is `/data/bilbo`

#### Scenario: The default root
- **WHEN** neither `BILBO_HOME` nor `XDG_DATA_HOME` is set and `HOME` is `/Users/a`
- **THEN** the store root is `/Users/a/.local/share/bilbo`

#### Scenario: macOS uses the same default
- **WHEN** bilbo runs on macOS with neither `BILBO_HOME` nor `XDG_DATA_HOME` set and `HOME` is `/Users/a`
- **THEN** the store root is `/Users/a/.local/share/bilbo`, not a folder under `/Users/a/Library`

#### Scenario: A relative XDG data home is ignored
- **WHEN** `BILBO_HOME` is unset, `XDG_DATA_HOME` is `data` and `HOME` is `/Users/a`
- **THEN** the store root is `/Users/a/.local/share/bilbo`

#### Scenario: A relative BILBO_HOME is refused
- **WHEN** `BILBO_HOME` is `store` and an agent runs any verb
- **THEN** bilbo prints a message saying `BILBO_HOME` must be absolute to stderr, exits 2, and touches no file

### Requirement: Notes layout
Notes SHALL live directly in `<root>/notes/`, one file per note, named `<kind>-<topic>.md`. The kind is one of `plan`, `spec`, `design`, `decision`, `gotcha`, `research`, `review`, `report` or `reference`. The topic is lowercase kebab-case ASCII: segments of `[a-z0-9]+` joined by single hyphens. An entry whose name starts with `.` is not a note and SHALL be ignored.

#### Scenario: A well-named note
- **WHEN** `<root>/notes/decision-note-store.md` exists
- **THEN** it is a note of kind `decision` with topic `note-store`

#### Scenario: A hidden entry is ignored
- **WHEN** `<root>/notes/.DS_Store` exists
- **THEN** bilbo treats it as neither a note nor a problem

#### Scenario: A bad name is not a valid note
- **WHEN** `<root>/notes/Idea-Foo.md`, `<root>/notes/idea-foo.md` or `<root>/notes/plan-foo--bar.md` exists
- **THEN** the store is invalid at that entry

#### Scenario: A subfolder is not allowed
- **WHEN** `<root>/notes/archive/` exists
- **THEN** the store is invalid at that entry

### Requirement: Topic uniqueness
A topic SHALL have at most one note in the store, whatever the kind.

#### Scenario: Different topics coexist
- **WHEN** the store holds `plan-release.md` and `plan-rollback.md`
- **THEN** the store is valid on this rule

#### Scenario: One topic under two kinds is invalid
- **WHEN** the store holds `plan-release.md` and `decision-release.md`
- **THEN** the store is invalid at both files

### Requirement: Frontmatter shape
A note SHALL open with a line holding exactly `---`, then its keys, then a second line holding exactly `---`. The keys SHALL be `id` and `created`, both required, and optionally `sources` and `scope`, in any order, each at most once. No other key is allowed.

#### Scenario: Minimal valid frontmatter
- **WHEN** a note's file starts with `---`, `id: 01M3YJ7R6HK6NQ30DCDB1P4DYB`, `created: 2026-10-02T14:23-03:00`, `---`
- **THEN** the frontmatter is valid

#### Scenario: A scope key is allowed
- **WHEN** a note's frontmatter holds `id`, `created` and `scope: work`
- **THEN** the frontmatter is valid

#### Scenario: A removed key is invalid
- **WHEN** a note's frontmatter also holds `kind: decision`, `supersedes: ...` or `project: bilbo`
- **THEN** the note is invalid, and the problem names that key

#### Scenario: Two scope lines are invalid
- **WHEN** a note's frontmatter holds `scope: work` and `scope: personal`
- **THEN** the note is invalid, and the problem names `scope`

#### Scenario: Missing frontmatter is invalid
- **WHEN** a note's first line is `# Title`
- **THEN** the note is invalid

### Requirement: Note id
`id` SHALL be a ULID written in its canonical form: 26 uppercase Crockford base32 characters (`0-9`, `A-Z` without `I`, `L`, `O`, `U`), the first of them `0` to `7`. Every id SHALL be unique in the store.

#### Scenario: A canonical ULID is valid
- **WHEN** a note has `id: 01M3YJ7R6HK6NQ30DCDB1P4DYB`
- **THEN** the id is valid

#### Scenario: A non-canonical id is invalid
- **WHEN** a note has `id: 01m3yj7r6hk6nq30dcdb1p4dyb` or `id: 01M3YJ7R6HK6NQ30DCDB1P4DY`
- **THEN** the note is invalid

#### Scenario: A shared id is invalid
- **WHEN** two notes have the same `id`
- **THEN** the store is invalid at both files

### Requirement: Created timestamp
`created` SHALL be a real local date and time, to the minute, with a numeric UTC offset, in the form `YYYY-MM-DDTHH:MM±HH:MM`. `-00:00` is not allowed; UTC is `+00:00`.

#### Scenario: A valid timestamp
- **WHEN** a note has `created: 2026-10-02T14:23-03:00`
- **THEN** `created` is valid

#### Scenario: Other forms are invalid
- **WHEN** a note has `created: 2026-10-02`, `created: 2026-10-02T14:23:05-03:00`, `created: 2026-10-02T17:23Z` or `created: 2026-10-02T14:23-00:00`
- **THEN** the note is invalid

#### Scenario: An impossible date is invalid
- **WHEN** a note has `created: 2026-02-30T10:00-03:00`
- **THEN** the note is invalid

### Requirement: Sources list
When `sources` is present, it SHALL be the line `sources:` followed by one or more items, each on its own line as two spaces, `- `, then a double-quoted `<type>: <value>`. The type is one of `url`, `code`, `doc` or `search`, and the value is not empty. A note with no sources SHALL omit the key.

#### Scenario: A valid sources list
- **WHEN** a note's frontmatter holds `sources:` and then the line `  - "url: https://github.com/delucca/bilbo"`
- **THEN** `sources` is valid

#### Scenario: An empty list is invalid
- **WHEN** a note has `sources: []`, or `sources:` with no items under it
- **THEN** the note is invalid

#### Scenario: An unknown type is invalid
- **WHEN** a note has the item `  - "web: https://example.org"`
- **THEN** the note is invalid

### Requirement: Title
After the frontmatter, the note SHALL hold exactly one level-1 heading, a line starting with `# `, outside fenced code blocks.

#### Scenario: One title
- **WHEN** a note's body holds `# Note store` and level-2 headings
- **THEN** the title is valid

#### Scenario: A heading inside a fence does not count
- **WHEN** a note's body holds `# Note store` and, inside a code fence, a line `# a shell comment`
- **THEN** the title is valid

#### Scenario: No title or two titles is invalid
- **WHEN** a note's body has no line starting with `# ` outside fences, or has two
- **THEN** the note is invalid

### Requirement: Scope key
When `scope` is present, it SHALL be one line, `scope: <name>`, where the name has the topic's grammar: segments of `[a-z0-9]+` joined by single hyphens. The key never changes the note's path. Whether the name is declared is not a store rule: `bilbo check` reports that from the config, under the `store-check` spec's Scope problems.

#### Scenario: A valid scope
- **WHEN** a note has `scope: client-x`
- **THEN** `scope` is valid

#### Scenario: Other forms are invalid
- **WHEN** a note has `scope: Work`, `scope: a b`, `scope: [work, personal]`, `scope:` or `scope: work-`
- **THEN** the note is invalid, and the problem names `scope`

#### Scenario: The scope is not in the path
- **WHEN** a note has `scope: work`
- **THEN** its file is still `<root>/notes/<kind>-<topic>.md`
