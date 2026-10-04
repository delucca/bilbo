# add-note-history smoke test (setup and watch service)

Run on macOS (Darwin 25.6, aarch64) on 2026-10-04 with `target/debug/bilbo` of the add-note-history-service worktree. Nothing touched the real `~/Library/LaunchAgents`, `~/.claude` or `~/.codex`: every run had a scratch `HOME`, `PATH=$T/bin:/usr/bin:/bin` and `CLAUDE_CONFIG_DIR`/`CODEX_HOME` under `$T`. `$T/bin/launchctl` is a shell fake that logs its arguments and records the bootstrapped plist in `$T/loaded`; there was no `claude` or `codex` on PATH, so the plugin question was skipped (`Neither claude nor codex was found`). `$B` is the bilbo binary.

## 5.5 The wizard question, by `/usr/bin/expect`

`lib.tcl` is the one of the add-setup smoke (`want` matches a regex, `snd` waits 600 ms before each send, `start` spawns `sh -c "exec $B setup > $T/<name>.report"`). Each script answers `Which embedder` with Enter (no embedder), then answers the new question.

The scripts (`lib.tcl` is sourced by each; the three paths are `p1.tcl` to `p3.tcl`). The raw transcripts (ANSI kept), the reports, the scripts and the fake's log are saved under `/Users/delucca/Notebooks/20261002T132305Z--bilbo-initial-launch__personal/work/add-note-history/smoke/`.

```tcl
# lib.tcl
set timeout 30
log_user 0
proc want {text} {
  expect -timeout 30 -re $text {} timeout {puts "TIMEOUT waiting for: $text"; exit 2} eof {puts "EOF waiting for: $text"; exit 3}
}
proc snd {s} { after 600; send -- $s }
proc finish {} { global spawn_id; catch {expect eof}; lassign [wait] pid id os code; puts "exit=$code"; exit 0 }
proc start {name cmd} {
  global env spawn_id
  set env(TERM) xterm-256color
  log_file -a -noappend $::env(T)/$name.transcript
  spawn sh -c "exec $cmd > $::env(T)/$name.report"
}
```

```tcl
# p1.tcl
source lib.tcl
start p1 "$env(B) setup"
want "Which embedder"
send "\r"
want "Record note history in the background"
snd "\r"
want "Apply these changes"
snd "y"
want "The report follows"
finish
```

```tcl
# p2.tcl
source lib.tcl
start p2 "$env(B) setup"
want "Which embedder"
send "\r"
want "Record note history in the background"
snd "n"
want "Apply these changes"
snd "y"
want "The report follows"
finish
```

```tcl
# p3.tcl
source lib.tcl
start p3 "$env(B) setup"
want "Which embedder"
send "\r"
want "Record note history in the background"
snd "n"
want "Apply these changes"
snd "n"
want "Cancelled"
finish
```

| Path | Answers | Exit | Result |
| --- | --- | --- | --- |
| p3 | watch `n`, apply `n` | 1 | `Cancelled. Nothing changed.`; no `LaunchAgents` folder, no `$T/loaded`, no store |
| p1 | watch Enter (default yes), apply `y` | 0 | `watch installed: watching $T/home/.local/share/bilbo/notes`; the watch plist is in the scratch `LaunchAgents` and `$T/loaded` names it |
| p2 | watch `n`, apply `y` | 0 | `watch removed: not chosen`; the plist is gone and `$T/loaded` is gone |

The transcript of p1 (ANSI stripped, `$T` abbreviated) shows the question after the plugin note and before the summary, defaulting to yes, and the summary line naming the agent:

```
◆  Which embedder should recall use?
│  ● No embedder (keyword search only)
...
◇  Which embedder should recall use?
│  No embedder
●  Neither claude nor codex was found, so the plugin is skipped.
◆  Record note history in the background?
│  ● Yes  / ○ No
◇  Record note history in the background?
│  Yes
◇  Setup will ───────────────────────────────╮
│  Create the store folder $T/home/.local/share/bilbo/notes
│  Write the config $T/home/.config/bilbo/config
│  Search by keywords only
│  Record note history in the background (launchd agent io.github.delucca.bilbo.watch)
├──────────────────────────────────────────╯
◆  Apply these changes?
```

