# Spec Delta

## REMOVED Requirements

### Requirement: One plugin for both tools
**Reason**: Its scenario "The plugin adds only skills" no longer holds once the plugin carries the digest hook, and a MODIFIED block cannot drop a scenario.
**Migration**: "One plugin for Claude Code and Codex" below replaces it, with the same layout and the hook allowed.

## ADDED Requirements

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
