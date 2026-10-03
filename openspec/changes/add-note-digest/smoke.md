# add-note-digest smoke test

Run on rivendell (macOS, Apple M5) on 2026-10-03 with `target/debug/bilbo` built (`cargo build --locked`) in the worktree. Embedder: nixpkgs' llama-cpp 9190 (`/nix/store/17pbdn18...-llama-cpp-9190/bin/llama-server`) in the foreground on 127.0.0.1:18737 with the pinned `Qwen3-Embedding-0.6B-Q8_0.gguf` (639,150,592 bytes, sha256 verified before the run; reused from a scratch folder instead of the plan's curl step), flags `--alias qwen3-embedding-0.6b --embedding --pooling last --ctx-size 4096 --batch-size 4096 --ubatch-size 4096 --parallel 1`; `/health` answered 1 s after start. `$T` is a scratch folder; every bilbo run carried `BILBO_HOME=$T/store`, `BILBO_CONFIG=$T/config`, `XDG_CACHE_HOME=$T/cache`, `XDG_STATE_HOME=$T/state`. The Codex steps added `CODEX_HOME=$T/codex-home`, `HOME=$T/home` and a PATH of the built bilbo, a folder with fake `launchctl` and `systemctl` (they log to `$T/managers.log`), a folder holding only `codex`, and `/usr/bin:/bin`. Pre-check: nothing on 18737 or 18738, `launchctl list | grep -i bilbo` empty. Raw transcripts are in the notebook's `work/add-note-digest/smoke/`.

Config (`$T/config`):

```
embedder.url = http://127.0.0.1:18737
embedder.model = qwen3-embedding-0.6b
embedder.query_prefix = "Instruct: Given a question, retrieve notes that answer it\nQuery: "
digest.log = on
```

## Summary

| Step | Expectation | Result |
|---|---|---|
| 1 store and index | three notes, `bilbo index` embeds them | pass: `embedded 7, kept 0, dropped 0` (7 passages), 0.34 s; `bilbo check` exit 0 |
| 2 digest by meaning, twice | first lists the layout note, second prints nothing, log `ranking: meaning` | pass: 0.11 s then 0.02 s; the first lists only `decision-flat-note-layout.md` |
| 3 noise | `ok thanks` and `hello` print nothing, log `passed` 0 | pass: 0.08 s and 0.01 s, both `passed: 0` |
| 4 stalled embedder | note listed by keywords, run under 1.5 s, `error` names the timeout | pass: 1.21 s, `ranking: keywords`, error `did not answer within 1.2 s` |
| 5 Claude Code end to end | the hook's block reaches the model, the result uses it | partial: the block arrived (hook_response, exit 0) and haiku opened exactly the note it named, but `--max-turns 1` ended the run on that tool call, so there is no final text; see "Deviation" |
| 6 Codex end to end | setup installs and trusts; `codex exec` without the bypass flag records the block as hook context; `--remove` drops the trust | pass: `hook installed`, then `hook kept`; the rollout holds the block as a `developer` message after the user prompt; `hook removed` and no `bilbo@bilbo:` key left |

## Step 1: store and index

```
$ bilbo new gotcha embedder-livelock --title "Embedder livelock on long chunks"   # then appended three paragraphs
$ bilbo new decision flat-note-layout --title "Flat note layout"                   # then appended three paragraphs
$ bilbo new reference release-steps --title "Release steps"                        # then appended a four-step list
/private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/store/notes/gotcha-embedder-livelock.md
/private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/store/notes/decision-flat-note-layout.md
/private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/store/notes/reference-release-steps.md
embedded 7, kept 0, dropped 0
real 0.34
user 0.00
sys 0.00
$ bilbo check
exit 0
```

## Step 2: digest by meaning

The prompt shares only "project" (and function words) with the layout note, whose text says "Every note lives directly in the notes folder, one Markdown file per topic".

