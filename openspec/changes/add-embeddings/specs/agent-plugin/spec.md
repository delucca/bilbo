# Spec Delta

## MODIFIED Requirements

### Requirement: The recall skill
The plugin SHALL hold a skill named `recall` in `skills/recall/SKILL.md`, whose frontmatter has only `name`, `description`, `license` and `allowed-tools`. The skill SHALL search notes only through `bilbo recall`, with the user's words after `--`, and SHALL act on the exit code: on 0 it shows the hits, says which query produced them, and passes on any `bilbo:` warning lines from stderr in one sentence; on 1 whose last stderr line is `bilbo: no notes match` it retries at most twice in the note's likely wording, then says nothing matched; on any other 1, or on 2, it shows bilbo's first stderr line and stops.

#### Scenario: Hits are shown
- **WHEN** a user asks to recall `flat layout` and `bilbo recall` exits 0
- **THEN** the agent says the hits came from `flat layout`, shows each hit's path, line, kind, created, heading path and snippet, and offers to open one

#### Scenario: Hits with a warning
- **WHEN** `bilbo recall` exits 0 and stderr is `bilbo: embedder unavailable (embedder http://bagend:8081 unreachable: Connection refused (os error 61)); keyword results only`
- **THEN** the agent shows the hits and says the results came from keywords alone because the embedder was unavailable

#### Scenario: Nothing matches
- **WHEN** `bilbo recall` exits 1 with `bilbo: no notes match` for the user's words and for two reworded queries
- **THEN** the agent says nothing matched and names the queries it tried

#### Scenario: Nothing matches after a warning
- **WHEN** `bilbo recall` exits 1 and stderr is `bilbo: 3 passages not indexed; run bilbo index` followed by `bilbo: no notes match`
- **THEN** the agent retries in the note's likely wording, as when nothing matches

#### Scenario: A usage error
- **WHEN** `bilbo recall` exits 2
- **THEN** the agent shows bilbo's first stderr line and runs no other query
