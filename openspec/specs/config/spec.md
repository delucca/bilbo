# config Specification

## Purpose
Where bilbo reads its settings and the exact shape of the settings file, so a human can point bilbo at an embedder once and every verb, hook and timer sees the same values.

## Requirements

### Requirement: Config location
bilbo SHALL read its settings from `BILBO_CONFIG` when it is set and not empty, then `$XDG_CONFIG_HOME/bilbo/config` when `XDG_CONFIG_HOME` is an absolute path, then `$HOME/.config/bilbo/config`. A relative `BILBO_CONFIG` SHALL be a usage error. A missing file SHALL mean every setting takes its default; a `BILBO_CONFIG` that names a missing file SHALL be an error, except for `setup`, which creates the file there. Only `recall`, `index`, `setup` and `digest` SHALL read settings; `new`, `check`, `library` and `cite` SHALL run whatever the config holds. Where this spec has a verb exit 2 on a config error, `digest` instead exits 0 and prints nothing to stdout, as the `cli` spec's exit codes require; it still names the error on stderr.

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

#### Scenario: Library ignores the config
- **WHEN** the config file holds `embeder.url = http://bagend:8081`, an unknown key, and an agent runs `bilbo library`
- **THEN** stderr is empty and the exit code is 0

#### Scenario: Cite ignores the config
- **WHEN** `BILBO_CONFIG` names a missing file and an agent runs `bilbo cite` on a draft whose citations all resolve
- **THEN** stderr is empty and the exit code is 0

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
