# Spec Delta

## Purpose

`bilbo check` lints the whole store against the `note-store` contract. Mistakes agents make while editing files directly then surface, without bilbo blocking or changing anything.

## ADDED Requirements

### Requirement: Report problems
`bilbo check` SHALL check every entry in `<root>/notes/` against every `note-store` rule. It SHALL print one line per problem to stdout as `<path relative to the root>: <message>`, sorted by path and then by message, and exit 1 when there is at least one problem. With no problems, it SHALL print nothing and exit 0.

#### Scenario: A clean store
- **WHEN** every note in the store follows the contract
- **THEN** stdout is empty and the exit code is 0

#### Scenario: A note with a bad timestamp
- **WHEN** `notes/plan-release.md` has `created: 2026-10-02`
- **THEN** stdout has a line that starts with `notes/plan-release.md: ` and names `created`, and the exit code is 1

#### Scenario: An empty store
- **WHEN** `<root>/notes/` exists and holds no notes
- **THEN** stdout is empty and the exit code is 0

### Requirement: Report every problem in one run
`bilbo check` SHALL report every problem it finds, including several in one file, rather than stop at the first. A problem that spans files, a shared id or a shared topic, SHALL be reported on each file involved.

#### Scenario: Two problems in one file
- **WHEN** `notes/plan-release.md` has a lowercase id and two `# ` titles
- **THEN** stdout has two lines for `notes/plan-release.md`, one naming `id` and one naming the title

#### Scenario: A shared id
- **WHEN** `notes/plan-a.md` and `notes/plan-b.md` have the same id
- **THEN** stdout has a line for each file that names the other

#### Scenario: A shared topic
- **WHEN** `notes/plan-release.md` and `notes/decision-release.md` both exist
- **THEN** stdout has a line for each file that names the other

### Requirement: Check is read-only
`bilbo check` SHALL NOT create, change, rename or delete any file or folder.

#### Scenario: A store with problems is left as found
- **WHEN** `bilbo check` runs on a store with problems
- **THEN** every entry under the root has the same bytes and modification time as before the run

### Requirement: A missing store is a problem
When `<root>/notes/` does not exist, `bilbo check` SHALL print `bilbo: no store at <root>` to stderr and exit 1. A misconfigured root then fails loudly instead of passing as clean.

#### Scenario: Wrong BILBO_HOME
- **WHEN** `BILBO_HOME` names a folder with no `notes/` inside it and an agent runs `bilbo check`
- **THEN** stderr is `bilbo: no store at <that folder>`, stdout is empty and the exit code is 1
