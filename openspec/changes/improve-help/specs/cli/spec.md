## MODIFIED Requirements

### Requirement: Verb dispatch
`bilbo` SHALL read its first argument as a verb and run that verb. The verbs are `new`, `check`, `recall`, `index`, `setup`, `digest`, `library`, `cite`, `watch`, `history`, `restore`, `scope`, `device`, `sync`, `pair` and `relay`. `help` SHALL be a form of Help, not a verb. Any other first argument, or no argument, SHALL be a usage error that prints `bilbo`'s own synopsis, the line `verbs: ` followed by the verbs as the overview orders them, joined by `, `, and a line naming `bilbo --help`.

#### Scenario: A known verb runs
- **WHEN** an agent runs `bilbo check`
- **THEN** bilbo runs the check verb

#### Scenario: Setup is a verb
- **WHEN** a user runs `bilbo setup --yes`
- **THEN** bilbo runs the setup verb

#### Scenario: Digest is a verb
- **WHEN** a hook runs `bilbo digest` with a hook payload on stdin
- **THEN** bilbo runs the digest verb

#### Scenario: Library is a verb
- **WHEN** an agent runs `bilbo library`
- **THEN** bilbo runs the library verb

#### Scenario: Cite is a verb
- **WHEN** an agent runs `bilbo cite` with a draft on stdin
- **THEN** bilbo runs the cite verb

#### Scenario: History verbs
- **WHEN** an agent runs `bilbo history release` or `bilbo restore release a1b2c3`, or a service runs `bilbo watch`
- **THEN** bilbo runs the history, restore or watch verb

#### Scenario: Scope is a verb
- **WHEN** an agent runs `bilbo scope` or `bilbo scope set work <file>`
- **THEN** bilbo runs the scope verb

#### Scenario: Device is a verb
- **WHEN** a user runs `bilbo device` or `bilbo device revoke bywater`
- **THEN** bilbo runs the device verb

#### Scenario: Sync is a verb
- **WHEN** an agent runs `bilbo sync` or `bilbo sync declare release "superseded"`
- **THEN** bilbo runs the sync verb

#### Scenario: Pair is a verb
- **WHEN** a user runs `bilbo pair` on an enrolled device, or `bilbo pair <code> --via <url>` on a new one
- **THEN** bilbo runs the pair verb

#### Scenario: Relay is a verb
- **WHEN** a user runs `bilbo relay --data /srv/relay --owner <fingerprint>`
- **THEN** bilbo runs the relay verb

#### Scenario: An unknown verb is a usage error
- **WHEN** an agent runs `bilbo frobnicate`
- **THEN** stderr is `bilbo: unknown verb 'frobnicate'`, `bilbo: usage: bilbo <verb> [<args>]...`, `bilbo: verbs: new, recall, check, history, restore, library, cite, scope, sync, device, pair, relay, setup, index, watch, digest` and `bilbo: see 'bilbo --help'`, the exit code is 2, and bilbo creates, changes or deletes no file

#### Scenario: No arguments is a usage error
- **WHEN** an agent runs `bilbo` with no arguments
- **THEN** stderr is `bilbo: missing verb` and the same three lines that follow an unknown verb, stdout is empty and the exit code is 2

### Requirement: Help
`bilbo --help`, `bilbo -h`, `bilbo help` and `bilbo help help` SHALL print the overview: what bilbo is, the verbs in groups with a one-line summary each, examples, the exit codes, the path rules and a docs link. `bilbo <verb> --help`, `-h` or `--help` given anywhere before a bare `--`, except as the value of `--title` of `new` or `library`, or of `--scope` of `new`, and `bilbo help <verb>` SHALL print that verb's page: its summary, synopsis, options, output, exit codes, examples and docs link. A subcommand's help is its verb's page. Help SHALL go to stdout and exit 0, in lines of at most 80 columns.

#### Scenario: Help goes to stdout
- **WHEN** an agent runs `bilbo --help`, `bilbo -h` or `bilbo help`
- **THEN** stdout holds the overview, the same for all three, stderr is empty and the exit code is 0

#### Scenario: A verb's help is its page
- **WHEN** an agent runs `bilbo recall --help`, `bilbo recall -h` or `bilbo help recall`
- **THEN** stdout is the same for all three, holds `bilbo recall <query>` and not `bilbo new <kind>`, and the exit code is 0

#### Scenario: A subcommand's help is its verb's page
- **WHEN** an agent runs `bilbo library land -h`
- **THEN** stdout is what `bilbo library --help` prints

#### Scenario: Help for an unknown verb is a usage error
- **WHEN** an agent runs `bilbo help frobnicate` or `bilbo frobnicate --help`
- **THEN** stdout is empty, stderr is what `bilbo frobnicate` prints, and the exit code is 2

#### Scenario: Help for help is the overview
- **WHEN** an agent runs `bilbo help help`
- **THEN** stdout is what `bilbo --help` prints and the exit code is 0

