# Spec Delta

## MODIFIED Requirements

### Requirement: Config location
bilbo SHALL read its settings from `BILBO_CONFIG` when it is set and not empty, then `$XDG_CONFIG_HOME/bilbo/config` when `XDG_CONFIG_HOME` is an absolute path, then `$HOME/.config/bilbo/config`. A relative `BILBO_CONFIG` SHALL be a usage error. A missing file SHALL mean every setting takes its default; a `BILBO_CONFIG` that names a missing file SHALL be an error, except for `setup`, which creates the file there. Only `recall`, `index`, `setup` and `digest` SHALL read settings; `new` and `check` SHALL run whatever the config holds. Where this spec has a verb exit 2 on a config error, `digest` instead exits 0 and prints nothing to stdout, as the `cli` spec's exit codes require; it still names the error on stderr.

#### Scenario: The default location
- **WHEN** neither `BILBO_CONFIG` nor `XDG_CONFIG_HOME` is set and `HOME` is `/Users/a`
- **THEN** bilbo reads `/Users/a/.config/bilbo/config`

#### Scenario: No config file is fine
- **WHEN** no config file exists at the default location and an agent runs `bilbo recall rollback`
- **THEN** recall runs keyword-only and prints nothing about the config

#### Scenario: An explicit file that does not exist
- **WHEN** `BILBO_CONFIG` is `/tmp/nope` and that file does not exist
- **THEN** `recall` and `index` print a message naming `/tmp/nope` to stderr and exit 2

#### Scenario: Digest with an explicit file that does not exist
- **WHEN** `BILBO_CONFIG` is `/tmp/nope`, that file does not exist, and a hook runs `bilbo digest`
- **THEN** stdout is empty, stderr is one line naming `/tmp/nope`, and the exit code is 0

#### Scenario: Setup creates the explicit file
- **WHEN** `BILBO_CONFIG` is `/tmp/b/config`, that file does not exist, and a user runs `bilbo setup --yes --embedder-url http://127.0.0.1:8081 --embedder-model m` against a working embedder
- **THEN** `/tmp/b/config` exists, holds those two settings, and the exit code is 0

#### Scenario: Check ignores the config
- **WHEN** `BILBO_CONFIG` names a missing file and an agent runs `bilbo check` on a clean store
- **THEN** stderr is empty and the exit code is 0

## ADDED Requirements

### Requirement: Digest settings
The digest keys SHALL be `digest.enable` (`on` or `off`, `on` by default; `off` turns the digest off), `digest.min_similarity` (a number from 0 to 1, 0.55 by default; the similarity a note's best passage needs to enter the digest when an embedder answers) and `digest.log` (`on` or `off`, `off` by default). Any other value SHALL be an error naming the key. The digest keys SHALL be valid with or without an embedder.

#### Scenario: Defaults
- **WHEN** the config file sets no digest key
- **THEN** the digest runs, its gate uses 0.55, and no digest log is written

#### Scenario: Digest keys alone
- **WHEN** the config file holds only `digest.log = on`
- **THEN** `bilbo recall rollback` runs keyword-only with nothing about the config on stderr

#### Scenario: A bad switch
- **WHEN** the config file holds `digest.log = yes`
- **THEN** every verb that reads settings reports an error naming `digest.log`, and `bilbo recall` exits 2

#### Scenario: A bad digest switch
- **WHEN** the config file holds `digest.enable = no`
- **THEN** every verb that reads settings reports an error naming `digest.enable`, and `bilbo recall` exits 2

#### Scenario: A similarity out of range
- **WHEN** the config file holds `digest.min_similarity = 1.5`
- **THEN** every verb that reads settings reports an error naming `digest.min_similarity`, and `bilbo index` exits 2
