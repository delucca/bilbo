# note-history Specification

## Purpose
Where bilbo keeps the past versions of each note, which of them it keeps, and `bilbo history`, which lists them, prints one, or shows what changed between two.

## Requirements

### Requirement: Where history lives
bilbo SHALL keep note history under `<root>/.bilbo/history/` and nowhere else. Each version SHALL hold the note's id, the file name and full bytes at that time, the time it was recorded, its event (`added`, `edited`, `renamed`, `deleted`, `restored`, `merged` or `left`), the versions it followed (two or more for `merged`), and, for a version that came through sync, the device that recorded it. A version's id SHALL be derived from the note's id, the versions it followed, the file name and the bytes, and from nothing else, never the device or the time, so two identical records of the same note produce the same id on any device and two notes never share one. A `left` version holds no bytes, and every rule for a `deleted` version applies to it. Removing `<root>/.bilbo/` SHALL lose the history and the sync state and nothing else: the next `bilbo watch` SHALL start over with `added` versions, and rebuild the sync state from the transport as the `sync-transport` spec's Resuming after lost state says.

#### Scenario: History sits beside the notes
- **WHEN** watch has recorded a version of `decision-release.md`
- **THEN** a file under `<root>/.bilbo/history/` holds the version, and `<root>/notes/` holds nothing new

#### Scenario: Two notes never share a version id
- **WHEN** a version of one note and a version of another note have the same parents, file name and bytes
- **THEN** their version ids differ

#### Scenario: Losing the history folder
- **WHEN** a user deletes `<root>/.bilbo/` and watch starts
- **THEN** every note has one `added` version, and every note in `notes/` is unchanged

#### Scenario: One version, two devices
- **WHEN** an edit recorded on device A syncs to device B
- **THEN** `bilbo history <note>` on both devices lists that version with the same 12-character id

#### Scenario: A left version has no text
- **WHEN** an agent names a `left` version in `bilbo history <note> <version>`
- **THEN** stdout is empty and the exit code is 1, as for a deletion

### Requirement: Retention
Pruning SHALL drop every version recorded more than `history.keep_days` days before it runs, except:
- each note's newest version from before that cutoff, so the note as it stood at the cutoff stays readable;
- for a note whose latest version is `deleted`, that version and the last version before it, so a deleted note's last text survives, whatever its age;
- for a note in a syncing scope, for each device of the scope that is not stale, the newest version that device is known to hold and every version that follows it, so that device's next edit still has its merge base; a device known to hold no version yet holds back every version of the note.

Content no kept version holds SHALL be removed from disk. When a note's history holds a record that cannot be read, other than an unfinished last one, pruning SHALL leave that note's history as it is, SHALL remove no content of any note, and SHALL print `bilbo: history/notes/<id>.jsonl line <n> is unreadable; no content removed` to stderr.

#### Scenario: Old edits are dropped
- **WHEN** `history.keep_days` is 90 and a note has versions from 200, 150, 100 and 10 days ago
- **THEN** after pruning it has the versions from 100 and 10 days ago

#### Scenario: A note untouched for a year
- **WHEN** `history.keep_days` is 90 and a note's only version is from 400 days ago
- **THEN** after pruning it still has that version

#### Scenario: A deleted note keeps its last text
- **WHEN** `history.keep_days` is 90 and a note was edited 300 days ago and deleted 200 days ago
- **THEN** after pruning `bilbo history <its id>` lists the deletion and the edit before it, and the edit's text can be printed

#### Scenario: An unreadable record stops content removal
- **WHEN** `history.keep_days` is 90, one note's log has a garbled line in the middle, and other notes have versions from 200 days ago
- **THEN** after pruning stderr names that log and line, and every content file that existed before still exists

#### Scenario: A device away for longer than the window
- **WHEN** `history.keep_days` is 90, `sync.stale_days` is 180, device `moria` last applied a note's version from 120 days ago, and the note has versions from 120, 100 and 10 days ago
- **THEN** after pruning all three remain, and `moria`'s edit made from the 120-day-old version merges without a conflict once it syncs

