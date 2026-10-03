# cli Specification

## Purpose
The `bilbo` command line as a whole: how a verb is picked, how usage and help are printed, and what exit codes and output streams mean for every verb.

## Requirements

### Requirement: Verb dispatch
`bilbo` SHALL read its first argument as a verb and run that verb. The verbs are `new`, `check`, `recall`, `index`, `setup` and `digest`. Any other first argument, or no argument, SHALL be a usage error.

#### Scenario: A known verb runs
- **WHEN** an agent runs `bilbo check`
- **THEN** bilbo runs the check verb

#### Scenario: Setup is a verb
- **WHEN** a user runs `bilbo setup --yes`
- **THEN** bilbo runs the setup verb

#### Scenario: Digest is a verb
- **WHEN** a hook runs `bilbo digest` with a hook payload on stdin
- **THEN** bilbo runs the digest verb

#### Scenario: An unknown verb is a usage error
- **WHEN** an agent runs `bilbo frobnicate`
- **THEN** bilbo prints a usage message that names the verbs `new`, `check`, `recall`, `index`, `setup` and `digest` to stderr, exits 2, and creates, changes or deletes no file

#### Scenario: No arguments is a usage error
- **WHEN** an agent runs `bilbo` with no arguments
- **THEN** bilbo prints the usage message to stderr and exits 2

### Requirement: Help
`bilbo --help` and `bilbo -h` SHALL print the usage message to stdout and exit 0.

#### Scenario: Help goes to stdout
- **WHEN** an agent runs `bilbo --help`
- **THEN** stdout holds the usage message, stderr is empty and the exit code is 0

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
stdout SHALL carry only a verb's result. Every diagnostic SHALL go to stderr as a line starting with `bilbo: `. The one exception is the interactive `setup` wizard, which draws its prompts, choices and progress on stderr without that prefix; its result, the step report, still goes to stdout.

#### Scenario: A failure leaves stdout empty
- **WHEN** `bilbo new` refuses a taken topic
- **THEN** stdout is empty and stderr holds a line starting with `bilbo: `

#### Scenario: The wizard draws on stderr
- **WHEN** a user runs `bilbo setup` in a terminal and answers every prompt
- **THEN** the prompts appear on stderr and stdout holds only the step report

#### Scenario: Non-interactive setup keeps the prefix
- **WHEN** `bilbo setup --yes` cannot run `launchctl`
- **THEN** every stderr line starts with `bilbo: `

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
- **THEN** bilbo prints the usage message to stderr and exits 2
