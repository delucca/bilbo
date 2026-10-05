# Spec Delta

## ADDED Requirements

### Requirement: Resolving a sync conflict
When `bilbo check` reports a `conflict:` line for the note it touched, the note skill SHALL replace each block with one passage keeping every fact of every side, unless the user said which side holds, and run `bilbo check` again. For dropped lines it then reports, the skill SHALL put them back or, when they are wrong or superseded, run `bilbo sync declare <topic> "<why>"` and name them in its report. `allowed-tools` SHALL include `Bash(bilbo sync declare *)`.

#### Scenario: Two dates for one release
- **WHEN** the agent updates `plan-release.md` and `bilbo check` reports its `## Rollout` passage in conflict, one side saying Monday and the other Friday
- **THEN** the agent writes one `## Rollout` passage that holds both dates and says they disagree, removes the markers, and `bilbo check` prints no line for the note

#### Scenario: A side the user overruled
- **WHEN** the user says Friday holds, and the agent keeps only the Friday side
- **THEN** the agent runs `bilbo sync declare release "<why>"` after `bilbo check` reports the dropped lines, and its report names the dropped Monday line

#### Scenario: A conflict in another note
- **WHEN** `bilbo check` reports a conflict in a note the skill did not touch
- **THEN** the agent leaves that note as it is and counts the line among the lines for other notes
