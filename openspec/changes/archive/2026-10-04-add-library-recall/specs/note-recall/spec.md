# Spec Delta

## MODIFIED Requirements

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

### Requirement: A missing store
Without `--library` and `--corpus`, when `<root>/notes/` does not exist, `bilbo recall` SHALL print `bilbo: no store at <root>` to stderr and exit 1. With either option, a missing `<root>/notes/` SHALL NOT stop recall, and the `library-recall` spec's Nothing in the library requirement applies instead.

#### Scenario: Wrong BILBO_HOME
- **WHEN** `BILBO_HOME` names a folder with no `notes/` inside it and an agent runs `bilbo recall rollback`
- **THEN** stderr is `bilbo: no store at <that folder>`, stdout is empty and the exit code is 1

#### Scenario: A library search without notes
- **WHEN** `<root>/notes/` does not exist, `<root>/library/go/` holds a source that mentions `rollback`, and an agent runs `bilbo recall rollback --library`
- **THEN** stderr does not say `no store`, and the source's block is printed
