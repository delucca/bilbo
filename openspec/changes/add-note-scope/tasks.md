# Tasks

Run every command below from the repo root as `nix develop -c sh -c '<command>'`, with `CARGO_TARGET_DIR` set to the checkout's `target`. A new file under `src/` or `tests/` needs `git add -N` before any Nix command can see it. This change builds on `add-note-history`: `src/swap.rs`, `src/versions.rs` (`history/lock` and the `.bilbo-restore-<id>` sweep) and `bilbo watch` must exist first.

## 1. Library modules

- [ ] 1.1 In `src/note.rs`, accept `scope` as a frontmatter key (at most once, the topic grammar, problems naming `scope`), return its raw value from `note::read`, and update the unexpected-line message to list `scope: `. Add a function that finds, for each of a set of marks, the first place holding it: the topic of the file name, then the `sources` items and body by line (fenced code included, the `scope:` line skipped). Word marks are compared through `rank::words`; path marks match in `~/` and absolute form up to a segment boundary. Add the raw `scope` value to `store::Stored` in `src/store.rs`, read in `store::read_notes`. Unit-test the `note-store` delta's scenarios and the `store-check` delta's Matching a mark scenarios, plus a mark found only in the topic. Verify with `cargo test --locked --bin bilbo note:: && cargo test --locked --bin bilbo store::`
- [ ] 1.2 In `src/config.rs`, parse `scope.<name>.sync|embedder|paths|marks` and `scope.default` as a key pattern beside `KEYS`. Check: name grammar, `default` reserved, `sync = off` only, `embedder = any|local`, comma lists, paths that are `~/`, `/`, absolute or start with `~/`, marks that are one word or a path, `scope.default` declared. Reject two scopes naming one folder, comparing after `~/` expansion and link resolution, and one mark in two scopes. Keep the scope lines as written in `Settings.scope_lines`, in file order. Add the declared scopes, the embedder rule of a `scope` value, and the resolution of a working directory against `paths`: whole folder names, links resolved, the longest wins, and a tie matches nothing. Update the unknown-key message to name the scope keys, and extend the pinned key lists in `tests/index.rs` and `tests/recall.rs`. Unit-test the `config` delta's Scope settings and Scope paths and marks scenarios, a runtime tie through a link made after load, and the `note-scope` delta's Embedder rule scenarios. Verify with `cargo test --locked --bin bilbo config:: && cargo test --locked --test recall --test index`

## 2. `bilbo new` (`note-create` delta)

- [ ] 2.1 In `src/new.rs`, load the config (exit 2 on an error), parse `--scope <name>` and `--scope=<name>` (once, declared), resolve the scope in the order of Scope of a new note using `std::env::current_dir`, write `scope: <name>` after `created`, and return the unassigned warning for `main` to print on stderr with exit 0. Update `wants_help` in `src/main.rs` so a `--scope` value is never read as `-h`, and the USAGE line for `new`. Cover every scenario of the `note-create` delta and the `config` delta's `New with no config file` in `tests/new.rs`, running the binary with `current_dir` set inside a temporary tree and `HOME` pointing at it. Verify with `cargo test --locked --test new && cargo test --locked --test cli --test recall`

## 3. `bilbo check` (`store-check` delta)

- [ ] 3.1 In `src/check.rs`, load the config (exit 2 on an error) after the argument check, and add the `scope: missing` and not-declared problems with the `holds marks of` suffix. A value that breaks the Scope key rule, or a repeated key, gets only that rule's problem. Add the mark warnings (file name or line), which print with the problems but leave the exit code at 0. Have `main` take the exit code from the problem count, not the line count. Cover every `store-check` delta scenario and the `config` delta's `Check ignores the config` and `Check reads the config` in `tests/check.rs`. Verify with `cargo test --locked --test check`

## 4. `bilbo scope` (`note-scope` and `cli` deltas)

- [ ] 4.1 Add `src/scope.rs`, its `mod` line, dispatch arm and USAGE line, and update the USAGE copies in `tests/cli.rs` and `tests/recall.rs`. Implement:
  - the listing: fixed columns per row kind, `default`, the `(unassigned)` row, a missing store counted as empty, and the no-scope stderr hint;
  - `set` with `--force`: one inserted or replaced line;
  - the refusals: not a note, no closing `---`, no canonical `id`, two `scope` lines;
  - the sequence from design.md: `history/lock`, the sweep through `versions.rs`, the hidden name `notes/.bilbo-restore-<id>`, `swap::exchange`, the swap back on a difference, deleting only bytes bilbo read or wrote, parking anything else, and the refusal on a filesystem without the swap;
  - per-file outcomes on stdout (`set`, `kept`, `kept <old>; --force replaces it`, `replaced`) and stderr. Exit 1 when any file failed. Exit 2 for an undeclared name or missing arguments.

  Cover every `note-scope` scenario in `tests/scope.rs`. For the two race scenarios, give `set` a hook called before each exchange, as change 1's restore has. A unit test passes a closure that writes the note's file, once before the first exchange and once before the swap back. The binary's hook does nothing. Check that the parked bytes are swept by a following `bilbo watch` scan. For `The change is in history`, run `bilbo watch` as a child and poll `bilbo history` with a deadline. Cover the `cli` delta's `Scope is a verb` in `tests/cli.rs`. Verify with `cargo test --locked --test scope && cargo test --locked --bin bilbo scope:: && cargo test --locked --test cli`
