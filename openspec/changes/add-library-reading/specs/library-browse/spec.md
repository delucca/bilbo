# Spec Delta

## MODIFIED Requirements

### Requirement: Corpus argument errors
`bilbo library <corpus>` SHALL exit 1 with `bilbo: no corpus '<corpus>' in <root>/library` on stderr when no such corpus folder exists. A corpus argument that breaks the name grammar SHALL be a usage error. `plan` and `read` are subcommands, so `bilbo library plan` and `bilbo library read` with no other argument SHALL be usage errors naming what is missing.

#### Scenario: An unknown corpus
- **WHEN** an agent runs `bilbo library haskell` and `<root>/library/haskell/` does not exist
- **THEN** stderr is `bilbo: no corpus 'haskell' in <root>/library`, stdout is empty and the exit code is 1

#### Scenario: A bad corpus name
- **WHEN** an agent runs `bilbo library Go`
- **THEN** bilbo prints a message naming `Go` to stderr and exits 2

#### Scenario: A subcommand without its arguments
- **WHEN** an agent runs `bilbo library plan`
- **THEN** bilbo prints a message naming the missing reference to stderr, exits 2, and writes no plan
