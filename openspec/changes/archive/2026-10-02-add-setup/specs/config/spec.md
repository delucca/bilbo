# Spec Delta

## MODIFIED Requirements

### Requirement: Config location
bilbo SHALL read its settings from `BILBO_CONFIG` when it is set and not empty, then `$XDG_CONFIG_HOME/bilbo/config` when `XDG_CONFIG_HOME` is an absolute path, then `$HOME/.config/bilbo/config`. A relative `BILBO_CONFIG` SHALL be a usage error. A missing file SHALL mean every setting takes its default; a `BILBO_CONFIG` that names a missing file SHALL be an error, except for `setup`, which creates the file there. Only `recall`, `index` and `setup` SHALL read settings; `new` and `check` SHALL run whatever the config holds.

#### Scenario: The default location
- **WHEN** neither `BILBO_CONFIG` nor `XDG_CONFIG_HOME` is set and `HOME` is `/Users/a`
- **THEN** bilbo reads `/Users/a/.config/bilbo/config`

#### Scenario: No config file is fine
- **WHEN** no config file exists at the default location and an agent runs `bilbo recall rollback`
- **THEN** recall runs keyword-only and prints nothing about the config

#### Scenario: An explicit file that does not exist
- **WHEN** `BILBO_CONFIG` is `/tmp/nope` and that file does not exist
- **THEN** `recall` and `index` print a message naming `/tmp/nope` to stderr and exit 2

#### Scenario: Setup creates the explicit file
- **WHEN** `BILBO_CONFIG` is `/tmp/b/config`, that file does not exist, and a user runs `bilbo setup --yes --embedder-url http://127.0.0.1:8081 --embedder-model m` against a working embedder
- **THEN** `/tmp/b/config` exists, holds those two settings, and the exit code is 0

#### Scenario: Check ignores the config
- **WHEN** `BILBO_CONFIG` names a missing file and an agent runs `bilbo check` on a clean store
- **THEN** stderr is empty and the exit code is 0
