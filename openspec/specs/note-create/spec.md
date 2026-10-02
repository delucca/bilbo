# note-create Specification

## Purpose
`bilbo new` starts a note with a fresh id, the current timestamp and the right path, so an agent can then write the body with its own file tools.

## Requirements

### Requirement: Create a note
`bilbo new <kind> <topic> [--title <text>]` SHALL create `<root>/notes/<kind>-<topic>.md`, creating the root and `notes/` when they are missing. It SHALL print the absolute path of the new file and a newline to stdout, and exit 0.

#### Scenario: The first note in a fresh store
- **WHEN** `BILBO_HOME` names a folder that does not exist and an agent runs `bilbo new decision note-store`
- **THEN** `<root>/notes/decision-note-store.md` exists, stdout is its absolute path and the exit code is 0

#### Scenario: An unwritable store
- **WHEN** `<root>/notes/` exists but cannot be written and an agent runs `bilbo new plan release`
- **THEN** bilbo prints the reason to stderr, exits 1 and leaves no file in `notes/`

### Requirement: New note content
The new file SHALL hold frontmatter with a fresh `id` and `created` set to the current local time to the minute with the local UTC offset, and no `sources` key. A blank line and `# <title>` follow. Without `--title`, the title is the topic with its hyphens turned into spaces and its first letter in uppercase.

#### Scenario: The default title
- **WHEN** an agent runs `bilbo new decision note-store` at 14:23 in UTC-3 on 2026-10-02
- **THEN** the file is `---`, `id: <26-character ULID>`, `created: 2026-10-02T14:23-03:00`, `---`, a blank line and `# Note store`

#### Scenario: An explicit title
- **WHEN** an agent runs `bilbo new decision note-store --title "bilbo's note store"`
- **THEN** the file's title line is `# bilbo's note store`

#### Scenario: A new note passes check
- **WHEN** an agent runs `bilbo new plan release` in an otherwise valid store, then `bilbo check`
- **THEN** `bilbo check` exits 0

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
`bilbo new` SHALL exit 2, without creating any file or folder, when the kind is not one of the nine kinds, when the topic is not lowercase kebab-case ASCII, or when the kind or topic is missing. An unknown kind SHALL produce a message that lists the nine kinds.

#### Scenario: Unknown kind
- **WHEN** an agent runs `bilbo new idea release`
- **THEN** bilbo exits 2, stderr lists the nine kinds, and the store root does not exist if it did not exist before

#### Scenario: Invalid topic
- **WHEN** an agent runs `bilbo new plan Release_Steps`
- **THEN** bilbo exits 2 and writes no file

### Requirement: No partial or doubled notes
`bilbo new` SHALL never leave a partly written file at a note's path. Two runs that request the same `<kind>-<topic>` at the same time SHALL produce exactly one note, and one of the two runs SHALL succeed.

#### Scenario: Two agents race for one note
- **WHEN** two `bilbo new plan release` processes start at the same moment in an empty store
- **THEN** one exits 0, the other exits 1, and `plan-release.md` holds the id that the successful run wrote
