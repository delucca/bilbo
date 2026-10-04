# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

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
