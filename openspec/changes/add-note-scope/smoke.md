# add-note-scope smoke test (task 7.2)

Run on rivendell (macOS) on 2026-10-05 from the snapshot `<snap>` = `/Users/delucca/Developer/delucca/.worktrees/bilbo.add-note-scope-smoke` (detached HEAD f6d6c6e), built with `nix develop <snap> -c sh -c 'export CARGO_TARGET_DIR=<snap>/target; cargo build --locked'` (bilbo 0.10.0). Claude Code 2.1.289, Codex 0.155.1. Everything lives under `$T` = `/private/tmp/claude-501/-Users-delucca-Developer/f83fca1c-9695-488c-9ca7-d0de5d45e48a/scratchpad/smoke`; nothing touched the real `~/.local/share/bilbo`, `~/.config/bilbo`, `~/.claude` settings or `~/.codex`. Raw streams: `$T/out/s1.jsonl`, `s3.jsonl`, `c1.jsonl`, `c2.jsonl`.

## Setup

`$T/config`:

```
scope.personal.paths = $T/personal
scope.work.paths = $T/work
scope.work.marks = acme
```

Environment of every run: `BILBO_HOME=$T/store`, `BILBO_CONFIG=$T/config`, `XDG_CACHE_HOME=$T/cache`, `<snap>/target/debug` first on `PATH`. The store did not exist at the start (`bilbo check` printed `bilbo: no store at $T/store`, exit 1); `bilbo new` made it. Before any run `bilbo scope` printed:

```
personal	0 notes	sync off	embedder any	paths $T/personal
work	0 notes	sync off	embedder any	paths $T/work
(unassigned)	0 notes	embedder any
```

## Summary

| Step | Expectation | Result |
|---|---|---|
| 1 Claude Code from `$T/work` | `scope: work`, no question | pass: `scope: work`, no question |
| 2 Claude Code from `$T/outside` (`-p`) | one question, cannot be answered | pass with a finding: note left unassigned, one question in the final reply, asked after the body was written |
| 3 `codex exec` from `$T/outside` | no question, unassigned, report says `unassigned` | pass with a finding: no answer possible, note unassigned, report says unassigned; the model did try to ask first |

## Step 1: Claude Code from the work folder

```
cd $T/work && claude -p --model sonnet --setting-sources "" --plugin-dir <snap>/plugins/bilbo --output-format stream-json --verbose --include-hook-events --max-turns 15 "note this gotcha: the acme deploy needs the staging token refreshed first" </dev/null
```

Tool calls: `Skill bilbo:note`; `command -v bilbo`; `bilbo recall --limit 5 -- 'acme deploy staging token refresh'` (exit 1, no store); `bilbo scope`; `bilbo new gotcha acme-deploy-staging-token --title 'Acme deploy needs a fresh staging token'` (no `--scope`, since the user named none); Read; Edit; `bilbo check` (no output). `permission_denials: []`. No question was asked.

The new file's frontmatter:

```
---
id: 01M4505BPNM9PH92GRV0WDRR8Y
created: 2026-10-05T00:01-03:00
scope: work
---
```

`bilbo check` afterwards: exit 0, empty.

Final report:

> I created the note at `$T/store/notes/gotcha-acme-deploy-staging-token.md`.
>
> It says to refresh the staging token before the acme deploy. Its scope is `work`, which `bilbo new` picked from the working directory, and I didn't ask you to confirm it. `bilbo check` came back clean.

## Step 2: Claude Code from outside both paths

```
cd $T/outside && claude -p --model sonnet --setting-sources "" --plugin-dir <snap>/plugins/bilbo --output-format stream-json --verbose --include-hook-events --max-turns 15 "note this gotcha: the globex VPN drops idle connections after 10 minutes" </dev/null
```

Tool calls: `Skill bilbo:note`; `command -v bilbo`; `bilbo recall` (exit 1, no notes match); `bilbo scope`; `bilbo new gotcha globex-vpn-idle-timeout --title 'Globex VPN idle timeout'` (exit 0, stderr `bilbo: no scope for <path>; scopes: personal, work; set one with bilbo scope set <name> <path>`); Read; Edit (body written); `bilbo check`. No `bilbo scope set` ran. The agent asked its one question only in its final reply, because `-p` ends the turn there. `permission_denials: []`.

