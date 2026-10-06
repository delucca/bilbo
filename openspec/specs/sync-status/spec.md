# sync-status Specification

## Purpose
`bilbo sync` tells a user or an agent whether sync is working: per scope, where it syncs, which devices keep up, how many notes sync, and which conflicts and dropped text wait. `bilbo sync declare` records that dropped text was dropped on purpose.

## Requirements

### Requirement: Status report
`bilbo sync` SHALL print, in order: per syncing scope by name, `scope <name> <url>: <n> notes, pushed <time>, pulled <time>` and a `device <scope name> <device name>: <state>` line per device of its latest manifest; `local: <n> notes sync nowhere`; by file, `conflict notes/<file>: <n> passages` and `dropped notes/<file>: <n> lines not declared` (`passage`, `line` for one); `notice <time> notes/<file>: <flag>` per flag of the last 7 days; and `change <time> <name>: device <device name> added by <signer> (manifest <n>)` or `change <time> <name>: epoch changed (manifest <n>)` per manifest change of the last 30 days that this device did not write. `<time>` is in the `created` form, or `never`.

#### Scenario: Two devices, in step
- **WHEN** this device and `bywater` sync `personal` with 42 notes, 17 notes have no scope, and nothing is open
- **THEN** stdout is `scope personal file:///srv/bilbo: 42 notes, pushed <time>, pulled <time>`, `device personal rhosgobel: this device`, `device personal bywater: up to date` and `local: 17 notes sync nowhere`, and the exit code is 0

#### Scenario: An open conflict
- **WHEN** `notes/gotcha-nix.md` holds an open conflict in one passage
- **THEN** stdout holds `conflict notes/gotcha-nix.md: 1 passage` and the exit code is 1

#### Scenario: A device added on another machine
- **WHEN** `morthond` was added to `personal` by `bilbo device recover` on `morthond` 3 days ago
- **THEN** stdout holds `change <that time> personal: device morthond added by owner key (manifest 4)`, so the user can revoke `morthond` if they do not know it

#### Scenario: A flag from yesterday
- **WHEN** an edit from `bywater` brought back a note deleted here yesterday
- **THEN** stdout holds a `notice` line naming the file and `edit-beat-delete`

### Requirement: Device states
A device's state SHALL be `this device`; `up to date` when it acknowledged every segment of this device that holds versions; `behind by <n> segments` when it did not; or `stale since <time>` when one of them was written more than `sync.stale_days` ago by this device's clock, `<time>` being when this device wrote the oldest. Versions held back because a version they follow has not arrived SHALL add `waiting <scope name> <device name>: <n> versions`.

#### Scenario: A laptop that was closed for a week
- **WHEN** `morthond` has not acknowledged this device's last 4 segments, the oldest from 7 days ago, and `sync.stale_days` is 180
- **THEN** its line is `device personal morthond: behind by 4 segments`

#### Scenario: A device gone for good
- **WHEN** `morthond` has not acknowledged a segment from 200 days ago and `sync.stale_days` is 180
- **THEN** its line ends with `stale since <the time this device wrote that segment>`

#### Scenario: A device with a wrong clock
- **WHEN** `morthond`'s clock runs a year behind and it acknowledges every segment within minutes
- **THEN** its line is `device personal morthond: up to date`

### Requirement: Status exit code and problems
`bilbo sync` SHALL exit 1 when a conflict or undeclared dropped text is open, a scope's transport failed or was full at its last attempt, watch stopped a scope (a pinned transport that differs from the config, a replaced confirmed manifest, a removed device), reading a device stopped at a missing or failing segment, or no `bilbo watch` runs against the store, and 0 otherwise. A stopped scope SHALL add watch's line for it to stderr. A stopped device SHALL add `bilbo: sync <name>: <device name> stopped at segment <seq>: <reason>` to stderr; when the segment fails to verify on a transport where its writer cannot replace it, the line SHALL end `; on <device name>, copy <root>/.bilbo/scopes/<scope id>/out/<seq>.seg over it`. A failed transport SHALL add `bilbo: sync <name>: <url> not reachable since <time>: <reason>` to stderr, and a full one `bilbo: sync <name>: full since <time>: <the transport's message>`. No running watcher SHALL add `bilbo: bilbo watch is not running; nothing syncs` to stderr.

