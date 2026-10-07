## MODIFIED Requirements

### Requirement: Human view of index
When stdout gets the `cli` spec's human view, `bilbo index` SHALL print `Embedded <n> passages`, `passage` for one, with the count in bold, then dim ` · kept <n> · dropped <n>`, numbers grouped by thousands, after `◆` when it embedded or dropped a passage and `◇` otherwise. Off a terminal it SHALL print the plain line, after the time the `cli` spec's Times in service logs requirement puts before it.

#### Scenario: New passages on a terminal
- **WHEN** a user runs `bilbo index` in a terminal and it embeds 49 passages and keeps 4,620
- **THEN** stdout is `◆  Embedded 49 passages · kept 4,620 · dropped 0`

#### Scenario: The timer's log keeps the line
- **WHEN** the timer runs `bilbo index` with stdout appended to its log
- **THEN** the log gains a line of the time, a space and `embedded 49, kept 4620, dropped 0`

#### Scenario: The journal gets the bare line
- **WHEN** systemd runs `bilbo index` with stdout to the journal and it embeds 49 passages and keeps 4,620
- **THEN** stdout is `embedded 49, kept 4620, dropped 0`
