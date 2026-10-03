# add-setup smoke test

Run on rivendell (macOS) on 2026-10-02 with `target/debug/bilbo` of the add-setup worktree, Claude Code 2.1.288 and codex-cli 0.155.1. `$T` is a scratch folder; every run carried `BILBO_HOME=$T/store`, `BILBO_CONFIG=$T/conf/config`, `XDG_STATE_HOME=$T/state`, `XDG_CACHE_HOME=$T/cache`, `CLAUDE_CONFIG_DIR=$T/claude` and `CODEX_HOME=$T/codex`. `$B` is the bilbo binary. The embedder is bagend over `ssh -L 18081:127.0.0.1:8081`; its warm `/v1/embeddings` answered `200` in 0.09 s.

## 4.3 The timer on macOS

Deviation from the plan: `bilbo new` writes a title-only note, which has no passage, so the first `kickstart` logged `embedded 0, kept 0, dropped 0`. A body line was appended to the note and `kickstart -k` ran again; it logged `embedded 1, kept 0, dropped 0`.

```
$ $B setup --yes --embedder-url http://127.0.0.1:18081 --embedder-model qwen3-embedding-0.6b --no-plugin
store created: $T/store/notes
config written: $T/conf/config
key skipped: local embedder
embedder ok: 1024 dimensions
claude skipped: --no-plugin
codex skipped: --no-plugin
timer installed: every 15 min
exit 0
$ $B new plan smoke-test --title Smoke test
$T/store/notes/plan-smoke-test.md
exit 0
$ plutil -p /Users/delucca/Library/LaunchAgents/io.github.delucca.bilbo.index.plist
{
  "EnvironmentVariables" => {
    "BILBO_CONFIG" => "$T/conf/config"
    "BILBO_HOME" => "$T/store"
    "XDG_CACHE_HOME" => "$T/cache"
    "XDG_STATE_HOME" => "$T/state"
  }
  "Label" => "io.github.delucca.bilbo.index"
  "ProgramArguments" => [
    0 => "$B"
    1 => "index"
  ]
  "RunAtLoad" => false
  "StandardErrorPath" => "$T/state/bilbo/index.log"
  "StandardOutPath" => "$T/state/bilbo/index.log"
  "StartInterval" => 900
}
exit 0
$ launchctl print gui/501/io.github.delucca.bilbo.index | grep -E ...
	path = /Users/delucca/Library/LaunchAgents/io.github.delucca.bilbo.index.plist
	state = not running
	stdout path = $T/state/bilbo/index.log
	stderr path = $T/state/bilbo/index.log
		XDG_CACHE_HOME => $T/cache
		XDG_STATE_HOME => $T/state
		BILBO_CONFIG => $T/conf/config
		BILBO_HOME => $T/store
	run interval = 900 seconds
$ launchctl kickstart gui/501/io.github.delucca.bilbo.index
exit 0
$ cat $T/state/bilbo/index.log
embedded 0, kept 0, dropped 0
exit 0
$ $B setup --yes --embedder-url http://127.0.0.1:18081 --embedder-model qwen3-embedding-0.6b --no-plugin
store kept: $T/store/notes
config kept: $T/conf/config
key skipped: local embedder
embedder skipped: config kept
claude skipped: --no-plugin
codex skipped: --no-plugin
timer kept
exit 0
$ (appended a body line to plan-smoke-test.md: a title-only note has no passage)
$ launchctl kickstart -k gui/501/io.github.delucca.bilbo.index
exit 0
$ cat $T/state/bilbo/index.log
embedded 0, kept 0, dropped 0
embedded 1, kept 0, dropped 0
exit 0
$ bilbo setup --remove --yes
store skipped: kept $T/store/notes
config skipped: kept $T/conf/config
key skipped: no key file
claude skipped: not installed
codex skipped: not installed
timer removed
exit 0
$ launchctl print gui/501/io.github.delucca.bilbo.index
Bad request.
Could not find service "io.github.delucca.bilbo.index" in domain for user gui: 501
exit 113
plist-gone
```

## 7.2 The wizard

