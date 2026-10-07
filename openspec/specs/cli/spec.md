# cli Specification

## Purpose
The `bilbo` command line as a whole: how a verb is picked, how usage and help are printed, and what exit codes and output streams mean for every verb.

## Requirements

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

### Requirement: Exit codes
Every verb except `digest` SHALL exit 0 when it succeeds, 1 when it refuses the request, finds problems or, for `recall`, finds nothing, and 2 on a usage error: an unknown verb or option, a missing or extra argument, or an invalid argument value. `digest` SHALL always exit 0, an unknown option and a config error included, because a prompt hook that exits 2 blocks the prompt.

#### Scenario: Success exits 0
- **WHEN** `bilbo new plan release-steps` creates a note
- **THEN** the exit code is 0

#### Scenario: A refusal exits 1
- **WHEN** `bilbo new plan release-steps` runs and `plan-release-steps.md` already exists
- **THEN** the exit code is 1

#### Scenario: No recall hits exits 1
- **WHEN** `bilbo recall wumpus` matches no note
- **THEN** the exit code is 1

#### Scenario: A usage error exits 2
- **WHEN** an agent runs `bilbo new plan`, with no topic
- **THEN** the exit code is 2

#### Scenario: Digest exits 0 even on bad input
- **WHEN** an agent runs `bilbo digest --verbose`
- **THEN** stdout is empty, stderr is one line starting with `bilbo: ` that names `--verbose`, and the exit code is 0

### Requirement: Output streams
stdout SHALL carry only a verb's result. Every diagnostic SHALL go to stderr. In stderr's plain view every line SHALL start with `bilbo: `. In stderr's human view a diagnostic SHALL instead start with a level mark and two spaces, `■` for an error, `▲` for a warning, `●` for progress and `○` for nothing found, with its later lines indented three columns. When stdout gets the human view, a verb's warnings SHALL follow its result; otherwise they SHALL precede it. The exceptions are the interactive `setup` wizard, its sync step's recovery phrase ceremony included, and the recovery phrase prompts of `device init` and `device recover`, which draw their prompts, choices and progress on stderr without a prefix or level mark; their result, the step report, still goes to stdout.

#### Scenario: A failure leaves stdout empty
- **WHEN** `bilbo new` refuses a taken topic with stderr piped
- **THEN** stdout is empty and stderr holds a line starting with `bilbo: `

#### Scenario: The wizard draws on stderr
- **WHEN** a user runs `bilbo setup` in a terminal and answers every prompt
- **THEN** the prompts appear on stderr and stdout holds only the step report

#### Scenario: Non-interactive setup keeps the prefix
- **WHEN** `bilbo setup --yes` cannot run `launchctl` and stderr is not a terminal
- **THEN** every stderr line starts with `bilbo: `

#### Scenario: The phrase never reaches stdout
- **WHEN** a user runs `bilbo device init > out.txt` in a terminal and confirms the phrase
- **THEN** the phrase was drawn on stderr, and `out.txt` holds only the step report

#### Scenario: The phrase in setup never reaches stdout
- **WHEN** a user turns sync on in `bilbo setup > out.txt` in a terminal and confirms a new phrase
- **THEN** the phrase was drawn on stderr, and `out.txt` holds only the step report

#### Scenario: A refusal on a terminal
- **WHEN** a user runs `bilbo recall --corpus nosuch -- busy` in a terminal with no agent marker
- **THEN** stderr is `■`, two spaces and `no corpus 'nosuch' in <root>/library`, with no `bilbo: `, and the exit code is 1

#### Scenario: Warnings follow the hits on a terminal
- **WHEN** a user runs `bilbo recall rollback` in a terminal, a note holds `rollback`, and one passage is not indexed
- **THEN** the hits come first and the last line is `▲`, two spaces and `1 passage not indexed; run bilbo index`

#### Scenario: Warnings precede the result down a pipe
- **WHEN** an agent runs `bilbo recall rollback` with stdout and stderr piped, and one passage is not indexed
- **THEN** stderr is `bilbo: 1 passage not indexed; run bilbo index` and stdout is the plain hit blocks

#### Scenario: An agent in a terminal keeps the prefix
- **WHEN** `bilbo check --verbose` runs with stderr a terminal and `CODEX_CI=1`
- **THEN** every stderr line starts with `bilbo: `

