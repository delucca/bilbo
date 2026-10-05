# note-scope Specification

## Purpose
What a note's scope means on a device: which scopes the config declares, when a note is unassigned, the embedder rule each note gets, and `bilbo scope`, which lists the scopes and assigns notes to them.

## Requirements

### Requirement: Declared and unassigned notes
A scope SHALL be declared on a device when its config holds at least one `scope.<name>.*` key, as the `config` spec's Scope settings say. A note SHALL belong to the scope its `scope` key names when that scope is declared. A note SHALL be unassigned when it has no `scope` key, when the value is not a valid name, or when it names a scope the config does not declare.

#### Scenario: A note in a declared scope
- **WHEN** the config holds `scope.work.embedder = local` and a note has `scope: work`
- **THEN** the note belongs to `work`

#### Scenario: No key
- **WHEN** the config declares `work` and a note has no `scope` key
- **THEN** the note is unassigned

#### Scenario: A name this device does not declare
- **WHEN** the config declares only `work` and a note has `scope: acme`
- **THEN** the note is unassigned, and its file still says `scope: acme`

### Requirement: Embedder rule
Each note SHALL have an embedder rule, `any` or `local`. A note in a declared scope SHALL take that scope's `scope.<name>.embedder`, `any` when unset. An unassigned note SHALL take `local` when any declared scope sets `embedder = local`, and `any` otherwise. With no scope declared, every note's rule SHALL be `any`. The `note-index` spec's Withheld passages requirement acts on the rule.

#### Scenario: The scope's own rule
- **WHEN** the config holds `scope.work.embedder = local` and `scope.personal.sync = off`
- **THEN** a note with `scope: work` has the rule `local`, and a note with `scope: personal` has the rule `any`

#### Scenario: Unassigned takes the strictest rule
- **WHEN** the config holds `scope.work.embedder = local` and a note has no `scope` key
- **THEN** the note has the rule `local`

#### Scenario: No scope asks for local
- **WHEN** the config holds only `scope.personal.sync = off` and a note has no `scope` key
- **THEN** the note has the rule `any`

#### Scenario: No scopes at all
- **WHEN** the config holds no `scope.*` key
- **THEN** every note has the rule `any`, whatever its `scope` key says

### Requirement: List the scopes
`bilbo scope` SHALL print one line per declared scope, sorted by name: the name, then tab-separated `<n> notes`, `sync <value>`, `embedder <rule>` and `paths <list as written>` (`paths -` when unset), then `default` when `scope.default` names it. A last line SHALL be `(unassigned)`, `<n> notes` and `embedder <rule>`, counting every unassigned note. It SHALL count the notes `recall` searches, treat a missing store as empty, and exit 0.

#### Scenario: Two scopes
- **WHEN** the config holds `scope.personal.sync = off`, `scope.work.embedder = local`, `scope.work.paths = ~/Developer/acme` and `scope.default = personal`, and the store holds 3 notes with `scope: personal`, 1 with `scope: work`, 1 with no key and 1 with `scope: acme`
- **THEN** stdout is `personal`, tab, `3 notes`, tab, `sync off`, tab, `embedder any`, tab, `paths -`, tab, `default`; then `work`, tab, `1 notes`, tab, `sync off`, tab, `embedder local`, tab, `paths ~/Developer/acme`; then `(unassigned)`, tab, `2 notes`, tab, `embedder local`; and the exit code is 0

#### Scenario: No scope declared
- **WHEN** the config holds no `scope.*` key and the store holds 4 notes
- **THEN** stdout is `(unassigned)`, tab, `4 notes`, tab, `embedder any`, stderr is `bilbo: no scopes declared; add scope.<name>.* keys to <config path>`, and the exit code is 0

#### Scenario: No store yet
- **WHEN** the config declares `work` and `<root>/notes/` does not exist
- **THEN** the `work` line says `0 notes`, the `(unassigned)` line says `0 notes`, and the exit code is 0

#### Scenario: An extra argument
- **WHEN** an agent runs `bilbo scope work` or `bilbo scope --json`
- **THEN** bilbo prints a message naming `work` or `--json` to stderr and exits 2

#### Scenario: A broken config
- **WHEN** the config holds `scope.work.embedder = remote` and an agent runs `bilbo scope`
- **THEN** stderr names `scope.work.embedder`, stdout is empty, and the exit code is 2

#### Scenario: Listing is read-only
- **WHEN** an agent runs `bilbo scope`
- **THEN** every entry under the root has the same bytes and modification time as before the run

### Requirement: Assign a scope
`bilbo scope set [--force] <name> <file>...` SHALL give each file the scope `<name>`. A file with no `scope` key SHALL gain `scope: <name>` as the last line of its frontmatter. A file whose key already holds `<name>` SHALL be left as it is. A file whose key holds another value SHALL be left as it is, unless `--force` is given, which replaces the value in place. No other byte of a file SHALL change. Each file SHALL be handled even when an earlier one fails.

#### Scenario: Filling a missing key
- **WHEN** the config declares `work` and `<root>/notes/gotcha-acme-deploy.md` has no `scope` key, and a user runs `bilbo scope set work <root>/notes/gotcha-acme-deploy.md`
- **THEN** the file's frontmatter ends with `scope: work` before its closing `---`, every other line is unchanged, stdout is `notes/gotcha-acme-deploy.md: set work`, and the exit code is 0

