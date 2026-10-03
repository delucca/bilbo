# Tasks

Run every `cargo` command below from the repo root as `nix develop -c sh -c '<command>'`, with `CARGO_TARGET_DIR` set to the checkout's `target`. The plugin validators run outside the dev shell, as `AGENTS.md` shows. A new file under `src/`, `tests/` or `plugins/` needs `git add -N` before any Nix command can see it. `$PLUGIN_CHECKS` below stands for `claude plugin validate . && claude plugin validate plugins/bilbo && PYTHONDONTWRITEBYTECODE=1 python3 ~/.codex/skills/.system/plugin-creator/scripts/validate_plugin.py plugins/bilbo`. Expect one missing-version warning from each `claude plugin validate`, and never pass `--strict`.

## 1. The note skill (`agent-plugin`: The note skill, The note on the same subject, Creating a note, Note content, Changing a note's kind, Check after every write, Report the note, A missing binary)

- [x] 1.1 Write `plugins/bilbo/skills/note/SKILL.md` as design.md's "The skill file" section gives it: the frontmatter exactly as shown, then a one-line purpose, the eight steps with the three exit-code tables, and the `Never` list, in `recall`'s style. Use these exact strings, which the tests look for:
  - the commands `command -v bilbo`, `bilbo recall --limit 5 -- '<words that name the subject>'`, `bilbo new <kind> <topic> [--title '<title>']`, `bilbo check`, and `mv -n '<root>/notes/<old kind>-<topic>.md' '<root>/notes/<new kind>-<topic>.md'`, each in its own `bash` block on one line;
  - the messages `note: bilbo is not on PATH; install the bilbo CLI first`, `bilbo: no notes match`, `bilbo: no store at <root>` and `already has a note`;
  - the `sources:` example from design.md's "Sources" section.

  In `plugins/bilbo/skills/recall/SKILL.md`, change the description's ending "NOT for writing a note." to "NOT for writing a note (note).". Verify with `test -f plugins/bilbo/skills/note/SKILL.md && head -6 plugins/bilbo/skills/note/SKILL.md`