#### Scenario: Help takes one verb
- **WHEN** an agent runs `bilbo help recall now`
- **THEN** stderr starts with `bilbo: unexpected argument 'now'` and the exit code is 2

#### Scenario: An unknown option is not help
- **WHEN** an agent runs `bilbo check --verbose`
- **THEN** bilbo prints a message naming `--verbose` as unknown to stderr and exits 2

### Requirement: Version
`bilbo --version`, as the only argument, SHALL print `bilbo <version>` and a newline to stdout and exit 0, where `<version>` is the `version` in `Cargo.toml` the binary was built from. `--version` after a verb SHALL stay an unknown option of that verb.

#### Scenario: The version goes to stdout
- **WHEN** a binary built from `version = "0.2.0"` runs `bilbo --version`
- **THEN** stdout is `bilbo 0.2.0` and a newline, stderr is empty and the exit code is 0

#### Scenario: The version is not a verb option
- **WHEN** an agent runs `bilbo check --version`
- **THEN** bilbo prints a message naming `--version` as unknown to stderr and exits 2

#### Scenario: Extra arguments are a usage error
- **WHEN** an agent runs `bilbo --version now`
- **THEN** stderr starts with `bilbo: unexpected argument 'now'` and the exit code is 2

## ADDED Requirements

### Requirement: Help styling
Help SHALL be plain text, except that headings and the words of a page's left column SHALL be bold, and nothing else, when stdout is a terminal, `NO_COLOR` is unset or empty, and `TERM` is not `dumb`. Bold SHALL never reach stderr.

#### Scenario: Piped help is plain
- **WHEN** an agent runs `bilbo --help | cat` with `TERM=xterm-256color`
- **THEN** stdout holds no escape byte

#### Scenario: NO_COLOR turns bold off
- **WHEN** a user runs `bilbo recall --help` in a terminal with `NO_COLOR=1`
- **THEN** stdout holds no escape byte

#### Scenario: A terminal gets bold headings
- **WHEN** a user runs `bilbo --help` in a terminal with `TERM=xterm-256color` and no `NO_COLOR`
- **THEN** the `Usage:` heading is drawn in bold and the line `bilbo keeps durable memory for coding agents: notes they write and recall,` is plain

### Requirement: Usage errors
A usage error SHALL print to stderr, each line starting with `bilbo: `: the reason, as its first line; then `usage: ` and the synopsis forms of the verb that was run, one unwrapped form per line, only those of its subcommand when the argument after the verb names one; then `see 'bilbo <verb> --help'`. With no verb, an unknown one or `help`, the forms and page are `bilbo`'s own. It SHALL never print the overview or a page, and SHALL exit 2.

#### Scenario: A verb's usage error
- **WHEN** an agent runs `bilbo recall --bogus`
- **THEN** stderr is exactly four lines: `bilbo: unknown option '--bogus'`, `bilbo: usage: bilbo recall <query>... [--kind <kind>]... [--limit <n>]`, `bilbo:        bilbo recall <query>... --library [--corpus <corpus>]... [--limit <n>]` and `bilbo: see 'bilbo recall --help'`, stdout is empty and the exit code is 2

#### Scenario: A subcommand's usage error
- **WHEN** an agent runs `bilbo library land`
- **THEN** stderr is `bilbo: missing <stage>`, `bilbo: usage: bilbo library land <stage> <corpus>/<name> --keep <a>-<b>[,<c>-<d>]... [--title <text>] [--replace [--force]]` and `bilbo: see 'bilbo library --help'`

#### Scenario: A config error has no usage text
- **WHEN** an agent runs `bilbo check` with `BILBO_HOME=store`
- **THEN** stderr is the one line `bilbo: BILBO_HOME must be an absolute path, got 'store'` and the exit code is 2

### Requirement: Verb suggestions
For an unknown verb, the reason SHALL end with `; did you mean '<verb>'?` when the name is a prefix of exactly one verb, or, when it is a prefix of none, when exactly one verb is nearest to it by edit distance, that distance being at most 2 and less than the name's length. Otherwise it SHALL suggest nothing.

#### Scenario: A typo gets a suggestion
- **WHEN** an agent runs `bilbo recal x`
- **THEN** stderr starts with `bilbo: unknown verb 'recal'; did you mean 'recall'?` and the exit code is 2

#### Scenario: A unique prefix gets a suggestion
- **WHEN** an agent runs `bilbo hist release`
- **THEN** stderr starts with `bilbo: unknown verb 'hist'; did you mean 'history'?`

#### Scenario: A name far from every verb gets none
- **WHEN** an agent runs `bilbo zzzzzz`
- **THEN** stderr starts with `bilbo: unknown verb 'zzzzzz'` and a newline, with no suggestion

#### Scenario: A prefix of several verbs gets none
- **WHEN** an agent runs `bilbo re`
- **THEN** stderr starts with `bilbo: unknown verb 're'` and a newline, with no suggestion
