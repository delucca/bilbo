## MODIFIED Requirements

### Requirement: Output streams
stdout SHALL carry only a verb's result. Every diagnostic SHALL go to stderr. In stderr's plain view every line SHALL start with `bilbo: `, after the time the Times in service logs requirement puts before it. In stderr's human view a diagnostic SHALL instead start with a level mark and two spaces, `■` for an error, `▲` for a warning, `●` for progress and `○` for nothing found, with its later lines indented three columns. When stdout gets the human view, a verb's warnings SHALL follow its result; otherwise they SHALL precede it. The exceptions are the interactive `setup` wizard, its sync step's recovery phrase ceremony included, the recovery phrase prompts of `device init` and `device recover`, `pair`'s prompts on a person's terminal, and `recall`'s spinner, which draw on stderr without a prefix or level mark; their result still goes to stdout.

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

#### Scenario: Pairing draws on stderr
- **WHEN** a user runs `bilbo pair > out.txt` in a terminal and the pairing succeeds
- **THEN** the code, the question and the spinner were drawn on stderr, and `out.txt` holds only the `paired` line

### Requirement: Usage errors
A usage error SHALL print to stderr: the reason, as its first line; then `usage: ` and the synopsis forms of the verb that was run, one unwrapped form per line, only those of its subcommand when the argument after the verb names one; then `see 'bilbo <verb> --help'`. With no verb, an unknown one or `help`, the forms and page are `bilbo`'s own. In stderr's plain view each line SHALL start with `bilbo: `; in its human view the reason SHALL follow `■` and the other lines SHALL be indented three columns, by the Output streams levels, and a `usage:` or `verbs:` line, or a form after the first, wider than the width SHALL wrap at its spaces, its later lines indented ten columns, under the text after the seven-column label. It SHALL never print the overview or a page, and SHALL exit 2.

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

#### Scenario: The verbs line wraps on a narrow terminal
- **WHEN** a user runs `bilbo recal x` in a terminal 60 columns wide with no agent marker
- **THEN** the verbs line is `   verbs: new, recall, check, history, restore, library,`, the next two lines start with ten spaces and hold the other verbs, and no stderr line is wider than 60 columns

#### Scenario: A long form wraps under its text
- **WHEN** a user runs `bilbo library land` in a terminal 80 columns wide with no agent marker
- **THEN** the usage line is `   usage: bilbo library land <stage> <corpus>/<name> --keep <a>-<b>[,<c>-<d>]...` and the next line is ten spaces and `[--title <text>] [--replace [--force]]`

#### Scenario: A pipe keeps one line per form
- **WHEN** an agent runs `bilbo recal x` with stderr piped
- **THEN** the verbs line is one line, `bilbo: verbs: ` and every verb, however wide

## ADDED Requirements

### Requirement: Times in service logs
Each line that `watch`, `relay` or `index` writes to stdout or stderr SHALL start with the local time, in RFC 3339 form to the second with its UTC offset, and a space, when the stream is a regular file, such as the log files the units `setup` installs write to. A terminal, a pipe and systemd's journal, which receives output through a socket and keeps its own times, SHALL get no time. Where the `note-watch`, `note-index` and `relay-server` requirements state a line, they state the text after that time. Help SHALL never carry a time, and no other verb's lines SHALL.

#### Scenario: The watch log
- **WHEN** `bilbo watch` starts at 01:02:03 on 2026-10-07 at offset -03:00 with stderr appended to `watch.log`
- **THEN** `watch.log` gains `2026-10-07T01:02:03-03:00 bilbo: watching <root>/notes`

#### Scenario: The index log keeps both streams in time
- **WHEN** the timer runs `bilbo index` with stdout and stderr appended to `index.log`, and it embeds 1 passage and withholds 2
- **THEN** each of the two lines it adds starts with a time and a space, one followed by `embedded 1, kept 0, dropped 0` and the other by `bilbo: withheld 2 passages from <url>: their scope allows only a loopback embedder`

#### Scenario: UTC
- **WHEN** `bilbo relay` runs with stderr appended to a file and the local time zone is UTC
- **THEN** its listening line starts with a time ending in `+00:00` and a space

#### Scenario: A pipe gets no time
- **WHEN** an agent runs `bilbo index` with stdout and stderr piped
- **THEN** stdout is `embedded <n>, kept <n>, dropped <n>` and every stderr line starts with `bilbo: `

#### Scenario: The journal gets no time
- **WHEN** systemd runs `bilbo relay` with stderr connected to the journal
- **THEN** the journal's line is `bilbo: relay listening on http://<address:port>`

#### Scenario: A file redirected by hand gets the time
- **WHEN** a user runs `bilbo index > index.txt 2>&1`
- **THEN** each line of `index.txt` starts with a time and a space

#### Scenario: A terminal gets no time
- **WHEN** a user runs `bilbo watch` in a terminal, or with stderr a terminal and `CODEX_CI=1`
- **THEN** no stderr line starts with a time

#### Scenario: Another verb in a file
- **WHEN** a user runs `bilbo recall rollback > hits.txt 2> err.txt`
- **THEN** neither file holds a time that bilbo added
