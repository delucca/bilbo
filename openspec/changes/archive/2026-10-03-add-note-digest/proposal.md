# Proposal

## Why

`recall` only helps when an agent thinks to run it, and agents mostly don't: they re-derive what a note already says. The legacy system's answer is a digest, a short list of relevant notes that a prompt hook injects before the agent reads the prompt. It works, but the legacy digest has known problems, recorded on 2026-09-27:

- Of six digests in one session, three were fully off-topic and one was mixed.
- Long prompts missed the 1.5 s budget and got no digest.
- One empty digest was never explained, because nothing logs what the digest saw.

The off-topic share shows again in this very session's digests. bilbo needs its own digest, on the ranking it already has, with those failures designed out, and delivered the way bilbo already reaches agents: through its plugin.

## What Changes

- `bilbo digest`: the command a UserPromptSubmit hook runs, for Claude Code and Codex alike. It reads the hook's JSON from stdin (`session_id`, `prompt`) and prints a digest block to stdout, or prints nothing.
- The plugin gains that hook. `plugins/bilbo/hooks/hooks.json` registers one UserPromptSubmit command hook, which both tools find without a manifest field. `bilbo setup` already installs the plugin, so the digest arrives with it. The command is guarded: a missing `bilbo` prints nothing, and the hook always exits 0, since both tools block the prompt on exit 2.
- Codex runs a plugin hook only once it is trusted. At the user's request (2026-10-03), `bilbo setup` marks the bilbo hook trusted when it installs the plugin in Codex, through Codex's own app-server, and reports it on a new `hook` line. `--remove` deletes that trust.
- The digest lists at most 6 notes on a session's first prompt and at most 3 on later ones. A note is shown once per session. The block stays within 9,000 bytes, under the 10,000 characters past which both tools move hook output to a file, and every line points at a file and line the agent can open.
- A stricter gate than `recall`:
  - With an embedder, a note enters only when its best passage reaches `digest.min_similarity` (0.55 by default). Keyword overlap alone never admits a note, since that is what let generic follow-ups ("ok, can you do the 3 specs?") pull in unrelated notes.
  - Without an embedder, or when the embedder misses the budget, a passage must share at least 3 distinct query words of 4 or more letters or digits.
- It never fails loudly. Every run exits 0. On an error that stops the digest, such as bad input, no store or an unwritable cache folder, stdout is empty and stderr holds one `bilbo: ` line.
- It is fast enough for every prompt. It holds a 1.5 s budget, giving the query embed what is left after reading the store, and embeds only the first 1,000 bytes of the prompt.
- `digest.enable = off` turns the digest off without disabling the plugin, which Claude Code would need, since it cannot disable one plugin hook.
- An opt-in digest log (`digest.log = on`) under `~/.local/state/bilbo/` records each run: when, the session, the start of the prompt, which ranking was used, how many notes passed the gate, what was shown, the time taken and any error. An empty digest can then be explained after the fact.
- A prompt that is only a command (`/opsx:apply`, `$skill`) is searched as its name and arguments without the sigil. A prompt whose first word starts with `/` but isn't a command name, such as a path, gets no digest.
- `bilbo setup` keeps the digest settings when it rewrites a config, and the home-manager module accepts them.

## Capabilities

### New Capabilities

- `note-digest`: `bilbo digest`, its hook input, gate, limits, session memory, output block, time budget and log.

### Modified Capabilities

- `cli`: Verb dispatch adds `digest`. Exit codes says `digest` always exits 0.
- `config`: Config location adds `digest` to the verbs that read settings. A new requirement adds the `digest.enable`, `digest.min_similarity` and `digest.log` keys.
- `agent-plugin`: the plugin holds skills and the digest hook, no longer skills only. A new requirement defines the hook.
- `setup`: Existing config file keeps the digest settings on a rewrite. Home-manager module accepts the digest keys in `settings`. Step report and Remove gain a `hook` line after `codex`. A new requirement, Codex hook trust, trusts the plugin's hook in Codex.

## Non-goals

- Editing Codex's files directly. The trust goes through Codex's app-server, which owns its config format.
- A flag to install the Codex plugin without trusting its hook. The user asked for the trust; a user can still disable the hook in Codex's hooks browser, which writes `enabled = false` next to the trust.
- Scopes and the scope leak the legacy digest had. bilbo has no scopes yet, and a store holds one person's notes.
- Sources and library entries in the digest.
- A daemon that keeps the store and cache in memory between prompts. The budget is met by reading them per prompt, as `recall` does.
- Tuning the gate from the log automatically. The log is for a human or an agent to read.
- A PreCompact hook nudging the agent to write notes. That belongs with the note skill.

## Impact

- New verb `src/digest.rs`. It builds on `config`, `embed`, `vectors`, `rank` and `store`, like `recall`, never on `recall` itself. The steps both verbs need move out of `recall.rs`: `vectors::lookup`, `rank::meaning`, `rank::snippet` and `embed::query`. `rank::shared` is new.
- `src/command.rs` gains a JSON-lines conversation with a child process, `src/agents.rs` the app-server requests and their parsing, and `src/setup.rs` the `hook` step; the fake `codex` in `tests/common/fakes.rs` learns `app-server`.
- `src/config.rs` gains the digest keys, `src/setup.rs` keeps them on a rewrite, and `flake.nix`'s module accepts them. Session memory lives under the cache folder, the log under the state folder (`store::state_dir`, already there).
- `plugins/bilbo/hooks/hooks.json` is new. `tests/plugin.rs` checks it and that no manifest names it.
- `tests/digest.rs` is new, on the fake embedder, which gains a delay switch. The 6 MiB generated store moves from `tests/recall.rs` to `tests/common/` so both speed tests share it.
- No new dependency: `serde_json` is already a direct one.
- Migration: on rivendell the legacy digest hook comes from managed settings. Once the plugin carries `bilbo digest`, both run until dnix drops the legacy hook.
