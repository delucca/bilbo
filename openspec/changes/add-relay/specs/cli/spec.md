# Spec Delta

## MODIFIED Requirements

### Requirement: Verb dispatch
`bilbo` SHALL read its first argument as a verb and run that verb. The verbs are `new`, `check`, `recall`, `index`, `setup`, `digest`, `watch`, `history`, `restore`, `scope`, `device`, `sync`, `pair` and `relay`. Any other first argument, or no argument, SHALL be a usage error.

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

#### Scenario: Pair is a verb
- **WHEN** a user runs `bilbo pair` on an enrolled device, or `bilbo pair <code> --via <url>` on a new one
- **THEN** bilbo runs the pair verb

#### Scenario: Relay is a verb
- **WHEN** a user runs `bilbo relay --data /srv/relay --owner <fingerprint>`
- **THEN** bilbo runs the relay verb

#### Scenario: An unknown verb is a usage error
- **WHEN** an agent runs `bilbo frobnicate`
- **THEN** bilbo prints a usage message that names the verbs `new`, `check`, `recall`, `index`, `setup`, `digest`, `watch`, `history`, `restore`, `scope`, `device`, `sync`, `pair` and `relay` to stderr, exits 2, and creates, changes or deletes no file

#### Scenario: No arguments is a usage error
- **WHEN** an agent runs `bilbo` with no arguments
- **THEN** bilbo prints the usage message to stderr and exits 2
