# Spec Delta

## MODIFIED Requirements

### Requirement: Config location
bilbo SHALL read its settings from `BILBO_CONFIG` when it is set and not empty, then `$XDG_CONFIG_HOME/bilbo/config` when `XDG_CONFIG_HOME` is an absolute path, then `$HOME/.config/bilbo/config`. A relative `BILBO_CONFIG` SHALL be a usage error. A missing file SHALL mean every setting takes its default; a `BILBO_CONFIG` that names a missing file SHALL be an error, except for `setup`, which creates the file there. Only `recall`, `index`, `setup`, `digest` and `watch` SHALL read settings; `new`, `check`, `library`, `cite`, `history` and `restore` SHALL run whatever the config holds. Where this spec has a verb exit 2 on a config error, `digest` instead exits 0 and prints nothing to stdout, as the `cli` spec's exit codes require; it still names the error on stderr.

#### Scenario: The default location
- **WHEN** neither `BILBO_CONFIG` nor `XDG_CONFIG_HOME` is set and `HOME` is `/Users/a`
- **THEN** bilbo reads `/Users/a/.config/bilbo/config`

#### Scenario: No config file is fine
- **WHEN** no config file exists at the default location and an agent runs `bilbo recall rollback`
- **THEN** recall runs keyword-only and prints nothing about the config

#### Scenario: An explicit file that does not exist
- **WHEN** `BILBO_CONFIG` is `/tmp/nope` and that file does not exist
- **THEN** `recall`, `index` and `watch` print a message naming `/tmp/nope` to stderr and exit 2

#### Scenario: Digest with an explicit file that does not exist
- **WHEN** `BILBO_CONFIG` is `/tmp/nope`, that file does not exist, and a hook runs `bilbo digest`
- **THEN** stdout is empty, stderr is one line naming `/tmp/nope`, and the exit code is 0

#### Scenario: Setup creates the explicit file
- **WHEN** `BILBO_CONFIG` is `/tmp/b/config`, that file does not exist, and a user runs `bilbo setup --yes --embedder-url http://127.0.0.1:8081 --embedder-model m` against a working embedder
- **THEN** `/tmp/b/config` exists, holds those two settings, and the exit code is 0

#### Scenario: Check ignores the config
- **WHEN** `BILBO_CONFIG` names a missing file and an agent runs `bilbo check` on a clean store
- **THEN** stderr is empty and the exit code is 0

#### Scenario: Library ignores the config
- **WHEN** the config file holds `embeder.url = http://bagend:8081`, an unknown key, and an agent runs `bilbo library`
- **THEN** stderr is empty and the exit code is 0

#### Scenario: Cite ignores the config
- **WHEN** `BILBO_CONFIG` names a missing file and an agent runs `bilbo cite` on a draft whose citations all resolve
- **THEN** stderr is empty and the exit code is 0

#### Scenario: History ignores the config
- **WHEN** `BILBO_CONFIG` names a missing file and an agent runs `bilbo history release` on a note with history
- **THEN** stdout lists the versions and the exit code is 0

## ADDED Requirements

### Requirement: History settings
The history key SHALL be `history.keep_days`: a whole number of days from 1 to 3650, 90 by default, the age past which pruning drops versions under the `note-history` spec's retention rule. Any other value SHALL be an error naming the key. It SHALL be valid with or without an embedder.

#### Scenario: The default
- **WHEN** the config file sets no history key
- **THEN** watch prunes versions older than 90 days

#### Scenario: A shorter window
- **WHEN** the config file holds `history.keep_days = 30`
- **THEN** watch prunes versions older than 30 days

#### Scenario: Not a number of days
- **WHEN** the config file holds `history.keep_days = 0`, `history.keep_days = 3651` or `history.keep_days = 2w`
- **THEN** every verb that reads settings reports an error naming `history.keep_days`, and `bilbo watch` exits 2
