# Spec Delta

## MODIFIED Requirements

### Requirement: Config location
bilbo SHALL read its settings from `BILBO_CONFIG` when it is set and not empty, then `$XDG_CONFIG_HOME/bilbo/config` when `XDG_CONFIG_HOME` is an absolute path, then `$HOME/.config/bilbo/config`. A relative `BILBO_CONFIG` SHALL be a usage error. A missing file SHALL mean every setting takes its default; a `BILBO_CONFIG` that names a missing file SHALL be an error, except for `setup`, which creates the file there. Only `recall`, `index`, `setup`, `digest`, `watch`, `new`, `check`, `scope` and `device` SHALL read settings; `library`, `cite`, `history` and `restore` SHALL run whatever the config holds. Where this spec has a verb exit 2 on a config error, `digest` instead exits 0 and prints nothing to stdout, as the `cli` spec's exit codes require; it still names the error on stderr.

#### Scenario: The default location
- **WHEN** neither `BILBO_CONFIG` nor `XDG_CONFIG_HOME` is set and `HOME` is `/Users/a`
- **THEN** bilbo reads `/Users/a/.config/bilbo/config`

#### Scenario: No config file is fine
- **WHEN** no config file exists at the default location and an agent runs `bilbo recall rollback`
- **THEN** recall runs keyword-only and prints nothing about the config

#### Scenario: An explicit file that does not exist
- **WHEN** `BILBO_CONFIG` is `/tmp/nope` and that file does not exist
- **THEN** `recall`, `index`, `watch`, `new`, `check`, `scope` and `device` print a message naming `/tmp/nope` to stderr and exit 2

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

#### Scenario: Library ignores the config
- **WHEN** the config file holds `embeder.url = http://bagend:8081`, an unknown key, and an agent runs `bilbo library`
- **THEN** stderr is empty and the exit code is 0

#### Scenario: Cite ignores the config
- **WHEN** `BILBO_CONFIG` names a missing file and an agent runs `bilbo cite` on a draft whose citations all resolve
- **THEN** stderr is empty and the exit code is 0

#### Scenario: History ignores the config
- **WHEN** `BILBO_CONFIG` names a missing file and an agent runs `bilbo history release` on a note with history
- **THEN** stdout lists the versions and the exit code is 0

### Requirement: Scope settings
The scope keys SHALL be `scope.<name>.sync`, `scope.<name>.embedder`, `scope.<name>.paths`, `scope.<name>.marks` and `scope.default`, where `<name>` has the topic's grammar and is not `default`. Any `scope.<name>.*` key declares the scope `<name>`. `sync` SHALL be `off`, its default, or a sync URL as the Scope sync URLs requirement gives. `embedder` SHALL be `any`, its default, or `local`. `scope.default` SHALL name a declared scope. Any other value, name or sub-key SHALL be an error naming the key. The keys SHALL be valid with or without an embedder.

#### Scenario: A full declaration
- **WHEN** the config file holds `scope.personal.sync = off`, `scope.work.embedder = local`, `scope.work.paths = ~/Developer/acme`, `scope.work.marks = acme, ~/Developer/acme` and `scope.default = personal`
- **THEN** `bilbo scope` lists `personal` and `work`, and `bilbo recall rollback` runs with nothing about the config on stderr

#### Scenario: One key declares a scope
- **WHEN** the config file holds only `scope.work.marks = acme`
- **THEN** `work` is declared, with `sync off` and `embedder any`

#### Scenario: Sync takes a URL
- **WHEN** the config file holds `scope.personal.sync = https://relay.example.net`
- **THEN** `bilbo scope` lists `personal` with `sync https://relay.example.net` and exits 0

#### Scenario: An unknown sync value
- **WHEN** the config file holds `scope.personal.sync = on`
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

## ADDED Requirements

### Requirement: Scope sync URLs
A sync URL SHALL be `file://` followed by an absolute path, taken literally with no percent-decoding, or `https://<host>[:<port>][/<prefix>]`, or `http://` of that shape only when the host is `localhost`, `127.0.0.1` or `::1`. It SHALL hold no user name, password, query, fragment or control character, an `https://` or `http://` URL SHALL hold no whitespace (a `file://` path may), and a port SHALL be 1 to 65535. Any other value SHALL be an error naming the key, and an error about a user name or password SHALL NOT repeat the URL.

#### Scenario: A folder
- **WHEN** the config file holds `scope.personal.sync = file:///Users/a/Library/Mobile Documents/bilbo`
- **THEN** the URL is accepted and names the folder `/Users/a/Library/Mobile Documents/bilbo`, and no verb reports a config error

#### Scenario: A relay with a port and a prefix
- **WHEN** the config file holds `scope.personal.sync = https://relay.example.net:8443/bilbo`
- **THEN** no verb reports a config error

#### Scenario: Plain HTTP to a loopback relay
- **WHEN** the config file holds `scope.personal.sync = http://127.0.0.1:8740`
- **THEN** no verb reports a config error

#### Scenario: Plain HTTP to another host
- **WHEN** the config file holds `scope.personal.sync = http://relay.example.net`
- **THEN** every verb that reads settings reports an error naming `scope.personal.sync`, and `bilbo device` exits 2

#### Scenario: A relative folder
- **WHEN** the config file holds `scope.personal.sync = file://Sync/bilbo` or `scope.personal.sync = ~/Sync/bilbo`
- **THEN** every verb that reads settings reports an error naming `scope.personal.sync`

#### Scenario: A password in the URL is not echoed
- **WHEN** the config file holds `scope.personal.sync = https://u:sekrit@relay.example.net`
- **THEN** every verb that reads settings prints a message naming `scope.personal.sync` to stderr, holds no part of `sekrit`, and exits 2

#### Scenario: Another scheme or a query
- **WHEN** the config file holds `scope.personal.sync = ftp://relay.example.net` or `scope.personal.sync = https://relay.example.net/?token=1`
- **THEN** every verb that reads settings reports an error naming `scope.personal.sync`