#### Scenario: Progress on a terminal
- **WHEN** a user runs `bilbo setup --yes` in a terminal and a step reports progress
- **THEN** that stderr line starts with `●` and two spaces, not `bilbo: `

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

### Requirement: Help styling
Help SHALL be plain text, except that headings and the words of a page's left column SHALL be bold, and nothing else, when stdout gets the human view and its escapes are allowed, by the Terminal views and Human view escapes requirements. Bold SHALL never reach stderr.

#### Scenario: Piped help is plain
- **WHEN** an agent runs `bilbo --help | cat` with `TERM=xterm-256color`
- **THEN** stdout holds no escape byte

#### Scenario: NO_COLOR turns bold off
- **WHEN** a user runs `bilbo recall --help` in a terminal with `NO_COLOR=1`
- **THEN** stdout holds no escape byte

#### Scenario: A terminal gets bold headings
- **WHEN** a user runs `bilbo --help` in a terminal with `TERM=xterm-256color` and no `NO_COLOR`
- **THEN** the `Usage:` heading is drawn in bold and the line `bilbo keeps durable memory for coding agents: notes they write and recall,` is plain

#### Scenario: An agent in a terminal gets plain help
- **WHEN** `bilbo --help` runs in a terminal with `TERM=xterm-256color` and `AI_AGENT=x`
- **THEN** stdout holds no escape byte

#### Scenario: No TERM, no bold
- **WHEN** a user runs `bilbo --help` in a terminal with `TERM` unset
- **THEN** stdout holds no escape byte

#### Scenario: CLICOLOR=0 turns bold off
- **WHEN** a user runs `bilbo --help` in a terminal with `TERM=xterm-256color` and `CLICOLOR=0`
- **THEN** stdout holds no escape byte

### Requirement: Usage errors
A usage error SHALL print to stderr: the reason, as its first line; then `usage: ` and the synopsis forms of the verb that was run, one unwrapped form per line, only those of its subcommand when the argument after the verb names one; then `see 'bilbo <verb> --help'`. With no verb, an unknown one or `help`, the forms and page are `bilbo`'s own. In stderr's plain view each line SHALL start with `bilbo: `; in its human view the reason SHALL follow `■` and the other lines SHALL be indented three columns, by the Output streams levels. It SHALL never print the overview or a page, and SHALL exit 2.

#### Scenario: A verb's usage error
- **WHEN** an agent runs `bilbo recall --bogus`
- **THEN** stderr is exactly four lines: `bilbo: unknown option '--bogus'`, `bilbo: usage: bilbo recall <query>... [--kind <kind>]... [--limit <n>]`, `bilbo:        bilbo recall <query>... --library [--corpus <corpus>]... [--limit <n>]` and `bilbo: see 'bilbo recall --help'`, stdout is empty and the exit code is 2

#### Scenario: A subcommand's usage error
- **WHEN** an agent runs `bilbo library land`
- **THEN** stderr is `bilbo: missing <stage>`, `bilbo: usage: bilbo library land <stage> <corpus>/<name> --keep <a>-<b>[,<c>-<d>]... [--title <text>] [--replace [--force]]` and `bilbo: see 'bilbo library --help'`

#### Scenario: A config error has no usage text
- **WHEN** an agent runs `bilbo check` with `BILBO_HOME=store`
- **THEN** stderr is the one line `bilbo: BILBO_HOME must be an absolute path, got 'store'` and the exit code is 2

#### Scenario: A usage error on a terminal
- **WHEN** a user runs `bilbo recal x` in a terminal with no agent marker
- **THEN** the first stderr line is `■`, two spaces and `unknown verb 'recal'; did you mean 'recall'?`, the usage, verbs and see lines follow indented three columns, no line starts with `bilbo: `, and the exit code is 2

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