- [x] 1.2 In `tests/plugin.rs`:
  - Rename `SKILL` to `RECALL_SKILL` and add `NOTE_SKILL` (`plugins/bilbo/skills/note/SKILL.md`). Add `NOTE_SKILL` to `plugin_files_exist`, and assert there that `plugins/bilbo/skills/` holds exactly the folders `note` and `recall`.
  - Factor the frontmatter parsing out of `recall_skill_frontmatter` into a helper that returns the `(key, value)` pairs.
  - Add `note_skill_frontmatter`: the keys are `["name", "description", "license", "allowed-tools"]` in order, the name is `note`, the license is `Apache-2.0`, the folder is `note`, and `allowed-tools` is exactly `Bash(command -v bilbo), Bash(bilbo recall *), Bash(bilbo new *), Bash(bilbo check), Bash(mv -n *), Read, Edit`.
  - Add `note_skill_description_says_when`: the description holds `later session`, `decision`, `gotcha`, `plan`, `note this`, `note that` and `NOT for`.
  - Add `note_skill_drives_bilbo`, modeled on `recall_skill_drives_bilbo`. It looks for the strings listed in 1.1, and fails on any of `nbrecall`, `mint-ulid`, `date +%F`, `Notebooks`, `notebook.org`, `argument-hint`, `supersedes:` or a line starting with `kind:`.
  - Add `skills_allow_every_command_they_run`, for both skills. Each non-empty line inside a ```` ```bash ```` fence must match an `allowed-tools` entry `Bash(<pattern>)`: a pattern ending in ` *` matches a line that starts with the pattern without the `*`, and any other pattern must equal the line. No such line may hold `&&`, `;` or `|`.

  Verify with `cargo test --locked --test plugin && $PLUGIN_CHECKS`
- [x] 1.3 Update the descriptions as design.md's "Manifests and README" table gives them:
  - `plugins/bilbo/.claude-plugin/plugin.json`, `plugins/bilbo/.codex-plugin/plugin.json` and `.claude-plugin/marketplace.json`'s `plugins[0].description`;
  - the Codex `interface`'s `shortDescription`, `longDescription`, `capabilities` (`["Read", "Write"]`) and `defaultPrompt` (two entries).

  In `tests/plugin.rs`, add `plugin_descriptions_agree`, which parses the three files with `serde_json` and asserts the three descriptions are equal and mention `Write` and `recall`. Add `codex_interface_names_write`, which asserts `interface.capabilities` is `["Read", "Write"]` and that `interface.defaultPrompt` has two entries of at most 128 characters each. Verify with `cargo test --locked --test plugin && $PLUGIN_CHECKS`

## 2. The compaction hook (`agent-plugin`: The compaction hook, One plugin for Claude Code and Codex; `setup`: Codex hook trust)

- [x] 2.1 Replace `plugins/bilbo/hooks/hooks.json` with the file in design.md's "The compaction hook" section, copying the SessionStart command byte for byte. In `tests/plugin.rs`:
  - Add `const COMPACT_COMMAND` with that command.
  - In `digest_hook_runs_bilbo_digest_and_never_blocks`, expect the event keys `["SessionStart", "UserPromptSubmit"]`. `serde_json`'s map sorts them.
  - Add `compaction_hook_runs_on_compact_only`: one SessionStart group, `matcher` exactly `"compact"`, and one handler equal to `{"type": "command", "command": COMPACT_COMMAND, "timeout": 5}`.
  - Change `run_hook` to take the event name (`run_hook("UserPromptSubmit", dir)`), keeping the two digest tests passing.
  - Add `compaction_hook_is_silent_without_bilbo`: exit 0, empty stdout and stderr.
  - Add `compaction_hook_nudges_without_running_bilbo`. Its fake `bilbo`, written by a child process as in `digest_hook_exits_zero_when_bilbo_fails`, touches a marker file and exits 2. The test asserts exit 0, stdout equal to the echoed line plus `\n`, an empty stderr, and no marker file.

  Verify with `cargo test --locked --test plugin && $PLUGIN_CHECKS`
- [x] 2.2 Record `tests/fixtures/agents/codex-app-server-list-two-hooks.txt` from Codex 0.155.1, following design.md's "Recording the fixture" steps, in a throwaway `HOME` and `CODEX_HOME` under `/private/tmp`, never `~/.codex`. Its reply lists `bilbo@bilbo:hooks/hooks.json:session_start:0:0` as `untrusted` and `…:user_prompt_submit:0:0` as `trusted`. Check that the reply lists both bilbo keys before saving it. If it lists only the digest hook, Codex kept its `plugins/cache/bilbo/bilbo/<version>/` copy on the same-version swap: run `codex plugin remove bilbo@bilbo`, then `codex plugin add bilbo@bilbo`, and ask `hooks/list` again. In `src/agents.rs`'s tests, add:
  - `trust_hooks_trusts_only_the_new_hook`: on that fixture, the `config/batchWrite` value holds only the `session_start` key with its `currentHash`, and the result is `Wrote { changed: false }`.
  - `trust_hooks_keeps_two_trusted_hooks`: the same fixture with both statuses set to `trusted` gives `Kept` and sends no `config/batchWrite`.
  - `forget_hooks_deletes_every_bilbo_key`: a `config/read` reply with both bilbo keys and one `other@plugin:` key gives `Ok(2)` and one `config/batchWrite` with two edits.

  In `src/setup.rs`, change the wizard summary's `" and trust its digest hook"` to `" and trust its hooks"`, and update `summary_names_the_hook_trust_when_codex_gets_the_plugin`. Verify with `cargo test --locked --bin bilbo agents:: && cargo test --locked --bin bilbo setup:: && cargo test --locked --test setup`

## 3. Docs

- [x] 3.1 Update `README.md` as design.md's "Manifests and README" section lists:
  - The intro and the "Agent plugin" bullet name the `note` skill.
  - "From an agent" describes `note`: when it writes, that it looks for the note on the subject first, creates through `bilbo new`, renames on a kind change, runs `bilbo check` and reports the path. It also describes the compaction hook: one line after each compaction, nothing without `bilbo`.
  - "Set up" and "Remove" speak of the plugin's hooks, not only the digest hook.

  Verify with `rg -q 'bilbo:note|note. skill' README.md && rg -q 'compact' README.md && ! rg -q "digest hook in Codex" README.md`
- [x] 3.2 Update `AGENTS.md` with what the code cannot show:
  - The `hooks.json` gotcha covers every command in the file, not one: never pass on bilbo's exit code, stay quiet without `bilbo` on `PATH`.
  - On an auto-compaction, neither tool passes PreCompact or PostCompact output to the model. SessionStart with the matcher `compact` is the event that does, in both (`openspec/changes/archive/<YYYY-MM-DD>-add-note-skill/probe.md`, the path the archive gives it; cite it the way the wizard gotcha cites the `smoke.md` of `2026-10-02-add-setup/`). Fill in the day the change is expected to be archived, and correct it in the archive step if the day differs.
  - Codex runs hooks under `$SHELL -lc`, and a login profile can rebuild `PATH`. A hook probe needs `bilbo` in a folder that profile keeps, such as `$HOME/.nix-profile/bin` of the throwaway `HOME`.

  Verify with `rg -q 'SessionStart' AGENTS.md && rg -q 'SHELL -lc' AGENTS.md`

## 4. Integration

- [x] 4.1 Smoke test in Claude Code, recorded in the change folder as `smoke.md` in the shape of add-note-digest's: a summary table, then one section per step, with commands and trimmed output. Build with `cargo build --locked`. `$T` is a scratch folder under `/private/tmp/claude-501/`, and every `bilbo` and `claude` run carries `BILBO_HOME=$T/store`, `BILBO_CONFIG=$T/config` (an empty file, so keyword-only) and `XDG_CACHE_HOME=$T/cache`. Run `claude -p --model sonnet --setting-sources "" --plugin-dir <worktree>/plugins/bilbo --output-format stream-json --verbose --include-hook-events --max-turns 15` from `$T/work`, with `target/debug` first on `PATH`. Pass no `--allowedTools`, so a command outside the skill's `allowed-tools` shows up as a permission denial. A throwaway `CLAUDE_CONFIG_DIR` is not logged in (`probe.md`), so the runs use the user's login with no user, project or local settings loaded. Record that, and the transcripts that land under `~/.claude/projects/`, in `smoke.md`. Steps:
  0. Seed the store with one malformed note, `$T/store/notes/report-seeded.md`, made by `bilbo new report seeded` with its `created` line then changed to `created: 2026-10-03`. `bilbo check` prints one line for it and exits 1.
  1. "We decided to tag releases only as v<version> on main. Keep this decision for later sessions." The skill runs `bilbo recall` and then `bilbo new decision …`, and `$T/store/notes/decision-*.md` exists. `bilbo check` prints no line for the new note, `report-seeded.md` is byte for byte unchanged, its frontmatter has only `id` and `created`, and the reply names the new note's absolute path and says one `bilbo check` line was left for another note.
  2. "Also keep that pushing a bare <version> tag publishes a second release." The skill finds the step 1 note through `recall` and updates it. The note keeps the same `id` and `created`, and `notes/` still holds one file.
  3. Start a plan note with `bilbo new plan rollout-order`, then ask the agent to keep, as a settled decision, that rollout goes Linux first. The file becomes `decision-rollout-order.md` with the same `id`. `bilbo check` still exits 1 because of the seeded note: it prints only the seed's line, and the reply reports it as left for another note.
  4. Pre-create both names: `bilbo new plan cache-layout` in `$T/store`, and `bilbo new decision cache-layout` with `BILBO_HOME=$T/other`, whose file is then moved into `$T/store/notes/`, so each has its own valid id. Ask the agent to keep, as a settled decision, that the cache stays flat. The agent renames nothing, updates `decision-cache-layout.md`, leaves the bytes of `plan-cache-layout.md` as they are, deletes neither file, and its reply reports the shared-topic lines of `bilbo check`. Delete both files after this step, so later steps start from the store steps 0 to 3 left.
  5. A passing remark: "note that the build is slow today; now list the files in this folder". The stream holds no `bilbo new` and no `Skill` call for `note`, and `notes/` is unchanged.
  6. The unprompted trigger: in a fresh session, ask the agent to find out why `$T/work/check.sh` fails, a script that fails only when `LC_ALL=C` is set. The prompt does not mention notes. Record in `smoke.md`, as a finding and not a failure, whether the agent runs the `note` skill unasked after finding the cause. The description alone did not make Sonnet do so; explicit asks and the compaction nudge work. Name a stronger nudge as a follow-up. Do not reword the prompt to name notes.
  7. A resumed prompt with `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE=1` compacts the session. The stream holds a `SessionStart:compact` hook response with the hook's line, and asked to quote its context, the agent quotes it.
  8. With `PATH` lacking `bilbo`, step 1's prompt gets `note: bilbo is not on PATH; install the bilbo CLI first`, and no file is written.

  Verify with `test -s openspec/changes/add-note-skill/smoke.md`
- [x] 4.2 Smoke test in Codex, appended to `smoke.md`. Use a throwaway `HOME` and `CODEX_HOME` under `/private/tmp`, a `PATH` of the built `bilbo`, a folder of fake `launchctl` and `systemctl`, a folder holding only `codex`, and `/usr/bin:/bin`. Link the built `bilbo` into `$HOME/.nix-profile/bin`, so Codex's login shell finds it. Steps:
  1. Run `bilbo setup --yes --no-timer --plugin-source <worktree>` twice. The lines are `hook installed: trusted in Codex`, then `hook kept: trusted in Codex`, and `config.toml` holds trust for both the `session_start` and the `user_prompt_submit` keys.
  2. `codex debug prompt-input hi` lists `bilbo:note` and `bilbo:recall`.
  3. Add the mock provider from `probe/codex-config.toml` to `config.toml` and run `probe/mock-responses.py`. Then run `codex exec` and `codex exec resume`, as in `probe.md`, without `--dangerously-bypass-hook-trust`. The post-compaction request holds the hook's line as a `developer` message.
  4. A real Codex model writes a note, with the user's Codex login. Before anything else, record `shasum -a 256 ~/.codex/auth.json`. Copy `~/.codex/auth.json` into the throwaway `CODEX_HOME` with mode 600 (and restore the real provider in its `config.toml` if the mock is set), then run `codex exec` there, with the built `bilbo` first on `PATH` and the throwaway `BILBO_HOME`, on a prompt that should make the agent write a note through `bilbo:note`. Record whether it did, and whether `bilbo check` passes. Afterwards compare the throwaway `auth.json` with the original by hash: a change means the refresh token may have rotated, so report it and write nothing to `~/.codex`. Delete the throwaway `auth.json` either way, and never print its contents.
  5. `bilbo setup --remove --yes` reports `hook removed`, and no `bilbo@bilbo:` key is left.

  Verify with `rg -q 'session_start' openspec/changes/add-note-skill/smoke.md`
- [x] 4.3 Run the full suite, the package check and the plugin validators. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked && nix flake check -L && $PLUGIN_CHECKS`
