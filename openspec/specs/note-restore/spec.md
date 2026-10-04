# note-restore Specification

## Purpose
`bilbo restore` puts a past version of a note back into the store as its newest version, without losing what the note held before or an edit that lands while it runs.

## Requirements

### Requirement: Restore a version
`bilbo restore <note> <version>` SHALL write the version's bytes to `<root>/notes/<the version's file name>`, record a `restored` version that follows the note's latest version, print `restored <file name> to <version>` to stdout and exit 0. `<note>` and `<version>` SHALL be named as in the `note-history` spec. Exactly two arguments SHALL be given; anything else SHALL be a usage error.

#### Scenario: Undo an edit
- **WHEN** an agent's rewrite dropped a section of `decision-release.md` and the agent runs `bilbo restore release <the version before it>`
- **THEN** the file holds that version's bytes, `bilbo history release` lists `restored` first, and the exit code is 0

#### Scenario: Restore a deleted note
- **WHEN** `decision-release.md` was deleted and no file has the topic `release`
- **THEN** `bilbo restore release <its last edited version>` writes `decision-release.md` back with that text

#### Scenario: A missing argument
- **WHEN** an agent runs `bilbo restore release`
- **THEN** bilbo prints a usage message to stderr, exits 2 and writes nothing

### Requirement: Nothing is lost on restore
Before replacing a note's file, restore SHALL record the file's current bytes as a version when history does not hold them yet. It SHALL replace the file with an atomic exchange. When the bytes it took out differ from the bytes it last recorded, because something wrote the file during the restore, it SHALL record them as a version too, before the `restored` one.

#### Scenario: An unrecorded edit is kept
- **WHEN** an agent edits `decision-release.md`, and a restore runs before watch records the edit
- **THEN** history holds the edit as an `edited` version, followed by the `restored` one

#### Scenario: A write during the restore
- **WHEN** an agent's write lands on `decision-release.md` between restore reading the file and swapping it
- **THEN** the agent's bytes are an `edited` version in history, and the file holds the restored version

#### Scenario: A file the watcher has not seen
- **WHEN** `decision-release.md` was deleted and recorded as `deleted`, an agent writes the same note back as `plan-release.md` before watch records it, and an agent restores a version named `decision-release.md`
- **THEN** restore treats `plan-release.md` as the note's current file: it records it, `decision-release.md` holds the version's bytes, `plan-release.md` no longer exists, and no two files hold the note's id

### Requirement: An interrupted restore
Restore SHALL work through one hidden file, `<root>/notes/.bilbo-restore-<id>`, and SHALL never leave two visible files holding the note's id, even when it is killed. Before it changes anything, restore SHALL sweep every `.bilbo-restore-<id>` file in `notes/`: when no version of note `<id>` holds that file's bytes, it SHALL record them as an `edited` version of that note under the note's latest file name and print `bilbo: recorded notes/.bilbo-restore-<id> from an interrupted restore` to stderr; it SHALL then delete the file. `bilbo watch` sweeps the same files at each scan.

#### Scenario: A leftover is recorded and removed
- **WHEN** `notes/.bilbo-restore-<id of release>` holds bytes no version of `release` holds, and an agent runs `bilbo restore release <a version>`
- **THEN** stderr names the leftover as recorded, history lists those bytes as an `edited` version before the `restored` one, and the leftover is gone

#### Scenario: A leftover already in history
- **WHEN** `notes/.bilbo-restore-<id of release>` holds the bytes of a version of `release`, and an agent runs `bilbo restore release <a version>`
- **THEN** the leftover is gone, no extra version is recorded, and stderr does not name it

#### Scenario: Killed between the swap and the rename
- **WHEN** a restore across a rename is killed after its exchange and before its rename
- **THEN** exactly one visible file holds the note's id, under the current name, holding the restored bytes, and the next `bilbo watch` scan records it without printing a shared-id line

### Requirement: Restoring across a rename
When the note's current file name differs from the version's, restore SHALL write the version's file name and remove the current file. When another note's file already holds the version's file name, or a file with the version's topic exists under another kind, restore SHALL print `bilbo: <file name> is taken by another note` to stderr, exit 1 and change nothing.

#### Scenario: Back to the old name
- **WHEN** `decision-release.md` was renamed to `plan-release.md` and an agent restores the version from before the rename
- **THEN** `decision-release.md` holds that version's bytes and `plan-release.md` no longer exists

#### Scenario: The old name is taken
- **WHEN** the version's file name is `decision-release.md` and another note now has `plan-release.md`
- **THEN** stderr says `decision-release.md is taken by another note`, the exit code is 1, and no file changes

### Requirement: What cannot be restored
Restoring a `deleted` version SHALL print `bilbo: version <version> of <note> is a deletion; delete the file instead` to stderr and exit 1. When the note's file already holds the version's bytes under the version's name, restore SHALL write nothing, record nothing, print `<file name> already matches <version>` to stdout and exit 0. On a filesystem that cannot exchange two files atomically, restore SHALL print `bilbo: cannot restore on this filesystem: it cannot swap files atomically` to stderr, exit 1 and change nothing.

#### Scenario: A deletion
- **WHEN** an agent runs `bilbo restore release <the deleted version>`
- **THEN** the exit code is 1 and no file changes

#### Scenario: Already there
- **WHEN** an agent restores the version the file already holds
- **THEN** stdout says it already matches, no version is recorded, and the exit code is 0

### Requirement: Restore and the watcher
Restore SHALL work whether or not `bilbo watch` is running. A running watcher SHALL NOT record the restored file again.

#### Scenario: With the watcher running
- **WHEN** watch is running and an agent restores a version
- **THEN** after 10 seconds `bilbo history release` lists exactly one `restored` version and nothing after it

#### Scenario: Without the watcher
- **WHEN** no watcher runs and an agent restores a version
- **THEN** history lists the `restored` version

### Requirement: A missing store
When `<root>/notes/` does not exist, `bilbo restore` SHALL print `bilbo: no store at <root>` to stderr, exit 1 and create nothing.

#### Scenario: Wrong BILBO_HOME
- **WHEN** `BILBO_HOME` names a folder with no `notes/` and an agent runs `bilbo restore release a1b2c3`
- **THEN** stderr is `bilbo: no store at <that folder>`, the exit code is 1, and nothing is created