The note has no `scope:` key (`id: 01M450642VY8HM45KAZ0FMDYVD`). `bilbo check`, run by the agent and again by hand:

```
notes/gotcha-globex-vpn-idle-timeout.md: scope: missing; scopes: personal, work
exit 1
```

Final report:

> Created the gotcha note, but it has no scope yet, so I need one answer from you.
>
> Path: `$T/store/notes/gotcha-globex-vpn-idle-timeout.md`
>
> It records that the Globex VPN drops idle connections after 10 minutes. I also added a short effect section and a keepalive workaround (for example `ServerAliveInterval 60` for SSH). I added the workaround myself; you didn't say it, so edit it out if you don't want it.
>
> The note was created in a directory that matches neither declared scope (`personal` or `work`), so `bilbo check` reports `scope: missing`, and the note is reported `unassigned` until you pick one. Which scope does it belong to? Globex sounds like `work`, but I won't set it without your answer. Once you say, I'll run `bilbo scope set <name>` on the note.
>
> `bilbo check` flagged only this note, so no lines were left for other notes.

Findings: the question came after the body was written, not before it (step 5 says to ask, then write the body). The agent never named `unassigned` as an option to the user, but did report the note as `unassigned`. It did not add a body beyond what the user said except an unrequested workaround, which it disclosed.

## Step 3: codex exec

Setup: throwaway `HOME=$T/home`, `CODEX_HOME=$T/home/.codex`; `PATH` of the built `bilbo`, fake `launchctl` and `systemctl` (`exit 0`), a folder holding only `codex`, and `/usr/bin:/bin`; the built `bilbo` linked into `$HOME/.nix-profile/bin` for Codex's login shell.

```
bilbo setup --yes --no-timer --plugin-source <snap>
  ... codex installed: <snap> / hook installed: trusted in Codex / watch installed: ...
```

`config` was unchanged by setup (diffed against a copy). `codex debug prompt-input hi` listed `bilbo:ingest`, `bilbo:note`, `bilbo:recall` and `bilbo:reference`. `~/.codex/auth.json` (sha256 `ecbdf9971d7d...`) was copied into the throwaway `CODEX_HOME` with mode 600; its hash was the same after the runs.

### First attempt: workspace-write, read-only fallback

```
cd $T/outside && codex exec --skip-git-repo-check --sandbox workspace-write --add-dir $T/store --json "note this gotcha: the initech batch export silently truncates files over 2 GB" </dev/null
```

As in the add-note-skill smoke, the managed requirements allow only ReadOnly and DangerFullAccess, so the sandbox was read-only. The model read the skill, ran `command -v bilbo` and `bilbo recall` (exit 1), then stopped: "I couldn't save the note because this session's filesystem is read-only. No note was created." `notes/` was unchanged.

### Second attempt: danger-full-access, approved once by the user

The user approved this single run, with the throwaway `HOME`, `CODEX_HOME` and `BILBO_HOME`, a scratch cwd and the same prompt. No other danger-full-access run was made.

```
cd $T/outside && codex exec --skip-git-repo-check --sandbox danger-full-access --json "note this gotcha: the initech batch export silently truncates files over 2 GB" </dev/null
```

Commands the model ran: `cat <plugin cache>/skills/note/SKILL.md`; `command -v bilbo`; `bilbo recall --limit 5 -- 'initech batch export'` (exit 1); `bilbo scope` (the `(unassigned)` line showed 1 note); `bilbo new gotcha initech-batch-export --title 'Initech batch export truncates files over 2 GB'` (exit 0, stderr `bilbo: no scope for <path>; scopes: personal, work; ...`); `cat` of the new file; `bilbo check` (exit 1). It ran no `bilbo scope set`.

The model quoted the skill's "ask the user once per run" rule and tried to ask: it printed the question "Which scope should hold the Initech batch export gotcha: work, personal, or unassigned?" with three options, and `stderr` shows `ERROR codex_core::tools::router: error=request_user_input is unavailable in Default mode`. With no answer possible it wrote the note and left it unassigned. A question was therefore attempted, but never reached a person.

