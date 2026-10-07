## ADDED Requirements

### Requirement: Human view of index
When stdout gets the `cli` spec's human view, `bilbo index` SHALL print `Embedded <n> passages`, `passage` for one, with the count in bold, then dim ` · kept <n> · dropped <n>`, numbers grouped by thousands, after `◆` when it embedded or dropped a passage and `◇` otherwise.

#### Scenario: New passages on a terminal
- **WHEN** a user runs `bilbo index` in a terminal and it embeds 49 passages and keeps 4,620
- **THEN** stdout is `◆  Embedded 49 passages · kept 4,620 · dropped 0`

#### Scenario: The timer's log keeps the line
- **WHEN** the timer runs `bilbo index` with stdout appended to its log
- **THEN** the log gains `embedded 49, kept 4620, dropped 0`