#### Scenario: An unreachable folder
- **WHEN** the last poll of `personal` could not read its folder
- **THEN** stderr names the URL and the reason, stdout still lists the scope, and the exit code is 1

#### Scenario: A reader stuck at a gap
- **WHEN** this device's last poll stopped reading `bywater` at a missing segment 3
- **THEN** stderr holds `bilbo: sync personal: bywater stopped at segment 3: missing`, and the exit code is 1

#### Scenario: A pinned transport that differs
- **WHEN** watch stopped `personal` because its manifest pins another URL than the config
- **THEN** stderr holds watch's pin line for `personal`, and the exit code is 1

#### Scenario: No watcher
- **WHEN** no watcher runs against the store
- **THEN** stderr holds the not-running line and the exit code is 1

### Requirement: No syncing scope
When no scope's `sync` is a URL, `bilbo sync` SHALL print `bilbo: no scope syncs; set scope.<name>.sync in <config path>` to stderr, print nothing to stdout and exit 1.

#### Scenario: Sync never turned on
- **WHEN** the config declares no scope and a user runs `bilbo sync`
- **THEN** stderr names the config path, stdout is empty and the exit code is 1

### Requirement: Declaring dropped text
`bilbo sync declare <note> <reason>` SHALL record that the dropped text of the note's latest resolved conflict was dropped on purpose, print `declared <file name>: <n> lines dropped on purpose` and exit 0, and the record SHALL sync with the note. `<note>` is named as in `note-history`; `<reason>` is one line of 1 to 500 characters, else a usage error. With nothing to declare it SHALL print `bilbo: <note> has no dropped text to declare` to stderr and exit 1.

#### Scenario: Declaring a drop
- **WHEN** an agent resolved a conflict in `plan-release.md` by keeping one side, dropping 3 lines, and runs `bilbo sync declare release "B's date was superseded"`
- **THEN** stdout is `declared plan-release.md: 3 lines dropped on purpose`, and `bilbo check` no longer reports the dropped lines, here and on the other devices once they pull

#### Scenario: Before the watcher records the resolution
- **WHEN** an agent removes a conflict's markers, dropping text, and runs `bilbo sync declare` before watch records the save
- **THEN** the declaration applies to that conflict, and once watch records the save `bilbo check` reports no dropped text

#### Scenario: Nothing to declare
- **WHEN** an agent runs `bilbo sync declare release "x"` and `release` has no dropped text
- **THEN** stderr is `bilbo: release has no dropped text to declare` and the exit code is 1

#### Scenario: A reason over two lines
- **WHEN** the reason holds a newline
- **THEN** bilbo prints a usage message to stderr and exits 2

### Requirement: Sync reads, declare records
`bilbo sync` SHALL NOT create, change, rename or delete any file or folder. `bilbo sync declare` SHALL write only under `<root>/.bilbo/`. Neither SHALL read or write the transport. Both SHALL read the device key to open scope names, under the `device-identity` spec's Where keys live rules.

#### Scenario: Status changes nothing
- **WHEN** an agent runs `bilbo sync`
- **THEN** every entry under the root and under the transport folder has the same bytes and modification time as before

### Requirement: Sync arguments and store
`bilbo sync` SHALL take no argument except the `declare` form. Any other argument SHALL be a usage error. When `<root>/notes/` does not exist, it SHALL print `bilbo: no store at <root>` to stderr and exit 1.

#### Scenario: An unknown argument
- **WHEN** an agent runs `bilbo sync now`
- **THEN** bilbo prints a message naming `now` to stderr and exits 2

#### Scenario: Wrong BILBO_HOME
- **WHEN** `BILBO_HOME` names a folder with no `notes/` and an agent runs `bilbo sync`
- **THEN** stderr is `bilbo: no store at <that folder>` and the exit code is 1