#### Scenario: Already in the scope
- **WHEN** the file already has `scope: work` and a user runs `bilbo scope set work <file>`
- **THEN** the file's bytes and modification time are unchanged, stdout is `notes/<file name>: kept work`, and the exit code is 0

#### Scenario: Another scope is kept without --force
- **WHEN** the file has `scope: personal` and a user runs `bilbo scope set work <file>`
- **THEN** the file is unchanged, stdout is `notes/<file name>: kept personal; --force replaces it`, and the exit code is 0

#### Scenario: Bulk triage in two passes
- **WHEN** a user runs `bilbo scope set work <root>/notes/gotcha-acme-*.md` and then `bilbo scope set personal <root>/notes/*.md`
- **THEN** the acme notes keep `scope: work`, every other note gets `scope: personal`, and both runs exit 0

#### Scenario: --force replaces it
- **WHEN** the file has `scope: personal` on line 4 and a user runs `bilbo scope set --force work <file>`
- **THEN** line 4 is `scope: work`, every other line is unchanged, and stdout is `notes/<file name>: replaced personal with work`

#### Scenario: --force mends an invalid value
- **WHEN** the file has `scope: Work` and a user runs `bilbo scope set --force work <file>`
- **THEN** that line becomes `scope: work`, and `bilbo check` reports no `scope` problem for the file

#### Scenario: A key not written as `scope: <value>`
- **WHEN** the file's key line is `scope:work` and a user runs `bilbo scope set work <file>`, then `bilbo scope set --force work <file>`
- **THEN** the first run leaves the file unchanged and prints `notes/<file name>: kept 'scope:work' as written; --force rewrites it` with exit 0, and the second turns the line into `scope: work` and prints `notes/<file name>: rewrote 'scope:work' as scope: work`

#### Scenario: Several files, one bad
- **WHEN** a user runs `bilbo scope set work <root>/notes/plan-a.md /tmp/x.md <root>/notes/plan-b.md`, and neither note has a `scope` key
- **THEN** both notes get `scope: work`, stderr names `/tmp/x.md` as not a note in `<root>/notes`, and the exit code is 1

#### Scenario: A scope this device does not declare
- **WHEN** the config does not declare `acme` and a user runs `bilbo scope set acme <file>`
- **THEN** bilbo prints a message naming `acme` and listing the declared scopes to stderr, exits 2, and changes no file

#### Scenario: Missing arguments
- **WHEN** a user runs `bilbo scope set work` with no file
- **THEN** bilbo prints a usage message to stderr, exits 2, and changes no file

#### Scenario: The change is in history
- **WHEN** `bilbo watch` is running and a user runs `bilbo scope set work <root>/notes/plan-a.md`
- **THEN** `bilbo history a` lists a new `edited` version whose text holds `scope: work`

### Requirement: Files scope set refuses
`bilbo scope set` SHALL refuse, unchanged, a path that is not a regular file directly in `<root>/notes/` with a note's name, a note whose text is not UTF-8, and a note whose frontmatter has no closing `---`, no canonical `id`, or more than one `scope` line. Each refusal SHALL print `bilbo: <path>: <reason>` to stderr and count as failed. A run with any failed file SHALL exit 1, after handling every file.

#### Scenario: A file with broken frontmatter
- **WHEN** `plan-a.md` starts with `# Title` and a user runs `bilbo scope set work <root>/notes/plan-a.md`
- **THEN** the file is unchanged, stderr names `notes/plan-a.md` and its frontmatter, and the exit code is 1

#### Scenario: Two scope lines
- **WHEN** `plan-a.md` holds `scope: work` and `scope: personal` and a user runs `bilbo scope set --force work <root>/notes/plan-a.md`
- **THEN** the file is unchanged, stderr names `notes/plan-a.md` and `scope`, and the exit code is 1

### Requirement: Assigning never loses a write
`bilbo scope set` SHALL work under the `note-restore` spec's history lock and hidden file, `<root>/notes/.bilbo-restore-<id>`, sweeping leftovers first as `bilbo restore` does. It SHALL swap the new text in atomically and compare what came out with what it read. When they differ, it SHALL swap back, so the other writer's bytes are in place, and report the file. It SHALL delete only bytes it read or wrote itself; other bytes stay at the hidden name for the next sweep to record.

#### Scenario: An agent writes during the swap
- **WHEN** an agent writes new text to `plan-a.md` after `bilbo scope set work` read it and before it swapped
- **THEN** `plan-a.md` holds the agent's text, stderr is `bilbo: notes/plan-a.md: changed while bilbo scope set ran; run it again`, and the exit code is 1

#### Scenario: A second write lands between the two swaps
- **WHEN** an agent writes `plan-a.md` before the first swap and again between the first swap and the swap back
- **THEN** `plan-a.md` holds the agent's first write, `notes/.bilbo-restore-<id of a>` holds the second, stderr names that hidden file, the exit code is 1, and the next `bilbo watch` scan records the second write as an `edited` version and removes the hidden file

#### Scenario: A filesystem that cannot swap
- **WHEN** the store sits on a filesystem that cannot exchange two files atomically and a user runs `bilbo scope set work <file>`
- **THEN** stderr is `bilbo: cannot set scopes on this filesystem: it cannot swap files atomically`, no file changes, and the exit code is 1
