# Spec Delta

## MODIFIED Requirements

### Requirement: Config location
bilbo SHALL read its settings from `BILBO_CONFIG` when it is set and not empty, then `$XDG_CONFIG_HOME/bilbo/config` when `XDG_CONFIG_HOME` is an absolute path, then `$HOME/.config/bilbo/config`. A relative `BILBO_CONFIG` SHALL be a usage error. A missing file SHALL mean every setting takes its default; a `BILBO_CONFIG` that names a missing file SHALL be an error, except for `setup`, which creates the file there. Only `recall`, `index`, `setup`, `digest`, `watch`, `new`, `check`, `scope`, `device` and `sync` SHALL read settings; `history` and `restore` SHALL run whatever the config holds. Where this spec has a verb exit 2 on a config error, `digest` instead exits 0 and prints nothing to stdout, as the `cli` spec's exit codes require; it still names the error on stderr.

#### Scenario: The default location
- **WHEN** neither `BILBO_CONFIG` nor `XDG_CONFIG_HOME` is set and `HOME` is `/Users/a`
- **THEN** bilbo reads `/Users/a/.config/bilbo/config`

#### Scenario: No config file is fine
- **WHEN** no config file exists at the default location and an agent runs `bilbo recall rollback`
- **THEN** recall runs keyword-only and prints nothing about the config

#### Scenario: An explicit file that does not exist
- **WHEN** `BILBO_CONFIG` is `/tmp/nope` and that file does not exist
- **THEN** `recall`, `index`, `watch`, `new`, `check`, `scope`, `device` and `sync` print a message naming `/tmp/nope` to stderr and exit 2

#### Scenario: Digest with an explicit file that does not exist
- **WHEN** `BILBO_CONFIG` is `/tmp/nope`, that file does not exist, and a hook runs `bilbo digest`
- **THEN** stdout is empty, stderr is one line naming `/tmp/nope`, and the exit code is 0

#### Scenario: Setup creates the explicit file
- **WHEN** `BILBO_CONFIG` is `/tmp/b/config`, that file does not exist, and a user runs `bilbo setup --yes --embedder-url http://127.0.0.1:8081 --embedder-model m` against a working embedder
- **THEN** `/tmp/b/config` exists, holds those two settings, and the exit code is 0

#### Scenario: Check ignores the config
- **WHEN** no config file exists at the default location, or the file sets only embedder and digest keys with an embedder that does not answer, and an agent runs `bilbo check` on a clean store
- **THEN** stderr is empty, no request reaches the embedder, and the exit code is 0

#### Scenario: Check reads the config
- **WHEN** `BILBO_CONFIG` names a missing file and an agent runs `bilbo check` on a clean store
- **THEN** stderr names that file, stdout is empty, and the exit code is 2

#### Scenario: New with no config file
- **WHEN** no config file exists at the default location and an agent runs `bilbo new plan release`
- **THEN** the note is created with no `scope` key, stderr is empty, and the exit code is 0

#### Scenario: History ignores the config
- **WHEN** `BILBO_CONFIG` names a missing file and an agent runs `bilbo history release` on a note with history
- **THEN** stdout lists the versions and the exit code is 0

## ADDED Requirements

### Requirement: Sync settings
The sync keys SHALL be `sync.poll_seconds`, a whole number from 1 to 3600, 30 by default, how often watch looks for other devices' segments; and `sync.stale_days`, a whole number from 1 to 3650, 180 by default, how long a device may leave a segment unacknowledged before it is stale. Any other value SHALL be an error naming the key. Both SHALL be valid with or without an embedder and with no scope declared.

#### Scenario: Defaults
- **WHEN** the config file sets no sync key and a scope syncs
- **THEN** watch polls every 30 seconds, and a device is stale after 180 days

#### Scenario: Fast polling for a test
- **WHEN** the config file holds `sync.poll_seconds = 1`
- **THEN** watch polls every second

#### Scenario: Out of range
- **WHEN** the config file holds `sync.poll_seconds = 0` or `sync.stale_days = 1y`
- **THEN** every verb that reads settings reports an error naming that key, and `bilbo watch` exits 2