```
$ printf '%s' '{"session_id":"smoke","prompt":"Why do we keep all our documents in a single directory instead of nesting them by project?"}' | /usr/bin/time -p bilbo digest   # twice
--- run 1
<!-- bilbo digest: 1 of 1 notes -->
Notes that may bear on this prompt (open the file to read more):
- /private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/store/notes/decision-flat-note-layout.md:8 (decision, 2026-10-03T15:11-03:00) Flat note layout > Decision: Every note lives directly in the notes folder, one Markdown file per topic, named after its kind and topic. There are no subfolders per project or per year.
real 0.11
user 0.00
sys 0.00
exit 0
--- run 2
real 0.02
user 0.00
sys 0.00
exit 0
```

The log after the two runs (`digest.jsonl`):

```
{"elapsed_ms":107,"passed":1,"prompt":"Why do we keep all our documents in a single directory instead of nesting them by project?","ranking":"meaning","session":"smoke","shown":["/private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/store/notes/decision-flat-note-layout.md"],"time":"2026-10-03T15:11:15-03:00"}
{"elapsed_ms":14,"passed":1,"prompt":"Why do we keep all our documents in a single directory instead of nesting them by project?","ranking":"meaning","session":"smoke","shown":[],"time":"2026-10-03T15:11:15-03:00"}
```

## Step 3: noise

```
--- prompt: ok thanks
real 0.08
user 0.00
sys 0.00
exit 0
--- prompt: hello
real 0.01
user 0.00
sys 0.00
exit 0
{"elapsed_ms":76,"passed":0,"prompt":"ok thanks","ranking":"meaning","session":"noise","shown":[],"time":"2026-10-03T15:11:21-03:00"}
{"elapsed_ms":10,"passed":0,"prompt":"hello","ranking":"meaning","session":"noise","shown":[],"time":"2026-10-03T15:11:21-03:00"}
```

## Step 4: stalled embedder

```
$ python3 -c '<accept on 127.0.0.1:18738, never answer>' &
$ printf '{"session_id":"stall","prompt":"which release steps push the version tag"}' | BILBO_CONFIG=$T/config.stall /usr/bin/time -p bilbo digest   # config.stall = config with port 18738
<!-- bilbo digest: 1 of 1 notes -->
Notes that may bear on this prompt (open the file to read more):
- /private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/store/notes/reference-release-steps.md:8 (reference, 2026-10-03T15:11-03:00) Release steps > Steps: 1. Bump the version in Cargo.toml and in the Codex plugin manifest together, then refresh the lockfile. 2. Merge the bump through a pull request. 3. Tag the commit on main that carries the bump with the letter v and the version, then push only that tag. 4. The release workflow publishes the GitHub R
bilbo: embedder http://127.0.0.1:18738 did not answer within 1.2 s
real 1.21
user 0.00
sys 0.00
{"elapsed_ms":1208,"error":"embedder http://127.0.0.1:18738 did not answer within 1.2 s","passed":1,"prompt":"which release steps push the version tag","ranking":"keywords","session":"stall","shown":["/private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/store/notes/reference-release-steps.md"],"time":"2026-10-03T15:11:29-03:00"}
```

The stderr line is `bilbo: `-prefixed, the block is on stdout, and the run took 1.21 s, the 1.2 s budget plus startup.

## Step 5: Claude Code end to end

```
$ cd $T && env PATH=$W/target/debug:$PATH claude -p "After the release bump is merged, what do I tag and push to publish a release? Answer from the notes the hook gave you and name the file you used." --plugin-dir $W/plugins/bilbo --model haiku --max-turns 1 --output-format stream-json --verbose --include-hook-events > claude.jsonl
real 11.17 s (about 6.4 s of API time)
```

The bilbo hook's `hook_response` (the legacy managed hooks also ran, with empty output):

