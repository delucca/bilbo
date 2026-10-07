## MODIFIED Requirements

### Requirement: List the scopes
`bilbo scope` SHALL print one line per declared scope, sorted by name: the name, then tab-separated `<n> notes` (`1 note` for one), `sync <value>`, `embedder <rule>` and `paths <list as written>` (`paths -` when unset), then `default` when `scope.default` names it. A last line SHALL be `(unassigned)`, `<n> notes` and `embedder <rule>`, counting every unassigned note. It SHALL count the notes `recall` searches, treat a missing store as empty, and exit 0.

#### Scenario: Two scopes
- **WHEN** the config holds `scope.personal.sync = off`, `scope.work.embedder = local`, `scope.work.paths = ~/Developer/acme` and `scope.default = personal`, and the store holds 3 notes with `scope: personal`, 1 with `scope: work`, 1 with no key and 1 with `scope: acme`
- **THEN** stdout is `personal`, tab, `3 notes`, tab, `sync off`, tab, `embedder any`, tab, `paths -`, tab, `default`; then `work`, tab, `1 note`, tab, `sync off`, tab, `embedder local`, tab, `paths ~/Developer/acme`; then `(unassigned)`, tab, `2 notes`, tab, `embedder local`; and the exit code is 0

#### Scenario: No scope declared
- **WHEN** the config holds no `scope.*` key and the store holds 4 notes
- **THEN** stdout is `(unassigned)`, tab, `4 notes`, tab, `embedder any`, stderr is `bilbo: no scopes declared; add scope.<name>.* keys to <config path>`, and the exit code is 0

#### Scenario: No store yet
- **WHEN** the config declares `work` and `<root>/notes/` does not exist
- **THEN** the `work` line says `0 notes`, the `(unassigned)` line says `0 notes`, and the exit code is 0

#### Scenario: An extra argument
- **WHEN** an agent runs `bilbo scope work` or `bilbo scope --json`
- **THEN** bilbo prints a message naming `work` or `--json` to stderr and exits 2

#### Scenario: A broken config
- **WHEN** the config holds `scope.work.embedder = remote` and an agent runs `bilbo scope`
- **THEN** stderr names `scope.work.embedder`, stdout is empty, and the exit code is 2

#### Scenario: Listing is read-only
- **WHEN** an agent runs `bilbo scope`
- **THEN** every entry under the root has the same bytes and modification time as before the run

## ADDED Requirements

### Requirement: Human view of the scopes
When stdout gets the `cli` spec's human view, `bilbo scope` SHALL print a table with the dim header `SCOPE`, `NOTES`, `SYNC`, `EMBEDDER` and `PATHS`, one row per declared scope with its name in bold and a dim `(default)` beside the default scope, the note counts right-aligned and paths from `~/`, then a dim `unassigned` row. The warning for no declared scope SHALL follow the table.

#### Scenario: Two scopes on a terminal
- **WHEN** the config declares `personal`, the default, and `work`, and a user runs `bilbo scope` in a terminal
- **THEN** the first line is the header, the next two start with `personal` and `work`, `(default)` stands beside `personal` only, and the last row starts with `unassigned`

#### Scenario: A pipe keeps the tabs
- **WHEN** an agent runs `bilbo scope` with stdout piped
- **THEN** stdout is the tab-separated lines of List the scopes