The note has no `scope:` key (`id: 01M450EYSJV3C4A73A0H4ZAGPF`), and the body is one line. `bilbo check` afterwards:

```
notes/gotcha-globex-vpn-idle-timeout.md: scope: missing; scopes: personal, work
notes/gotcha-initech-batch-export.md: scope: missing; scopes: personal, work
exit 1
```

Final report:

> Created [gotcha note](<$T/store/notes/gotcha-initech-batch-export.md>): Initech batch export silently truncates files over 2 GB.
>
> Scope: unassigned; check reports missing scope. One check issue remains for another note.

### Cleanup

`bilbo setup --remove --yes` printed `codex removed`, `hook removed`, `watch removed`. The throwaway `$T/home/.codex`, with the copied `auth.json`, was deleted.

## Rerun after the skill edit

The note skill was edited after the runs above: the scope question is the next action after `bilbo new`, before any Edit, and a run that cannot ask must not try. Only step 2 was rerun, against the main worktree `/Users/delucca/Developer/delucca/.worktrees/bilbo.add-note-scope` (HEAD 46094eb, working tree dirty with the edit), with a bilbo built from it (`CARGO_TARGET_DIR=$T/target`, `cargo build --locked`) first on `PATH`, the same `$T/config` and `$T/store`, and Claude Code 2.1.289.

```
cd $T/outside && claude -p --model sonnet --setting-sources "" --plugin-dir /Users/delucca/Developer/delucca/.worktrees/bilbo.add-note-scope/plugins/bilbo --output-format stream-json --verbose --include-hook-events --max-turns 15 "note this gotcha: the umbrella corp nightly sync skips rows with a null updated_at" </dev/null
```

Tool calls, in order: `Skill bilbo:note`; `command -v bilbo`; two `bilbo recall` queries (exit 1, no match); `bilbo scope`; `bilbo new gotcha umbrella-sync-null-updated-at --title 'Umbrella Corp nightly sync skips null updated_at'` (exit 0, stderr `bilbo: no scope for <path>; scopes: personal, work; set one with bilbo scope set <name> <path>`); Read; Edit (body); `bilbo check`. `permission_denials: []`. No `bilbo scope set` ran.

- **When the question came:** never as a tool call, and not between `bilbo new` and the Edit. The agent had no question tool in `-p` mode, so it wrote the body straight after `bilbo new` and put the question only in the final reply. The new wording is therefore followed in its fallback branch ("when the question cannot be delivered, set nothing, write the body and report the note `unassigned`"), but the final reply still asks the user which scope to set, so the run did not know it could not ask until the end. It did not stall or try a question tool.
- **Scope line:** the note has no `scope:` key (`id: 01M45177FE422FEYR3RRKP1045`).
- **`bilbo check`** (the agent's run, and again by hand), exit 1:

```
notes/gotcha-globex-vpn-idle-timeout.md: scope: missing; scopes: personal, work
notes/gotcha-initech-batch-export.md: scope: missing; scopes: personal, work
notes/gotcha-umbrella-sync-null-updated-at.md: scope: missing; scopes: personal, work
```

The first two lines are the notes of the earlier runs.

Final report:

> I created the gotcha note, but it has no scope yet. I couldn't ask you which scope it belongs to, so I set none.
>
> The note is at `$T/store/notes/gotcha-umbrella-sync-null-updated-at.md`.
>
> It says the Umbrella Corp nightly sync skips rows with a null `updated_at`. I also added two suggested actions that you didn't state: backfill `updated_at` on rows that should sync, and check for null `updated_at` first when data looks missing downstream.
>
> `bilbo check` reports `scope: missing` for this note. It reports the same for `gotcha-globex-vpn-idle-timeout.md` and `gotcha-initech-batch-export.md`, which I left alone.
>
> The declared scopes are `personal` and `work`. Umbrella Corp sounds like `work`, but tell me which one. Then I'll run `bilbo scope set` on the note.

Result: pass. The note is unassigned, no question tool was tried, and the report says it could not ask. The agent again added unrequested suggested actions to the body and disclosed them.