#### Scenario: A device that never acknowledged
- **WHEN** device `moria` joined 100 days ago, has acknowledged no segment and is not stale, and a note has versions from 150 and 10 days ago
- **THEN** after pruning both remain

#### Scenario: A stale device holds nothing back
- **WHEN** `moria` has been stale since 200 days ago with `sync.stale_days` at 180
- **THEN** pruning keeps versions as if `moria` did not exist

### Requirement: Naming a note
`<note>` SHALL be either a canonical ULID, naming the note with that id, or a topic. A topic SHALL name the note whose file in `<root>/notes/` has that topic. When no file has it, it SHALL name the most recently deleted note whose last file had that topic. When files of two or more notes have that topic under different kinds, it SHALL be a usage error that lists each file with its note's id. Anything else SHALL be a usage error. When the named note has no history, bilbo SHALL print `bilbo: no history for <note>` to stderr and exit 1.

#### Scenario: By topic
- **WHEN** `notes/decision-release.md` exists and an agent runs `bilbo history release`
- **THEN** it lists the versions of that file's note

#### Scenario: A deleted note by topic
- **WHEN** `decision-release.md` was deleted and no file has the topic `release`
- **THEN** `bilbo history release` lists the deleted note's versions, newest first, starting with `deleted`

#### Scenario: By id
- **WHEN** an agent runs `bilbo history 01M3YJ7R6HK6NQ30DCDB1P4DYB`
- **THEN** it lists the versions of the note with that id, whatever its file is called now

#### Scenario: An unknown note
- **WHEN** no file and no history has the topic `wumpus` and an agent runs `bilbo history wumpus`
- **THEN** stderr is `bilbo: no history for wumpus`, stdout is empty and the exit code is 1

#### Scenario: One topic, two notes
- **WHEN** `notes/decision-release.md` and `notes/plan-release.md` hold different ids and an agent runs `bilbo history release`
- **THEN** bilbo prints a usage message naming both files and their ids to stderr, stdout is empty, and the exit code is 2

#### Scenario: Not a topic
- **WHEN** an agent runs `bilbo history Release_Notes`
- **THEN** bilbo prints a usage message to stderr and exits 2

### Requirement: Listing versions
`bilbo history <note>` SHALL print one line per kept version, newest first, as `<version> <time> <event> <file name>`, followed by ` from <device name>` for a version another device recorded, and by ` [<flag>, ...]` for a version with flags. `<version>` is the first 12 hexadecimal characters of the version's id. `<time>` is the time the version records, to the minute, in the `created` form, with the UTC offset it was recorded under, whatever the viewer's time zone. A `left` version shows the file name of the version it followed. Pruned versions SHALL NOT be listed. The exit code SHALL be 0.

#### Scenario: A short history
- **WHEN** `decision-release.md` was added and then edited once
- **THEN** stdout is two lines, the first ending in `edited decision-release.md` and the second in `added decision-release.md`, each starting with 12 hexadecimal characters and a time like `2026-10-03T14:23-03:00`

#### Scenario: A merge with a conflict
- **WHEN** an edit from `bagend` merged here with a local edit and left one passage in conflict
- **THEN** the first line ends in `merged decision-release.md [conflict]`, and a line below it ends in `edited decision-release.md from bagend`

### Requirement: Naming a version
A `<version>` argument SHALL be 6 to 64 hexadecimal characters, matched as a prefix of the ids of the named note's kept versions. A prefix that matches more than one SHALL be a usage error that lists the matching versions. A prefix that matches none SHALL print `bilbo: no version <version> of <note>` to stderr and exit 1.

#### Scenario: A short prefix
- **WHEN** an agent runs `bilbo history release a1b2c3` and exactly one version id starts with `a1b2c3`
- **THEN** that version is printed

#### Scenario: Too short
- **WHEN** an agent runs `bilbo history release a1b`
- **THEN** bilbo prints a usage message to stderr and exits 2

