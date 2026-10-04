# Spec Delta

## MODIFIED Requirements

### Requirement: Verb dispatch
`bilbo` SHALL read its first argument as a verb and run that verb. The verbs are `new`, `check`, `recall`, `index`, `setup`, `digest`, `library`, `cite`, `watch`, `history` and `restore`. Any other first argument, or no argument, SHALL be a usage error.

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

#### Scenario: An unknown verb is a usage error
- **WHEN** an agent runs `bilbo frobnicate`
- **THEN** bilbo prints a usage message that names the verbs `new`, `check`, `recall`, `index`, `setup`, `digest`, `library`, `cite`, `watch`, `history` and `restore` to stderr, exits 2, and creates, changes or deletes no file

#### Scenario: No arguments is a usage error
- **WHEN** an agent runs `bilbo` with no arguments
- **THEN** bilbo prints the usage message to stderr and exits 2