Paths 2 to 6 ran against the config path 1 wrote, not an emptied machine (path 2 declines and path 5 cancels, so nothing changed before path 3). `lib.tcl` is shared by every script; each `snd` waits 800 ms first, because a key sent right after a prompt title can be lost or echoed by the tty.

```tcl
set timeout 30
log_user 0
proc want {text} {
  global spawn_id timeout
  expect -ex $text {} timeout {puts "TIMEOUT waiting for: $text"; exit 2} eof {puts "EOF waiting for: $text"; exit 3}
}
proc snd {s} { after 800; send -- $s }
proc finish {} { global spawn_id; catch {expect eof}; lassign [wait] pid id os code; puts "exit=$code"; exit 0 }
proc start {name cmd} {
  global env spawn_id
  set env(TERM) xterm-256color
  log_file -a -noappend $::env(T)/$name.transcript
  spawn sh -c "exec $cmd > $::env(T)/$name.report"
}
```

### Path 1. Keyword only

```tcl
source lib.tcl
start p1 "$env(B) setup"
want "Which embedder"
send "\r"
want "Install the bilbo plugin in"
send "\r"
want "Apply these changes"
send "y"
want "The report follows"
finish
```

Transcript (ANSI stripped, blank lines dropped):

```
spawn sh -c exec $B setup > $T/p1.report
┌  bilbo setup
│
●  Notes go to $T/store/notes. Set BILBO_HOME to put them elsewhere.
│  
◆  Which embedder should recall use?
│  ● No embedder (keyword search only)
│  ○ Ollama on this machine 
│  ○ OpenAI 
│  ○ Another OpenAI-compatible URL 
└  
◇  Which embedder should recall use?
│  No embedder 
│
◆  Install the bilbo plugin in
│  ◼ Claude Code (/Users/delucca/.local/bin/claude)
│  ◼ Codex 
└  
◇  Install the bilbo plugin in
│  Claude Code 
│  Codex 
│
◇  Setup will ─────────────────────────────────────────────────────────────────╮
│                                                                              │
│  Create the store folder /private/tmp/claude-501/-Users-delucca-Developer/   │
│  ef040f33-b9af-4558-b6d1-2796a702aeda/scratchpad/S/smoke.4xjOZ9/store/notes  │
│  Write the config /private/tmp/claude-501/-Users-delucca-Developer/          │
│  ef040f33-b9af-4558-b6d1-2796a702aeda/scratchpad/S/smoke.4xjOZ9/conf/config  │
│  Search by keywords only                                                     │
│  Install the bilbo plugin in Claude Code from delucca/bilbo#v0.1.0           │
│  Install the bilbo plugin in Codex from delucca/bilbo#v0.1.0                 │
├──────────────────────────────────────────────────────────────────────────────╯
│
◆  Apply these changes?
│  ● Yes  / ○ No 
└  
◇  Apply these changes?
│  Yes 
│
└  Done. The report follows.
```

Report file:

```
store created: $T/store/notes
config written: $T/conf/config
key skipped: no embedder
embedder skipped: none configured
claude installed: delucca/bilbo#v0.1.0
codex installed: delucca/bilbo#v0.1.0
timer skipped: no embedder
```

exit=0. Every report line matches the step pattern.

### Path 2. OpenAI, pasted dummy key, 401

```tcl
source lib.tcl
start p2 "$env(B) setup"
want "Which embedder"
snd "jj\r"
want "Model name"
snd "\r"
want "advanced settings"
snd "n"
want "API key come from"
snd "\r"
want "API key"
snd "[redacted]\r"
want "401"
want "What now"
snd "jj\r"
want "Install the bilbo plugin in"
snd "\r"
want "Apply these changes"
snd "n"
want "Cancelled"
finish
```

Transcript (ANSI stripped, blank lines dropped):

