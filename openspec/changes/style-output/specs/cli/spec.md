## MODIFIED Requirements

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

## ADDED Requirements

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
