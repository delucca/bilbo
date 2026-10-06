# config Specification

## Purpose
Where bilbo reads its settings and the exact shape of the settings file, so a human can point bilbo at an embedder once and every verb, hook and timer sees the same values.

## Requirements

### Requirement: Config location
bilbo SHALL read its settings from `BILBO_CONFIG` when it is set and not empty, then `$XDG_CONFIG_HOME/bilbo/config` when `XDG_CONFIG_HOME` is an absolute path, then `$HOME/.config/bilbo/config`. A relative `BILBO_CONFIG` SHALL be a usage error. A missing file SHALL mean every setting takes its default; a `BILBO_CONFIG` that names a missing file SHALL be an error, except for `setup`, which creates the file there. Only `recall`, `index`, `setup`, `digest`, `watch`, `new`, `check`, `scope`, `device`, `sync` and `pair` SHALL read settings; `library`, `cite`, `history`, `restore` and `relay` SHALL run whatever the config holds. Where this spec has a verb exit 2 on a config error, `digest` instead exits 0 and prints nothing to stdout, as the `cli` spec's exit codes require; it still names the error on stderr.

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

#### Scenario: Library ignores the config
- **WHEN** the config file holds `embeder.url = http://bagend:8081`, an unknown key, and an agent runs `bilbo library`
- **THEN** stderr is empty and the exit code is 0

#### Scenario: Cite ignores the config
- **WHEN** `BILBO_CONFIG` names a missing file and an agent runs `bilbo cite` on a draft whose citations all resolve
- **THEN** stderr is empty and the exit code is 0

#### Scenario: History ignores the config
- **WHEN** `BILBO_CONFIG` names a missing file and an agent runs `bilbo history release` on a note with history
- **THEN** stdout lists the versions and the exit code is 0

#### Scenario: Pair reads the config
- **WHEN** `BILBO_CONFIG` names a missing file and a user runs `bilbo pair` or `bilbo pair <code> --via <url>`
- **THEN** bilbo prints a message naming the file to stderr, exits 2, and creates no mailbox

#### Scenario: Relay ignores the config
- **WHEN** `BILBO_CONFIG` names a missing file and a user runs `bilbo relay --data /srv/relay --owner <fingerprint>`
- **THEN** the relay starts and prints nothing about the config

### Requirement: Config format
The config file SHALL hold one setting per line as `<key> = <value>`, with blank lines and lines starting with `#` ignored. Keys are known names, each at most once. A value runs to the end of the line with surrounding spaces trimmed. A value wrapped in double quotes keeps its spaces, and inside it `\"` is a quote. In any value, `\n` stands for a newline and `\\` for a backslash. A `#` after the key is part of the value. Any other line, an unknown key, a repeated key, an empty value for any key but `embedder.query_prefix`, or text after a closing quote SHALL be an error naming the file and line.

#### Scenario: A valid file
- **WHEN** the file holds `# embedder on bagend`, a blank line and `embedder.url = http://bagend:8081`
- **THEN** the embedder URL is `http://bagend:8081`

#### Scenario: An escaped newline
- **WHEN** the file holds `embedder.query_prefix = "Instruct: find notes\nQuery: "`
- **THEN** the query prefix is `Instruct: find notes`, a newline, then `Query: ` with its trailing space

#### Scenario: An unquoted value is trimmed
- **WHEN** the file holds `embedder.model = qwen3-embedding-0.6b   ` with trailing spaces
- **THEN** the model is `qwen3-embedding-0.6b`

#### Scenario: An empty value
- **WHEN** the file holds `embedder.model =` on line 2
- **THEN** every verb that reads settings prints a message naming the file, line 2 and `embedder.model` to stderr and exits 2

#### Scenario: An unknown key
- **WHEN** the file holds `embeder.url = http://bagend:8081` on line 3
- **THEN** every verb that reads settings prints a message naming the file, line 3 and `embeder.url` to stderr and exits 2

### Requirement: Embedder settings
The embedder keys SHALL be `embedder.url` (an `http` or `https` URL; without it bilbo is keyword-only), `embedder.model` (required with a URL), `embedder.token_file` or `embedder.token_env` (at most one; a bearer token read from that file, an absolute path or one starting with `~/`, or from that variable, with surrounding whitespace trimmed), `embedder.query_prefix` (text put before every query, empty by default) and `embedder.min_similarity` (a number from 0 to 1, 0.5 by default).

#### Scenario: A URL without a model
- **WHEN** the file sets `embedder.url` and not `embedder.model`
- **THEN** every verb that reads settings prints a message naming `embedder.model` to stderr and exits 2

#### Scenario: Both token sources
- **WHEN** the file sets both `embedder.token_file` and `embedder.token_env`
- **THEN** every verb that reads settings prints a message naming both keys to stderr and exits 2

#### Scenario: A similarity out of range
- **WHEN** the file holds `embedder.min_similarity = 1.5`
- **THEN** every verb that reads settings prints a message naming `embedder.min_similarity` to stderr and exits 2

### Requirement: Secrets stay out of output
bilbo SHALL NOT print an embedder token, in full or in part, on stdout or stderr. A missing or empty token file or variable SHALL be reported by its path or variable name only. An `embedder.url` that holds a user name or password SHALL be an error that does not repeat the URL.

#### Scenario: An empty token variable
- **WHEN** `embedder.token_env = EMBED_TOKEN` and `EMBED_TOKEN` is empty, and an agent runs `bilbo index`
- **THEN** stderr names `EMBED_TOKEN` and the exit code is 1

#### Scenario: A password in the URL is not echoed
- **WHEN** the file holds `embedder.url = http://u:sekrit@bagend:8081`
- **THEN** every verb that reads settings prints a message naming `embedder.url` to stderr, holds no part of `sekrit`, and exits 2

#### Scenario: A rejected token is not echoed
- **WHEN** the embedder answers 401 to a request carrying the token
- **THEN** stderr names the URL and the status 401 and holds no part of the token

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
