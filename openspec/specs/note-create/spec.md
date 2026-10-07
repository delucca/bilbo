# note-create Specification

## Purpose
`bilbo new` starts a note with a fresh id, the current timestamp and the right path, so an agent can then write the body with its own file tools.

## Requirements

### Requirement: Create a note
`bilbo new <kind> <topic> [--title <text>] [--scope <name>]` SHALL create `<root>/notes/<kind>-<topic>.md`, creating the root and `notes/` when they are missing. It SHALL print the absolute path of the new file and a newline to stdout, and exit 0. `--scope=<name>` SHALL be the same as `--scope <name>`.

#### Scenario: The first note in a fresh store
- **WHEN** `BILBO_HOME` names a folder that does not exist and an agent runs `bilbo new decision note-store`
- **THEN** `<root>/notes/decision-note-store.md` exists, stdout is its absolute path and the exit code is 0

#### Scenario: An unwritable store
- **WHEN** `<root>/notes/` exists but cannot be written and an agent runs `bilbo new plan release`
- **THEN** bilbo prints the reason to stderr, exits 1 and leaves no file in `notes/`

#### Scenario: The scope after `=`
- **WHEN** the config declares `work` and an agent runs `bilbo new plan release --scope=work`
- **THEN** the file is the same as with `--scope work`

### Requirement: New note content
The new file SHALL hold frontmatter with a fresh `id` and `created` set to the current local time to the minute with the local UTC offset, then `scope: <name>` when the note gets a scope under Scope of a new note, and no `sources` key. A blank line and `# <title>` follow. Without `--title`, the title is the topic with its hyphens turned into spaces and its first letter in uppercase.

#### Scenario: The default title
- **WHEN** no scope is declared and an agent runs `bilbo new decision note-store` at 14:23 in UTC-3 on 2026-10-02
- **THEN** the file is `---`, `id: <26-character ULID>`, `created: 2026-10-02T14:23-03:00`, `---`, a blank line and `# Note store`

#### Scenario: A scope line
- **WHEN** the config declares `work` and an agent runs `bilbo new decision note-store --scope work` at 14:23 in UTC-3 on 2026-10-02
- **THEN** the file is `---`, `id: <26-character ULID>`, `created: 2026-10-02T14:23-03:00`, `scope: work`, `---`, a blank line and `# Note store`

