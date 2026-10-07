## ADDED Requirements

### Requirement: Human view of check
When stdout gets the `cli` spec's human view, `bilbo check` SHALL print each file that has a line of Report problems once, in bold, then each of its lines indented two columns under it: `■` for a problem and `▲` for a warning, the warning without its ` (warning)` suffix, the message's part before its first `: ` dim and aligned within the file, then the rest; a blank line SHALL separate files. A summary line SHALL follow: `■  <p> problems in <f> files`, with `, <w> warnings` after the problems when there are warnings, or `▲  <w> warnings in <f> files` when every line is a warning, each count singular for one. A clean store SHALL print one line, `◆  No problems in <n> notes and <c> corpora`, `1 note` and `1 corpus` for one. The exit code SHALL be as in Report problems.

#### Scenario: Problems in two files
- **WHEN** `notes/plan-a.md` lacks `created` and a title, `notes/plan-b.md` has a bad id, and a user runs `bilbo check` in a terminal
- **THEN** stdout shows `notes/plan-a.md` once with two `■` lines under it, `notes/plan-b.md` once with one, the last line is `■  3 problems in 2 files`, and the exit code is 1

#### Scenario: Only a warning
- **WHEN** the only line `check` reports is a scope-mark warning on `notes/plan-a.md` and a user runs `bilbo check` in a terminal
- **THEN** the line under `notes/plan-a.md` starts with `▲`, holds no ` (warning)`, the last line is `▲  1 warning in 1 file`, and the exit code is 0

#### Scenario: A clean store on a terminal
- **WHEN** a store of 12 notes and 2 corpora has no problem and a user runs `bilbo check` in a terminal
- **THEN** stdout is `◆  No problems in 12 notes and 2 corpora` and the exit code is 0

#### Scenario: A clean store down a pipe
- **WHEN** an agent runs `bilbo check` on a clean store with stdout piped
- **THEN** stdout is empty and the exit code is 0
