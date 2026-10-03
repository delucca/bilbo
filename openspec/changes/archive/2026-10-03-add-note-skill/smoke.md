# add-note-skill smoke test

Run on rivendell (macOS) on 2026-10-03 with `target/debug/bilbo` built (`cargo build --locked`) in the worktree, Claude Code 2.1.288 and Codex 0.155.1. Scratch folders: `/private/tmp/claude-501/n3` (Claude Code, `$T`) and `/private/tmp/claude-501/n4` (Codex). Every `bilbo` and `claude` run carried `BILBO_HOME=$T/store`, `BILBO_CONFIG=$T/config` (empty, keyword-only) and `XDG_CACHE_HOME=$T/cache`, with `target/debug` first on `PATH`. The Claude runs were `claude -p --model sonnet --setting-sources "" --plugin-dir <worktree>/plugins/bilbo --output-format stream-json --verbose --include-hook-events --max-turns 15` from `$T/work`, with no `--allowedTools`; every run ended with `permission_denials: []`. A throwaway `CLAUDE_CONFIG_DIR` is not logged in (`probe.md`), so the runs used the user's login with no user, project or local settings loaded; dnix's managed hooks still ran beside the plugin's (the extra "Notes that match this prompt" blocks come from them). The sessions' transcripts landed in `~/.claude/projects/-private-tmp-claude-501-n3-work/`. Raw streams are in `$T/out/s<step>.jsonl`.

## Summary

| Step | Expectation | Result |
|---|---|---|
| 0 seed | one malformed note, `bilbo check` prints one line, exit 1 | pass |
| 1 create | recall, `bilbo new decision`, check clean for the new note, seed untouched, reply names the path and the left-over line | pass: `decision-release-tags.md`, seed hash unchanged |
| 2 update | recall finds the step 1 note, updates it, same `id` and `created`, one file | pass: only a "Gotcha: bare tags" section was added |
| 3 kind change | `mv -n` to `decision-rollout-order.md`, same `id` | pass: `bilbo check` exits 1 and prints only the seed's line, which the reply reports as left |
| 4 both names exist | nothing renamed, the decision file updated, the plan file's bytes kept, neither deleted, shared-topic lines reported | pass twice: see below (the second attempt kept the first stream as `s4a.jsonl`, `s4.jsonl` is the rerun) |
| 5 passing remark | no `bilbo new`, no `note` Skill call, `notes/` unchanged | pass |
| 6 unprompted trigger | recorded finding: does the agent write a note unasked after finding the cause | finding: it did not in two attempts; the description alone does not trigger Sonnet |
| 7 compaction | `SessionStart:compact` response with the line, quoted by the agent | pass |
| 8 no bilbo on PATH | the exact PATH message, no file written | pass |

## Step 0: seed

```
$ bilbo new report seeded     # then: created: 2026-10-03
$ bilbo check
notes/report-seeded.md: created: '2026-10-03' is not YYYY-MM-DDTHH:MM±HH:MM, a real local time to the minute (line 3)
exit 1
```

## Step 1: create

Prompt: "We decided to tag releases only as v<version> on main. Keep this decision for later sessions."

```
Skill bilbo:note
Bash  command -v bilbo                       -> .../target/debug/bilbo
Bash  bilbo recall --limit 5 -- 'release tags version main'   -> exit 1, bilbo: no notes match
Bash  bilbo new decision release-tags        -> .../store/notes/decision-release-tags.md
Read, Edit (body)
Bash  bilbo check                            -> exit 1, one line, for notes/report-seeded.md
```

The reply named `/private/tmp/claude-501/n3/store/notes/decision-release-tags.md`, said `bilbo check` still reported one line for another note, and that it did not touch it. `shasum -c` on `report-seeded.md` printed OK. The skill wrote a "Why" section the user had not given; the reply said so.

## Step 2: update

Prompt: "Also keep that pushing a bare <version> tag publishes a second release."

`bilbo recall` returned `decision-release-tags.md`, the agent read it and Edited in a `## Gotcha: bare tags` section. `diff` against the step 1 copy shows only that added section; `id` and `created` are unchanged, and `notes/` holds the same two files.

## Step 3: kind change

