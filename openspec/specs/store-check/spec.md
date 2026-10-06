# store-check Specification

## Purpose
`bilbo check` lints the whole store: the notes against the `note-store` contract and the scope rules, and the library against the `library-store` contract. Mistakes agents make while editing files directly then surface, without bilbo blocking or changing anything.

## Requirements

### Requirement: Report problems
`bilbo check` SHALL check every entry in `<root>/notes/` against every `note-store` rule and the scope rules below, and every entry in `<root>/library/` against every `library-store` rule. It SHALL print one line per problem and per warning to stdout as `<path relative to the root>: <message>`, sorted by path and then by message, and exit 1 when there is at least one problem. A warning SHALL NOT change the exit code. With no problems and no warnings, it SHALL print nothing and exit 0.

#### Scenario: A clean store
- **WHEN** every note and every library file in the store follows its contract
- **THEN** stdout is empty and the exit code is 0

#### Scenario: A note with a bad timestamp
- **WHEN** `notes/plan-release.md` has `created: 2026-10-02`
- **THEN** stdout has a line that starts with `notes/plan-release.md: ` and names `created`, and the exit code is 1

#### Scenario: An empty store
- **WHEN** `<root>/notes/` exists and holds no notes, and `<root>/library/` does not exist
- **THEN** stdout is empty and the exit code is 0

#### Scenario: An edited source
- **WHEN** an agent changed a word in the body of `library/go/effective-go.md`
- **THEN** stdout has a line that starts with `library/go/effective-go.md: ` and names `digest`, and the exit code is 1

#### Scenario: A stub entry
- **WHEN** `library/go/guide.md` still holds `TODO: describe this source.` under `## errors`
- **THEN** stdout has a line that starts with `library/go/guide.md: ` and names the entry `errors`, and the exit code is 1

#### Scenario: Warnings alone
- **WHEN** the only line `bilbo check` prints is a Scope marks warning
- **THEN** stdout holds that line and the exit code is 0

### Requirement: Report every problem in one run
`bilbo check` SHALL report every problem it finds, including several in one file, rather than stop at the first. A problem that spans files, a shared id or a shared topic, SHALL be reported on each file involved, whether the files sit in `notes/`, in `library/` or in both.

#### Scenario: Two problems in one file
- **WHEN** `notes/plan-release.md` has a lowercase id and two `# ` titles
- **THEN** stdout has two lines for `notes/plan-release.md`, one naming `id` and one naming the title

#### Scenario: A shared id
- **WHEN** `notes/plan-a.md` and `notes/plan-b.md` have the same id
- **THEN** stdout has a line for each file that names the other

#### Scenario: A shared topic
- **WHEN** `notes/plan-release.md` and `notes/decision-release.md` both exist
- **THEN** stdout has a line for each file that names the other

#### Scenario: A note and a source share an id
- **WHEN** `notes/plan-a.md` and `library/go/effective-go.md` have the same id
- **THEN** stdout has a line for each file that names the other

### Requirement: Check is read-only
`bilbo check` SHALL NOT create, change, rename or delete any file or folder.

#### Scenario: A store with problems is left as found
- **WHEN** `bilbo check` runs on a store with problems
- **THEN** every entry under the root has the same bytes and modification time as before the run

### Requirement: A missing store is a problem
When neither `<root>/notes/` nor `<root>/library/` exists, `bilbo check` SHALL print `bilbo: no store at <root>` to stderr and exit 1. A misconfigured root then fails loudly instead of passing as clean. A store with only one of the two folders SHALL be checked.

#### Scenario: Wrong BILBO_HOME
- **WHEN** `BILBO_HOME` names a folder with neither `notes/` nor `library/` inside it and an agent runs `bilbo check`
- **THEN** stderr is `bilbo: no store at <that folder>`, stdout is empty and the exit code is 1

#### Scenario: A store with only a library
- **WHEN** `<root>/library/` holds valid corpora and `<root>/notes/` does not exist
- **THEN** stdout is empty and the exit code is 0

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
