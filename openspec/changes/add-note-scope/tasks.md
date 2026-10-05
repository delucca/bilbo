# Tasks

Run every command below from the repo root as `nix develop -c sh -c '<command>'`, with `CARGO_TARGET_DIR` set to the checkout's `target`. A new file under `src/` or `tests/` needs `git add -N` before any Nix command can see it. This change builds on `add-note-history`, archived and released in 0.10.0. A module added before the verb that calls it is dead code to clippy, so until sections 2 to 5 have all landed, run clippy as `cargo clippy --locked --all-targets -- -D warnings -A dead_code`; task 8.2 runs it without the allowance.

## 1. Groundwork

- [x] 1.1 Move recall's word rule from `src/search/rank.rs` to `src/shared/text.rs`, unchanged: `words`, `each_word` (now `pub`), `is_mark` and `base`, with their unit tests (`words_fold_case_and_accents`, `words_expand_letters_without_a_base`, `words_split_on_underscore_and_punctuation`, `words_drop_one_char_words`, `marks_join_and_vanish`, `letters_outside_the_table_pass_through`, `every_table_letter_folds_to_ascii`). `rank` calls `text::each_word`; `src/search/recall.rs` and `src/search/digest.rs` call `text::words`; `rank`'s other tests import `crate::shared::text::words`. Verify with `cargo test --locked --bin bilbo shared::text:: && cargo test --locked --bin bilbo search:: && cargo test --locked --test layout`
- [x] 1.2 In `src/note/mod.rs`, accept `scope` as a frontmatter key in `Keys::key` (at most once, the topic grammar through `store::is_topic`, problems naming `scope`), give `note::Note` what `check` needs (the valid value, or that the key is absent, or that it broke a rule), and add `scope: ` to `unexpected_line`'s list. Add the valid value to `documents::Stored` in `src/search/documents.rs`, read in `read_notes`. Unit-test the `note-store` delta's scenarios. Verify with `cargo test --locked --bin bilbo note:: && cargo test --locked --bin bilbo search::documents::`
- [x] 1.3 Add `src/note/marks.rs` and its `pub mod` line in `src/note/mod.rs`: for one mark, the first place holding it, the topic of the file name, then the `sources` items and the body by line (fenced code included, the `scope:` line and the other keys skipped). A word mark is compared through `text::words`; a path mark matches each of the forms it is given up to a segment boundary (the next character is not a letter, a digit, `-`, `_` or `.`). Take the marks as text, so the module needs no config type. Unit-test the `store-check` delta's Matching a mark scenarios and a mark found only in the topic. Verify with `cargo test --locked --bin bilbo note::marks::`
- [x] 1.4 In `src/shared/config.rs`, parse `scope.<name>.sync|embedder|paths|marks` and `scope.default` as a key pattern beside `KEYS`. Check: name grammar, `default` reserved, `sync = off` only, `embedder = any|local`, comma lists, paths that are `~/`, `/`, absolute or start with `~/`, marks that are one word (`text::words`) or a path, `scope.default` declared. Reject two scopes naming one folder, comparing after `~/` expansion and link resolution, and one mark in two scopes. Keep the scope lines as written in `Settings.scope_lines`, in file order. Add the declared scopes, the embedder rule of a `scope` value, the forms of each path mark (`~/` and the home folder's absolute path), and the resolution of a working directory against `paths`: whole folder names, each entry's links resolved again at the match, the longest wins, and a tie matches nothing. Update the unknown-key message to name the scope keys, and extend the pinned key lists in `tests/index.rs` and `tests/recall.rs`. Unit-test the `config` delta's Scope settings and Scope paths and marks scenarios, a tie through a link made after load, and the `note-scope` delta's Embedder rule scenarios. Verify with `cargo test --locked --bin bilbo shared::config:: && cargo test --locked --test recall --test index`
- [x] 1.5 Give `new`, `check` and `index` the results this change fills, with no change in behavior, so later tasks need no edit to `src/main.rs`: `note::new::run` returns the path and an optional stderr line, `check::run` its lines and whether any of them is a problem (as `cite`'s `failed`), and `search::index::run` its stdout line and its stderr lines. `main` prints the stderr lines and takes `check`'s exit code from that flag. `wants_help` skips the value after `new`'s `--scope`, as it does for `--title`. Verify with `cargo test --locked --test new --test check --test index --test cli`

## 2. `bilbo new` (`note-create` delta)

- [x] 2.1 In `src/note/new.rs`, load the config (exit 2 on an error), parse `--scope <name>` and `--scope=<name>` (once, declared), resolve the scope in the order of Scope of a new note using `std::env::current_dir`, write `scope: <name>` after `created` through `note::render`, and return the unassigned warning as 1.5's stderr line, with exit 0. Cover every scenario of the `note-create` delta and the `config` delta's `New with no config file` in `tests/new.rs`, running the binary with `current_dir` set inside a temporary tree and `HOME` pointing at it. Verify with `cargo test --locked --test new && cargo test --locked --bin bilbo note:: && cargo test --locked --test cli`

## 3. `bilbo check` (`store-check` delta)

- [x] 3.1 In `src/check.rs`, load the config (exit 2 on an error) after the argument check, and add the `scope: missing` and not-declared problems with the `holds marks of` suffix. A value that breaks the Scope key rule, or a repeated key, gets only that rule's problem. Add the mark warnings (file name or line) through `note::marks`, which print with the problems but leave the exit code at 0. Set 1.5's flag from the problems alone. Library files get no scope rule. Cover every `store-check` delta scenario and the `config` delta's `Check ignores the config` and `Check reads the config` in `tests/check.rs`. Verify with `cargo test --locked --test check`

## 4. `bilbo scope` (`note-scope` and `cli` deltas)

- [x] 4.1 Move restore's hidden-file writer, `write_temp` in `src/note/restore.rs`, to `src/note/versions.rs` as a `pub` function, unchanged, and call it from restore. Verify with `cargo test --locked --bin bilbo note:: && cargo test --locked --test restore`
- [x] 4.2 Add `src/note/scope.rs`, its `pub mod` line in `src/note/mod.rs`, `note/scope.rs` in `VERBS` in `tests/layout.rs`, its dispatch arm and USAGE lines in `src/main.rs` (with `[--scope <name>]` on `new`'s line), the same USAGE text in `tests/cli.rs` and `tests/recall.rs`, and `note/scope.rs` in the verb list of AGENTS.md's Architecture rules. Implement:
  - the listing: fixed columns per row kind, `default`, the `(unassigned)` row, the notes counted through `documents::read_notes`, a missing store counted as empty, and the no-scope stderr hint;
  - `set` with `--force`: one inserted or replaced line;
  - the refusals: not a note, no closing `---`, no canonical `id`, two `scope` lines;
  - the sequence from design.md: `versions::lock`, `versions::sweep_restore_leftovers`, the hidden name `versions::restore_path`, `swap::exchange`, the swap back on a difference, deleting only bytes bilbo read or wrote, parking anything else, and the refusal on a filesystem without the swap (`swap::UNSUPPORTED`);
  - per-file outcomes on stdout (`set`, `kept`, `kept <old>; --force replaces it`, `replaced`) and stderr. Exit 1 when any file failed. Exit 2 for an undeclared name or missing arguments. Take the lock only when some argument is a note, so a run on a missing store creates nothing.

  Cover every `note-scope` scenario in `tests/scope.rs`. For the two race scenarios and the filesystem without the swap, give `set` a hook called before each exchange, as restore has a step hook: it may write the note's file, or fail, as a filesystem without the call does. A unit test in `src/note/scope.rs` passes a closure that writes the note's file once before the first exchange and once before the swap back, after recording a version of the note so its history exists; it then checks that `versions::sweep_restore_leftovers`, what the next `bilbo watch` scan runs, records the parked bytes as `edited` and removes the hidden file. The binary's hook does nothing. For `The change is in history`, run `bilbo watch` as a child and poll `bilbo history` with a deadline. Cover the `cli` delta's `Scope is a verb` in `tests/cli.rs`. Verify with `cargo test --locked --test scope && cargo test --locked --bin bilbo note::scope:: && cargo test --locked --test cli --test recall --test layout`
- [x] 4.3 Update `README.md`:
  - what a scope is, and the `scope.*` keys with an example;
  - how `bilbo new` picks a scope. A worktree outside a listed folder is not covered, so list the group folder that holds the clones and `.worktrees/`;
  - `~/` as a `paths` entry;
  - marks: list words first (the employer, its products, its repo names); path marks catch absolute paths in bodies and code blocks;
  - the `check` lines and the warning;
  - `bilbo scope`, with its columns per row kind, and `bilbo scope set`;
  - triaging existing notes with two glob passes.

  Verify with `rg -q 'bilbo scope set' README.md && rg -q 'scope.default' README.md && rg -q 'worktree' README.md`

## 5. The embedder rule (`note-index`, `note-recall` and `note-digest` deltas)

- [x] 5.1 Remote-URL tests reach the fake embedder through `http://0.0.0.0:<port>`. `config::is_local` calls that host remote, and a connect to `0.0.0.0` lands on the fake's `127.0.0.1` listener on macOS and Linux. Add a test in `tests/index.rs` that sends one input this way and counts it at the fake, and run it on Linux too, in the container of task 8.1, and in `nix flake check`, before relying on it. Add the trick to AGENTS.md's Gotchas. Verify with `cargo test --locked --test index zero_address && rg -q '0.0.0.0' AGENTS.md`
- [x] 5.2 In `src/search/vectors.rs`, beside `lookup`, add the function that gives the withheld inputs: when `config::is_local(embedder.url)` is false, the inputs that only notes with the rule `local` hold, by `Stored`'s scope and `config`'s rule. In `src/search/recall.rs`, leave them out of the `not indexed` count and use no cached vector for them. In `src/search/digest.rs`, use no cached vector for them and admit a withheld passage on the keyword gate while the embedder answers. Cover `Withheld passages are not reported` in `tests/recall.rs`, and the `note-digest` delta's three new scenarios in `tests/digest.rs`. Verify with `cargo test --locked --bin bilbo search:: && cargo test --locked --test recall --test digest`
- [x] 5.3 In `src/search/index.rs`, leave the withheld inputs out of the needed ones before the diff against the cache, so their cached vectors are dropped, and return the withheld line, counting distinct inputs, as 1.5's stderr line. Cover the `note-index` delta's Withheld passages scenarios in `tests/index.rs`, counting the inputs the fake received through `0.0.0.0`. Verify with `cargo test --locked --test index`
- [x] 5.4 Document the embedder rule in `README.md`, beside the embedder settings:
  - the withheld line;
  - that recall and the digest reach withheld notes by keywords;
  - that declaring the first `embedder = local` scope withholds every untriaged note, so triage first or run `--embedder-local`;
  - that a tunnel on `localhost` counts as local.

  Verify with `rg -q 'withheld' README.md && rg -q 'tunnel' README.md`

## 6. Setup and the home-manager module (`setup` delta)

- [x] 6.1 Keep `Settings.scope_lines` on a config rewrite, after the digest and history lines: `Facts.kept` in `src/setup/facts.rs`, `Plan.kept` in `src/setup/plan.rs`, `config_step` in `src/setup/apply.rs` and `config::render` take owned key strings, and `src/setup/fakes.rs` follows. Cover the `setup` delta's `The wizard keeps the scope settings` in `src/setup/driven.rs`, beside `the_wizard_keeps_the_history_setting`. Verify with `cargo test --locked --bin bilbo setup:: && cargo test --locked --bin bilbo shared::config::`
- [x] 6.2 In `flake.nix`, give `programs.bilbo.settings` a freeform type. Add an assertion that rejects, by name, every key that is neither in `keys` nor a well-formed `scope.<name>.sync|embedder|paths|marks` or `scope.default`, so `embeder.url` still fails. Write the scope keys after the fixed keys, in sorted order. Add flake checks for `Scope settings from Nix` and `A bad scope key fails evaluation` to the existing module check, and keep `An unknown setting fails evaluation` passing. Verify with `nix flake check -L`

## 7. The note skill (`agent-plugin` delta)

- [x] 7.1 Update `plugins/bilbo/skills/note/SKILL.md`:
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

- [ ] 8.1 Run the suite on Linux in a memory-capped container, as change 1's task 7.2 did, so the `0.0.0.0` connect and `renameat2` are exercised. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked` inside the container
- [ ] 8.2 Run the full suite and the package check. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked && nix flake check -L`
