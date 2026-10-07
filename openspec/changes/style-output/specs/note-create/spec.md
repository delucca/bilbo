## ADDED Requirements

### Requirement: Human view of new
When stdout gets the `cli` spec's human view, `bilbo new` SHALL print `◆  Created <kind> <topic>`, the kind cyan and the topic bold, then the new file's path from `~/`, dim and indented three columns. The scope warning SHALL follow on stderr.

#### Scenario: A note created on a terminal
- **WHEN** a user runs `bilbo new gotcha sqlite-busy-timeout` in a terminal with `HOME=/Users/a` and the default root
- **THEN** stdout is `◆  Created gotcha sqlite-busy-timeout` and `   ~/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md`

#### Scenario: Command substitution gets the path
- **WHEN** a user runs `vim "$(bilbo new gotcha sqlite-busy-timeout)"` in a terminal
- **THEN** the substitution is the absolute path alone
