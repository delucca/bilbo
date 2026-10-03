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
When `bilbo` is not on PATH, the recall skill SHALL stop with the line `recall: bilbo is not on PATH; install the bilbo CLI first` and SHALL NOT search the notes any other way.

#### Scenario: bilbo is not installed
- **WHEN** `command -v bilbo` prints nothing
- **THEN** the agent prints that line and stops, without reading or grepping any note

### Requirement: One plugin for Claude Code and Codex
The repository SHALL ship one plugin named `bilbo` in `plugins/bilbo/`, with a Claude Code manifest at `.claude-plugin/plugin.json` and a Codex manifest at `.codex-plugin/plugin.json` inside it. It SHALL list the plugin in a Claude Code marketplace at `.claude-plugin/marketplace.json` and a Codex marketplace at `.agents/plugins/marketplace.json`, both named `bilbo`, with the source `./plugins/bilbo`. The plugin SHALL hold skills and the digest hook, and no subagent or MCP server.

#### Scenario: Claude Code installs the plugin
- **WHEN** a user runs `claude plugin marketplace add delucca/bilbo` and then `claude plugin install bilbo@bilbo`
- **THEN** the plugin is installed and its `recall` skill and digest hook are available

#### Scenario: Codex installs the plugin
- **WHEN** a user runs `codex plugin marketplace add delucca/bilbo` and then `codex plugin add bilbo@bilbo`
- **THEN** Codex offers the skill `bilbo:recall` and lists the digest hook for review

#### Scenario: The plugin adds no subagent or MCP server
- **WHEN** the plugin is installed in either tool
- **THEN** it adds no subagent and no MCP server, and no hook but the digest hook

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
