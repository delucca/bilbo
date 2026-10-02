# Spec Delta

## ADDED Requirements

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