```
spawn sh -c exec $B setup > $T/p2.report
┌  bilbo setup
│
●  Notes go to $T/store/notes. Set BILBO_HOME to put them elsewhere.
│  
◆  Which embedder should recall use?
│  ● No embedder (keyword search only)
│  ○ Ollama on this machine 
│  ○ OpenAI 
│  ○ Another OpenAI-compatible URL 
└  
◆  Which embedder should recall use?
│  ○ No embedder 
│  ● Ollama on this machine (not found on localhost:11434)
│  ○ OpenAI 
│  ○ Another OpenAI-compatible URL 
└  
◆  Which embedder should recall use?
│  ○ No embedder 
│  ○ Ollama on this machine 
│  ● OpenAI (api.openai.com)
│  ○ Another OpenAI-compatible URL 
└  
◇  Which embedder should recall use?
│  OpenAI 
│
◆  Model name
│  text-embedding-3-small (default)
└  
◇  Model name
│  text-embedding-3-small
│
◆  Change advanced settings (the query prefix)?
│  ○ Yes  / ● No 
└  
◇  Change advanced settings (the query prefix)?
│  No 
│
◆  Where does the API key come from?
│  ○ An environment variable 
│  ○ A file 
│  ● Paste it now (saved to $T/conf/token, readable only by you)
│  ○ No key 
└  
◇  Where does the API key come from?
│  Paste it now 
│
◆  API key
│   
└  
[redacted]
◆  API key
│  ▪ 
└  
◆  API key
│  ▪▪ 
└  
◆  API key
│  ▪▪▪ 
└  
◆  API key
│  ▪▪▪▪ 
└  
◆  API key
│  ▪▪▪▪▪ 
└  
◆  API key
│  ▪▪▪▪▪▪ 
└  
◆  API key
│  ▪▪▪▪▪▪▪ 
└  
◆  API key
│  ▪▪▪▪▪▪▪▪ 
└  
◆  API key
│  ▪▪▪▪▪▪▪▪▪ 
└  
◆  API key
│  ▪▪▪▪▪▪▪▪▪▪ 
└  
◆  API key
│  ▪▪▪▪▪▪▪▪▪▪▪ 
└  
◆  API key
│  ▪▪▪▪▪▪▪▪▪▪▪▪ 
└  
◆  API key
│  ▪▪▪▪▪▪▪▪▪▪▪▪▪ 
└  
◆  API key
│  ▪▪▪▪▪▪▪▪▪▪▪▪▪▪ 
└  
◆  API key
│  ▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪ 
└  
◆  API key
│  ▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪ 
└  
◆  API key
│  ▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪ 
└  
◆  API key
│  ▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪ 
└  
◆  API key
│  ▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪ 
└  
◇  API key
│  ▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪
│
◒  Checking text-embedding-3-small at https://api.openai.com                    ◐  Checking text-embedding-3-small at https://api.openai.com                    ◓  Checking text-embedding-3-small at https://api.openai.com                    ◒  Checking text-embedding-3-small at https://api.openai.com                    ◐  Checking text-embedding-3-small at https://api.openai.com                    ◓  Checking text-embedding-3-small at https://api.openai.com                    ▲  embedder https://api.openai.com answered 401                                 │                                                                               ◓  Checking text-embedding-3-small at https://api.openai.com                    ◆  The embedder check failed. What now?
│  ● Try again 
│  ○ Change the embedder settings 
│  ○ Continue with keyword search only 
└  
◆  The embedder check failed. What now?
│  ○ Try again 
│  ● Change the embedder settings 
│  ○ Continue with keyword search only 
└  
◆  The embedder check failed. What now?
│  ○ Try again 
│  ○ Change the embedder settings 
│  ● Continue with keyword search only 
└  
◇  The embedder check failed. What now?
│  Continue with keyword search only 
│
◆  Install the bilbo plugin in
│  ◼ Claude Code (/Users/delucca/.local/bin/claude)
│  ◼ Codex 
└  
◇  Install the bilbo plugin in
│  Claude Code 
│  Codex 
│
◇  Setup will ─────────────────────────────────────────────────────────────────╮
│                                                                              │
│  Keep the store folder /private/tmp/claude-501/-Users-delucca-Developer/     │
│  ef040f33-b9af-4558-b6d1-2796a702aeda/scratchpad/S/smoke.4xjOZ9/store/notes  │
│  Keep the config /private/tmp/claude-501/-Users-delucca-Developer/ef040f33-  │
│  b9af-4558-b6d1-2796a702aeda/scratchpad/S/smoke.4xjOZ9/conf/config           │
│  Search by keywords only                                                     │
│  Keep the bilbo plugin in Claude Code                                        │
│  Keep the bilbo plugin in Codex                                              │
├──────────────────────────────────────────────────────────────────────────────╯
│
◆  Apply these changes?
│  ● Yes  / ○ No 
└  
◇  Apply these changes?
│  No 
│
└  Cancelled. Nothing changed.
bilbo: setup cancelled; nothing changed
```

