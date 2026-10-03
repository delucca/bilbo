# add-local-embedder smoke test

Run on rivendell (macOS, Apple M5) on 2026-10-03 with `target/debug/bilbo` built (`cargo build --locked`) from the R2 snapshot, the single commit `1f06350` of the frozen copy of the worktree. Local embedder: nixpkgs' llama-cpp 9190 (`/nix/store/17pbdn18...-llama-cpp-9190/bin/llama-server`) and Homebrew's `llama.cpp` 0.5.0. `$T` is a scratch folder; every run carried `BILBO_HOME=$T/store`, `BILBO_CONFIG=$T/conf/config`, `XDG_CACHE_HOME=$T/cache`, `XDG_STATE_HOME=$T/state`, and a PATH without `claude` or `codex` (`/usr/bin:/bin` plus llama-server's folder). The launchd agent is the real `io.github.delucca.bilbo.embedder` in the real `~/Library/LaunchAgents`. Pre-check passed: no plist, `launchctl print` failed, nothing on 8737. Raw transcripts are in the notebook's `work/add-local-embedder/smoke/`.

## Summary

| Step | Result |
|---|---|
| 1 network unit test | pass (`model::tests::pinned_url_honors_a_range`, 1 passed) |
| 2 batch, nixpkgs | pass: whole run 28.6 s wall (download + load + check), RSS 1,147,376 KB (1.09 GiB) after the first check, SHA-256 equals the pinned etag |
| 3 index and recall | pass: `embedded 3`; the query shares no word with the note that ranks first |
| 4 rerun | pass: `model kept`, `server kept`, plist mtime unchanged |
| 5 wizard via expect | pass, after one script fix (the local choice is preselected on a rerun, so `j` moved to Ollama); resumed 600,000,000 bytes under the progress bar |
| 6 Homebrew | install and `--llama-server /opt/homebrew/bin/llama-server` pass; the plan's `server updated` step failed on R2 (Misbehavior 1, fixed in fix round 2: three real updates in a row pass, and both update cases pass in the S2 rerun); `brew install` upgraded an existing formula (Misbehavior 2) |
| 7 remove | pass: `server removed`, `model skipped: kept <path>`, plist gone; on R2 `launchctl print` exited 113 only after about a second (Misbehavior 1), and after fix round 2 it exits 113 at once |

Cold first query: 0.30 s from `kickstart -k` to `/health` 200, then 0.011 s for the first embed (model file in the page cache), 0.008 s for the second; well under 15 s. RSS 1.09 GiB (nix) and 1.13 GiB (Homebrew build) with the model loaded after one request: about the wizard's "about 1 GB".

## Misbehavior 1 (FIXED in fix round 2, verified in the S2 rerun below): update or reinstall of a running server fails with `Bootstrap failed: 5`

Fix round 2 made every launchd `bootout` a `bootout --wait`; see "S2 rerun" and `work/add-local-embedder/smoke/fix2-update.txt`. Original finding:

`launchctl bootout` returns while a loaded llama-server is still shutting down (`state = SIGTERMed`, pid alive), so the `bootstrap` that follows fails with `5: Input/output error`. The planner's probe used a 19.7 MB model, whose process exits within 20 ms. It happened three times:

1. Step 6, first run: the nix-path server was running; `setup --embedder-local --llama-server /opt/homebrew/bin/llama-server` (expected `server updated`) exited 1: `launchctl bootstrap gui/501 .../io.github.delucca.bilbo.embedder.plist failed: Bootstrap failed: 5: Input/output error; setup changed nothing else`, report `server failed: ...; see $T/state/bilbo/embedder.log`. Right after it `launchctl print` showed the old job as `state = SIGTERMed`, pid 55327 still there, `rss` 0. The run deleted the files it wrote, so the plist was gone and, a moment later, so was the old service. The same command run again then said `server installed` (not `updated`).
2. Reproduction: the running Homebrew-path server, `setup ... --llama-server <nix path>`: the same failure, exit 1 (the shell line printed `exit 0` is grep's); afterwards no plist and no service.
3. Step 7: `setup --remove --yes` then `launchctl print` immediately printed exit 0 (job still listed while terminating), 113 after about 1 s; port 8737 free. Likewise after the earlier `--remove` in step 5.

So the spec'd "Service follows its inputs" (`server updated`) is not reliable on a real launchd with the real model; a plain reinstall over a running server fails the same way. Cause: `launchctl bootout` without `--wait` does not wait for the old process to exit. Fixed in fix round 2 with `bootout --wait`.

## Misbehavior 2: `brew install llama.cpp` upgraded an installed formula

Homebrew installed `libomp 23.1.2`, `ggml 0.25.3`, `llama.cpp 0.5.0` and also installed `openssl@3 3.6.5` and `ca-certificates 2026-09-25` as dependency upgrades (`==> Upgrading llama.cpp dependency: openssl@3`), next to the old versions (Homebrew keeps the old Cellar directories; it ran no cleanup). The second `comm` of step 6 is therefore not empty. I did not touch them. After uninstalling the three added formulae, `brew list --formula --versions` still differs from the "before" list by exactly those two lines.

## Step 1: network unit test

`cargo test --locked --bin bilbo model:: -- --ignored`: `test model::tests::pinned_url_honors_a_range ... ok`, `1 passed; 0 failed`, 0.99 s.

## Step 2: batch, nixpkgs build

```
$ env PATH=$SMOKEPATH bilbo setup --yes --no-timer --embedder-local   # /usr/bin/time -p: real 28.58 user 2.90 sys 0.91
exit 0
---stdout
store created: $T/store/notes
config written: $T/conf/config
key skipped: local embedder
model installed: $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server installed: 127.0.0.1:8737
embedder ok: 1024 dimensions
claude skipped: not found
codex skipped: not found
timer skipped: --no-timer
---stderr
bilbo: downloading Qwen3-Embedding-0.6B-Q8_0.gguf (639 MB) to $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
real 28.58
user 2.90
sys 0.91
```

The whole run, 639,150,592 bytes through ureq included, took 28.6 s, so the download was about 27 s (about 24 MB/s).

```
$ plutil -p plist
{
  "KeepAlive" => true
  "Label" => "io.github.delucca.bilbo.embedder"
  "ProgramArguments" => [
    0 => "/nix/store/17pbdn18pmzb9zm2z2jf1mv60sr6babc-llama-cpp-9190/bin/llama-server"
    1 => "--model"
    2 => "$T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf"
    3 => "--alias"
    4 => "qwen3-embedding-0.6b"
    5 => "--embedding"
    6 => "--pooling"
    7 => "last"
    8 => "--host"
    9 => "127.0.0.1"
    10 => "--port"
    11 => "8737"
    12 => "--ctx-size"
    13 => "4096"
    14 => "--batch-size"
    15 => "4096"
    16 => "--ubatch-size"
    17 => "4096"
    18 => "--parallel"
    19 => "1"
  ]
  "RunAtLoad" => true
  "StandardErrorPath" => "$T/state/bilbo/embedder.log"
  "StandardOutPath" => "$T/state/bilbo/embedder.log"
}
$ launchctl print
	state = running
	program = /nix/store/17pbdn18pmzb9zm2z2jf1mv60sr6babc-llama-cpp-9190/bin/llama-server
	pid = 54990
		state = active
		state = active
	properties = keepalive | runatload | inferred program
$ lsof
COMMAND     PID    USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
llama-ser 54990 delucca    3u  IPv4 0xd49e3cb2f5d0d85a      0t0  TCP 127.0.0.1:8737 (LISTEN)
pid=54990
rss_kb=1147376
$ shasum
06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439  $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
total 1277960
-rw-r--r--@ 1 delucca  wheel  639150592 Oct  3 02:03 Qwen3-Embedding-0.6B-Q8_0.gguf
$ embedder.log head
0.00.192.597 E ggml_metal_library_init_from_source: error compiling source
0.00.192.603 W ggml_metal_device_init: - the tensor API is not supported in this environment - disabling
0.00.218.195 I log_info: verbosity = 3 (adjust with the `-lv N` CLI arg)
0.00.218.196 I device_info:
0.00.218.200 I   - MTL0    : Apple M5 (18186 MiB, 18185 MiB free)
0.00.218.204 I   - CPU     : Apple M5 (24576 MiB, 24576 MiB free)
0.00.218.444 I system_info: n_threads = 4 (n_threads_batch = 4) / 10 | MTL : EMBED_LIBRARY = 1 | CPU : NEON = 1 | ARM_FMA = 1 | FP16_VA = 1 | DOTPROD = 1 | LLAMAFILE = 1 | ACCELERATE = 1 | REPACK = 1 | 
0.00.218.955 I srv          init: running without SSL
0.00.219.490 I srv          init: using 9 threads for HTTP server
0.00.220.240 I srv         start: binding port with default address family
0.00.229.559 I srv          main: loading model
0.00.229.915 I srv    load_model: loading model '$T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf'
0.00.230.535 I common_init_result: fitting params to device memory ...
0.00.230.535 I common_init_result: (for bugs during this step try to reproduce them with -fit off, or provide --verbose logs if the bug only occurs with -fit on)
0.00.396.315 W load: control-looking token: 128247 '</s>' was not control-type; this is probably a bug in the model. its type will be overridden
0.00.611.322 W llama_context: n_ctx_seq (4096) < n_ct```

The log starts with `ggml_metal_library_init_from_source: error compiling source` and `the tensor API is not supported in this environment - disabling`; the server falls back and works (1024 dimensions), so this is noise, but it is in the log setup points the user at.

## Step 3: index and recall

Deviation: `bilbo new` writes a title-only note, so a paragraph was appended to each.

```
$T/store/notes/plan-smoke-a.md
$T/store/notes/plan-smoke-b.md
$T/store/notes/plan-smoke-c.md
$ bilbo index
embedded 3, kept 0, dropped 0
real 0.16
user 0.00
sys 0.00
exit 0
$ bilbo recall
$T/store/notes/plan-smoke-a.md:6	plan	2026-10-03T02:03-03:00
Sourdough starter care
Feed the starter flour and water every twelve hours and keep it somewhere warm so the wild yeast stays active and bubbly.

$T/store/notes/plan-smoke-b.md:6	plan	2026-10-03T02:03-03:00
Quarterly tax filing
File estimated payments with the revenue agency before the deadline and keep every receipt for the accountant.
real 0.02
user 0.00
sys 0.00
exit 0
```

## Step 4: rerun and cold start

```
$ step 4 rerun
mtime_before=1791003782
store kept: $T/store/notes
config kept: $T/conf/config
key skipped: local embedder
model kept: $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server kept: 127.0.0.1:8737
embedder skipped: config kept
claude skipped: not found
codex skipped: not found
timer skipped: --no-timer
exit 0
mtime_after=1791003782
$ cold start: kickstart -k, then first embed timing
health ready after 0.30s; first embed 0.011s; dims 1024
second embed 0.008s
```

## Step 5: wizard through expect

Preparation: `setup --remove --yes`, then the model was cut to a 600,000,000-byte `.part`.

```
$ setup --remove --yes
store skipped: kept $T/store/notes
config skipped: kept $T/conf/config
key skipped: no key file
model skipped: kept $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server removed
claude skipped: not found
codex skipped: not found
timer skipped: not installed
exit 0
print exit 0
ls: /Users/delucca/Library/LaunchAgents/io.github.delucca.bilbo.embedder.plist: No such file or directory
total 1171880
-rw-r--r--@ 1 delucca  wheel  600000000 Oct  3 02:03 Qwen3-Embedding-0.6B-Q8_0.gguf.part
```

First attempt: the script sent `j` at "Which embedder", but the config from step 2 is local, so the choice was preselected and `j` moved to Ollama; the script timed out at the timer prompt after Ollama (nothing was written; the first-attempt transcript is `s5-first-attempt-raw.txt`). Fixed by sending `
`. `wiz.exp` as run:

```tcl
set timeout 30
log_user 0
log_file -a -noappend $env(T)/wiz.transcript
set env(TERM) xterm-256color
proc want {text} { expect -timeout 60 -re $text {} timeout {puts "TIMEOUT waiting for: $text"; exit 2} eof {puts "EOF waiting for: $text"; exit 3} }
proc snd {s} { after 800; send -- $s }
spawn sh -c "exec $env(B) setup > $env(T)/wiz.report"
want {Which embedder}
snd "\r"
want {Keep the index fresh}
snd "jj\r"
want {Download Qwen3}
want {1 GB}
want {first index}
want {Apply these changes}
snd "y"
want {Downloading}
want {ready}
want {1024-dimensional}
want {Run the first index now}
snd "y"
catch {expect eof}
lassign [wait] pid id os code
puts "exit=$code"
```

Result `exit=0`; report:

```
store kept: $T/store/notes
config kept: $T/conf/config
key skipped: local embedder
model installed: $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server installed: 127.0.0.1:8737
embedder ok: 1024 dimensions
claude skipped: not found
codex skipped: not found
timer skipped: not chosen
```

Transcript, ANSI stripped (`perl -pe 's/\[[0-9;?]*[A-Za-z]//g; s/
//g'`), blank lines dropped; progress-bar frames collapse into one line because the `
` redraws are removed:

```
spawn sh -c exec $SCRATCH/S/target/debug/bilbo setup > /private/tmp/claude-501/-Users-delucca-Developer/2aa4d9f5
┌  bilbo setup
│
●  Notes go to $T/store/notes. Set BILBO_HOME to put them elsewhere.
│
◆  Which embedder should recall use?
│  ○ No embedder
│  ● Local embedder, run by bilbo (llama-server found, 639 MB download)
│  ○ Ollama on this machine
│  ○ OpenAI
│  ○ Another OpenAI-compatible URL
└
◇  Which embedder should recall use?
│  Local embedder, run by bilbo
│
●  Neither claude nor codex was found, so the plugin is skipped.
│
◆  Keep the index fresh in the background?
│  ● Every 15 minutes
│  ○ Another interval
│  ○ No timer
└
◆  Keep the index fresh in the background?
│  ○ Every 15 minutes
│  ● Another interval
│  ○ No timer
└
◆  Keep the index fresh in the background?
│  ○ Every 15 minutes
│  ○ Another interval
│  ● No timer
└
◇  Keep the index fresh in the background?
│  No timer
│
◇  Setup will ─────────────────────────────────────────────────────────────────╮
│                                                                              │
│  Keep the store folder /private/tmp/claude-501/-Users-delucca-               │
│  Developer/2aa4d9f5-15a3-4a39-9b21-3793c47b0010/scratchpad/smoke.43JdoS/     │
│  store/notes                                                                 │
│  Keep the config /private/tmp/claude-501/-Users-delucca-Developer/2aa4d9f5-  │
│  15a3-4a39-9b21-3793c47b0010/scratchpad/smoke.43JdoS/conf/config             │
│  Embed with qwen3-embedding-0.6b at http://127.0.0.1:8737                    │
│  Download Qwen3-Embedding-0.6B-Q8_0.gguf (639 MB) to /private/tmp/claude-    │
│  501/-Users-delucca-Developer/2aa4d9f5-15a3-4a39-9b21-3793c47b0010/          │
│  scratchpad/smoke.43JdoS/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf   │
│  Run /nix/store/17pbdn18pmzb9zm2z2jf1mv60sr6babc-llama-cpp-                  │
│  9190/bin/llama-server on 127.0.0.1:8737 as the launchd agent                │
│  io.github.delucca.bilbo.embedder                                            │
│  llama-server keeps about 1 GB of memory in use                              │
│  The first index of a large store takes a while                              │
├──────────────────────────────────────────────────────────────────────────────╯
│
◆  Apply these changes?
│  ● Yes  / ○ No
└
◇  Apply these changes?
│  Yes
│
◒  Downloading Qwen3-Embedding-0.6B-Q8_0.gguf (639 MB) [00:00:00] [□□□□□□□□□□□□□□□□□□□□□□□□□□□□□□] 0 B/609.54 MiB (0s)                                          ◐  Downloading Qwen3-Embedding-0.6B-Q8_0
│  ● Yes  / ○ No
└
◇  Run the first index now? The store holds 3 notes.
│  Yes
│
◒  Indexing 3 notes                                                             ◐  Indexing 3 notes                                                             ◇  embedded 0, kept 3, dropped 0        
```

Spinner and bar texts seen in the raw transcript: `Downloading Qwen3-Embedding-0.6B-Q8_0.gguf (639 MB)` from 572.20 MiB (the resumed 600,000,000 bytes) to 609.54 MiB/609.54 MiB in 3 s, with one first frame at `0 B/609.54 MiB`; `Starting llama-server and loading the model`, `llama-server is ready`, `Checking qwen3-embedding-0.6b at http://127.0.0.1:8737`, `http://127.0.0.1:8737 answered with 1024-dimensional vectors`; `Run the first index now? The store holds 3 notes.` then `embedded 0, kept 3, dropped 0`. The finished file has the pinned SHA-256 `06507c7b...e439`.

## Step 6: Homebrew

Before: 119 formulae. `brew install llama.cpp` took 12 s.

```
$ comm -13 before after (added/changed lines)
ca-certificates 2026-09-25 2026-08-13 2026-05-14 2026-07-16
ggml 0.25.3
libomp 23.1.2
llama.cpp 0.5.0
openssl@3 3.6.3 3.6.5
$ comm -23 before after (removed/old versions; must be empty)
ca-certificates 2026-08-13 2026-05-14 2026-07-16
openssl@3 3.6.3
$ brew-added.txt
ggml
libomp
llama.cpp
$ llama-server --version
0.00.000.206 I srv  llama_server: initializing ...
version: 0.5.0 (build 11146, commit 7fe450e19)
built with AppleClang 21.0.0.21000334 for Darwin arm64
lrwxr-xr-x@ 1 delucca  admin  42 Oct  3 02:05 /opt/homebrew/bin/llama-server -> ../Cellar/llama.cpp/0.5.0/bin/llama-server
```

setup with the Homebrew path (first run failed, see Misbehavior 1; the rerun installed it):

```
$ setup --yes --no-timer --embedder-local --llama-server /opt/homebrew/bin/llama-server
bilbo: launchctl bootstrap gui/501 /Users/delucca/Library/LaunchAgents/io.github.delucca.bilbo.embedder.plist failed: Bootstrap failed: 5: Input/output error; setup changed nothing else
model kept: $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server failed: launchctl bootstrap gui/501 /Users/delucca/Library/LaunchAgents/io.github.delucca.bilbo.embedder.plist failed: Bootstrap failed: 5: Input/output error; see $T/state/bilbo/embedder.log
exit 1
$ plist program
/Users/delucca/Library/LaunchAgents/io.github.delucca.bilbo.embedder.plist: (The file “io.github.delucca.bilbo.embedder.plist” couldn’t be opened because there is no such file.)
	state = SIGTERMed
	program = /nix/store/17pbdn18pmzb9zm2z2jf1mv60sr6babc-llama-cpp-9190/bin/llama-server
	pid = 55327
		state = active
		state = active
pid=55327 rss_kb=     0
$ index/recall
bilbo: embedder http://127.0.0.1:8737 unreachable: Connection refused (os error 61)
bilbo: embedder unavailable (embedder http://127.0.0.1:8737 unreachable: Connection refused (os error 61)); keyword results only
bilbo: 1 passages not indexed; run bilbo index
$T/store/notes/plan-smoke-b.md:6	plan	2026-10-03T02:03-03:00
Quarterly tax filing
File estimated payments with the revenue agency before the deadline and keep every receipt for the accountant.

$ rerun
stat: /Users/delucca/Library/LaunchAgents/io.github.delucca.bilbo.embedder.plist: stat: No such file or directory
store kept: $T/store/notes
config kept: $T/conf/config
key skipped: local embedder
model kept: $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server installed: 127.0.0.1:8737
embedder ok: 1024 dimensions
claude skipped: not found
codex skipped: not found
timer skipped: --no-timer
$ log head
0.00.192.597 E ggml_metal_library_init_from_source: error compiling source
0.00.192.603 W ggml_metal_device_init: - the tensor API is not supported in this environment - disabling
0.00.218.195 I log_info: verbosity = 3 (adjust with the `-lv N` CLI arg)
0.14.172.072 I slot launch_slot_: id  0 | task 0 | processing task, is_child = 0
0.14.206.212 I slot      release: id  0 | task 0 | stop processing: n_tokens = 5, truncated = 0
```

State, kept rerun and the update-race reproduction:

```
$ state after the rerun
  "ProgramArguments" => [
    0 => "/opt/homebrew/bin/llama-server"
    1 => "--model"
	state = running
	program = /opt/homebrew/bin/llama-server
	pid = 57770
		state = active
		state = active
COMMAND     PID    USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
llama-ser 57770 delucca    3u  IPv4 0xa186c50838f8f99b      0t0  TCP 127.0.0.1:8737 (LISTEN)
pid=57770 rss_kb=1184704
$ index/recall
embedded 1, kept 2, dropped 0
$T/store/notes/plan-smoke-a.md:6	plan	2026-10-03T02:03-03:00
Sourdough starter care
Feed the starter flour and water every twelve hours and keep it somewhere warm so the wild yeast stays active and bubbly.
$ rerun (kept)
key skipped: local embedder
model kept: $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server kept: 127.0.0.1:8737
embedder skipped: config kept
$ update race repro: switch to nix path while the server is loaded
bilbo: launchctl bootstrap gui/501 /Users/delucca/Library/LaunchAgents/io.github.delucca.bilbo.embedder.plist failed: Bootstrap failed: 5: Input/output error; setup changed nothing else
model kept: $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server failed: launchctl bootstrap gui/501 /Users/delucca/Library/LaunchAgents/io.github.delucca.bilbo.embedder.plist failed: Bootstrap failed: 5: Input/output error; see $T/state/bilbo/embedder.log
exit 0
Bad request.
Could not find service "io.github.delucca.bilbo.embedder" in domain for user gui: 501
```

The plist named `/opt/homebrew/bin/llama-server`, not a `Cellar/` path (Q2).

## Step 7: remove

```
$ install brew-path server again
key skipped: local embedder
model kept: $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server installed: 127.0.0.1:8737
embedder ok: 1024 dimensions
$ step 7 remove
store skipped: kept $T/store/notes
config skipped: kept $T/conf/config
key skipped: no key file
model skipped: kept $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server removed
claude skipped: not found
codex skipped: not found
timer skipped: not installed
exit 0
print exit immediately: 0
print exit after 5s: 113
lsof exit 1
plist grep exit 1
```

## Cleanup

Uninstalled exactly the three added formulae:

```
$ brew uninstall (the added)
ggml libomp llama.cpp 
Uninstalling /opt/homebrew/Cellar/ggml/0.25.3... (38 files, 4.9MB)
Uninstalling /opt/homebrew/Cellar/libomp/23.1.2... (11 files, 1.8MB)
Uninstalling /opt/homebrew/Cellar/llama.cpp/0.5.0... (70 files, 16.0MB)
$ diff before final
7c7
< ca-certificates 2026-08-13 2026-05-14 2026-07-16
---
> ca-certificates 2026-09-25 2026-08-13 2026-05-14 2026-07-16
91c91
< openssl@3 3.6.3
---
> openssl@3 3.6.3 3.6.5
diff exit 1
ls: /opt/homebrew/bin/llama-server: No such file or directory
```

The agent `io.github.delucca.bilbo.embedder` was removed by step 7 (`launchctl print` exits 113, no plist, nothing on 8737); the verified model was copied to the session scratchpad `model-cache/` and the temp folder deleted.

## S2 rerun (after fix round 2)

Run on rivendell on 2026-10-03 from the R3 snapshot (commit `b1376b0`), `cargo build --locked`, nixpkgs llama-cpp 9190 only (no Homebrew). Same env rules as above; the model was copied from the session's model-cache into the temp cache (the real download is proven in step 2), so the run says `model kept`. Pre-check passed. Raw transcripts: `work/add-local-embedder/smoke/s2/`.

| Step | Result |
|---|---|
| batch install | pass: `model kept`, `server installed: 127.0.0.1:8737`, `embedder ok: 1024 dimensions`, exit 0; RSS 1,150,352 KB |
| index and recall | pass: `embedded 3`; the bread-culture query ranks the starter note first |
| rerun | pass: `model kept`, `server kept`, plist mtime unchanged |
| update to a symlink of the same binary | pass: `server updated`, exit 0, 6.65 s, `launchctl` shows the symlink path, running, new pid, `embedder ok` |
| update back to the original path | pass: `server updated`, exit 0, 0.56 s, original path, running |
| index and recall after the updates | pass: `embedded 0, kept 3`, same top result |
| wizard via expect (fresh dirs, cached model) | pass, `exit=0`; report `model kept`, `server installed`, `embedder ok: 1024 dimensions`, `timer skipped: not chosen` |
| `--remove` | pass: `server removed`, `model skipped: kept <path>`; `launchctl print` exits 113 immediately (it was 0 for about a second before the fix), still 113 after 2 s |

Misbehavior 1 is fixed: both update cases that failed twice before pass. Misbehavior 2 (Homebrew upgrading `openssl@3` and `ca-certificates`) stays as recorded; Homebrew was not touched.

Wizard script note: with the model already in place the summary says `Keep the model <path>` instead of `Download Qwen3-Embedding-0.6B-Q8_0.gguf (639 MB) ...`, so the first attempt (waiting for `Download Qwen3`) timed out at the summary, before `Apply these changes` (nothing written; transcript `s2-wizard-first-attempt-raw.transcript`); the script then waited for `Keep the model`. The summary still shows `llama-server keeps about 1 GB of memory in use` and `The first index of a large store takes a while`, then the spinners `Starting llama-server and loading the model`, `llama-server is ready`, `Checking qwen3-embedding-0.6b at http://127.0.0.1:8737` and `http://127.0.0.1:8737 answered with 1024-dimensional vectors`, and `Run the first index now? The store holds 0 notes.`

### Batch, updates and recall

```
$ batch install (cached model)
store created: $T/store/notes
config written: $T/conf/config
key skipped: local embedder
model kept: $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server installed: 127.0.0.1:8737
embedder ok: 1024 dimensions
claude skipped: not found
codex skipped: not found
timer skipped: --no-timer
exit 0
$ plist/launchctl/lsof/rss
  "ProgramArguments" => [
    0 => "/nix/store/17pbdn18pmzb9zm2z2jf1mv60sr6babc-llama-cpp-9190/bin/llama-server"
	state = running
	program = /nix/store/17pbdn18pmzb9zm2z2jf1mv60sr6babc-llama-cpp-9190/bin/llama-server
	pid = 68280
		state = active
		state = active
COMMAND     PID    USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
llama-ser 68280 delucca    3u  IPv4 0x64f5a62aaa7491d1      0t0  TCP 127.0.0.1:8737 (LISTEN)
rss_kb=1150352
$ index
embedded 3, kept 0, dropped 0
$ recall
$T/store/notes/plan-s2-a.md:6	plan	2026-10-03T02:26-03:00
Note a
Feed the starter flour and water every twelve hours and keep it somewhere warm so the wild yeast stays active.
$ rerun
key skipped: local embedder
model kept: $SCRATCH/smoke2.h5bho7/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server kept: 127.0.0.1:8737
embedder skipped: config kept
exit 0 mtime 1791005190 -> 1791005190
$ update via symlink
key skipped: local embedder
model kept: $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server updated: 127.0.0.1:8737
embedder ok: 1024 dimensions
real 6.65
exit 0
    0 => "$T/lnk/llama-server"
	state = running
	program = $SCRATCH/smoke2.h5bho7/lnk/llama-server
	pid = 68461
		state = active
		state = active
$ update back to original
key skipped: local embedder
model kept: $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server updated: 127.0.0.1:8737
embedder ok: 1024 dimensions
real 0.56
exit 0
    0 => "/nix/store/17pbdn18pmzb9zm2z2jf1mv60sr6babc-llama-cpp-9190/bin/llama-server"
	state = running
	pid = 68762
		state = active
		state = active
$ index+recall after updates
embedded 0, kept 3, dropped 0
$T/store/notes/plan-s2-a.md:6	plan	2026-10-03T02:26-03:00
Note a
```

### Wizard

`wiz.exp` as run is `s2/wiz.exp`. Report:

```
store created: $T/store/notes
config written: $T/conf/config
key skipped: local embedder
model kept: $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server installed: 127.0.0.1:8737
embedder ok: 1024 dimensions
claude skipped: not found
codex skipped: not found
timer skipped: not chosen
```

### Remove

```
$ --remove
store skipped: kept $T/store/notes
config skipped: kept $T/conf/config
key skipped: no key file
model skipped: kept $T/cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf
server removed
claude skipped: not found
codex skipped: not found
timer skipped: not installed
exit 0
print immediately: 113
print after 2s: 113
(eval):10: command not found: lsof
lsof exit 127
0
```

(`lsof` was not on that shell's PATH; the cleanup below ran it.)

### Cleanup

`setup --remove --yes`, `launchctl bootout` and plist removal again, temp folders deleted: `launchctl print` exits 113, `lsof` finds nothing on 8737, no bilbo plist, no llama-server process.
