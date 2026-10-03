# Spec Delta

## MODIFIED Requirements

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