`bilbo new plan rollout-order` (id `01M42076XNR41QXA57E26ACAS5`), then: "Keep, as a settled decision, that rollout goes Linux first."

```
Bash  bilbo recall --limit 5 -- 'rollout Linux first'   -> plan-rollout-order.md
Bash  mv -n '.../notes/plan-rollout-order.md' '.../notes/decision-rollout-order.md'
Edit  body
Bash  bilbo check   -> exit 1, the seed's line only
```

`decision-rollout-order.md` holds the same `id`. No `bilbo check` line names it. The reply said it renamed the plan to the decision kind and left the seed alone.

## Step 4: both names exist

`bilbo new plan cache-layout` in `$T/store`, `bilbo new decision cache-layout` with `BILBO_HOME=$T/other` moved into `$T/store/notes/`, each with its own id. `bilbo check` printed the two shared-topic lines, plus the seed's.

Prompt: "Keep, as a settled decision, that the cache stays flat."

Both attempts gave the same result. The agent's `bilbo recall` returned both files, it Read them, and Edited `decision-cache-layout.md` (the file of the requested kind), with no `mv`. `bilbo check` then showed the two shared-topic lines, which the agent reported ("`plan-cache-layout.md` uses the same topic ... I left it", suggesting the user delete it or give it another topic). `plan-cache-layout.md` kept its bytes; `decision-cache-layout.md` did not, because the agent wrote the decision into it.

This is the expected result: with both files present there is nothing to rename, so the agent updates the note of the wanted kind, leaves the other file's bytes alone, deletes and renames neither, and reports the shared-topic lines. Run by hand, `mv -n plan-cache-layout.md decision-cache-layout.md` in that store exits 0 and leaves both files in place, as design.md says. Both files were deleted afterwards.

## Step 5: passing remark

Prompt: "note that the build is slow today; now list the files in this folder". The stream holds one tool call, `ls -la`, no `Skill` call and no `bilbo new`. `notes/` is unchanged (`shasum -c` over its files, no failures). The reply said "I've noted that the build is slow today" with no file.

## Step 6: unprompted trigger

`$T/work/check.sh` compares `é-release` and `z-release` under `LC_COLLATE="${LC_ALL:-en_US.UTF-8}"`: it passes plain and fails with `LC_ALL=C`. Prompt (attempt 1): "Find out why check.sh fails when LC_ALL=C is set in the environment. I only need the cause." Attempt 2 dropped the last sentence, which could discourage a note, and named nothing about notes either.

In both attempts the agent found the cause and answered. The stream holds no `Skill` call and `notes/` is unchanged. Finding: the description's "or when this session settled something durable" does not make Sonnet run `note` after a debugging answer. The skill triggers on an explicit ask (steps 1 to 4) and does not trigger on its own. Explicit asks (steps 1 to 4) and the compaction nudge (step 7) do work. Follow-up: a stronger nudge, such as a Stop-style reminder or a line in the plugin that asks the agent to consider a note at the end of a task. I did not reword the prompt to name notes.

## Step 7: compaction

A resumed session (`--resume <step 1 session>`) with `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE=1`. The stream holds `compact_boundary` with `trigger: auto` (24,428 tokens before, 3,325 after), and a `SessionStart:compact` hook response with the output:

```
Context was compacted. If this session settled something later sessions should know, such as a decision, a gotcha or a plan, save it with the bilbo note skill once the current task allows.
```

Asked to quote any line mentioning the note skill and compaction, the agent quoted exactly that line.

## Step 8: no bilbo on PATH

A first attempt with only the worktree folder removed still found the user's installed `/etc/profiles/per-user/delucca/bin/bilbo`, so it wrote nothing new but proves nothing. Second attempt: `PATH` of a folder holding only `claude`, then `/usr/bin:/bin`, and a fresh empty `BILBO_HOME`.

```
Skill bilbo:note
Bash  command -v bilbo   -> Exit code 1
reply: note: bilbo is not on PATH; install the bilbo CLI first
```

`$T/store8` stayed empty.

## Codex

Throwaway `HOME` and `CODEX_HOME` under `/private/tmp/claude-501/n4`; `PATH` of the built `bilbo`, fake `launchctl` and `systemctl`, a folder holding only `codex`, and `/usr/bin:/bin`. The built `bilbo` was linked into `$HOME/.nix-profile/bin` for Codex's login shell.

