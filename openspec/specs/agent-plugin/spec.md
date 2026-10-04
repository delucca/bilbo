# agent-plugin Specification

## Purpose
How bilbo reaches agents: one plugin for Claude Code and Codex, installed from this repository, whose skills drive the `bilbo` CLI.

## Requirements

### Requirement: Plugin versions
The Claude Code manifest and its marketplace entry SHALL NOT set a `version`. The Codex manifest's `version` SHALL equal the `version` in `Cargo.toml`.

#### Scenario: Claude Code follows commits
- **WHEN** a commit changes the plugin on `main` and a user updates the plugin
- **THEN** Claude Code installs that commit, with no version bump

#### Scenario: A version bump that skips the plugin
- **WHEN** `Cargo.toml`'s `version` changes and `.codex-plugin/plugin.json` keeps the old one
- **THEN** `cargo test` fails

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

### Requirement: A missing binary
When `bilbo` is not on PATH, the recall skill SHALL stop with the line `recall: bilbo is not on PATH; install the bilbo CLI first`, the note skill with the line `note: bilbo is not on PATH; install the bilbo CLI first`, the reference skill with the line `reference: bilbo is not on PATH; install the bilbo CLI first`, and the ingest skill with the line `ingest: bilbo is not on PATH; install the bilbo CLI first`. None of them SHALL search, read or write the notes, the sources or the library any other way.

#### Scenario: bilbo is not installed
- **WHEN** `command -v bilbo` prints nothing
- **THEN** the agent prints that line and stops, without reading or grepping any note

#### Scenario: The note skill without bilbo
- **WHEN** a user asks to keep a decision for later sessions and `command -v bilbo` prints nothing
- **THEN** the agent prints `note: bilbo is not on PATH; install the bilbo CLI first` and writes no file

#### Scenario: The reference skill without bilbo
- **WHEN** a user asks what Effective Go says about goroutines and `command -v bilbo` prints nothing
- **THEN** the agent prints `reference: bilbo is not on PATH; install the bilbo CLI first`, reads no source file, and does not answer from memory

#### Scenario: The ingest skill without bilbo
- **WHEN** a user asks to ingest `https://go.dev/doc/effective_go` and `command -v bilbo` prints nothing
- **THEN** the agent prints `ingest: bilbo is not on PATH; install the bilbo CLI first`, fetches nothing and writes no file

### Requirement: One plugin for Claude Code and Codex
The repository SHALL ship one plugin named `bilbo` in `plugins/bilbo/`, with a Claude Code manifest at `.claude-plugin/plugin.json` and a Codex manifest at `.codex-plugin/plugin.json` inside it. It SHALL list the plugin in a Claude Code marketplace at `.claude-plugin/marketplace.json` and a Codex marketplace at `.agents/plugins/marketplace.json`, both named `bilbo`, with the source `./plugins/bilbo`. The plugin SHALL hold skills, the digest hook and the compaction hook, and no subagent or MCP server.

#### Scenario: Claude Code installs the plugin
- **WHEN** a user runs `claude plugin marketplace add delucca/bilbo` and then `claude plugin install bilbo@bilbo`
- **THEN** the plugin is installed and its `recall`, `note`, `reference` and `ingest` skills, its digest hook and its compaction hook are available

#### Scenario: Codex installs the plugin
- **WHEN** a user runs `codex plugin marketplace add delucca/bilbo` and then `codex plugin add bilbo@bilbo`
- **THEN** Codex offers the skills `bilbo:recall`, `bilbo:note`, `bilbo:reference` and `bilbo:ingest` and lists the digest hook and the compaction hook for review

#### Scenario: The plugin adds no subagent or MCP server
- **WHEN** the plugin is installed in either tool
- **THEN** it adds no subagent and no MCP server, and no hook but the digest hook and the compaction hook

### Requirement: The digest hook
The plugin SHALL hold `hooks/hooks.json`, which both tools read without a manifest field, registering one UserPromptSubmit hook with no matcher: a command hook whose command is exactly `command -v bilbo >/dev/null 2>&1 || exit 0; bilbo digest; exit 0` and whose `timeout` is 5 seconds. Neither manifest SHALL name the file. The hook SHALL print what `bilbo digest` prints, which each tool adds to the agent's context, and SHALL always exit 0, so it never blocks a prompt. Codex runs the hook only once it is trusted; `bilbo setup` marks it trusted when it installs the plugin in Codex, as the `setup` spec's Codex hook trust says.