Report of p1:

```
store created: $T/home/.local/share/bilbo/notes
config written: $T/home/.config/bilbo/config
key skipped: no embedder
model skipped: not local
server skipped: not local
embedder skipped: none configured
claude skipped: not found
codex skipped: not found
hook skipped: no codex plugin
timer skipped: no embedder
watch installed: watching $T/home/.local/share/bilbo/notes
```

Report of p2 (rerun on the p1 machine) ends `timer skipped: no embedder` and `watch removed: not chosen`, with `store kept` and `config kept` first. The fake's log across p1 and p2:

```
bootout --wait gui/501/io.github.delucca.bilbo.watch
bootstrap gui/501 $T/home/Library/LaunchAgents/io.github.delucca.bilbo.watch.plist
bootout --wait gui/501/io.github.delucca.bilbo.watch
```

Not run here: a real `launchctl` (the recorded add-setup run used the user's launchd; this one used the fake, as instructed) and the `watch` verb itself, which another stream implements, so the service file names `<bilbo> watch` but no process was started.

## The macOS-only tests

`cargo test --locked --test setup` on macOS: `test result: ok. 126 passed; 0 failed`, in about 22 s, with the fake `launchctl` doing what the real one would for the three jobs. `cargo test --locked --bin bilbo host::timer::` (41 tests) and `setup::` (100 tests) also pass here, and `nix flake check -L` passed on aarch64-darwin (aarch64-linux and x86_64-linux were omitted as incompatible).

## Integration (task 7.1)

Run on rivendell (macOS, Darwin 25.6, aarch64) on 2026-10-04 with `target/debug/bilbo` built from commit `daf20f3` of the add-note-history worktree (the worktree also carried uncommitted edits to `AGENTS.md` and `README.md`, which do not affect the binary). The real `launchctl`, a scratch `HOME` and a scratch `BILBO_HOME` (`$S/home`, `$S/store`, `XDG_*` and `BILBO_CONFIG` unset), `$S` being `/private/tmp/claude-501/-Users-delucca-Developer/c46ef7b3-4114-45d4-865c-ec1b290b982a/scratchpad/smoke`. `$B` is the binary. Before starting, `launchctl list | grep -i bilbo` and `ls ~/Library/LaunchAgents | grep -i bilbo` were both empty, and they were again at the end. Paths are shortened to `$S`; versions are the 12-character ids bilbo prints. The scratch `HOME` already held an empty `.local/share/bilbo/notes` and a config from an earlier run (`config kept`); neither is used, since `BILBO_HOME` is set.

### 1. Install the watcher: PASS

```
$ HOME=$S/home BILBO_HOME=$S/store $B setup --yes --no-plugin --no-timer
store created: $S/store/notes
config kept: $S/home/.config/bilbo/config
key skipped: no embedder
...
timer skipped: --no-timer
watch installed: watching $S/store/notes
exit=0
$ launchctl list | grep -i bilbo
71231	0	io.github.delucca.bilbo.watch
$ launchctl print gui/$(id -u)/io.github.delucca.bilbo.watch
	path = $S/home/Library/LaunchAgents/io.github.delucca.bilbo.watch.plist
	state = running
	arguments = { .../target/debug/bilbo  watch }
	stdout path = $S/home/.local/state/bilbo/watch.log
	environment = { BILBO_HOME => $S/store ... }
$ cat $S/home/.local/state/bilbo/watch.log
bilbo: watching $S/store/notes
```

The plist is under the scratch `HOME`, and the service carries only `BILBO_HOME`.

### 2. Create a note: PASS

```
$ $B new decision smokehist --title "Smoke history"
$S/store/notes/decision-smokehist.md
$ $B history smokehist        (polled; first answer after 1 s)
c40415670868 2026-10-04T19:59-03:00 added decision-smokehist.md
```

### 3. Edit with Claude Code: PASS

```
$ claude -p "Append one line saying 'Edited by Claude Code.' at the end of the file $S/store/notes/decision-smokehist.md" --allowedTools Edit,Read     (real HOME)
I added the line `Edited by Claude Code.` to the end of `decision-smokehist.md`. ...
$ $B history smokehist        (polled; version after 2 s)
8e6a4f0459eb 2026-10-04T19:59-03:00 edited decision-smokehist.md
c40415670868 2026-10-04T19:59-03:00 added decision-smokehist.md
```

### 4. Edit with a shell redirect: PASS

```
$ echo "Appended by shell redirect." >> $S/store/notes/decision-smokehist.md
$ $B history smokehist        (polled; version after 4 s)
94c65683bd58 2026-10-04T19:59-03:00 edited decision-smokehist.md
8e6a4f0459eb 2026-10-04T19:59-03:00 edited decision-smokehist.md
c40415670868 2026-10-04T19:59-03:00 added decision-smokehist.md
```

### 5. Rename: PASS

```
$ mv notes/decision-smokehist.md notes/plan-smokehist2.md
$ $B history smokehist2
7c11230abfcf 2026-10-04T19:59-03:00 renamed plan-smokehist2.md
94c65683bd58 ... edited decision-smokehist.md
8e6a4f0459eb ... edited decision-smokehist.md
c40415670868 ... added decision-smokehist.md
$ $B history smokehist
bilbo: no history for smokehist
$ $B history 01M44J90ER4WM7XAB5153D0D79 | head -1
7c11230abfcf 2026-10-04T19:59-03:00 renamed plan-smokehist2.md
```

Same note under the new name. A topic names the note by its current file's topic, so the old topic `smokehist` no longer resolves and the id does (the `Naming a note` requirement).

### 6. List and diff: PASS

```
$ $B history smokehist2 --diff c40415670868
--- decision-smokehist.md@c40415670868
+++ plan-smokehist2.md@now
@@ -4,3 +4,5 @@
 ---
 
 # Smoke history
+Edited by Claude Code.
+Appended by shell redirect.
exit=0
$ $B history smokehist2 --diff c40415670868 8e6a4f0459eb
--- decision-smokehist.md@c40415670868
+++ decision-smokehist.md@8e6a4f0459eb
@@ -4,3 +4,4 @@
 # Smoke history
+Edited by Claude Code.
exit=0
```

### 7. Restore the first version: PASS

```
$ $B restore smokehist2 c40415670868
restored decision-smokehist.md to c40415670868
exit=0
$ ls notes
decision-smokehist.md          (plan-smokehist2.md is gone; the file holds the first version's bytes)
$ $B history smokehist
f6ff50587ce3 2026-10-04T20:00-03:00 restored decision-smokehist.md
7c11230abfcf 2026-10-04T19:59-03:00 renamed plan-smokehist2.md
94c65683bd58 2026-10-04T19:59-03:00 edited decision-smokehist.md
8e6a4f0459eb 2026-10-04T19:59-03:00 edited decision-smokehist.md
c40415670868 2026-10-04T19:59-03:00 added decision-smokehist.md
```

Twelve seconds later the list still had 5 versions: the watcher recorded nothing for the restore's own write. The restore wrote the old name and removed the new one, as `Restoring across a rename` says.

### 8. A second watcher waits: PASS

```
$ $B watch  (same HOME and BILBO_HOME, in the background)
stderr:
bilbo: bilbo watch is already running for $S/store; waiting
$ kill -0 <pid>     -> alive after 4 s
$ kill <pid>        -> exit 143 (SIGTERM); the launchd watcher (pid 71231) kept running
```

### 9. History's watcher warning: PASS

```
(launchd watcher and a waiting second one running)
$ $B history smokehist >/dev/null     -> stderr empty, exit 0
(after setup --remove, no watcher)
$ $B history smokehist
bilbo: bilbo watch is not running; recent edits may not be recorded
f6ff50587ce3 ... restored decision-smokehist.md
...
exit=0
```

### 10. Remove: PASS

```
$ $B setup --remove --yes
store skipped: kept $S/store/notes
config skipped: kept $S/home/.config/bilbo/config
...
timer skipped: not installed
watch removed
exit=0
$ launchctl list | grep -i bilbo      (empty)
$ ls ~/Library/LaunchAgents | grep -i bilbo      (empty)
$ pgrep -fl "bilbo watch"      (nothing)
```

Every step passed.
