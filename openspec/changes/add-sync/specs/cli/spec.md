# Spec Delta

## MODIFIED Requirements

### Requirement: Verb dispatch
`bilbo` SHALL read its first argument as a verb and run that verb. The verbs are `new`, `check`, `recall`, `index`, `setup`, `digest`, `watch`, `history`, `restore`, `scope`, `device` and `sync`. Any other first argument, or no argument, SHALL be a usage error.

#### Scenario: A known verb runs
- **WHEN** an agent runs `bilbo check`
- **THEN** bilbo runs the check verb

#### Scenario: Setup is a verb
- **WHEN** a user runs `bilbo setup --yes`
- **THEN** bilbo runs the setup verb

#### Scenario: Digest is a verb
- **WHEN** a hook runs `bilbo digest` with a hook payload on stdin
- **THEN** bilbo runs the digest verb

#### Scenario: History verbs
- **WHEN** an agent runs `bilbo history release` or `bilbo restore release a1b2c3`, or a service runs `bilbo watch`
- **THEN** bilbo runs the history, restore or watch verb

#### Scenario: Scope is a verb
- **WHEN** an agent runs `bilbo scope` or `bilbo scope set work <file>`
- **THEN** bilbo runs the scope verb

#### Scenario: Device is a verb
- **WHEN** a user runs `bilbo device` or `bilbo device revoke bagend`
- **THEN** bilbo runs the device verb

#### Scenario: Sync is a verb
- **WHEN** an agent runs `bilbo sync` or `bilbo sync declare release "superseded"`
- **THEN** bilbo runs the sync verb

#### Scenario: An unknown verb is a usage error
- **WHEN** an agent runs `bilbo frobnicate`
- **THEN** bilbo prints a usage message that names the verbs `new`, `check`, `recall`, `index`, `setup`, `digest`, `watch`, `history`, `restore`, `scope`, `device` and `sync` to stderr, exits 2, and creates, changes or deletes no file

#### Scenario: No arguments is a usage error
- **WHEN** an agent runs `bilbo` with no arguments
- **THEN** bilbo prints the usage message to stderr and exits 2

### Requirement: Output streams
stdout SHALL carry only a verb's result. Every diagnostic SHALL go to stderr as a line starting with `bilbo: `. The exceptions are the interactive `setup` wizard, its sync step's recovery phrase ceremony included, and the recovery phrase prompts of `device init` and `device recover`, which draw their prompts, choices and progress on stderr without that prefix; their result, the step report, still goes to stdout.

#### Scenario: A failure leaves stdout empty
- **WHEN** `bilbo new` refuses a taken topic
- **THEN** stdout is empty and stderr holds a line starting with `bilbo: `

#### Scenario: The wizard draws on stderr
- **WHEN** a user runs `bilbo setup` in a terminal and answers every prompt
- **THEN** the prompts appear on stderr and stdout holds only the step report

#### Scenario: Non-interactive setup keeps the prefix
- **WHEN** `bilbo setup --yes` cannot run `launchctl`
- **THEN** every stderr line starts with `bilbo: `

#### Scenario: The phrase never reaches stdout
- **WHEN** a user runs `bilbo device init > out.txt` in a terminal and confirms the phrase
- **THEN** the phrase was drawn on stderr, and `out.txt` holds only the step report

#### Scenario: The phrase in setup never reaches stdout
- **WHEN** a user turns sync on in `bilbo setup > out.txt` in a terminal and confirms a new phrase
- **THEN** the phrase was drawn on stderr, and `out.txt` holds only the step report
