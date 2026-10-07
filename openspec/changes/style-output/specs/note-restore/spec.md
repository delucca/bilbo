## ADDED Requirements

### Requirement: Human view of restore
When stdout gets the `cli` spec's human view, a restore SHALL print `◆  Restored <file name> to <version>`, the file name bold and the version cyan, and a restore whose note already holds the version's bytes SHALL print `◇  <file name> already matches <version>`. Its warnings SHALL follow on stderr.

#### Scenario: A restore on a terminal
- **WHEN** a user runs `bilbo restore release d4f94f03789f` in a terminal
- **THEN** stdout is `◆  Restored decision-release.md to d4f94f03789f` and the exit code is 0

#### Scenario: A restore down a pipe
- **WHEN** an agent runs `bilbo restore release d4f94f03789f` with stdout piped
- **THEN** stdout is `restored decision-release.md to d4f94f03789f`
