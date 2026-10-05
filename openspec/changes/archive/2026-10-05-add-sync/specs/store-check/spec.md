# Spec Delta

## ADDED Requirements

### Requirement: Sync conflicts
Judging each file as it is now, `bilbo check` SHALL add problem lines, sorted with the others: per open conflict block, `notes/<file>: conflict: '<heading path>' holds <n> sides; keep what is right, remove the markers`; per passage with undeclared dropped text, `notes/<file>: conflict: dropped <n> lines of '<heading path>', first "<line>"; restore them or run bilbo sync declare <topic> "<why>"`, `<line>` cut to 80 characters; per marker line, outside fences, that matches the `note-merge` spec's full marker form and belongs to no open conflict, `notes/<file>: line <n>: stray conflict marker`.

#### Scenario: An open conflict
- **WHEN** `notes/gotcha-nix.md` holds an open conflict block for `## Flakes` with two sides
- **THEN** stdout holds `notes/gotcha-nix.md: conflict: 'Nix > Flakes' holds 2 sides; keep what is right, remove the markers` and the exit code is 1

#### Scenario: Resolved, before watch records it
- **WHEN** an agent has just removed the markers, keeping one side, and watch has not recorded the save
- **THEN** `bilbo check` already reports the dropped lines of the other side, not the conflict

#### Scenario: Resolved keeping everything
- **WHEN** an agent removes the markers and keeps every line of both sides
- **THEN** `bilbo check` reports nothing for the note

#### Scenario: A stray marker
- **WHEN** line 14 of `notes/plan-x.md` is `>>>>>>> bilbo` and the note has no open conflict
- **THEN** stdout holds `notes/plan-x.md: line 14: stray conflict marker`

#### Scenario: A quoted marker
- **WHEN** a note shows `>>>>>>> bilbo` inside a fenced code block, or a line `>>>>>>> bilbo was here` outside one
- **THEN** `bilbo check` reports no stray marker for it

#### Scenario: Check stays read-only
- **WHEN** `bilbo check` reports a conflict
- **THEN** every entry under the root, `<root>/.bilbo/` included, has the same bytes and modification time as before

### Requirement: A note that left its scope
For 30 days after this device recorded a version that moved a note out of a syncing scope, `bilbo check` SHALL print the warning `notes/<file>: scope: left '<name>'; other devices of '<name>' no longer hold this note`, unless the note is back in that scope. The warning SHALL NOT change the exit code.

#### Scenario: A dropped scope key
- **WHEN** an agent dropped `scope: personal` from `notes/plan-release.md` yesterday and the note now has no `scope`
- **THEN** stdout holds `notes/plan-release.md: scope: left 'personal'; other devices of 'personal' no longer hold this note` beside the Scope problems requirement's `scope: missing` line

#### Scenario: Back in the scope
- **WHEN** the agent puts `scope: personal` back
- **THEN** the left warning is gone