#### Scenario: Claude Code injects the digest
- **WHEN** the plugin is installed in Claude Code, `bilbo` is on PATH and a note passes the digest gate for the user's prompt
- **THEN** the agent receives the digest block as context for that prompt

#### Scenario: Codex after bilbo setup
- **WHEN** `bilbo setup` installed the plugin in Codex and trusted its hook, and a note passes the digest gate
- **THEN** the agent receives the digest block as context for that prompt, with no hook review asked first

#### Scenario: Codex without trust
- **WHEN** the plugin was added to Codex by hand and nobody trusted its hook
- **THEN** Codex does not run `bilbo digest` and the prompt goes on

#### Scenario: bilbo is not installed
- **WHEN** `bilbo` is not on the PATH the tool gives its hooks
- **THEN** the hook prints nothing, exits 0, and the tool shows no hook error

#### Scenario: An older bilbo
- **WHEN** the `bilbo` on PATH has no `digest` verb and exits 2 with a usage message
- **THEN** the hook exits 0 and the prompt is not blocked

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

### Requirement: The reference skill
The plugin SHALL hold a skill named `reference` in `skills/reference/SKILL.md`, with the brief its readers get in `skills/reference/references/reader.md`. The frontmatter SHALL have only `name`, `description`, `license` and `allowed-tools`, and `allowed-tools` SHALL cover every command the skill runs. The description SHALL say to use it for a question answered from library sources, and not for notes or for adding a source.

#### Scenario: A question about a source
- **WHEN** a user asks "what does Effective Go say about goroutine leaks?"
- **THEN** the agent runs the `reference` skill

#### Scenario: A question about the notes
- **WHEN** a user asks "what did we decide about the release tags?"
- **THEN** the agent runs `recall`, not `reference`

### Requirement: Reference picks
The reference skill SHALL find corpora only through `bilbo library` (or the corpus the user named), read each chosen guide through `bilbo library <corpus>`, and pick the sources whose entries answer the question, never a catalog whole. Before it plans or reads, it SHALL post the picks as a message of its own: per corpus, `<k> of <n>` sources with `<n>` from `bilbo library`, and one clause per pick.

#### Scenario: Picks come first
- **WHEN** the skill picks `go/effective-go` and `go/errors` from a corpus of 14 sources
- **THEN** a message reading `picks from go (2 of 14 sources):` with one line per pick precedes any `bilbo library plan` call

#### Scenario: A catalog by section
- **WHEN** the question names the lint `needless_return` and `rust/clippy-lints` is a catalog
- **THEN** the skill picks `rust/clippy-lints#needless_return`, and never the whole catalog

#### Scenario: No corpus fits
- **WHEN** no corpus listed by `bilbo library` covers the question
- **THEN** the skill says so, names the corpora, plans and reads nothing, and does not answer from memory

### Requirement: Reference reads
The reference skill SHALL read sources only through `bilbo library plan` and `bilbo library read`, never with another tool on a source file. One partition it reads itself. With the Agent tool and two to six partitions, it spawns one `general-purpose` reader per partition in one turn, given `references/reader.md`, the question, the plan and its slices; above six, it asks the user to narrow the question or accept a sample. Without the Agent tool, it reads slices in order up to 100,000 tokens.

#### Scenario: Three partitions in Claude Code
- **WHEN** the plan has three partitions and the Agent tool is available
- **THEN** the skill spawns three `general-purpose` readers in one turn, and none of them is a subagent the plugin ships

#### Scenario: Codex with a large plan
- **WHEN** the skill runs without the Agent tool and the plan holds 160,000 tokens
- **THEN** it reads the slices in order while the tokens read stay within 100,000, and its answer's coverage line names the rest as not read

#### Scenario: A cut-off slice
- **WHEN** a `bilbo library read` result has no `-- end slice` line, a gap in its line numbers, or a truncation notice
- **THEN** the skill reads that slice again with `--part 1/2` and `--part 2/2`

#### Scenario: Never a direct read
- **WHEN** the skill knows a source's path from `bilbo library show`
- **THEN** it still reads the source only through `bilbo library read`

### Requirement: Reference answer
The reference skill SHALL answer only from what was read, each claim with one `bilbo:` citation. It SHALL run `bilbo cite` with every plan of the run on the whole draft, act on each verdict, and run it again after any change until every verdict is `ok`. The answer SHALL end with the picks, then cite's `citations:`, `coverage:` and `picked:` lines as cite printed them.