### Requirement: Terminal views
bilbo SHALL judge stdout and stderr each on its own. A stream SHALL get the human view only when it is an interactive terminal and none of `AI_AGENT`, `CLAUDE_CODE_CHILD_SESSION`, `CODEX_THREAD_ID` and `CODEX_CI` is set and not empty; otherwise it SHALL get the plain view, whose bytes the verbs' own requirements state. `CLAUDECODE` SHALL NOT count as an agent marker, and `CLICOLOR_FORCE` and `FORCE_COLOR` SHALL never change a view. Only the verbs whose own requirements state a human view SHALL print anything but their plain view on stdout; `digest`, `cite` and `library plan`, `read`, `stage` and `land` SHALL print their plain view on stdout even when it is a terminal.

#### Scenario: A pipe gets the plain view
- **WHEN** a user runs `bilbo recall rollback | cat` in a terminal
- **THEN** stdout is byte for byte what the note-recall requirements state

#### Scenario: An agent's terminal gets the plain view
- **WHEN** `bilbo recall rollback` runs with stdout a terminal and `CLAUDE_CODE_CHILD_SESSION=1`
- **THEN** stdout is the plain hit blocks

#### Scenario: An IDE terminal is a person's
- **WHEN** a user runs `bilbo recall rollback` in a terminal where only `CLAUDECODE=1` is set
- **THEN** stdout is the human view

#### Scenario: An empty marker is no marker
- **WHEN** a user runs `bilbo recall rollback` in a terminal with `AI_AGENT` set to the empty string
- **THEN** stdout is the human view

#### Scenario: Forced colour never reaches a pipe
- **WHEN** an agent runs `bilbo scope` with stdout piped and `CLICOLOR_FORCE=1` and `FORCE_COLOR=1`
- **THEN** stdout holds no escape byte and is the plain listing

#### Scenario: A protocol verb on a terminal
- **WHEN** a user runs `bilbo library plan go/effective-go` in a terminal
- **THEN** stdout is the same bytes as `bilbo library plan go/effective-go | cat` prints, apart from the plan id

### Requirement: Human view escapes
The human view SHALL use escapes only when `NO_COLOR` is unset or empty, `CLICOLOR` is not `0`, and `TERM` is set and not `dumb`; otherwise it SHALL keep its layout and marks with no escape byte. Its escapes SHALL be only bold, dim, and red, green, yellow, blue and cyan text, each colour on a mark or word that says the same. It SHALL fit its prose and diagnostics (recall's hits, `check`'s messages, stderr lines and a guide's paragraphs) to the terminal's width, else to `COLUMNS` when that is a number above 0, else to 80 columns, never wider than 100 nor narrower than 40, wrapping a word wider than the line without dropping characters, marking text it cuts in a preview or a title line with `…`; tables are not cut. Outside macOS, unless `LANG` ends in `UTF-8`, its marks, `›` and `…` SHALL be ASCII. The setup wizard and the phrase prompts SHALL keep their own drawing and SHALL use colour on stderr exactly when stderr's human view may use escapes.

#### Scenario: NO_COLOR keeps the layout
- **WHEN** a user runs `bilbo recall rollback` in a terminal with `NO_COLOR=1`
- **THEN** stdout is the human view's lines with no escape byte

#### Scenario: A dumb terminal keeps the layout
- **WHEN** a user runs `bilbo check` in a terminal with `TERM=dumb` and a clean store
- **THEN** stdout is one line starting with `◆` and holding no escape byte

#### Scenario: Only the basic colours
- **WHEN** a user runs `bilbo recall rollback`, `bilbo check` or `bilbo recall --corpus nosuch -- x` in a terminal with `TERM=xterm-256color`
- **THEN** no escape on stdout or stderr selects a bright, 256 or true colour or a background

#### Scenario: A narrow terminal
- **WHEN** a user runs `bilbo recall rollback` in a terminal 60 columns wide
- **THEN** no stdout line is wider than 60 columns

#### Scenario: A wide terminal
- **WHEN** a user runs `bilbo recall rollback` in a terminal 200 columns wide
- **THEN** no stdout line is wider than 100 columns

#### Scenario: No UTF-8 locale
- **WHEN** a user on Linux runs `bilbo check` in a terminal with `LANG=C` and a clean store
- **THEN** stdout is one line starting with `*` and two spaces

#### Scenario: The wizard follows NO_COLOR
- **WHEN** a user runs `bilbo setup` in a terminal with `NO_COLOR=1`
- **THEN** the wizard's drawing on stderr selects no colour, and its marks and boxes are drawn as without `NO_COLOR`
