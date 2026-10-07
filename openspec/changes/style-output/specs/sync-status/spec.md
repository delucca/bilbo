## MODIFIED Requirements

### Requirement: Status report
`bilbo sync` SHALL print, in order: per syncing scope by name, `scope <name> <url>: <n> notes, pushed <time>, pulled <time>` (`1 note` for one) and a `device <scope name> <device name>: <state>` line per device of its latest manifest; `local: <n> notes sync nowhere` (`local: 1 note syncs nowhere` for one); by file, `conflict notes/<file>: <n> passages` and `dropped notes/<file>: <n> lines not declared` (`passage`, `line` for one); `notice <time> notes/<file>: <flag>` per flag of the last 7 days; and `change <time> <name>: device <device name> added by <signer> (manifest <n>)` or `change <time> <name>: epoch changed (manifest <n>)` per manifest change of the last 30 days that this device did not write. `<time>` is in the `created` form, or `never`.

#### Scenario: Two devices, in step
- **WHEN** this device and `bywater` sync `personal` with 42 notes, 17 notes have no scope, and nothing is open
- **THEN** stdout is `scope personal file:///srv/bilbo: 42 notes, pushed <time>, pulled <time>`, `device personal rhosgobel: this device`, `device personal bywater: up to date` and `local: 17 notes sync nowhere`, and the exit code is 0

#### Scenario: One note each
- **WHEN** `personal` syncs with 1 note and 1 note has no scope
- **THEN** stdout holds `scope personal <url>: 1 note, pushed <time>, pulled <time>` and `local: 1 note syncs nowhere`

#### Scenario: An open conflict
- **WHEN** `notes/gotcha-nix.md` holds an open conflict in one passage
- **THEN** stdout holds `conflict notes/gotcha-nix.md: 1 passage` and the exit code is 1

#### Scenario: A device added on another machine
- **WHEN** `morthond` was added to `personal` by `bilbo device recover` on `morthond` 3 days ago
- **THEN** stdout holds `change <that time> personal: device morthond added by owner key (manifest 4)`, so the user can revoke `morthond` if they do not know it

#### Scenario: A flag from yesterday
- **WHEN** an edit from `bywater` brought back a note deleted here yesterday
- **THEN** stdout holds a `notice` line naming the file and `edit-beat-delete`

## ADDED Requirements

### Requirement: Human view of sync
When stdout gets the `cli` spec's human view, `bilbo sync` SHALL print per syncing scope its name in bold and its URL dim with paths from `~`, an indented line of its note count and the push and pull times as ages, and an indented line per device with `◆` and green for this device and `up to date`, and `▲` and yellow for any other state; then the `local` line, then the other lines of Status report. Problems SHALL follow on stderr as warnings, and the exit code SHALL be as in Status exit code and problems.

#### Scenario: A device behind on a terminal
- **WHEN** `bywater` is behind by 2 segments in `personal` and a user runs `bilbo sync` in a terminal
- **THEN** a line under `personal` starts with `▲` and holds `bywater` and `behind by 2 segments`

#### Scenario: A pipe keeps the lines
- **WHEN** an agent runs `bilbo sync` with stdout piped
- **THEN** stdout is the lines of Status report
