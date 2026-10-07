## ADDED Requirements

### Requirement: Human view of history
When stdout gets the `cli` spec's human view, `bilbo history <note>` SHALL print the newest version's file name in bold, then one indented row per version: the version in cyan, the event in an aligned column, `added` in green and `restored` in cyan, the time as an age, then `from <device>`, the flags and a file name that differs from the first, dim. `--diff` SHALL colour `---` and `+++` bold, `@@` lines cyan, removed lines red and added lines green, and with no difference print `◇  no changes between <a> and <b>`, `now` for the file. A printed version SHALL stay its bytes.

#### Scenario: A version list on a terminal
- **WHEN** `decision-release.md` was added 3 days ago and edited 16 minutes ago, and a user runs `bilbo history release` in a terminal
- **THEN** the first line is `decision-release.md`, the next row holds `edited` and `16 min ago`, and the last holds `added` and `3 days ago`

#### Scenario: Equal versions on a terminal
- **WHEN** a user runs `bilbo history release --diff <a> <b>` in a terminal and the two versions hold the same bytes
- **THEN** stdout is `◇  no changes between <a> and <b>` and the exit code is 0

#### Scenario: Equal versions down a pipe
- **WHEN** an agent runs `bilbo history release --diff <a> <b>` with stdout piped and the two versions hold the same bytes
- **THEN** stdout is empty

#### Scenario: A printed version on a terminal
- **WHEN** a user runs `bilbo history release <version>` in a terminal
- **THEN** stdout is the version's bytes, with no escape byte added
