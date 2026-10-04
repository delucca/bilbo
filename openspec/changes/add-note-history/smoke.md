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