- [ ] 4.2 Update `README.md`:
  - what a scope is, and the `scope.*` keys with an example;
  - how `bilbo new` picks a scope. A worktree outside a listed folder is not covered, so list the group folder that holds the clones and `.worktrees/`;
  - `~/` as a `paths` entry;
  - marks: list words first (the employer, its products, its repo names); path marks catch absolute paths in bodies and code blocks;
  - the `check` lines and the warning;
  - `bilbo scope`, with its columns per row kind, and `bilbo scope set`;
  - triaging existing notes with two glob passes.

  Verify with `rg -q 'bilbo scope set' README.md && rg -q 'scope.default' README.md && rg -q 'worktree' README.md`

## 5. The embedder rule (`note-index`, `note-recall` and `note-digest` deltas)

- [ ] 5.1 Remote-URL tests reach the fake embedder through `http://0.0.0.0:<port>`. `config::is_local` calls that host remote, and a connect to `0.0.0.0` lands on the fake's `127.0.0.1` listener on macOS and Linux. Add a test in `tests/index.rs` that sends one input this way and counts it at the fake, and run it on Linux too, in the container of task 8.1, before relying on it. Add the trick to AGENTS.md's Gotchas. Verify with `cargo test --locked --test index zero_address && rg -q '0.0.0.0' AGENTS.md`
- [ ] 5.2 In `src/index.rs`, compute each note's rule from `Stored`'s scope and `config`. Leave out the inputs that only notes with the rule `local` hold when `config::is_local(embedder.url)` is false, drop their cached vectors, and return the withheld count (distinct inputs) for `main` to print on stderr. In `src/recall.rs`, leave the same inputs out of the `not indexed` count. In `src/digest.rs`, admit a withheld passage on the keyword gate while the embedder answers. Cover the `note-index` delta's Withheld passages scenarios in `tests/index.rs`, counting the inputs the fake received through `0.0.0.0`. Cover `Withheld passages are not reported` in `tests/recall.rs`, and the `note-digest` delta's three new scenarios in `tests/digest.rs`. Verify with `cargo test --locked --test index --test recall --test digest`
- [ ] 5.3 Document the embedder rule in `README.md`, beside the embedder settings:
  - the withheld line;
  - that recall and the digest reach withheld notes by keywords;
  - that declaring the first `embedder = local` scope withholds every untriaged note, so triage first or run `--embedder-local`;
  - that a tunnel on `localhost` counts as local.

  Verify with `rg -q 'withheld' README.md && rg -q 'tunnel' README.md`

## 6. Setup and the home-manager module (`setup` delta)

- [ ] 6.1 In `src/setup.rs`, keep `Settings.scope_lines` on a config rewrite, after the digest and history lines. Cover the `setup` delta's `The wizard keeps the scope settings` in `src/setup.rs`'s unit tests, as the digest and history cases are. Verify with `cargo test --locked --bin bilbo setup::`
- [ ] 6.2 In `flake.nix`, give `programs.bilbo.settings` a freeform type. Add an assertion that rejects, by name, every key that is neither in `keys` nor a well-formed `scope.<name>.sync|embedder|paths|marks` or `scope.default`, so `embeder.url` still fails. Write the scope keys after the fixed keys, in sorted order. Add flake checks for `Scope settings from Nix` and `A bad scope key fails evaluation`, and keep the existing `An unknown setting fails evaluation` check passing. Verify with `nix flake check -L`

## 7. The note skill (`agent-plugin` delta)

- [ ] 7.1 Update `plugins/bilbo/skills/note/SKILL.md`:
  - a step that runs `bilbo scope` before `bilbo new`;
  - `--scope` only for a scope the user named;
  - the unassigned stderr line and the one question;
  - `bilbo scope set`, with `--force` only after the user's answer;
  - keeping `scope:` as found on edits;
  - step 6's sources placement: "after the last key, before the closing `---`", not "between `created` and the closing `---`";
  - the `scope:` rows in the `bilbo new` and `bilbo check` tables: `missing`; `not declared`, reported; warnings, reported;
  - the report naming the scope or `unassigned`;
  - `Bash(bilbo scope)` and `Bash(bilbo scope set *)` in `allowed-tools`.

  Extend `tests/plugin.rs` to check that `allowed-tools` lists both commands. Verify with `cargo test --locked --test plugin && claude plugin validate plugins/bilbo && PYTHONDONTWRITEBYTECODE=1 python3 ~/.codex/skills/.system/plugin-creator/scripts/validate_plugin.py plugins/bilbo`
- [ ] 7.2 Smoke-test the skill in a scratch `BILBO_HOME` and `BILBO_CONFIG` declaring `personal` and `work`, with `scope.work.paths` set to a scratch folder. Run Claude Code with the plugin from that folder, and Codex with `codex exec` from another one, each asked to keep a gotcha. Record the commands, the `scope:` each note got, the question or its absence, and the report in `openspec/changes/add-note-scope/smoke.md`. Verify with `rg -q 'scope: work' openspec/changes/add-note-scope/smoke.md && rg -q 'unassigned' openspec/changes/add-note-scope/smoke.md`

## 8. Integration

- [ ] 8.1 Run the suite on Linux in a memory-capped container, as change 1's task 7.2 does, so the `0.0.0.0` connect and `renameat2` are exercised. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked` inside the container
- [ ] 8.2 Run the full suite and the package check. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked && nix flake check -L`
