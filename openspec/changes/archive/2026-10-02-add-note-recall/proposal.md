# Proposal

## Why

Agents can write notes with `bilbo new`, but nothing in bilbo finds a note again, so a note written today is lost to every later session unless the agent already knows its filename. The legacy notes system answers that with `nbrecall`, which needs a running embedder and a SQLite index. bilbo needs a way to read notes back before the skills and hooks can move onto it, and the smallest useful one is keyword search: no embedder and no index to keep fresh.

Keyword search alone is a real but partial answer. On the legacy blind query set, run against the 424 legacy notes on 2026-10-02, keyword ranking put the right note in the top 5 for 23 of 37 queries, against 29 of 37 for the legacy keyword-plus-embeddings search. It matched on queries that reuse the note's own words and fell behind on paraphrases (2 of 9 against 5 of 9) and on queries in another language than the note (3 of 7 against 5 of 7). The add-embeddings change closes that gap. This change ships the half that works with nothing else installed. Agents reach the verb through a skill, so this change also ships the `recall` skill, packaged as a plugin for Claude Code and Codex.

## What Changes

- `bilbo recall <query>...`: ranks the notes in the store against the query words and prints the best matches, best first. Each hit gives the file path and line of the best-matching passage, the note's kind and `created`, the heading path of that passage, and a one-line snippet.
- `--kind <kind>` (repeatable) narrows the hits to those kinds. `--limit <n>` caps them, 10 by default.
- Matching ignores case and Latin accents, so `decisao` finds `decisão`. A note is one hit at most, at its best passage.
- No hits exits 1, so an agent can tell "nothing matched" from a result without parsing.
- No index, no cache and no new dependency: `recall` reads the store on every run. At the size of the legacy store (424 notes, 5.6 MiB) that stays well inside a second.
- A `bilbo` plugin for Claude Code and Codex in `plugins/bilbo/`, listed in a marketplace file for each tool at the repo root, so an agent's user installs it with one command. Its `recall` skill runs `bilbo recall` with the user's words, retries in the note's likely wording when nothing matches (there is no stemming), and stops with a clear line when `bilbo` is not installed.

## Capabilities

### New Capabilities

- `note-recall`: `bilbo recall`, which ranks the notes in the store against a query and prints the best passages.
- `agent-plugin`: the `bilbo` plugin for Claude Code and Codex, its marketplaces, and the `recall` skill that drives `bilbo recall`.

### Modified Capabilities

- `cli`: the Verb dispatch requirement adds `recall` to the verbs. The Exit codes requirement adds "finds nothing" to what exit 1 means, for `recall`.

## Non-goals

- Embeddings, meaning-based ranking and the index. The add-embeddings change adds them, without changing what `recall` prints.
- The digest and the prompt hook (the add-note-digest change).
- Searching sources or the library. The library change does not exist yet.
- Stemming or synonyms: `notes` does not match `note`. Embeddings cover that ground.
- JSON output. The skills read the text form, and nothing else consumes hits yet.
- Hooks, subagents and MCP servers in the plugin. The digest's prompt hook joins it in add-note-digest.
- Installing the binary. Neither plugin system installs one; the skill says so when `bilbo` is missing.

## Impact

- New modules: `src/recall.rs` (the verb) and `src/rank.rs` (passages, tokens and ranking, which the index and the digest reuse later). `src/note.rs` returns the `created` value and where the body starts, alongside the problems it already returns.
- `src/main.rs` gains the dispatch arm and USAGE line. `tests/recall.rs` is new. `AGENTS.md`'s Architecture section lists the new modules and the new test file.
- No new dependency. `jiff` stays the only one.
- New top-level `plugins/` and `.claude-plugin/`, and `.agents/plugins/marketplace.json`. `tests/plugin.rs` checks the plugin files and keeps the Codex plugin's version equal to `Cargo.toml`'s. `AGENTS.md` describes the layout.
