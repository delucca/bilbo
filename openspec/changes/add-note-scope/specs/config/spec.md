# Spec Delta

## MODIFIED Requirements

### Requirement: Config location
bilbo SHALL read its settings from `BILBO_CONFIG` when it is set and not empty, then `$XDG_CONFIG_HOME/bilbo/config` when `XDG_CONFIG_HOME` is an absolute path, then `$HOME/.config/bilbo/config`. A relative `BILBO_CONFIG` SHALL be a usage error. A missing file SHALL mean every setting takes its default; a `BILBO_CONFIG` that names a missing file SHALL be an error, except for `setup`, which creates the file there. Only `recall`, `index`, `setup`, `digest`, `watch`, `new`, `check` and `scope` SHALL read settings; `history` and `restore` SHALL run whatever the config holds. Where this spec has a verb exit 2 on a config error, `digest` instead exits 0 and prints nothing to stdout, as the `cli` spec's exit codes require; it still names the error on stderr.

#### Scenario: The default location
- **WHEN** neither `BILBO_CONFIG` nor `XDG_CONFIG_HOME` is set and `HOME` is `/Users/a`
- **THEN** bilbo reads `/Users/a/.config/bilbo/config`

#### Scenario: No config file is fine
- **WHEN** no config file exists at the default location and an agent runs `bilbo recall rollback`
- **THEN** recall runs keyword-only and prints nothing about the config

#### Scenario: An explicit file that does not exist
- **WHEN** `BILBO_CONFIG` is `/tmp/nope` and that file does not exist
- **THEN** `recall`, `index`, `watch`, `new`, `check` and `scope` print a message naming `/tmp/nope` to stderr and exit 2

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

### Requirement: Scope settings
The scope keys SHALL be `scope.<name>.sync`, `scope.<name>.embedder`, `scope.<name>.paths`, `scope.<name>.marks` and `scope.default`, where `<name>` has the topic's grammar and is not `default`. Any `scope.<name>.*` key declares the scope `<name>`. `sync` SHALL be `off`, its default. `embedder` SHALL be `any`, its default, or `local`. `scope.default` SHALL name a declared scope. Any other value, name or sub-key SHALL be an error naming the key. The keys SHALL be valid with or without an embedder.

#### Scenario: A full declaration
- **WHEN** the config file holds `scope.personal.sync = off`, `scope.work.embedder = local`, `scope.work.paths = ~/Developer/acme`, `scope.work.marks = acme, ~/Developer/acme` and `scope.default = personal`
- **THEN** `bilbo scope` lists `personal` and `work`, and `bilbo recall rollback` runs with nothing about the config on stderr

#### Scenario: One key declares a scope
- **WHEN** the config file holds only `scope.work.marks = acme`
- **THEN** `work` is declared, with `sync off` and `embedder any`

#### Scenario: An unknown sync value
- **WHEN** the config file holds `scope.personal.sync = https://relay.example.net`
- **THEN** every verb that reads settings reports an error naming `scope.personal.sync`, and `bilbo scope` exits 2

#### Scenario: A bad embedder rule
- **WHEN** the config file holds `scope.work.embedder = remote`
- **THEN** every verb that reads settings reports an error naming `scope.work.embedder`, and `bilbo index` exits 2

#### Scenario: A bad name or sub-key
- **WHEN** the config file holds `scope.Work.sync = off`, `scope.work.colour = red` or `scope.default.sync = off`
- **THEN** every verb that reads settings reports an error naming that key, and `bilbo check` exits 2

#### Scenario: A default that is not declared
- **WHEN** the config file holds `scope.default = acme` and no other `scope.acme.*` key
- **THEN** every verb that reads settings reports an error naming `scope.default` and `acme`, and `bilbo new plan release` exits 2 and writes no file

### Requirement: Scope paths and marks
`scope.<name>.paths` and `scope.<name>.marks` SHALL be comma-separated lists, each item trimmed and not empty. A path SHALL be `~/` (the home folder), `/`, or absolute or starting with `~/`, a trailing `/` dropped. A mark starting with `/` or `~/` is a path mark, held to the same rule. Any other mark SHALL be one word as `recall` defines words. Two scopes' `paths` naming one folder, after `~/` expansion and resolving links, or two scopes' `marks` holding one mark, SHALL be an error naming both keys.

#### Scenario: A list
- **WHEN** the config file holds `scope.work.paths = ~/Developer/acme, /srv/acme/`
- **THEN** `bilbo scope` shows `paths ~/Developer/acme, /srv/acme/` for `work`, and a working directory under `/srv/acme` picks `work`

#### Scenario: A relative path
- **WHEN** the config file holds `scope.work.paths = Developer/acme`
- **THEN** every verb that reads settings reports an error naming `scope.work.paths`

#### Scenario: A mark that is not one word
- **WHEN** the config file holds `scope.work.marks = acme corp` or `scope.work.marks = acme,,beta`
- **THEN** every verb that reads settings reports an error naming `scope.work.marks`

#### Scenario: One path in two scopes
- **WHEN** the config file holds `scope.work.paths = ~/Developer/acme` and `scope.personal.paths = ~/Developer/acme/`
- **THEN** every verb that reads settings reports an error naming `scope.work.paths` and `scope.personal.paths`

#### Scenario: One folder under two spellings
- **WHEN** `HOME` is `/Users/a`, the config file holds `scope.work.paths = ~/Developer/acme` and `scope.personal.paths = /Users/a/src`, and `/Users/a/src` is a link to `/Users/a/Developer/acme`
- **THEN** every verb that reads settings reports an error naming `scope.work.paths` and `scope.personal.paths`

#### Scenario: The home folder as a path
- **WHEN** the config file holds `scope.personal.paths = ~/` and `scope.work.paths = ~/Developer/acme`
- **THEN** a working directory under `~/Developer/acme` picks `work`, and any other one under the home folder picks `personal`
