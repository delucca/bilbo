# Spec Delta

## MODIFIED Requirements

### Requirement: One plugin for Claude Code and Codex
The repository SHALL ship one plugin named `bilbo` in `plugins/bilbo/`, with a Claude Code manifest at `.claude-plugin/plugin.json` and a Codex manifest at `.codex-plugin/plugin.json` inside it. It SHALL list the plugin in a Claude Code marketplace at `.claude-plugin/marketplace.json` and a Codex marketplace at `.agents/plugins/marketplace.json`, both named `bilbo`, with the source `./plugins/bilbo`. The plugin SHALL hold skills, the digest hook and the compaction hook, and no subagent or MCP server.

#### Scenario: Claude Code installs the plugin
- **WHEN** a user runs `claude plugin marketplace add delucca/bilbo` and then `claude plugin install bilbo@bilbo`
- **THEN** the plugin is installed and its `recall` and `note` skills, its digest hook and its compaction hook are available

#### Scenario: Codex installs the plugin
- **WHEN** a user runs `codex plugin marketplace add delucca/bilbo` and then `codex plugin add bilbo@bilbo`
- **THEN** Codex offers the skills `bilbo:recall` and `bilbo:note` and lists the digest hook and the compaction hook for review

#### Scenario: The plugin adds no subagent or MCP server
- **WHEN** the plugin is installed in either tool
- **THEN** it adds no subagent and no MCP server, and no hook but the digest hook and the compaction hook

### Requirement: A missing binary
When `bilbo` is not on PATH, the recall skill SHALL stop with the line `recall: bilbo is not on PATH; install the bilbo CLI first`, and the note skill with the line `note: bilbo is not on PATH; install the bilbo CLI first`. Neither SHALL search, read or write the notes any other way.

#### Scenario: bilbo is not installed
- **WHEN** `command -v bilbo` prints nothing
- **THEN** the agent prints that line and stops, without reading or grepping any note

#### Scenario: The note skill without bilbo
- **WHEN** a user asks to keep a decision for later sessions and `command -v bilbo` prints nothing
- **THEN** the agent prints `note: bilbo is not on PATH; install the bilbo CLI first` and writes no file

## ADDED Requirements

### Requirement: The note skill
The plugin SHALL hold a skill named `note` in `skills/note/SKILL.md`. Its frontmatter SHALL have only `name`, `description`, `license` and `allowed-tools`, and `allowed-tools` SHALL cover every command the skill runs. The description SHALL say when to write a note: the user asks to keep something for later sessions, or the session settled something durable such as a decision, a gotcha or a plan. It SHALL also say that a passing "note that…" in conversation is not a reason to write one.

#### Scenario: The user asks to keep something
- **WHEN** a user says "keep this decision about the release tags for later sessions"
- **THEN** the agent runs the `note` skill

#### Scenario: The session settled a gotcha
- **WHEN** a reader checks the `note` skill's description
- **THEN** it names the case where the session settled something durable, such as a decision, a gotcha or a plan, as a reason to write a note, next to the explicit asks

#### Scenario: A passing remark
- **WHEN** a user says "note that the build is slow today" while asking for something else
- **THEN** the agent does not run the `note` skill

### Requirement: The note on the same subject
A note's subject is the question, decision or component it records, under any kind or wording; it is not the `topic` in its file name. Before it creates a note, the note skill SHALL run `bilbo recall` with words that name the subject, and SHALL read the hits that may be on the same subject. When one is, the skill SHALL update that note instead of creating another. When `bilbo recall` exits 1 with a last stderr line of `bilbo: no notes match` or `bilbo: no store at <root>`, the skill SHALL go on to create the note. On any other non-zero exit, it SHALL show bilbo's first stderr line and write nothing.

#### Scenario: A note on the subject exists
- **WHEN** `decision-release-tags.md` covers the subject and `bilbo recall` lists it
- **THEN** the agent edits `decision-release-tags.md`, leaves its `id` and `created` as they were, and runs no `bilbo new`

#### Scenario: No note on the subject
- **WHEN** `bilbo recall` exits 1 with `bilbo: no notes match`
- **THEN** the agent creates the note with `bilbo new`

#### Scenario: Recall cannot run
- **WHEN** `bilbo recall` exits 2 because `BILBO_HOME` is relative
- **THEN** the agent shows bilbo's first stderr line and writes no file

### Requirement: Creating a note
The note skill SHALL create a note only with `bilbo new <kind> <topic>`, adding `--title <text>` when the default title does not read well. It SHALL then write the body below the title in the file whose path `bilbo new` printed, leaving the `id`, `created` and title that `bilbo new` wrote. It SHALL act on the exit code of `bilbo new`, as the scenarios say.

#### Scenario: A new note
- **WHEN** `bilbo new gotcha macos-test-timeout` exits 0 and prints a path
- **THEN** the agent writes the body into that file, below `# Macos test timeout`

#### Scenario: The topic is taken
- **WHEN** `bilbo new decision release-tags` exits 1 with `bilbo: topic 'release-tags' already has a note: <path>`
- **THEN** the agent reads the file at `<path>` and updates it instead

#### Scenario: A bad argument
- **WHEN** `bilbo new` exits 2 for an unknown kind, an invalid topic or an unusable title
- **THEN** the agent fixes that argument and runs `bilbo new` once more, and if it exits 2 again, shows bilbo's first stderr line and stops