#### Scenario: An explicit title
- **WHEN** an agent runs `bilbo new decision note-store --title "bilbo's note store"`
- **THEN** the file's title line is `# bilbo's note store`

#### Scenario: A new note passes check
- **WHEN** no scope is declared, or `--scope` names a declared scope, and an agent runs `bilbo new plan release` with it in an otherwise valid store, then `bilbo check`
- **THEN** `bilbo check` exits 0

#### Scenario: An unassigned note fails check by design
- **WHEN** the config declares `work`, no `paths` entry holds the working directory, no `scope.default` is set, and an agent runs `bilbo new plan release` in an otherwise valid store, then `bilbo check`
- **THEN** `bilbo check` prints `notes/plan-release.md: scope: missing; scopes: work` and exits 1

#### Scenario: An unusable title is refused
- **WHEN** an agent runs `bilbo new plan release --title ""`, or passes a title that holds a line break
- **THEN** bilbo exits 2 and writes no file

### Requirement: Refuse a taken topic
`bilbo new` SHALL refuse when any note in the store already has the requested topic, whatever its kind. It SHALL exit 1, name the existing file in a message on stderr, and leave the store unchanged.

#### Scenario: The same topic under another kind
- **WHEN** `plan-release.md` exists and an agent runs `bilbo new decision release`
- **THEN** bilbo exits 1, stderr names `plan-release.md`, and no `decision-release.md` exists

#### Scenario: The same file already exists
- **WHEN** `plan-release.md` exists and an agent runs `bilbo new plan release`
- **THEN** bilbo exits 1, and the bytes of `plan-release.md` are unchanged

### Requirement: Reject invalid arguments
`bilbo new` SHALL exit 2, without creating any file or folder, when the kind is not one of the nine kinds, when the topic is not lowercase kebab-case ASCII, when the kind or topic is missing, or when `--scope` is given twice, without a value, or with a name the config does not declare. An unknown kind SHALL produce a message that lists the nine kinds. An undeclared scope SHALL produce a message that lists the declared scopes, or says that none is declared.

#### Scenario: Unknown kind
- **WHEN** an agent runs `bilbo new idea release`
- **THEN** bilbo exits 2, stderr lists the nine kinds, and the store root does not exist if it did not exist before

#### Scenario: Invalid topic
- **WHEN** an agent runs `bilbo new plan Release_Steps`
- **THEN** bilbo exits 2 and writes no file

#### Scenario: Unknown scope
- **WHEN** the config declares `personal` and `work`, and an agent runs `bilbo new plan release --scope acme`
- **THEN** bilbo exits 2, stderr names `acme` and lists `personal, work`, and writes no file

#### Scenario: No scope declared
- **WHEN** the config holds no `scope.*` key and an agent runs `bilbo new plan release --scope work`
- **THEN** bilbo exits 2, stderr names `work` and says no scope is declared in the config path, and writes no file

#### Scenario: Scope given twice
- **WHEN** an agent runs `bilbo new plan release --scope work --scope work`
- **THEN** bilbo exits 2 and writes no file

### Requirement: No partial or doubled notes
`bilbo new` SHALL never leave a partly written file at a note's path. Two runs that request the same `<kind>-<topic>` at the same time SHALL produce exactly one note, and one of the two runs SHALL succeed.

#### Scenario: Two agents race for one note
- **WHEN** two `bilbo new plan release` processes start at the same moment in an empty store
- **THEN** one exits 0, the other exits 1, and `plan-release.md` holds the id that the successful run wrote

### Requirement: Scope of a new note
`bilbo new` SHALL give the note the first of: the `--scope` value; the declared scope with the longest `scope.<name>.paths` entry that is its own working directory or a folder above it, compared by whole folder names after resolving links; `scope.default`. With none, the note SHALL get no `scope` key and, when any scope is declared, stderr SHALL be `bilbo: no scope for <path>; scopes: <names>; set one with bilbo scope set <name> <path>`, exit 0.

#### Scenario: The flag wins
- **WHEN** `scope.work.paths = ~/Developer/acme`, an agent runs `bilbo new plan release --scope personal` in `~/Developer/acme/api`, and `personal` is declared
- **THEN** the note has `scope: personal`

#### Scenario: The working directory picks the scope
- **WHEN** `scope.work.paths = ~/Developer/acme` and an agent runs `bilbo new plan release` in `~/Developer/acme/api`
- **THEN** the note has `scope: work` and stderr is empty

#### Scenario: The longest path wins
- **WHEN** `scope.personal.paths = ~/Developer` and `scope.work.paths = ~/Developer/acme`, and an agent runs `bilbo new plan release` in `~/Developer/acme`
- **THEN** the note has `scope: work`

#### Scenario: A sibling folder does not match
- **WHEN** `scope.work.paths = ~/Developer/acme`, no `scope.default` is set, and an agent runs `bilbo new plan release` in `~/Developer/acme-tools`
- **THEN** the note has no `scope` key

#### Scenario: The default
- **WHEN** `scope.default = personal`, no `paths` entry holds the working directory, and an agent runs `bilbo new plan release`
- **THEN** the note has `scope: personal`

#### Scenario: Nothing matches
- **WHEN** the config declares `personal` and `work` with no `scope.default`, no `paths` entry holds the working directory, and an agent runs `bilbo new plan release`
- **THEN** the note exists with no `scope` key, stdout is its path, stderr is `bilbo: no scope for <path>; scopes: personal, work; set one with bilbo scope set <name> <path>`, and the exit code is 0

#### Scenario: No scope declared
- **WHEN** the config holds no `scope.*` key and an agent runs `bilbo new plan release`
- **THEN** the note has no `scope` key and stderr is empty

### Requirement: Human view of new
When stdout gets the `cli` spec's human view, `bilbo new` SHALL print `◆  Created <kind> <topic>`, the kind cyan and the topic bold, then the new file's path from `~/`, dim and indented three columns. The scope warning SHALL follow on stderr.

#### Scenario: A note created on a terminal
- **WHEN** a user runs `bilbo new gotcha sqlite-busy-timeout` in a terminal with `HOME=/Users/a` and the default root
- **THEN** stdout is `◆  Created gotcha sqlite-busy-timeout` and `   ~/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md`

#### Scenario: Command substitution gets the path
- **WHEN** a user runs `vim "$(bilbo new gotcha sqlite-busy-timeout)"` in a terminal
- **THEN** the substitution is the absolute path alone