### Setup, twice

```
$ bilbo setup --yes --no-timer --plugin-source <worktree>
codex installed: /Users/delucca/Developer/delucca/.worktrees/bilbo.add-note-skill
hook installed: trusted in Codex
$ bilbo setup --yes --no-timer --plugin-source <worktree>
codex kept
hook kept: trusted in Codex
```

`config.toml` held `[hooks.state."bilbo@bilbo:hooks/hooks.json:session_start:0:0"]` (`sha256:a81858be...`) and `[hooks.state."bilbo@bilbo:hooks/hooks.json:user_prompt_submit:0:0"]` (`sha256:25e2d1e2...`, the one-hook hash).

### Skills

`codex debug prompt-input hi` lists `bilbo:note` and `bilbo:recall`.

### Compaction hook, mock provider

`probe/codex-config.toml`'s provider (prepended to `config.toml`, port 18799) with `probe/mock-responses.py`. `codex exec --skip-git-repo-check --json "First prompt: remember HERON."`, then `codex exec resume ... "Second prompt: what is the codeword?"`, both with `</dev/null` and without `--dangerously-bypass-hook-trust`. The mock saw three requests (3 and 5 input items for the first two, 6 for the third). The third, after the compaction, holds this `developer` message:

```
Context was compacted. If this session settled something later sessions should know, such as a decision, a gotcha or a plan, save it with the bilbo note skill once the current task allows.
```

### A real Codex model and the note skill

Two runs. Both copied `~/.codex/auth.json` (sha256 `ecbdf9971d7d...`, hashed first) into the throwaway `CODEX_HOME` with mode 600, and both deleted the copy afterwards.

First run, `--sandbox workspace-write --add-dir $T/store` (`$T` = `/private/tmp/claude-501/n4`):

- The managed requirements refuse the sandbox: `invalid value for sandbox_mode: WorkspaceWrite is not in the allowed set [ReadOnly, DangerFullAccess]`. Codex fell back to read-only.
- The model picked the skill on its own, cat its `SKILL.md` from the plugin cache, ran `command -v bilbo` and `bilbo recall --limit 5 -- 'release tags'` (exit 1), then stopped: "No note was created."

Second run, approved by the user, once: a fresh throwaway `HOME`, `CODEX_HOME` and `BILBO_HOME` under `/private/tmp/claude-501/n5`, the built `bilbo` first on `PATH` and linked into `$HOME/.nix-profile/bin`, the plugin installed and trusted by the built `bilbo setup` (`codex installed`, `hook installed: trusted in Codex`), `cwd` a throwaway folder. `codex exec --skip-git-repo-check --sandbox danger-full-access --json "We decided to tag releases only as v<version> on main. Keep this decision for later sessions."`:

```
msg  I'll use the bilbo note skill to save this release-tagging decision for later sessions.
cmd  cat .../plugins/cache/bilbo/bilbo/0.4.0/skills/note/SKILL.md
cmd  command -v bilbo                                   -> built bilbo
cmd  bilbo recall --limit 5 -- 'release tags'           -> exit 1
cmd  bilbo recall --limit 5 -- 'tagging releases main'  -> exit 1
cmd  bilbo new decision release-tags                    -> exit 0
cmd  cat .../store/notes/decision-release-tags.md
cmd  bilbo check                                        -> exit 0
msg  Created [release-tags decision](.../decision-release-tags.md) ... `bilbo check` passed; 0 outstanding lines for other notes.
```

The note was written: `decision-release-tags.md` with an `id`, `created`, the title "Release tags" and a two-line body (no `sources`, no extra keys). A separate `bilbo check` afterwards exited 0 with no output. The model ran the two recall queries (the skill's one retry) before creating, and the reply gave the path.

`auth.json`: the throwaway copy and `~/.codex/auth.json` both hash to `ecbdf9971d7d...` after the run, so the token did not rotate. The copy is deleted; nothing was written to `~/.codex`.
The `n5` plugin was removed afterwards with `bilbo setup --remove` (`hook removed`).

### Remove

```
$ bilbo setup --remove --yes
codex removed
hook removed
```

`config.toml` then held no `bilbo@bilbo:` key.