```
<!-- bilbo digest: 1 of 1 notes -->
Notes that may bear on this prompt (open the file to read more):
- /private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/store/notes/reference-release-steps.md:8 (reference, 2026-10-03T15:11-03:00) Release steps > Steps: 1. Bump the version in Cargo.toml and in the Codex plugin manifest together, then refresh the lockfile. 2. Merge the bump through a pull request. 3. Tag the commit on main that carries the bump with the letter v and the version, then push only that tag. 4. The release workflow publishes the GitHub R
exit_code 0
```

What the model did (`claude.jsonl`): thinking, then one `Read` of `$T/store/notes/reference-release-steps.md`, the note the digest named; the file shows the four release steps. The result line is `subtype: error_max_turns`, `is_error: true`, "Reached maximum number of turns (1)": the one allowed turn was spent on the tool call, so the answer text was never produced.

### Deviation

Plan step 5 expected a `result` that names the note or its content. The plan's `--max-turns 1` cannot allow that when the model chooses to open the file the digest points to. The hook delivery and the model acting on it are shown; the final sentence is not. A rerun with `--max-turns 2` would show it (one more haiku turn, not run: the approval covered one turn).

## Step 6: Codex end to end

```
$ export CODEX_HOME=$T/codex-home HOME=$T/home
$ env PATH=$SPATH bilbo setup --yes --no-timer --plugin-source $W      # twice; real 0.29 s, then 0.05 s
--- setup run 1
store kept: /private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/store/notes
config kept: /private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/config
key skipped: local embedder
model skipped: not asked
server skipped: not asked
embedder skipped: config kept
claude skipped: not found
codex installed: /Users/delucca/Developer/delucca/.worktrees/bilbo.add-note-digest
hook installed: trusted in Codex
timer skipped: --no-timer
real 0.29
user 0.03
sys 0.08
exit 0
--- setup run 2
store kept: /private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/store/notes
config kept: /private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/config
key skipped: local embedder
model skipped: not asked
server skipped: not asked
embedder skipped: config kept
claude skipped: not found
codex kept
hook kept: trusted in Codex
timer skipped: --no-timer
real 0.05
user 0.02
sys 0.02
exit 0
```

`$CODEX_HOME/config.toml` after the first run:

```
[marketplaces.bilbo]
source_type = "local"
source = "/Users/delucca/Developer/delucca/.worktrees/bilbo.add-note-digest"

[plugins."bilbo@bilbo"]
enabled = true

[hooks.state]

[hooks.state."bilbo@bilbo:hooks/hooks.json:user_prompt_submit:0:0"]
trusted_hash = "sha256:25e2d1e2cbe24cbc61bd80f28e947112c2db5245aaf103860d7721932066d0ed"
```

```
$ env PATH=$SPATH timeout 60 codex exec --skip-git-repo-check --json "<same question>" </dev/null     # no --dangerously-bypass-hook-trust; exit 1 after 16 s
{"type":"thread.started","thread_id":"01a102f7-806e-7cd3-bf7b-fd84c2ddd690"}
{"type":"turn.started"}
{"type":"error","message":"Reconnecting... 2/5 (unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: wss://api.openai.com/v1/responses, cf-ray: a44dea0ccfcf4ae9-GRU)"}
{"type":"error","message":"Reconnecting... 3/5 (unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: wss://api.openai.com/v1/responses, cf-ray: a44dea1068218e32-GRU)"}
{"type":"error","message":"Reconnecting... 4/5 (unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: wss://api.openai.com/v1/responses, cf-ray: a44dea16cfdaaf5c-GRU)"}
{"type":"error","message":"Reconnecting... 5/5 (unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: wss://api.openai.com/v1/responses, cf-ray: a44dea21c8f8147b-GRU)"}
{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Falling back from WebSockets to HTTPS transport. unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: wss://api.openai.com/v1/responses, cf-ray: a44d
{"type":"error","message":"Reconnecting... 1/5 (unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: https://api.openai.com/v1/responses, cf-ray: a44dea387ec61f6a-GRU, request id: req_515c193644774469ac2fcd64a1c7982d)"}
{"type":"error","message":"Reconnecting... 2/5 (unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: https://api.openai.com/v1/responses, cf-ray: a44dea3b7970fd72-GRU, request id: req_c056377d81cb423b88cf3792dc8cbf59)"}
{"type":"error","message":"Reconnecting... 3/5 (unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: https://api.openai.com/v1/responses, cf-ray: a44dea401fa8ae87-GRU, request id: req_e9470e1af45a46aba1d6f0cd8c2bc808)"}
{"type":"error","message":"Reconnecting... 4/5 (unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: https://api.openai.com/v1/responses, cf-ray: a44dea4678f61f6a-GRU, request id: req_dacc5ee48fe44202b80b9799dad6d245)"}
{"type":"error","message":"Reconnecting... 5/5 (unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: https://api.openai.com/v1/responses, cf-ray: a44dea52bc1a06c5-GRU, request id: req_26b8a35ea47f4f56a252e101b009763f)"}
{"type":"error","message":"unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: https://api.openai.com/v1/responses, cf-ray: a44dea6948b6a44b-GRU, request id: req_26370039cd5141489fdf544844d03a67"}
{"type":"turn.failed","error":{"message":"unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: https://api.openai.com/v1/responses, cf-ray: a44dea6948b6a44b-GRU, request id: req_26370039cd5141489fdf544844d03a67"}}
```

