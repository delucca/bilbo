# Spec Delta

## MODIFIED Requirements

### Requirement: Creating a note
The note skill SHALL create a note only with `bilbo new <kind> <topic>`, adding `--title <text>` when the default title does not read well, and `--scope <name>` only as Choosing a scope says. It SHALL then write the body below the title in the file whose path `bilbo new` printed, leaving the `id`, `created`, `scope` and title that `bilbo new` wrote. It SHALL act on the exit code of `bilbo new`, as the scenarios say.

#### Scenario: A new note
- **WHEN** `bilbo new gotcha macos-test-timeout` exits 0 and prints a path
- **THEN** the agent writes the body into that file, below `# Macos test timeout`

#### Scenario: The topic is taken
- **WHEN** `bilbo new decision release-tags` exits 1 with `bilbo: topic 'release-tags' already has a note: <path>`
- **THEN** the agent reads the file at `<path>` and updates it instead

#### Scenario: A bad argument
- **WHEN** `bilbo new` exits 2 for an unknown kind, an invalid topic, an unusable title or an undeclared scope
- **THEN** the agent fixes that argument, picking a scope only from the ones bilbo listed, and runs `bilbo new` once more, and if it exits 2 again, shows bilbo's first stderr line and stops

#### Scenario: Another failure
- **WHEN** `bilbo new` exits 1 with a message that names no existing note, such as an unwritable `notes/`
- **THEN** the agent shows bilbo's first stderr line, writes no file and stops

### Requirement: Note content
The note skill SHALL keep the frontmatter to `id`, `created` and, when present, `scope` and `sources`. It SHALL never change `id` or `created`, and SHALL keep `scope` as found unless Choosing a scope says otherwise. It SHALL add to `sources` only what was read or run in the session, each as a `note-store` item, and SHALL never invent a source. It SHALL write only the file of the note it creates or updates.

#### Scenario: Sources from the session
- **WHEN** the agent read `src/new.rs` and fetched `https://example.org/spec` in this session, and the note rests on both
- **THEN** the note's `sources` lists `  - "code: src/new.rs"` and `  - "url: https://example.org/spec"`

#### Scenario: Nothing was read
- **WHEN** the note records a decision the user stated, and nothing was read or run for it
- **THEN** the note has no `sources` key

#### Scenario: No legacy keys
- **WHEN** the agent writes or updates any note
- **THEN** its frontmatter has no `kind`, `supersedes` or other key beyond `id`, `created`, `scope` and `sources`

#### Scenario: An update keeps the scope
- **WHEN** the agent updates `decision-release-tags.md`, which has `scope: work`, and the user said nothing about its scope
- **THEN** the file still has `scope: work` on the same line

### Requirement: Check after every write
After every create, edit or rename, the note skill SHALL run `bilbo check`. It SHALL fix every problem line that names the note it touched, except a line about a topic or id the note shares with another file, a `scope: '<name>' is not declared` line, or a warning line, which it SHALL report instead, and run `bilbo check` again, at most three runs in all. A `scope: missing` line SHALL be handled as Choosing a scope says. It SHALL leave lines for other notes as they are and give their count in its report. When `bilbo check` exits 1 with `bilbo: no store at <root>`, or exits 2, it SHALL show bilbo's first stderr line and stop.

#### Scenario: A clean store
- **WHEN** `bilbo check` exits 0 after the agent wrote the note
- **THEN** the agent reports the note

#### Scenario: A problem in the note
- **WHEN** `bilbo check` prints `notes/gotcha-macos-test-timeout.md: sources: ...` after the agent wrote `sources: []`
- **THEN** the agent fixes the key, runs `bilbo check` again, and it prints no line for that note

#### Scenario: A shared topic is reported, not fixed
- **WHEN** `bilbo check` prints `notes/plan-release-tags.md: topic: 'release-tags' is also the topic of notes/decision-release-tags.md` after a rename found both files
- **THEN** the agent changes neither file and reports the line

#### Scenario: A mark warning is reported, not fixed
- **WHEN** `bilbo check` exits 0 and prints `notes/gotcha-deploy.md: scope: 'personal' but line 9 holds 'acme', a mark of 'work' (warning)` for the note the agent wrote
- **THEN** the agent leaves the note's scope and text as they are and reports the line

#### Scenario: Problems elsewhere
- **WHEN** `bilbo check` prints lines only for other notes
- **THEN** the agent changes none of those notes and says how many lines it left

### Requirement: Report the note
When it is done, the note skill SHALL say whether it created, updated or renamed the note, give the note's absolute path, and name the note's scope, or say `unassigned` when the note has none or one this device does not declare.

#### Scenario: A created note
- **WHEN** the agent created `<root>/notes/gotcha-macos-test-timeout.md` with `scope: work` and `bilbo check` passed for it
- **THEN** the agent says it created the note, prints that absolute path, and names the scope `work`

#### Scenario: An unassigned note
- **WHEN** the agent created a note that has no `scope` key
- **THEN** the agent's report says the note is `unassigned`

#### Scenario: Nothing written
- **WHEN** the skill stopped before writing, for a missing binary or a failed `bilbo` command
- **THEN** the agent does not say a note was created or updated

## ADDED Requirements

### Requirement: Choosing a scope
Before `bilbo new`, the note skill SHALL run `bilbo scope`. With no scope declared, it SHALL pass no `--scope` and ask nothing about scopes. Otherwise it SHALL pass `--scope` with the scope the user named, or else let `bilbo new` resolve one. When the note ends up unassigned, or the scope looks wrong for the subject, it SHALL ask the user once, naming the scopes, and apply the answer with `bilbo scope set`. With no answer, it SHALL leave the scope as it is.

#### Scenario: The user names the scope
- **WHEN** `bilbo scope` lists `personal` and `work`, and the user says "keep this work gotcha about the deploy"
- **THEN** the agent runs `bilbo new gotcha <topic> --scope work`

#### Scenario: No scope declared
- **WHEN** `bilbo scope` prints only the `(unassigned)` line
- **THEN** the agent runs `bilbo new` without `--scope` and asks no question about scopes

#### Scenario: The new note is unassigned
- **WHEN** `bilbo new` exits 0 with `bilbo: no scope for <path>; scopes: personal, work; ...` on stderr, and the user answers `work` to the agent's one question
- **THEN** the agent runs `bilbo scope set work <path>` before it writes the body

#### Scenario: Nobody to ask
- **WHEN** the note ends up unassigned in a run that cannot ask the user, such as `codex exec`
- **THEN** the agent sets no scope, writes the note, and reports it as `unassigned`

#### Scenario: A resolved scope that looks wrong
- **WHEN** `bilbo new` gave the note `scope: personal` from the working directory, the note records the user's employer's deploy, and the user answers `work`
- **THEN** the agent runs `bilbo scope set --force work <path>`

#### Scenario: A missing scope on an update
- **WHEN** the agent updated a note with no `scope` key and `bilbo check` prints `scope: missing; scopes: personal, work` for it
- **THEN** the agent asks the user once which scope the note belongs to, applies the answer with `bilbo scope set`, and otherwise reports the note as `unassigned`

#### Scenario: Asking once
- **WHEN** the agent already asked the user about the note's scope in this run
- **THEN** it asks no second question about it
