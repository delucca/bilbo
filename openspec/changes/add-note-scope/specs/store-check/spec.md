# Spec Delta

## MODIFIED Requirements

### Requirement: Report problems
`bilbo check` SHALL check every entry in `<root>/notes/` against every `note-store` rule and the scope rules below. It SHALL print one line per problem and per warning to stdout as `<path relative to the root>: <message>`, sorted by path and then by message, and exit 1 when there is at least one problem. A warning SHALL NOT change the exit code. With no problems and no warnings, it SHALL print nothing and exit 0.

#### Scenario: A clean store
- **WHEN** every note in the store follows the contract
- **THEN** stdout is empty and the exit code is 0

#### Scenario: A note with a bad timestamp
- **WHEN** `notes/plan-release.md` has `created: 2026-10-02`
- **THEN** stdout has a line that starts with `notes/plan-release.md: ` and names `created`, and the exit code is 1

#### Scenario: An empty store
- **WHEN** `<root>/notes/` exists and holds no notes
- **THEN** stdout is empty and the exit code is 0

#### Scenario: Warnings alone
- **WHEN** the only line `bilbo check` prints is a Scope marks warning
- **THEN** stdout holds that line and the exit code is 0

## ADDED Requirements

### Requirement: Scope problems
`bilbo check` SHALL read the config, and a config error SHALL exit 2 as for `recall`. When the config declares at least one scope, a note with no `scope` key SHALL be a problem: `scope: missing; scopes: <names>`. A note whose valid `scope` value names a scope the config does not declare SHALL be a problem whether or not any scope is declared: `scope: '<name>' is not declared in <config path>`, then `; scopes: <names>` when there are any. A `scope` value that breaks the `note-store` Scope key rule, or a repeated key, SHALL get only that rule's problem.

#### Scenario: No scopes, no scope lines
- **WHEN** the config holds no `scope.*` key and no note has a `scope` key
- **THEN** `bilbo check` prints no line about `scope`

#### Scenario: A missing scope
- **WHEN** the config declares `personal` and `work`, and `notes/plan-release.md` has no `scope` key
- **THEN** stdout has `notes/plan-release.md: scope: missing; scopes: personal, work` and the exit code is 1

#### Scenario: An undeclared scope
- **WHEN** the config declares only `work`, and `notes/plan-release.md` has `scope: acme`
- **THEN** stdout has `notes/plan-release.md: scope: 'acme' is not declared in <config path>; scopes: work` and the exit code is 1

#### Scenario: An invalid value is reported once
- **WHEN** the config declares `work` and `notes/plan-release.md` has `scope: Work`
- **THEN** stdout has one line for the file naming `scope`, and no `is not declared` or `missing` line for it

#### Scenario: A scope with no scopes declared
- **WHEN** the config holds no `scope.*` key and `notes/plan-release.md` has `scope: work`
- **THEN** stdout has `notes/plan-release.md: scope: 'work' is not declared in <config path>` and the exit code is 1

#### Scenario: A broken config
- **WHEN** the config holds `scope.work.embedder = remote` and an agent runs `bilbo check`
- **THEN** stderr names `scope.work.embedder`, stdout is empty, and the exit code is 2

### Requirement: Scope marks
For each declared scope other than its own whose mark a note holds, a note in a declared scope SHALL get the warning `scope: '<own>' but <place> holds '<mark>', a mark of '<other>' (warning)`, where `<place>` is `the file name` or `line <n>`, the first place holding a mark of that scope. An unassigned note SHALL instead have its `scope: missing` or not-declared problem end with `; holds marks of <scope names>`. A mark never holds anything back.

#### Scenario: A mark of another scope
- **WHEN** `scope.work.marks = acme` and `notes/gotcha-deploy.md` has `scope: personal` and says `Acme's deploy` on line 9, its first mention
- **THEN** stdout has `notes/gotcha-deploy.md: scope: 'personal' but line 9 holds 'acme', a mark of 'work' (warning)`, and the exit code is 0 when no other line is printed

#### Scenario: A mark in the file name
- **WHEN** `scope.work.marks = acme` and `notes/gotcha-acme-deploy.md` has `scope: personal` and never says `acme` in its text
- **THEN** stdout has `notes/gotcha-acme-deploy.md: scope: 'personal' but the file name holds 'acme', a mark of 'work' (warning)`

#### Scenario: A mark of the note's own scope
- **WHEN** `scope.work.marks = acme` and a note with `scope: work` mentions `acme`
- **THEN** `bilbo check` prints no line about marks for it

#### Scenario: An unassigned note holding marks
- **WHEN** the config declares `personal` and `work`, `scope.work.marks = acme`, and `notes/plan-x.md` has no `scope` key and mentions `acme`
- **THEN** stdout has `notes/plan-x.md: scope: missing; scopes: personal, work; holds marks of work`

### Requirement: Matching a mark
Marks SHALL be searched in the topic of a note's file name, its `sources` items and its body, fenced code included. A word mark SHALL match a whole word as `recall` compares words, ignoring case and Latin accents. A path mark SHALL match its text when the next character is not a letter, a digit, `-`, `_` or `.`. A path mark inside the home folder SHALL match in either form, starting with `~/` or with the home folder's absolute path.

#### Scenario: A word in a source
- **WHEN** `scope.work.marks = acme` and a note's only mention is the item `  - "url: https://wiki.acme.example/deploy"`
- **THEN** the note holds a mark of `work`

#### Scenario: Part of a word does not match
- **WHEN** `scope.work.marks = acme` and a note says `acmeish` and nothing else close
- **THEN** the note holds no mark of `work`

#### Scenario: A path in either form
- **WHEN** `HOME` is `/Users/a`, `scope.work.marks = ~/Developer/acme`, and a note says `/Users/a/Developer/acme/api/main.go`
- **THEN** the note holds a mark of `work`

#### Scenario: A sibling path does not match
- **WHEN** `scope.work.marks = ~/Developer/acme` and a note says `~/Developer/acme-tools`
- **THEN** the note holds no mark of `work`
