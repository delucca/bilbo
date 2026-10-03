# Proposal

## Why

The bilbo plugin lets an agent find notes (`recall`, the digest) but nothing tells it when to write one, how to start it with `bilbo new`, or to run `bilbo check` after editing. Agents fall back on the legacy dnix `note` skill, which writes into notebooks with keys bilbo's `note-store` spec forbids (`kind`, `supersedes`), so bilbo's store stays empty and the digest has nothing to show.

The legacy setup also nudges the agent before compaction with a PreCompact hook. A probe on 2026-10-03 (`probe.md`) showed that nudge never reaches the model on an auto-compaction, in Claude Code or in Codex. Only a SessionStart hook on the `compact` source does, in both.

## What Changes

- A `note` skill in the plugin (`plugins/bilbo/skills/note/SKILL.md`, offered as `bilbo:note`). Its description leads with when to write a note: the user asks to keep something for later sessions, or the session settled something durable such as a decision, a gotcha or a plan. It does not fire on a passing "note that…" in conversation.
- The skill's flow:
  - It stops with `note: bilbo is not on PATH; install the bilbo CLI first` when `bilbo` is missing, as `recall` does.
  - It runs `bilbo recall` to find a note on the same subject, because `bilbo new` only catches an exact topic.
  - It updates that note in place, or runs `bilbo new <kind> <topic> [--title]` and writes the body into the file `new` prints.
  - It runs `bilbo check` after every create or edit and fixes every line that names the note it touched.
  - It acts on each exit code of `recall`, `new` and `check` from a table.
  - It reports the note's path.
- A note's kind can change: the skill renames `<kind>-<topic>.md` to the new kind with `mv -n`, keeping `id` and `created`, then runs `bilbo check`.
- `sources` lists only what was read or run in the session, in the `note-store` format. The skill never invents one.
- A compaction hook in `plugins/bilbo/hooks/hooks.json`: a SessionStart hook with the matcher `compact`. When `bilbo` is on PATH, it prints one line asking the agent to save what the session settled with the note skill. Without `bilbo` it prints nothing. It always exits 0.
- `bilbo setup` already trusts every bilbo hook in Codex, so it trusts the new one too. A re-run after an upgrade trusts only the new hook. The wizard's summary says "trust its hooks" instead of "trust its digest hook".
- The manifests and the Claude Code marketplace entry describe writing notes as well as recalling them. Codex's `interface` gains the `Write` capability and a second default prompt.

## Capabilities

### New Capabilities

None. The skill and the hook belong to the plugin, which `agent-plugin` already covers.

### Modified Capabilities

- `agent-plugin`:
  - One plugin for Claude Code and Codex: the plugin holds the `recall` and `note` skills, the digest hook and the compaction hook.
  - A missing binary: the rule now covers both skills, each with its own line.
  - New requirements: The note skill, Changing a note's kind, and The compaction hook.
- `setup`: Codex hook trust gains a scenario for an upgrade that adds a hook. The requirement's text already says "every untrusted or changed bilbo hook".

## Non-goals

- Changing or removing the legacy dnix `note` skill or its PreCompact hook. Both stay until a later cutover. Two note skills side by side is accepted.
- Renaming `recall` or `note`.
- Notebooks, `~/Notebooks`, `--notebook`, Org links, `mint-ulid.py`, `date +%F`, `argument-hint`, the `kind` and `supersedes` keys and the supersede step. bilbo has one store, and `bilbo new` mints the id and the timestamp.
- A `bilbo` verb that edits a note or renames it. Agents edit with their own file tools, as `note-create` intends.
- A PreCompact or PostCompact hook, and blocking compaction. Neither tool shows their output to the model on an auto-compaction (see design.md).
- A Stop or SessionEnd hook that asks for a note at the end of every session.
- Writing sources into `library/`. The library has no verb yet.

## Impact

- `plugins/bilbo/skills/note/SKILL.md` (new).
- `plugins/bilbo/hooks/hooks.json` gains the SessionStart entry.
- `plugins/bilbo/.claude-plugin/plugin.json`, `plugins/bilbo/.codex-plugin/plugin.json` and `.claude-plugin/marketplace.json` get new descriptions. Codex's `interface` gets new capabilities and prompts.
- `tests/plugin.rs`: tests for the note skill, a check that `allowed-tools` covers every command in both skills, the compaction hook's tests, and the digest test updated now that `hooks.json` has two events.
- `src/agents.rs`: unit tests for trusting two hooks, on a new fixture `tests/fixtures/agents/codex-app-server-list-two-hooks.txt` recorded from Codex 0.155.1. The trust code needs no change.
- `src/setup.rs`: the wizard summary's wording and its unit test.
- `README.md` and `AGENTS.md`.
- No new dependency and no new verb.
- Migration: on rivendell the legacy PreCompact hook comes from dnix's managed settings, and the legacy `note` skill from dnix. Both keep running beside the plugin's.
