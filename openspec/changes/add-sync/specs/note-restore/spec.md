# Spec Delta

## MODIFIED Requirements

### Requirement: Restore a version
`bilbo restore <note> <version>` SHALL write the version's bytes to `<root>/notes/<the version's file name>`, record a `restored` version that follows the note's latest version, print `restored <file name> to <version>` to stdout and exit 0. `<note>` and `<version>` SHALL be named as in the `note-history` spec. Exactly two arguments SHALL be given; anything else SHALL be a usage error. When the version's `scope` value differs from the current file's, restore SHALL also print `bilbo: <file name> leaves scope '<name>' with this version; keep it with bilbo scope set <name> <path>` to stderr, naming the current file's scope, with `--force` before `<name>` when the version holds another `scope` value, and still exit 0.

#### Scenario: Undo an edit
- **WHEN** an agent's rewrite dropped a section of `decision-release.md` and the agent runs `bilbo restore release <the version before it>`
- **THEN** the file holds that version's bytes, `bilbo history release` lists `restored` first, and the exit code is 0

#### Scenario: Restore a deleted note
- **WHEN** `decision-release.md` was deleted and no file has the topic `release`
- **THEN** `bilbo restore release <its last edited version>` writes `decision-release.md` back with that text

#### Scenario: A missing argument
- **WHEN** an agent runs `bilbo restore release`
- **THEN** bilbo prints a usage message to stderr, exits 2 and writes nothing

#### Scenario: A version from before the scope
- **WHEN** `decision-release.md` holds `scope: personal` and an agent restores a version from before the note had a scope
- **THEN** the file holds that version's bytes, stderr is `bilbo: decision-release.md leaves scope 'personal' with this version; keep it with bilbo scope set personal notes/decision-release.md`, and the exit code is 0

#### Scenario: A version in another scope
- **WHEN** `decision-release.md` holds `scope: personal` and an agent restores a version that held `scope: work`
- **THEN** stderr is `bilbo: decision-release.md leaves scope 'personal' with this version; keep it with bilbo scope set --force personal notes/decision-release.md`, and the exit code is 0

#### Scenario: The same scope
- **WHEN** the restored version holds the same `scope` line as the current file
- **THEN** stderr holds no leaves-scope line

### Requirement: Nothing is lost on restore
Before replacing a note's file, restore SHALL record the file's current bytes as a version when history does not hold them yet, as the `note-sync` spec's Stale base requirement records a save when a sync write is pending for the note. It SHALL replace the file with an atomic exchange. When the bytes it took out differ from the bytes it last recorded, because something wrote the file during the restore, it SHALL record them as a version too, before the `restored` one. A pending stale-base entry SHALL keep its base, with the `restored` version as its written version, so a later stale save is still merged.

#### Scenario: An unrecorded edit is kept
- **WHEN** an agent edits `decision-release.md`, and a restore runs before watch records the edit
- **THEN** history holds the edit as an `edited` version, followed by the `restored` one

#### Scenario: A write during the restore
- **WHEN** an agent's write lands on `decision-release.md` between restore reading the file and swapping it
- **THEN** the agent's bytes are an `edited` version in history, and the file holds the restored version

#### Scenario: A file the watcher has not seen
- **WHEN** `decision-release.md` was deleted and recorded as `deleted`, an agent writes the same note back as `plan-release.md` before watch records it, and an agent restores a version named `decision-release.md`
- **THEN** restore treats `plan-release.md` as the note's current file: it records it, `decision-release.md` holds the version's bytes, `plan-release.md` no longer exists, and no two files hold the note's id

#### Scenario: A stale save after restore
- **WHEN** an agent read a note at H, B's edit of `## Setup` was written into it by sync, the user restores a version that keeps `## Setup` as B wrote it, and the agent then saves its edit of `## Rollout` made from H
- **THEN** the file holds both B's `## Setup` and the agent's `## Rollout`, and history lists a `merged` version flagged `stale-base`

### Requirement: An interrupted restore
Restore SHALL work through one hidden file, `<root>/notes/.bilbo-restore-<id>`, and SHALL never leave two visible files holding the note's id, even when it is killed. Before it changes anything, restore SHALL sweep every `.bilbo-restore-<id>` file in `notes/`: when the bytes are a version waiting to be written from another device, it SHALL leave the file for `bilbo watch`, which completes that write; when no version of note `<id>` holds that file's bytes, it SHALL record them as an `edited` version of that note under the note's latest file name and print `bilbo: recorded notes/.bilbo-restore-<id> from an interrupted restore` to stderr; it SHALL then delete the file. `bilbo watch` sweeps the same files at each scan.

#### Scenario: A leftover is recorded and removed
- **WHEN** `notes/.bilbo-restore-<id of release>` holds bytes no version of `release` holds, and an agent runs `bilbo restore release <a version>`
- **THEN** stderr names the leftover as recorded, history lists those bytes as an `edited` version before the `restored` one, and the leftover is gone

#### Scenario: A leftover already in history
- **WHEN** `notes/.bilbo-restore-<id of release>` holds the bytes of a version of `release`, and an agent runs `bilbo restore release <a version>`
- **THEN** the leftover is gone, no extra version is recorded, and stderr does not name it

#### Scenario: Killed between the swap and the rename
- **WHEN** a restore across a rename is killed after its exchange and before its rename
- **THEN** exactly one visible file holds the note's id, under the current name, holding the restored bytes, and the next `bilbo watch` scan records it without printing a shared-id line

#### Scenario: An interrupted sync write
- **WHEN** `notes/.bilbo-restore-<id of release>` holds a version of `release` that watch was writing from another device when it was killed, and an agent runs `bilbo restore release <a version>`
- **THEN** restore does not record those bytes as `edited`, and the next watch completes the write
