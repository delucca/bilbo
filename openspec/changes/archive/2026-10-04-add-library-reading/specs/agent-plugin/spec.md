# Spec Delta

## MODIFIED Requirements

### Requirement: One plugin for Claude Code and Codex
The repository SHALL ship one plugin named `bilbo` in `plugins/bilbo/`, with a Claude Code manifest at `.claude-plugin/plugin.json` and a Codex manifest at `.codex-plugin/plugin.json` inside it. It SHALL list the plugin in a Claude Code marketplace at `.claude-plugin/marketplace.json` and a Codex marketplace at `.agents/plugins/marketplace.json`, both named `bilbo`, with the source `./plugins/bilbo`. The plugin SHALL hold skills, the digest hook and the compaction hook, and no subagent or MCP server.

#### Scenario: Claude Code installs the plugin
- **WHEN** a user runs `claude plugin marketplace add delucca/bilbo` and then `claude plugin install bilbo@bilbo`
- **THEN** the plugin is installed and its `recall`, `note` and `reference` skills, its digest hook and its compaction hook are available

#### Scenario: Codex installs the plugin
- **WHEN** a user runs `codex plugin marketplace add delucca/bilbo` and then `codex plugin add bilbo@bilbo`
- **THEN** Codex offers the skills `bilbo:recall`, `bilbo:note` and `bilbo:reference` and lists the digest hook and the compaction hook for review

#### Scenario: The plugin adds no subagent or MCP server
- **WHEN** the plugin is installed in either tool
- **THEN** it adds no subagent and no MCP server, and no hook but the digest hook and the compaction hook

### Requirement: A missing binary
When `bilbo` is not on PATH, the recall skill SHALL stop with the line `recall: bilbo is not on PATH; install the bilbo CLI first`, the note skill with the line `note: bilbo is not on PATH; install the bilbo CLI first`, and the reference skill with the line `reference: bilbo is not on PATH; install the bilbo CLI first`. None of them SHALL search, read or write the notes or the sources any other way.

#### Scenario: bilbo is not installed
- **WHEN** `command -v bilbo` prints nothing
- **THEN** the agent prints that line and stops, without reading or grepping any note

#### Scenario: The note skill without bilbo
- **WHEN** a user asks to keep a decision for later sessions and `command -v bilbo` prints nothing
- **THEN** the agent prints `note: bilbo is not on PATH; install the bilbo CLI first` and writes no file

#### Scenario: The reference skill without bilbo
- **WHEN** a user asks what Effective Go says about goroutines and `command -v bilbo` prints nothing
- **THEN** the agent prints `reference: bilbo is not on PATH; install the bilbo CLI first`, reads no source file, and does not answer from memory

## ADDED Requirements

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