Report file:

```
```

exit=1 (cancelled at the summary). `grep -c` for the dummy key on the report: 0. On the raw transcript: 1, the pty echoing the key line in clear once, between the `API key` title and the masked redraw; it is replaced by `[redacted]` above. `$T/conf/token` does not exist. The first two attempts of this path hung on `expect` (a matcher bug in `want`, then a lost Enter), and no key was typed in them past the prompt.

The first run's raw transcript held the key in clear (the tty echoed it between key reads); it was redacted above. The cause and the fix are in design.md's cliclack section, and section 7.2 below ("Rerun after the echo fix") shows the rerun.

### Path 3. bagend over the tunnel

```tcl
source lib.tcl
start p3 "$env(B) setup"
want "Which embedder"
snd "jjj\r"
want "Embedder URL"
snd "http://127.0.0.1:18081\r"
want "Model name"
snd "qwen3-embedding-0.6b\r"
want "advanced settings"
snd "n"
set t0 [clock milliseconds]
want "1024-dimensional"
set t1 [clock milliseconds]
puts "check_ms=[expr {$t1-$t0}]"
want "Install the bilbo plugin in"
snd "\r"
want "Keep the index fresh"
snd "\r"
want "Apply these changes"
snd "y"
want "Run the first index now"
snd "y"
want "embedded"
finish
```

Transcript (ANSI stripped, blank lines dropped):