#### Scenario: An unread quote
- **WHEN** `bilbo cite --plan <plan>` gives a citation `unread`
- **THEN** the skill reads the slice the detail names and checks again, or drops the claim

#### Scenario: A wrong anchor
- **WHEN** a citation is `quote_elsewhere`
- **THEN** the skill takes the anchor from the detail and checks again

#### Scenario: Coverage is copied, not written
- **WHEN** the skill ends its answer
- **THEN** its `coverage:` lines are those of its last `bilbo cite` run, word for word

### Requirement: Support judgment
After `bilbo cite` passes, the reference skill SHALL judge whether each quote supports its claim as written, and SHALL rewrite a claim the quote supports only in part to what the quote says, and drop a claim it does not support. `references/reader.md` SHALL hold this judgment as a section of its own that a verifier in another skill can paste whole.

#### Scenario: A lost qualifier
- **WHEN** a claim says "Go always reuses the loop variable" and its `ok` quote says the variable is reused "before Go 1.22"
- **THEN** the skill rewrites the claim to say before Go 1.22, or drops it

#### Scenario: An ok citation is not enough
- **WHEN** a quote is on the claim's topic but does not state the claim
- **THEN** the skill drops the claim even though `bilbo cite` gave it `ok`

### Requirement: The reader brief
`references/reader.md` SHALL tell a reader to read only its slices of the plan, only through `bilbo library read`, to check every slice's end marker and line numbers, to make at most ten claims each with a `bilbo:` citation, to run `bilbo cite --plan <plan>` on its claims before it reports, and to end with `read:` and `not read:` lines naming slices. It SHALL forbid searching, other files, writes and spawning.

#### Scenario: A reader's prompt
- **WHEN** the skill spawns `reader-2` for slices 7 to 12 of a plan
- **THEN** the prompt holds the question, the plan, `slices 7-12` and the brief's rules

#### Scenario: A reader that does not report
- **WHEN** a reader returns without a `read:` line
- **THEN** the skill asks it once by name to finish, and if it still does not, says in the answer that its slices were read by a reader that did not report

### Requirement: The ingest skill
The plugin SHALL hold a skill named `ingest` in `skills/ingest/SKILL.md`. Its frontmatter SHALL have only `name`, `description`, `license` and `allowed-tools`, and `allowed-tools` SHALL cover every command the skill runs. The description SHALL say that it adds a web page, a text file or a PDF's text to the bilbo library as a source, and that it is not for notes.

#### Scenario: The user asks to ingest a page
- **WHEN** a user says "ingest https://go.dev/doc/effective_go into the library"
- **THEN** the agent runs the `ingest` skill

#### Scenario: A note is not a source
- **WHEN** a user says "keep this decision about the release tags for later sessions"
- **THEN** the agent does not run the `ingest` skill

### Requirement: Ingest routes
The ingest skill SHALL put text into the library only through `bilbo library stage` and `bilbo library land`. It SHALL stage a URL with `bilbo library stage '<url>'`, a local text file with `--origin`, and a saved HTML page with `--html` and `--origin`. For a PDF it SHALL write the text with `pdftotext -layout` and stage that file with `--origin`. It SHALL never use WebFetch or text it wrote itself, and SHALL never write or edit a source or a staged `capture.md`.

#### Scenario: A web page
- **WHEN** a user asks to ingest `https://go.dev/doc/effective_go`
- **THEN** the agent runs `bilbo library stage 'https://go.dev/doc/effective_go'` and makes no WebFetch call

#### Scenario: A PDF
- **WHEN** `bilbo library stage` refuses `https://example.org/paper.pdf` as a PDF, and `pdftotext` is on PATH
- **THEN** the agent downloads it with `curl`, writes its text with `pdftotext -layout`, stages the text file with `--origin "url: https://example.org/paper.pdf"`, and the landed source has `capture: external`

#### Scenario: A page bilbo cannot fetch
- **WHEN** `bilbo library stage` exits 1 with a `403` for the user's URL
- **THEN** the agent says bilbo could not fetch it, asks the user to save the page with a tool of their own and give its path, and makes no WebFetch call

#### Scenario: No PDF tool
- **WHEN** the source is a PDF and `command -v pdftotext` prints nothing
- **THEN** the agent says that `pdftotext` (poppler) is needed or a text file of the PDF, stages nothing and writes no file