The turn fails at the API (401, no login), as planned. The rollout (`codex-rollout.jsonl`) holds the block as a `response_item` of role `developer`, right after the user prompt (ordinal 8) and before `task_complete` (ordinal 11), with the `hooks.additional_context` kind in its metadata:

```
ordinal 10, response_item, role developer
<!-- bilbo digest: 1 of 1 notes -->
Notes that may bear on this prompt (open the file to read more):
- /private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/store/notes/reference-release-steps.md:8 (reference, 2026-10-03T15:11-03:00) Release steps > Steps: 1. Bump the version in Cargo.toml and in the Codex plugin manifest together, then refresh the lockfile. 2. Merge the bump through a pull request. 3. Tag the commit on main that carries the bump with the letter v and the version, then push only that tag. 4. The release workflow publishes the GitHub R
```

The new log line (the session id is the Codex thread id):

```
{"elapsed_ms":66,"passed":1,"prompt":"After the release bump is merged, what do I tag and push to publish a release? Answer from the notes the hook gave you and name the file you used.","ranking":"meaning","session":"01a102f7-806e-7cd3-bf7b-fd84c2ddd690","shown":["/private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/store/notes/reference-release-steps.md"],"time":"2026-10-03T15:12:17-03:00"}
```

```
$ env PATH=$SPATH bilbo setup --remove --yes
store skipped: kept /private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/store/notes
config skipped: kept /private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX/config
key skipped: no key file
model skipped: no model
server skipped: not installed
claude skipped: not found
codex removed
hook removed
timer skipped: not installed
$ cat $CODEX_HOME/config.toml      # grep -c 'bilbo@bilbo:' prints 0

[hooks.state]

[projects."/private/tmp/claude-501/-Users-delucca-Developer/054aa191-0231-4203-a43f-9d67b2ee0baa/scratchpad/smoke.tZe8UX"]
trust_level = "trusted"
```

`$T/managers.log` was never created: setup called neither `launchctl` nor `systemctl`.

## Cleanup and real-config checks

- llama-server and the python listener were killed; `lsof` shows nothing on 18737 or 18738.
- `launchctl list | grep -i bilbo` is empty, as before the run; no bilbo plist in `~/Library/LaunchAgents`.
- `~/.codex/config.toml` (mtime 13:04, before the run) holds the user's own nix-installed `bilbo@bilbo` plugin and no `bilbo@bilbo:hooks` key; `~/.config/bilbo/config` is the home-manager symlink; `~/.cache/bilbo` does not exist. None was written.
- The one Claude Code turn ran on the user's login, so a session transcript landed in the real `~/.claude/projects`; no plugin was installed (`--plugin-dir` only).
