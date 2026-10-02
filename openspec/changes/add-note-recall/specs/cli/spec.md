# Spec Delta

## MODIFIED Requirements

### Requirement: Verb dispatch
`bilbo` SHALL read its first argument as a verb and run that verb. The verbs are `new`, `check` and `recall`. Any other first argument, or no argument, SHALL be a usage error.

#### Scenario: A known verb runs
- **WHEN** an agent runs `bilbo check`
- **THEN** bilbo runs the check verb

#### Scenario: An unknown verb is a usage error
- **WHEN** an agent runs `bilbo frobnicate`
- **THEN** bilbo prints a usage message that names the verbs `new`, `check` and `recall` to stderr, exits 2, and creates, changes or deletes no file

#### Scenario: No arguments is a usage error
- **WHEN** an agent runs `bilbo` with no arguments
- **THEN** bilbo prints the usage message to stderr and exits 2

### Requirement: Exit codes
Every verb SHALL exit 0 when it succeeds, 1 when it refuses the request, finds problems or, for `recall`, finds nothing, and 2 on a usage error: an unknown verb or option, a missing or extra argument, or an invalid argument value.

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
