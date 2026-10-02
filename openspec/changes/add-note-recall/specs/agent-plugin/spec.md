# Spec Delta

## Purpose

How bilbo reaches agents: one plugin for Claude Code and Codex, installed from this repository, whose skills drive the `bilbo` CLI.

## ADDED Requirements

### Requirement: One plugin for both tools
The repository SHALL ship one plugin named `bilbo` in `plugins/bilbo/`, with a Claude Code manifest at `.claude-plugin/plugin.json` and a Codex manifest at `.codex-plugin/plugin.json` inside it. It SHALL list the plugin in a Claude Code marketplace at `.claude-plugin/marketplace.json` and a Codex marketplace at `.agents/plugins/marketplace.json`, both named `bilbo`, with the source `./plugins/bilbo`. The plugin SHALL hold skills only.

#### Scenario: Claude Code installs the plugin
- **WHEN** a user runs `claude plugin marketplace add delucca/bilbo` and then `claude plugin install bilbo@bilbo`
- **THEN** the plugin is installed and its `recall` skill is available

#### Scenario: Codex installs the plugin
- **WHEN** a user runs `codex plugin marketplace add delucca/bilbo` and then `codex plugin add bilbo@bilbo`
- **THEN** Codex offers the skill `bilbo:recall`

#### Scenario: The plugin adds only skills
- **WHEN** the plugin is installed in either tool
- **THEN** it adds no hook, subagent or MCP server

### Requirement: Plugin versions
The Claude Code manifest and its marketplace entry SHALL NOT set a `version`. The Codex manifest's `version` SHALL equal the `version` in `Cargo.toml`.

#### Scenario: Claude Code follows commits
- **WHEN** a commit changes the plugin on `main` and a user updates the plugin
- **THEN** Claude Code installs that commit, with no version bump

#### Scenario: A version bump that skips the plugin
- **WHEN** `Cargo.toml`'s `version` changes and `.codex-plugin/plugin.json` keeps the old one
- **THEN** `cargo test` fails

### Requirement: The recall skill
The plugin SHALL hold a skill named `recall` in `skills/recall/SKILL.md`, whose frontmatter has only `name`, `description`, `license` and `allowed-tools`. The skill SHALL search notes only through `bilbo recall`, with the user's words after `--`, and SHALL act on the exit code: on 0 it shows the hits and says which query produced them; on 1 with `bilbo: no notes match` it retries at most twice in the note's likely wording, then says nothing matched; on any other 1, or on 2, it shows bilbo's first stderr line and stops.

#### Scenario: Hits are shown
- **WHEN** a user asks to recall `flat layout` and `bilbo recall` exits 0
- **THEN** the agent says the hits came from `flat layout`, shows each hit's path, line, kind, created, heading path and snippet, and offers to open one

#### Scenario: Nothing matches
- **WHEN** `bilbo recall` exits 1 with `bilbo: no notes match` for the user's words and for two reworded queries
- **THEN** the agent says nothing matched and names the queries it tried

#### Scenario: A usage error
- **WHEN** `bilbo recall` exits 2
- **THEN** the agent shows bilbo's first stderr line and runs no other query

### Requirement: A missing binary
When `bilbo` is not on PATH, the recall skill SHALL stop with the line `recall: bilbo is not on PATH; install the bilbo CLI first` and SHALL NOT search the notes any other way.

#### Scenario: bilbo is not installed
- **WHEN** `command -v bilbo` prints nothing
- **THEN** the agent prints that line and stops, without reading or grepping any note
