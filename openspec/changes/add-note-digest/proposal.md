# Proposal

## Why

`recall` only helps when an agent thinks to run it, and agents mostly don't: they re-derive what a note already says. The legacy system's answer is a digest, a short list of relevant notes that a prompt hook injects before the agent reads the prompt. It works, but the legacy digest has known problems, recorded on 2026-09-27:

- Of six digests in one session, three were fully off-topic and one was mixed.
- Long prompts missed the 1.5 s budget and got no digest.
- One empty digest was never explained, because nothing logs what the digest saw.

The off-topic share shows again in this very session's digests. bilbo needs its own digest, on the ranking it already has, with those failures designed out.

## What Changes

- `bilbo digest`: the command a UserPromptSubmit hook runs, for Claude Code and Codex alike. It reads the hook's JSON from stdin (`session_id`, `prompt`) and prints a digest block to stdout, or prints nothing.
- The digest lists at most 6 notes on a session's first prompt and at most 3 on later ones. A note is shown once per session. The block stays under 12,000 bytes, and every line points at a file and line the agent can open.
- A stricter gate than `recall`:
  - With an embedder, a note enters only when its best passage reaches `digest.min_similarity` (0.55 by default). Keyword overlap alone never admits a note, since that is what let generic follow-ups ("ok, can you do the 3 specs?") pull in unrelated notes.
  - Without an embedder, or when the embedder misses the budget, a passage must share at least 3 distinct query words of 4 or more letters.
- It never fails loudly. Every run exits 0. On any error, a missed time budget or a gate nobody passes, stdout is empty.
- It is fast enough for every prompt. It holds a 1.5 s budget, giving the query embed what is left after reading the store, and embeds only the first 1,000 bytes of the prompt.
- An opt-in digest log (`digest.log = on`) under `~/.local/state/bilbo/` records each run: when, the session, the start of the prompt, which ranking was used, how many notes passed the gate, what was shown, the time taken and any error. An empty digest can then be explained after the fact.
- A prompt that is only a command (`/opsx:apply`, `$skill`) is searched as its name and arguments without the sigil. A prompt whose first word starts with `/` but isn't a command name, such as a path, gets no digest.

## Capabilities

### New Capabilities

- `note-digest`: `bilbo digest`, its hook input, gate, limits, session memory, output block, time budget and log.

### Modified Capabilities

- `cli`: Verb dispatch adds `digest`. Exit codes says `digest` always exits 0.
- `config`: a new requirement adds the `digest.min_similarity` and `digest.log` keys.

## Non-goals

- Installing the hook into Claude Code or Codex settings. The skills-and-hooks delivery change does that. The maintainer's dnix wires it by hand until then.
- Scopes and the scope leak the legacy digest had. bilbo has no scopes yet, and a store holds one person's notes.
- Sources and library entries in the digest.
- A daemon that keeps the store and cache in memory between prompts. The budget is met by reading them per prompt, as `recall` does.
- Tuning the gate from the log automatically. The log is for a human or an agent to read.
- A PreCompact hook nudging the agent to write notes. That belongs with the skills.

## Impact

- New module `src/digest.rs` (the verb). It builds on `config`, `embed`, `vectors`, `rank`, `store` and `note`, like `recall`, never on `recall` itself.
- `store::Env` gains `xdg_state_home`. `src/config.rs` gains the digest keys. Session memory lives under the cache folder, the log under the state folder.
- `serde_json`, already in through `ureq`, parses the hook input and writes log lines. It becomes a direct dependency with the same version.
- `tests/digest.rs` is new, using the fake embedder from add-embeddings.
- Order: applies after add-embeddings is archived, since its `config` delta and its gate depend on that change.