#### Scenario: No such version
- **WHEN** no version of `release` starts with `ffffff`
- **THEN** stderr is `bilbo: no version ffffff of release` and the exit code is 1

### Requirement: Printing a version
`bilbo history <note> <version>` SHALL print the version's bytes to stdout exactly as recorded and exit 0. For a `deleted` version, it SHALL print `bilbo: version <version> of <note> is a deletion` to stderr and exit 1. When the version's content is gone from history, because a prune removed it while bilbo read, it SHALL print `bilbo: version <version> of <note> was pruned` to stderr and exit 1.

#### Scenario: The exact text
- **WHEN** an agent runs `bilbo history release <version of the added note>`
- **THEN** stdout is byte for byte the file as it was when added

#### Scenario: A deletion has no text
- **WHEN** an agent names the `deleted` version
- **THEN** stdout is empty and the exit code is 1

#### Scenario: Content pruned during the read
- **WHEN** a version is still in its note's log but its content file under `<root>/.bilbo/history/` is gone, and an agent prints that version
- **THEN** stderr says the version was pruned, stdout is empty, and the exit code is 1

### Requirement: Diff
`bilbo history <note> --diff <a> [<b>]` SHALL print a unified diff, with 3 lines of context, from version `<a>` to version `<b>`. Without `<b>`, it diffs against the note's file in `<root>/notes/` as it is now. The header lines SHALL be `--- <file name of a>@<a>` and `+++ <file name of b>@<b>`, or, without `<b>`, `+++ <current file name>@now`. A `deleted` version SHALL be diffed as empty text. With no difference, stdout SHALL be empty, and the exit code SHALL be 0 either way. Without `<b>`, for a note that has no file now, bilbo SHALL print `bilbo: <note> has no file; name two versions` to stderr and exit 1.

#### Scenario: What an edit changed
- **WHEN** an agent runs `bilbo history release --diff <added> <edited>` after a paragraph was appended
- **THEN** stdout holds a hunk whose `+` lines are the appended paragraph

#### Scenario: Against the file on disk
- **WHEN** an agent edits `decision-release.md` and, before watch records it, runs `bilbo history release --diff <latest>`
- **THEN** the diff shows the unrecorded edit, under `+++ decision-release.md@now`

#### Scenario: Against now after a rename
- **WHEN** `decision-release.md` was renamed to `plan-release.md` and an agent runs `bilbo history release --diff <a version from before the rename>`
- **THEN** the header lines are `--- decision-release.md@<that version>` and `+++ plan-release.md@now`

#### Scenario: A deleted note against now
- **WHEN** `release` is deleted and an agent runs `bilbo history release --diff <some version>`
- **THEN** stderr names `release` and says to name two versions, and the exit code is 1

### Requirement: Stale history warning
When no `bilbo watch` is running against the store, `bilbo history` SHALL also print `bilbo: bilbo watch is not running; recent edits may not be recorded` to stderr. Its stdout and exit code SHALL be unaffected.

#### Scenario: No watcher
- **WHEN** no watcher runs and an agent runs `bilbo history release`
- **THEN** stdout lists the versions, stderr holds the warning, and the exit code is 0

#### Scenario: A watcher
- **WHEN** a watcher runs against the store
- **THEN** stderr is empty

### Requirement: History is read-only
`bilbo history` SHALL NOT create, change, rename or delete any file or folder.

#### Scenario: Nothing changes
- **WHEN** an agent runs every form of `bilbo history`
- **THEN** every entry under the root has the same bytes and modification time as before

### Requirement: A missing store
When `<root>/notes/` does not exist, `bilbo history` SHALL print `bilbo: no store at <root>` to stderr and exit 1.

#### Scenario: Wrong BILBO_HOME
- **WHEN** `BILBO_HOME` names a folder with no `notes/` and an agent runs `bilbo history release`
- **THEN** stderr is `bilbo: no store at <that folder>` and the exit code is 1
