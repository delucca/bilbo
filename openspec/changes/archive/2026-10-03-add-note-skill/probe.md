# Compaction hook probe

Run on rivendell (macOS) on 2026-10-03 with Claude Code 2.1.288 and Codex 0.155.1. The question was which compaction-time hook output reaches the model in each tool, and whether the agent gets a turn before an auto-compaction. The scripts are in `probe/`. `$PROBE` is a scratch folder. Raw transcripts stayed in the session scratchpad and were not kept.

`probe/hook.sh <label> plain|json|none <event>` appends its label and its stdin to `$PROBE/hooks.log`, then prints one line:

- `plain` prints `<label>: the word of this hook is <label>-WORD`.
- `json` prints `{"hookSpecificOutput":{"hookEventName":"<event>","additionalContext":"<that line>"}}`.
- `none` prints nothing.

Each probe registered these hooks:

| Event | Matcher | Modes |
|---|---|---|
| PreCompact | none | `plain`, `json` |
| PostCompact | none | `plain`, `json` |
| SessionStart | `compact` | `plain` |
| SessionStart | none | `none`, as a control |

## Docs

- Claude Code (https://code.claude.com/docs/en/hooks):
  - PreCompact plain stdout goes "to the debug log only, not shown to the user or added to Claude's context".
  - PostCompact plain stdout is "shown to the user only".
  - Both list `additionalContext` among their JSON fields.
  - SessionStart, whose `source` can be `compact`, is one of the events whose plain stdout "Claude can see and act on".
- Codex (https://learn.chatgpt.com/docs/hooks, redirected from developers.openai.com/codex/hooks):
  - PreCompact and PostCompact ignore plain stdout, and their JSON takes only the common fields.
  - SessionStart matches on `source` (`startup`, `resume`, `clear`, `compact`). Its stdout "becomes extra developer context", and after a compaction it runs "before the next model request".
  - Plugin hooks use the same event schemas and the same trust review as user hooks.

## Claude Code

The probe ran `claude -p --model haiku --setting-sources "" --plugin-dir $PROBE/plugin` (`probe/claude-hooks.json` as the plugin's `hooks/hooks.json`). A throwaway `CLAUDE_CONFIG_DIR` was not logged in ("Not logged in · Please run /login"). So the runs used the user's login, with no user, project or local settings loaded. They wrote session transcripts under `~/.claude/projects/-private-tmp-claude-501-…-scratchpad-probe-work/`. The first run's haiku also saved the probe's codeword as a memory file there (`memory/codeword.md`). dnix's managed hooks ran beside the probe's, including the legacy PreCompact `echo`.

Manual compaction (`claude -p "/compact" --resume <id>`):

- All six probe hooks ran (`hooks.log`).
- Plain PreCompact and PostCompact stdout, the legacy `BEFORE COMPACTING…` line included, landed after the summary, in the `/compact` command's `<local-command-stdout>` message.
- Both JSON hooks failed validation: `hookSpecificOutput.hookEventName: expected one of "PreToolUse" | "UserPromptSubmit" | "UserPromptExpansion" | "SessionStart" | "Setup" | "PreModelSwitch" | …`. In 2.1.288, `additionalContext` is not accepted for PreCompact or PostCompact, whatever the docs say.
- SessionStart `compact` stdout became a `hook_success` attachment.
- On the next turn, asked to list every `-WORD` token in its context, haiku listed all five, each from where it appeared. All of it arrived after the compaction, none before.

Auto-compaction (a fresh session, then a resumed prompt with `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE=1`):

- `compact_boundary` had `trigger: auto`, and compaction ran when the prompt was submitted, before any model turn.
- PreCompact, PostCompact and SessionStart `compact` all ran (`hooks.log`).
- Asked the same question in that same turn, haiku listed only `SESSIONSTARTCOMPACT-WORD`, "in the SessionStart:compact hook success message". The transcript holds no PreCompact or PostCompact output.

## Codex

A throwaway `CODEX_HOME` held `probe/codex-config.toml`, with a model provider pointing at `probe/mock-responses.py` on 127.0.0.1:18799. The mock answers every Responses request with one message, reports 50,000 input tokens, and logs each request body. Replace `<probe-work>` in `probe/codex-config.toml` with the probe's working folder: Codex must trust its cwd. `model_auto_compact_token_limit = 20000` then forces a compaction before the next turn. No login was needed. The runs were `codex exec --skip-git-repo-check --json "First prompt: remember HERON."`, then `codex exec resume … <thread> "Second prompt: what is the codeword?"`.

User hooks (`probe/codex-hooks.json` as `$CODEX_HOME/hooks.json`, with `--dangerously-bypass-hook-trust`):

- All six hooks ran on the second run (`hooks.log`).
- The mock saw three requests:
  1. The first turn.
  2. The compaction request ("You are performing a CONTEXT CHECKPOINT COMPACTION…"), with no hook output in it.
  3. The post-compaction turn. It held `CXSESSIONSTARTCOMPACT-WORD` as a `developer` message just before the user's prompt, and no PreCompact or PostCompact word.

The plugin, trusted by `bilbo setup`:

- The worktree's plugin was copied to a scratch source, and its `hooks.json` gained a SessionStart `compact` hook with the guarded command shape (`command -v bilbo >/dev/null 2>&1 || exit 0; echo 'PROBE-NUDGE: context was compacted.'`).
- The built `target/debug/bilbo setup --yes --no-timer --plugin-source <scratch>` ran with a throwaway `HOME` and `CODEX_HOME`, and fake `launchctl` and `systemctl`.
- First run: `codex installed`, `hook installed: trusted in Codex`. Second run: `codex kept`, `hook kept: trusted in Codex`.
- `config.toml` then held two trust entries:
  - `bilbo@bilbo:hooks/hooks.json:session_start:0:0`
  - `bilbo@bilbo:hooks/hooks.json:user_prompt_submit:0:0`, with hash `sha256:25e2d1e2…`. That is the hash recorded in `tests/fixtures/agents/codex-app-server-list-untrusted.txt` for the one-hook plugin, so adding an event does not change the digest hook's hash.
- The same two `codex exec` runs, without `--dangerously-bypass-hook-trust`, put `PROBE-NUDGE: context was compacted.` into the post-compaction request as a `developer` message just before the user's prompt.
- The first attempt showed nothing. Codex runs hooks under `$SHELL -lc`, and the login profile (nix-darwin's `/etc/profile`) rebuilt `PATH` without the scratch folders, so `command -v bilbo` failed and the hook printed nothing, as designed. A link to the built `bilbo` in `$HOME/.nix-profile/bin`, which the login `PATH` keeps, fixed it.

## Conclusions

- Neither tool gives the agent a turn before an auto-compaction. A PreCompact hook can only block it (Claude Code: exit 2 or `decision: block`; Codex: `continue: false`), and blocking an auto-compaction stalls a full context.
- PreCompact and PostCompact output never reaches the model on an auto-compaction, in either tool. On a manual `/compact`, Claude Code shows it only afterwards, as command output. The legacy PreCompact nudge has been reaching the model only on manual compactions, and only after the fact.
- SessionStart with the matcher `compact` reaches the model on every compaction in both tools, right before the next model request, as plain stdout. It is the hook to ship.