### Requirement: Choosing keep ranges
Before landing, the ingest skill SHALL read the staged `capture.md` with Read around the suggested `keep`'s first and last lines, at each navigation suspect and near each lost heading. It SHALL keep the document's own text, headings, code and tables, and drop navigation, breadcrumbs, in-page tables of contents, footers and cookie notices, cutting only between blocks and never inside a code fence. A stage warning that its ranges cannot resolve SHALL go into its report.

#### Scenario: Navigation before the text
- **WHEN** stage warns `navigation suspect: lines 29-158, 52 lines of links only`, and the document's title is on line 160
- **THEN** the agent's `--keep` starts after line 158

#### Scenario: A defect ranges cannot fix
- **WHEN** stage warns `unclosed fence: the code fence on line 812 is never closed`
- **THEN** the agent leaves `capture.md` as it is, and its report names the warning

### Requirement: Landing a source
The ingest skill SHALL land with `bilbo library land <stage> <corpus>/<name> --keep <ranges> --title '<title>'`. It SHALL pick the corpus from `bilbo library`, an existing one whose guide fits or a new name, and a source name in the topic grammar. When stage prints an `existing:` line or `land` exits 1 naming `--replace`, the skill SHALL land with `--replace` only after the user asked to re-ingest that source or agreed when asked.

#### Scenario: A new source
- **WHEN** stage prints no `existing:` line and the user asked to ingest a Go page
- **THEN** the agent lands it into the `go` corpus with a name such as `effective-go` and an explicit `--title`

#### Scenario: A re-ingest the user asked for
- **WHEN** the user asks to refresh `go/effective-go` and stage prints `existing: go/effective-go`
- **THEN** the agent lands with `--replace` onto `go/effective-go`

#### Scenario: A source that already exists
- **WHEN** stage prints `existing: go/effective-go` and the user only asked to ingest the URL
- **THEN** the agent asks the user whether to replace `go/effective-go` before running `land`, and runs no `land --replace` without a yes

### Requirement: The guide entry after ingest
After a land, the ingest skill SHALL read the source and write its guide entry with Edit in the `guide.md` that `land` printed. It reads the outline with `bilbo library show <corpus>/<name>`, and the body only through `bilbo library plan <corpus>/<name>` and `bilbo library read`, up to 60,000 tokens; Read is for `capture.md` and the `guide.md` it edits, never for a source. The entry is two or three sentences of the agent's own words, based on what was read, on what the source covers, when to consult it, and what it gets wrong or leaves out, replacing the `TODO` or stale line. A new corpus's lead replaces its `TODO` line. The skill SHALL then run `bilbo check` as the scenarios say.

#### Scenario: A new entry
- **WHEN** `land` added `## effective-go` with `TODO: describe this source.`
- **THEN** the agent replaces that line with its sentences, and `bilbo check` prints no line for `library/go/`

#### Scenario: The body is read through a plan
- **WHEN** `bilbo library show go/effective-go` prints `tokens: 38485`
- **THEN** the agent runs `bilbo library plan go/effective-go` and `bilbo library read` for every slice of the plan, and makes no Read call on `effective-go.md`

#### Scenario: A large source
- **WHEN** `bilbo library show rust/clippy-lints` prints `catalog: yes` and over 60,000 tokens
- **THEN** the agent reads slices up to 60,000 tokens, picking sections by anchor for the catalog, says in the entry that it is a reference to look things up in, and reads no slice past that budget

#### Scenario: A re-ingest
- **WHEN** `land --replace` put the stale line under `## effective-go`
- **THEN** the agent revises the prose after reading the new body, removes the stale line, and `bilbo check` prints no line for `library/go/`

#### Scenario: Problems elsewhere
- **WHEN** `bilbo check` prints lines only for notes or for another corpus
- **THEN** the agent changes none of those files and says how many lines it left

### Requirement: Report the ingest
When it is done, the ingest skill SHALL give the source's absolute path, its id, its `<corpus>/<name>`, whether it is fetched or `capture: external`, its kept ranges, and the stage warnings it left unresolved. When it stopped before `land` succeeded, it SHALL not say a source was added.

#### Scenario: A fetched source
- **WHEN** the agent landed `go/effective-go` from a URL stage and `bilbo check` passed for it
- **THEN** its report names `<root>/library/go/effective-go.md`, the id `land` printed, `go/effective-go`, that bilbo fetched it, and the `--keep` ranges

#### Scenario: Stopped early
- **WHEN** `bilbo library stage` refused the URL and the user gave no file
- **THEN** the agent does not say a source was added
