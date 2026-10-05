# Proposal

## Why

Remote sync (the planning notebook's `design-bilbo-remote-sync.md`, "Decisions taken (2026-10-03)") must know which notes may leave a device. One store mixes notes from different worlds: personal projects, an employer's code, a client's systems. Today a note carries no such mark, so sync could not tell them apart, and the configured embedder already receives the text of every note, whatever it came from. This change gives each note a scope, and gives each scope a policy, before any sync exists. It is useful on its own: a scope can already keep its notes' text away from a remote embedder.

## What Changes

- A note may carry one new frontmatter key, `scope: <name>`, with the topic's grammar. It is optional and freely editable. The store stays flat: the scope never changes a note's path.
- Scopes are declared in the config, one group of keys per name: `scope.<name>.sync` (only `off` in this change), `scope.<name>.embedder` (`any` or `local`), `scope.<name>.paths` and `scope.<name>.marks`. `scope.default` optionally names the scope new notes get. Any `scope.<name>.*` key declares the scope.
- A note with no `scope` key, or whose key names a scope this device does not declare, is unassigned. An unassigned note gets the strictest embedder rule any declared scope asks for.
- `bilbo new <kind> <topic> --scope <name>` writes the key. Without the flag, `new` takes the scope whose `paths` holds its own working directory (the longest match), else `scope.default`, else it creates the note unassigned and prints one stderr line listing the scopes. It never refuses to create a note for want of a scope.
- `bilbo check` reads the config. Once any scope is declared, a note without a scope is a problem. A note naming an undeclared scope is always a problem. A note holding a mark of another scope (`scope.<name>.marks`: path prefixes and words found in its sources or body) gets a warning line, which does not change the exit code.
- New verb `bilbo scope`: lists the declared scopes with their policy and note counts, then the unassigned count. `bilbo scope set <name> <file>...` adds a missing `scope` key; `--force` replaces one that is there.
- `bilbo index` withholds from a non-loopback embedder the passages of notes whose rule is `local`. `bilbo recall` ranks them by keywords and does not count them as "not indexed". The digest admits them on its keyword gate, so they never drop out of it.
- The note skill learns the scope names through `bilbo scope`, passes `--scope` when the user named one, asks the user once when the note ends up unassigned or the scope looks wrong, keeps `scope:` as found when it edits a note, and reports the note's scope or `unassigned`.
- `bilbo setup` keeps the scope keys when it rewrites the config, and the home-manager module accepts them in `settings`.

## Capabilities

### New Capabilities

- `note-scope`: what a scope means on a device (declared, unassigned, the embedder rule) and the `bilbo scope` verb (list, `set`).

### Modified Capabilities

- `note-store`: Frontmatter shape admits `scope`. A new requirement, Scope key, fixes its form.
- `note-create`: Create a note, New note content and Reject invalid arguments gain `--scope`. A new requirement, Scope of a new note, gives the resolution order.
- `store-check`: Report problems gains warning lines, which leave the exit code alone. New requirements: Scope problems, Scope marks and Matching a mark.
- `config`: Config location adds `new`, `check` and `scope` to the verbs that read settings, so its "Check ignores the config" scenario becomes "Check reads the config". New requirements: Scope settings and Scope paths and marks.
- `note-index`: Index the store excludes withheld passages. A new requirement, Withheld passages, says which.
- `note-recall`: Keyword fallback leaves withheld passages out of the "not indexed" count.
- `note-digest`: The gate admits a withheld passage on the keyword gate while the embedder answers.
- `cli`: Verb dispatch adds `scope`.
- `setup`: Existing config file keeps the scope settings on a rewrite. Home-manager module accepts the scope keys.
- `agent-plugin`: Creating a note, Note content, Check after every write and Report the note gain the scope steps. A new requirement, Choosing a scope, says how the skill picks one.

## Non-goals

- Sync. `scope.<name>.sync` accepts only `off` here. `add-device-keys` widens it to URLs, and `add-sync` acts on it.
- A scope clash when two devices set different scopes on one note. It needs a merge, so it belongs to `add-sync`.
- Holding anything back on a mark. A mark only warns, here and later in sync.
- Recall isolation. `recall` and the digest show notes of every scope.
- Scope semantics in the digest. It reads no `cwd`, sets no scope, adds no line and shows every scope. Its one change is the keyword gate for withheld passages.
- Keeping the recall query or the digest prompt from a remote embedder. They are the agent's words, not note text.
- Scope folders, a list of scopes per note, a boolean `share:` key, or an employer or tenant concept.
- A setup step or wizard question for scopes. A user declares scopes by editing the config.
- Assigning existing notes automatically. They stay unassigned until a user or an agent sets their scope.

## Impact

- `src/shared/text.rs` takes recall's word rule (`words` and its folding) from `src/search/rank.rs`, unchanged, because the config check and the mark finder need it and neither may use the search domain.
- `src/note/mod.rs` reads and validates the `scope` key, and renders it in a new note. A new module, `src/note/marks.rs`, finds marks in a note's topic and text.
- `src/shared/config.rs` parses the `scope.*` keys, which are a pattern rather than fixed names, resolves a working directory against `paths`, and gives each note its embedder rule.
- New verb `src/note/scope.rs`, its `pub mod` line, its path in `VERBS` in `tests/layout.rs`, its dispatch arm and USAGE line. `scope set` writes through `host::swap::exchange`, and through `history/lock`, the `.bilbo-restore-<id>` sweep and the hidden-file writer in `src/note/versions.rs` (the writer moves there from `src/note/restore.rs`, so a verb uses no other verb's module).
- `src/note/new.rs` and `src/check.rs` start reading the config. A broken config now stops them with exit 2. `check` returns its warnings apart from its problems, and `main` takes the exit code from the problems.
- `src/search/index.rs`, `src/search/recall.rs` and `src/search/digest.rs` apply the embedder rule through one function in `src/search/vectors.rs`. `documents::Stored` carries each note's `scope` value.
- `src/setup/` keeps the scope lines on a config rewrite (`facts.rs`, `plan.rs`, `apply.rs`). `flake.nix`'s module accepts `scope.*` keys in `settings`.
- `plugins/bilbo/skills/note/SKILL.md` gains the scope steps and `Bash(bilbo scope)` and `Bash(bilbo scope set *)` in `allowed-tools`.
- Tests: `tests/scope.rs` for the new verb, and new cases in `tests/new.rs`, `tests/check.rs`, `tests/index.rs`, `tests/recall.rs`, `tests/digest.rs`, `tests/cli.rs`, `tests/plugin.rs`, `src/shared/config.rs`, `src/note/` and `src/setup/driven.rs`. Remote-embedder tests reach the loopback fake through `http://0.0.0.0:<port>`.
- No new dependency.
- Migration: none. With no `scope.*` key in the config, every verb behaves as before, except that `new` and `check` now fail on a broken config.
