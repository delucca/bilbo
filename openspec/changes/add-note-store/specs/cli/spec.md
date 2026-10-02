# Spec Delta

## Purpose

The `bilbo` command line as a whole: how a verb is picked, how usage and help are printed, and what exit codes and output streams mean for every verb.

## ADDED Requirements

### Requirement: Verb dispatch
`bilbo` SHALL read its first argument as a verb and run that verb. The verbs are `new` and `check`. Any other first argument, or no argument, SHALL be a usage error.

#### Scenario: A known verb runs
- **WHEN** an agent runs `bilbo check`
- **THEN** bilbo runs the check verb

#### Scenario: An unknown verb is a usage error
- **WHEN** an agent runs `bilbo frobnicate`
- **THEN** bilbo prints a usage message that names the verbs `new` and `check` to stderr, exits 2, and creates, changes or deletes no file

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
Every verb SHALL exit 0 when it succeeds, 1 when it refuses the request or finds problems, and 2 on a usage error: an unknown verb or option, a missing or extra argument, or an invalid argument value.

#### Scenario: Success exits 0
- **WHEN** `bilbo new plan release-steps` creates a note
- **THEN** the exit code is 0

#### Scenario: A refusal exits 1
- **WHEN** `bilbo new plan release-steps` runs and `plan-release-steps.md` already exists
- **THEN** the exit code is 1

#### Scenario: A usage error exits 2
- **WHEN** an agent runs `bilbo new plan`, with no topic
- **THEN** the exit code is 2

### Requirement: Output streams
stdout SHALL carry only a verb's result. Every diagnostic SHALL go to stderr as a line starting with `bilbo: `.

#### Scenario: A failure leaves stdout empty
- **WHEN** `bilbo new` refuses a taken topic
- **THEN** stdout is empty and stderr holds a line starting with `bilbo: `