```
spawn sh -c exec $B setup > $T/p3.report
┌  bilbo setup
│
●  Notes go to $T/store/notes. Set BILBO_HOME to put them elsewhere.
│  
◆  Which embedder should recall use?
│  ● No embedder (keyword search only)
│  ○ Ollama on this machine 
│  ○ OpenAI 
│  ○ Another OpenAI-compatible URL 
└  
◆  Which embedder should recall use?
│  ○ No embedder 
│  ● Ollama on this machine (not found on localhost:11434)
│  ○ OpenAI 
│  ○ Another OpenAI-compatible URL 
└  
◆  Which embedder should recall use?
│  ○ No embedder 
│  ○ Ollama on this machine 
│  ● OpenAI (api.openai.com)
│  ○ Another OpenAI-compatible URL 
└  
◆  Which embedder should recall use?
│  ○ No embedder 
│  ○ Ollama on this machine 
│  ○ OpenAI 
│  ● Another OpenAI-compatible URL 
└  
◇  Which embedder should recall use?
│  Another OpenAI-compatible URL 
│
◆  Embedder URL
│   
└  
◆  Embedder URL
│  h 
└  
◆  Embedder URL
│  ht 
└  
◆  Embedder URL
│  htt 
└  
◆  Embedder URL
│  http 
└  
◆  Embedder URL
│  http: 
└  
◆  Embedder URL
│  http:/ 
└  
◆  Embedder URL
│  http:// 
└  
◆  Embedder URL
│  http://1 
└  
◆  Embedder URL
│  http://12 
└  
◆  Embedder URL
│  http://127 
└  
◆  Embedder URL
│  http://127. 
└  
◆  Embedder URL
│  http://127.0 
└  
◆  Embedder URL
│  http://127.0. 
└  
◆  Embedder URL
│  http://127.0.0 
└  
◆  Embedder URL
│  http://127.0.0. 
└  
◆  Embedder URL
│  http://127.0.0.1 
└  
◆  Embedder URL
│  http://127.0.0.1: 
└  
◆  Embedder URL
│  http://127.0.0.1:1 
└  
◆  Embedder URL
│  http://127.0.0.1:18 
└  
◆  Embedder URL
│  http://127.0.0.1:180 
└  
◆  Embedder URL
│  http://127.0.0.1:1808 
└  
◆  Embedder URL
│  http://127.0.0.1:18081 
└  
◇  Embedder URL
│  http://127.0.0.1:18081
│
◆  Model name
│   
└  
◆  Model name
│  q 
└  
◆  Model name
│  qw 
└  
◆  Model name
│  qwe 
└  
◆  Model name
│  qwen 
└  
◆  Model name
│  qwen3 
└  
◆  Model name
│  qwen3- 
└  
◆  Model name
│  qwen3-e 
└  
◆  Model name
│  qwen3-em 
└  
◆  Model name
│  qwen3-emb 
└  
◆  Model name
│  qwen3-embe 
└  
◆  Model name
│  qwen3-embed 
└  
◆  Model name
│  qwen3-embedd 
└  
◆  Model name
│  qwen3-embeddi 
└  
◆  Model name
│  qwen3-embeddin 
└  
◆  Model name
│  qwen3-embedding 
└  
◆  Model name
│  qwen3-embedding- 
└  
◆  Model name
│  qwen3-embedding-0 
└  
◆  Model name
│  qwen3-embedding-0. 
└  
◆  Model name
│  qwen3-embedding-0.6 
└  
◆  Model name
│  qwen3-embedding-0.6b 
└  
◇  Model name
│  qwen3-embedding-0.6b
│
◆  Change advanced settings (the query prefix)?
│  ○ Yes  / ● No 
└  
◇  Change advanced settings (the query prefix)?
│  No 
│
◒  Checking qwen3-embedding-0.6b at http://127.0.0.1:18081                      ◐  Checking qwen3-embedding-0.6b at http://127.0.0.1:18081                      ◓  Checking qwen3-embedding-0.6b at http://127.0.0.1:18081                      ◇  http://127.0.0.1:18081 answered with 1024-dimensional vectors                │                                                                               ◓  Checking qwen3-embedding-0.6b at http://127.0.0.1:18081                      ◆  Install the bilbo plugin in
│  ◼ Claude Code (/Users/delucca/.local/bin/claude)
│  ◼ Codex 
└  
◇  Install the bilbo plugin in
│  Claude Code 
│  Codex 
│
◆  Keep the index fresh in the background?
│  ● Every 15 minutes 
│  ○ Another interval 
│  ○ No timer 
└  
◇  Keep the index fresh in the background?
│  Every 15 minutes 
│
◇  Setup will ─────────────────────────────────────────────────────────────────╮
│                                                                              │
│  Keep the store folder /private/tmp/claude-501/-Users-delucca-Developer/     │
│  ef040f33-b9af-4558-b6d1-2796a702aeda/scratchpad/S/smoke.4xjOZ9/store/notes  │
│  Update the config /private/tmp/claude-501/-Users-delucca-Developer/         │
│  ef040f33-b9af-4558-b6d1-2796a702aeda/scratchpad/S/smoke.4xjOZ9/conf/config  │
│  (the old one becomes config.bak)                                            │
│  Embed with qwen3-embedding-0.6b at http://127.0.0.1:18081 (1024             │
│  dimensions)                                                                 │
│  Keep the bilbo plugin in Claude Code                                        │
│  Keep the bilbo plugin in Codex                                              │
│  Run bilbo index every 15 min (launchd agent io.github.delucca.bilbo.index)  │
├──────────────────────────────────────────────────────────────────────────────╯
│
◆  Apply these changes?
│  ● Yes  / ○ No 
└  
◇  Apply these changes?
│  Yes 
│
◆  Run the first index now? The store holds 0 notes.
│  ● Yes  / ○ No 
└  
◇  Run the first index now? The store holds 0 notes.
│  Yes 
│
◒  Indexing 0 notes                                                             ◐  Indexing 0 notes                                                             ◇  embedded 0, kept 0, dropped 0                                                │                                                                               ◐  Indexing 0 notes                                                             └  Done. The report follows.
```