#### Scenario: Another failure
- **WHEN** `bilbo new` exits 1 with a message that names no existing note, such as an unwritable `notes/`
- **THEN** the agent shows bilbo's first stderr line, writes no file and stops

### Requirement: Note content
The note skill SHALL keep the frontmatter to `id`, `created` and, when there are sources, `sources`. It SHALL never change `id` or `created`. It SHALL add to `sources` only what was read or run in the session, each as a `note-store` item, and SHALL never invent a source. It SHALL write only the file of the note it creates or updates.

#### Scenario: Sources from the session
- **WHEN** the agent read `src/new.rs` and fetched `https://example.org/spec` in this session, and the note rests on both
- **THEN** the note's `sources` lists `  - "code: src/new.rs"` and `  - "url: https://example.org/spec"`

#### Scenario: Nothing was read
- **WHEN** the note records a decision the user stated, and nothing was read or run for it
- **THEN** the note has no `sources` key

#### Scenario: No legacy keys
- **WHEN** the agent writes or updates any note
- **THEN** its frontmatter has no `kind`, `supersedes` or other key beyond `id`, `created` and `sources`

### Requirement: Changing a note's kind
When the note on a subject now records another kind, such as a plan that became a decision, the note skill SHALL rename `<old kind>-<topic>.md` to `<new kind>-<topic>.md` in the same folder, without overwriting an existing file. The rename SHALL keep the note's `id` and `created`. The skill SHALL then run `bilbo check`, as for any edit.

#### Scenario: A plan becomes a decision
- **WHEN** `plan-release-tags.md` exists and the session settles the plan
- **THEN** `decision-release-tags.md` holds the same `id` and `created`, `plan-release-tags.md` no longer exists, and `bilbo check` reports nothing for it

#### Scenario: The new name is taken
- **WHEN** both `plan-release-tags.md` and `decision-release-tags.md` exist
- **THEN** the agent renames nothing, updates `decision-release-tags.md` (the file of the requested kind), leaves the bytes of `plan-release-tags.md` as they are, deletes neither file, and reports the shared-topic lines of `bilbo check` so the user can decide

### Requirement: Check after every write
After every create, edit or rename, the note skill SHALL run `bilbo check`. It SHALL fix every problem line that names the note it touched, except a line about a topic or id the note shares with another file, which it SHALL report instead, and run `bilbo check` again, at most three runs in all. It SHALL leave lines for other notes as they are and give their count in its report. When `bilbo check` exits 1 with `bilbo: no store at <root>`, or exits 2, it SHALL show bilbo's first stderr line and stop.

#### Scenario: A clean store
- **WHEN** `bilbo check` exits 0 after the agent wrote the note
- **THEN** the agent reports the note

#### Scenario: A problem in the note
- **WHEN** `bilbo check` prints `notes/gotcha-macos-test-timeout.md: sources: ...` after the agent wrote `sources: []`
- **THEN** the agent fixes the key, runs `bilbo check` again, and it prints no line for that note

#### Scenario: A shared topic is reported, not fixed
- **WHEN** `bilbo check` prints `notes/plan-release-tags.md: topic: 'release-tags' is also the topic of notes/decision-release-tags.md` after a rename found both files
- **THEN** the agent changes neither file and reports the line

#### Scenario: Problems elsewhere
- **WHEN** `bilbo check` prints lines only for other notes
- **THEN** the agent changes none of those notes and says how many lines it left

### Requirement: Report the note
When it is done, the note skill SHALL say whether it created, updated or renamed the note, and give the note's absolute path.

#### Scenario: A created note
- **WHEN** the agent created `<root>/notes/gotcha-macos-test-timeout.md` and `bilbo check` passed for it
- **THEN** the agent says it created the note and prints that absolute path

#### Scenario: Nothing written
- **WHEN** the skill stopped before writing, for a missing binary or a failed `bilbo` command
- **THEN** the agent does not say a note was created or updated

### Requirement: The compaction hook
`hooks/hooks.json` SHALL register one SessionStart hook with the matcher `compact`: a command hook whose command is exactly `command -v bilbo >/dev/null 2>&1 || exit 0; echo 'Context was compacted. If this session settled something later sessions should know, such as a decision, a gotcha or a plan, save it with the bilbo note skill once the current task allows.'` and whose `timeout` is 5 seconds. It SHALL always exit 0.

#### Scenario: Claude Code after an auto-compaction
- **WHEN** the plugin is installed in Claude Code, `bilbo` is on PATH, and the context is compacted automatically
- **THEN** the agent's next request holds the hook's line as context

#### Scenario: Codex after bilbo setup
- **WHEN** `bilbo setup` installed the plugin in Codex and trusted its hooks, and Codex compacts the context
- **THEN** the next model request holds the hook's line as a developer message before the user's prompt

#### Scenario: bilbo is not installed
- **WHEN** `bilbo` is not on the PATH the tool gives its hooks and the context is compacted
- **THEN** the hook prints nothing, exits 0, and the tool shows no hook error

#### Scenario: A session that did not compact
- **WHEN** a session starts, resumes or is cleared
- **THEN** the hook does not run
