# Spec Delta

## MODIFIED Requirements

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

### Requirement: Listing versions
`bilbo history <note>` SHALL print one line per kept version, newest first, as `<version> <time> <event> <file name>`, followed by ` from <device name>` for a version another device recorded, and by ` [<flag>, ...]` for a version with flags. `<version>` is the first 12 hexadecimal characters of the version's id. `<time>` is the time the version records, to the minute, in the `created` form, with the UTC offset it was recorded under, whatever the viewer's time zone. A `left` version shows the file name of the version it followed. Pruned versions SHALL NOT be listed. The exit code SHALL be 0.

#### Scenario: A short history
- **WHEN** `decision-release.md` was added and then edited once
- **THEN** stdout is two lines, the first ending in `edited decision-release.md` and the second in `added decision-release.md`, each starting with 12 hexadecimal characters and a time like `2026-10-03T14:23-03:00`

#### Scenario: A merge with a conflict
- **WHEN** an edit from `bagend` merged here with a local edit and left one passage in conflict
- **THEN** the first line ends in `merged decision-release.md [conflict]`, and a line below it ends in `edited decision-release.md from bagend`