Report file:

```
store kept: $T/store/notes
config updated: $T/conf/config
key skipped: local embedder
embedder ok: 1024 dimensions
claude kept
codex kept
timer installed: every 15 min
```

exit=0. Embedder check time, from sending the last answer to the `1024-dimensional` line: 181 ms (warm model). Limit: 15 s. `config.bak` equals path 1's config (diff empty); the config holds the Qwen prefix line. The store held 0 notes, so the first index embedded 0.

### Path 4. Rerun

```tcl
source lib.tcl
start p4 "$env(B) setup"
want "Which embedder"
snd "\r"
want "Embedder URL"
snd "\r"
want "Model name"
snd "\r"
want "advanced settings"
snd "n"
want "Install the bilbo plugin in"
snd "\r"
want "Keep the index fresh"
snd "\r"
want "Apply these changes"
snd "y"
want "Run the first index now"
snd "n"
finish
```

Transcript (ANSI stripped, blank lines dropped):

```
spawn sh -c exec $B setup > $T/p4.report
┌  bilbo setup
│
●  Notes go to $T/store/notes. Set BILBO_HOME to put them elsewhere.
│  
◆  Which embedder should recall use?
│  ○ No embedder 
│  ○ Ollama on this machine 
│  ○ OpenAI 
│  ● Another OpenAI-compatible URL 
└  
◇  Which embedder should recall use?
│  Another OpenAI-compatible URL 
│
◆  Embedder URL
│  http://127.0.0.1:18081 (default)
└  
◇  Embedder URL
│  http://127.0.0.1:18081
│
◆  Model name
│  qwen3-embedding-0.6b (default)
└  
◇  Model name
│  qwen3-embedding-0.6b
│
◆  Change advanced settings (the query prefix)?
│  ○ Yes  / ● No 
└  
◇  Change advanced settings (the query prefix)?
│  No 
│
◒  Checking qwen3-embedding-0.6b at http://127.0.0.1:18081                      ◐  Checking qwen3-embedding-0.6b at http://127.0.0.1:18081                      ◇  http://127.0.0.1:18081 answered with 1024-dimensional vectors                │                                                                               ◐  Checking qwen3-embedding-0.6b at http://127.0.0.1:18081                      ◆  Install the bilbo plugin in
│  ◼ Claude Code (/Users/delucca/.local/bin/claude)
│  ◼ Codex 
└  
◇  Install the bilbo plugin in
│  Claude Code 
│  Codex 
│
◆  Keep the index fresh in the background?
│  ● Every 15 minutes 
│  ○ Another interval 
│  ○ No timer 
└  
◇  Keep the index fresh in the background?
│  Every 15 minutes 
│
◇  Setup will ─────────────────────────────────────────────────────────────────╮
│                                                                              │
│  Keep the store folder /private/tmp/claude-501/-Users-delucca-Developer/     │
│  ef040f33-b9af-4558-b6d1-2796a702aeda/scratchpad/S/smoke.4xjOZ9/store/notes  │
│  Keep the config /private/tmp/claude-501/-Users-delucca-Developer/ef040f33-  │
│  b9af-4558-b6d1-2796a702aeda/scratchpad/S/smoke.4xjOZ9/conf/config           │
│  Embed with qwen3-embedding-0.6b at http://127.0.0.1:18081 (1024             │
│  dimensions)                                                                 │
│  Keep the bilbo plugin in Claude Code                                        │
│  Keep the bilbo plugin in Codex                                              │
│  Keep the index timer                                                        │
├──────────────────────────────────────────────────────────────────────────────╯
│
◆  Apply these changes?
│  ● Yes  / ○ No 
└  
◇  Apply these changes?
│  Yes 
│
◆  Run the first index now? The store holds 0 notes.
│  ● Yes  / ○ No 
└  
◇  Run the first index now? The store holds 0 notes.
│  No 
│
└  Done. The report follows.
```

Report file:

```
store kept: $T/store/notes
config kept: $T/conf/config
key skipped: local embedder
embedder ok: 1024 dimensions
claude kept
codex kept
timer kept
```

exit=0. mtimes (`stat -f %m`) before and after, identical: config 1790991641, config.bak 1790991134, plist 1790991641.

### Path 5. Ctrl-C

```tcl
source lib.tcl
start p5 "$env(B) setup"
want "Which embedder"
snd "\003"
want "Cancelled"
finish
```

Transcript (ANSI stripped, blank lines dropped):

```
spawn sh -c exec $B setup > $T/p5.report
┌  bilbo setup
│
●  Notes go to $T/store/notes. Set BILBO_HOME to put them elsewhere.
│  
◆  Which embedder should recall use?
│  ○ No embedder 
│  ○ Ollama on this machine 
│  ○ OpenAI 
│  ● Another OpenAI-compatible URL 
└  
■  Which embedder should recall use?
│  Another OpenAI-compatible URL 
└  Operation cancelled.
└  Cancelled. Nothing changed.
bilbo: setup cancelled; nothing changed
```

Report file:

```
```

exit=1. `find` and `ls -lR` of `$T` (without the transcripts) and `ls -l` of the plist before and after: identical. The machine was not empty: path 3 had left the config, the timer and both plugins.

### Path 6. Remove

```tcl
source lib.tcl
start p6 "$env(B) setup --remove"
want "Remove these"
snd "y"
finish
```

Transcript (ANSI stripped, blank lines dropped):

```
spawn sh -c exec $B setup --remove > $T/p6.report
┌  bilbo setup --remove
│
◇  Setup will remove ───────────────────────────╮
│                                               │
│  Remove the bilbo plugin from Claude Code     │
│  Remove the bilbo plugin from Codex           │
│  Remove the index timer                       │
│  Keep the store, the config and the key file  │
├───────────────────────────────────────────────╯
│
◆  Remove these?
│  ○ Yes  / ● No 
└  
◇  Remove these?
│  Yes 
│
└  Done. The report follows.
```

Report file:

```
store skipped: kept $T/store/notes
config skipped: kept $T/conf/config
key skipped: no key file
claude removed
codex removed
timer removed
```

exit=0. After it: `launchctl print gui/501/io.github.delucca.bilbo.index` exits 113, the plist is gone, `claude plugin marketplace list --json` prints `[]` and `codex plugin marketplace list --json` prints `{"marketplaces": []}`.

## 7.2 Rerun after the echo fix

Run on rivendell (macOS) on 2026-10-02 with `target/debug/bilbo` built from the fixed code (the `password` adapter clears ECHO on /dev/tty for the whole prompt). `$T` is a fresh scratch folder with the same six variables as above, exported from one file and asserted set before every run. No launchd agent and no ssh tunnel. Before and after: `launchctl print gui/501/io.github.delucca.bilbo.index` exits 113 and the plist does not exist.

Each script spawns `sh -c "$B setup > $T/<name>.report; rc=$?; stty -a > $T/<name>.stty 2>&1; exit $rc"`, so `stty -a` reads the same pty after bilbo exits and `exit=` is bilbo's code. `lib.tcl` differs from the first run's: `want` is the flat `expect -timeout 30 -re $text {} timeout {...} eof {...}` form, `snd` sleeps 600 ms before each send, and `drain` is `expect -timeout 3 -re {NOMATCH-DRAIN}`.

```
$ expect p1.tcl            # path 1, as in the plan
```

Path 1 report (all seven lines as in the first run, `claude installed` and `codex installed` from `delucca/bilbo#v0.1.0`):

```
store created: $T/store/notes
config written: $T/conf/config
key skipped: no embedder
embedder skipped: none configured
claude installed: delucca/bilbo#v0.1.0
codex installed: delucca/bilbo#v0.1.0
timer skipped: no embedder
```

exit=0, 0 matches of the key fragment in its transcript, tty `echo` after.

### Path 2, three key-sending modes

```tcl
# usage: expect p2.tcl <name> <burst|slow|trickle>
source lib.tcl
lassign $argv name mode
start $name "$env(B) setup"
want {Which embedder}
snd "jj\r"
want {Model name}
snd "\r"
want {advanced settings}
snd "n"
want {API key come from}
snd "\r"
want {API key\r}
sleep 0.6
set key "[redacted]\r"   ;# the 19-byte dummy key plus Enter
if {$mode eq "burst"} { send -- $key } elseif {$mode eq "slow"} { foreach c [split $key ""] { send -- $c; sleep 0.15 } } else { foreach c [split $key ""] { send -- $c } }
want {401}
drain
want {What now}
snd "jj\r"
want {Install the bilbo plugin in}
snd "\r"
want {Apply these changes}
snd "n"
want {Cancelled}
finish
```

`start` and `finish` are `lib.tcl`'s: `start` opens `log_file -a -noappend $T/<name>.transcript` (the RAW transcript, ANSI kept) and spawns the shell above. The three runs: `expect p2.tcl p2a burst` (the key in one `send`), `expect p2.tcl p2b slow` (one char every 150 ms) and `expect p2.tcl p2c trickle` (one `send` per byte, no delay).

| mode | full key in raw transcript | key fragment in raw transcript | full key in report | key fragment in report | exit | tty after |
| --- | --- | --- | --- | --- | --- | --- |
| burst | 0 | 0 | 0 | 0 | 1 | echo |
| slow (150 ms) | 0 | 0 | 0 | 0 | 1 | echo |
| trickle | 0 | 0 | 0 | 0 | 1 | echo |

Counts come from `grep -ac` for the full dummy key (`sk-` plus the fragment) and `LC_ALL=C grep -ac` for the fragment (the dummy key without its `sk-` prefix and digits) on the raw files. Every raw transcript has 155 lines and one `answered 401` line, so the run reached the check. The report of each run is empty (the wizard cancelled at the summary), `$T/conf/token` does not exist after any of them, and the last lines of each transcript are `Cancelled. Nothing changed.` and `bilbo: setup cancelled; nothing changed`. The masked key shows only as `▪` marks, one per byte.

### The tty after each run

`stty -a` run by the spawned shell after bilbo exited, `lflags` line, identical for path 1, the three path 2 runs and both Ctrl-C runs:

```
lflags: icanon isig iexten echo echoe -echok echoke -echonl echoctl
```

`echo` is on in all six; none shows `-echo`.

### Path 5 and Ctrl-C at the key prompt

| run | trigger | exit | report | tty after | `ls -lR` before/after |
| --- | --- | --- | --- | --- | --- |
| p5 | Ctrl-C (`\003`) at `Which embedder` | 1 | empty | echo | identical |
| p5b | trickle `sk-smoke-dum` (12 bytes), drain, Ctrl-C at `API key` | 1 | empty | echo | identical |

The `ls -lR` snapshot covers `claude`, `codex`, `conf`, `store`, `state` and `cache` under `$T` and the plist path (382 lines, identical before and after both runs). Both transcripts end with `Operation cancelled.`, `Cancelled. Nothing changed.` and `bilbo: setup cancelled; nothing changed`; the p5b one shows twelve `▪` and 0 matches of the 12-byte fragment.

### Cleanup

```
$ $B setup --remove --yes
store skipped: kept $T/store/notes
config skipped: kept $T/conf/config
key skipped: no key file
claude removed
codex removed
timer skipped: not installed
exit 0
$ rm -rf $T            # gone
$ launchctl print gui/501/io.github.delucca.bilbo.index    # exit 113, before and after the whole run
$ ls ~/Library/LaunchAgents/io.github.delucca.bilbo.index.plist    # no such file
```

### Deviations from the plan

- The first loop over the three modes ran under zsh, which does not word-split a string, so `expect` got a wrong name and exited at `Which embedder` before any bilbo run; no key was sent. The loop was redone with explicit arguments.
- `exit=` first read 0 because `sh -c` ended with `stty`; the spawn line now passes bilbo's `rc` through, and all path 2 and Ctrl-C runs were redone (the first path 2 attempt's files were overwritten).
