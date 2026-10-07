# Manual tests

The unit tests drive the setup wizard and the recovery phrase ceremony through
a scripted `Prompter`. `Terminal` in `src/host/prompt.rs`, the cliclack
adapter, is the only code they cannot reach. After changing it, run the
procedures below in a real pseudo-terminal with `expect`, on the platform you
changed.

## A throwaway world

Every run happens in a world: a temporary folder that holds the store, the
config, the state, the cache and a `HOME`. Save this as `world.sh` and source
it in a fresh `sh` for each world, with `B` set to the binary under test
(`cargo build --locked`, then `B=$PWD/target/debug/bilbo`):

```sh
T=$(mktemp -d)
export T HOME="$T/home" BILBO_HOME="$T/store" BILBO_CONFIG="$T/conf/config" \
  XDG_STATE_HOME="$T/state" XDG_CACHE_HOME="$T/cache" XDG_CONFIG_HOME="$T/config" \
  CLAUDE_CONFIG_DIR="$T/claude" CODEX_HOME="$T/codex" PATH=/usr/bin:/bin:/usr/sbin:/sbin
mkdir -p "$HOME"
```

- Start the shell as `env -u AI_AGENT -u CLAUDE_CODE_CHILD_SESSION -u CODEX_THREAD_ID -u CODEX_CI B=... sh`. Under an
  agent marker the wizard keeps sync off and `bilbo device init` refuses to
  show a phrase.
- The `PATH` above has no `claude` or `codex`, so setup skips the plugin
  instead of installing it from GitHub. Add `llama-server`'s folder for the
  local embedder.
- The timer, the watcher and the local embedder load real jobs even in a
  world: launchd agents `io.github.delucca.bilbo.{index,watch,embedder}` in
  your GUI domain, or systemd user units on Linux. Installing one replaces
  bilbo's own job of that name, so run those paths on a machine without bilbo
  set up, and end with `$B setup --remove --yes` in the same world. The other
  paths answer no to the timer and the watcher and load nothing.
- Delete `$T` when done.

## Expect helpers

Save as `lib.tcl` beside the scripts below, and run each script as
`expect <script>.tcl <args>` from that folder.

```tcl
set timeout 60
log_user 0
set env(TERM) xterm-256color
# Waits for a regexp on the screen; a timeout or an early exit fails the script.
proc want {re} {
  global spawn_id
  expect -re $re {} timeout {puts "TIMEOUT waiting for: $re"; exit 2} eof {puts "EOF waiting for: $re"; exit 3}
}
# A key sent right after a prompt title can be lost or echoed by the tty.
proc snd {s} { global spawn_id; after 600; send -- $s }
# Spawns a shell line with stdout in $T/<name>.out and the raw screen in $T/<name>.raw;
# stty -a reads the pty after bilbo exits, and the exit code is bilbo's.
proc start {name line} {
  global spawn_id env
  log_file -a -noappend $env(T)/$name.raw
  spawn sh -c "$line > $env(T)/$name.out; rc=\$?; stty -a > $env(T)/$name.stty 2>&1; exit \$rc"
}
proc finish {} {
  global spawn_id
  catch {expect eof {} timeout {puts "TIMEOUT waiting for the exit"; close}}
  lassign [wait] pid id os code
  puts "exit=$code"
  exit 0
}
```

- cliclack redraws a prompt on every key. Wait for a title, answer it once,
  then wait for the next title; never loop on a title already answered.
- A select starts on the answer the current config implies, so on a rerun
  count the `j` presses from there.
- A confirm takes `y` or `n` without Enter.
- To read a transcript, strip the escapes:
  `perl -pe 's/\e\[[0-9;?]*[A-Za-z]//g; s/\r//g' "$T/<name>.raw"`.

## Terminal views

The unit and binary tests cover the terminal view through a pseudo-terminal, but
not how it looks in a real terminal. Run this once per release, in a world with
a store of two notes that hold `<word>`, one corpus, and `scope.personal` and
`scope.work` declared. Run each command under `script`, which gives it a
terminal:

```sh
# macOS
script -q /dev/null "$B" recall <word>
# Linux
script -qc "$B recall <word>" /dev/null
```

1. `recall <word>`: ranked hits, bold titles, a dim meta line with `~/` paths,
   then the count line. `recall wumpus`: `○  no notes match 'wumpus'` and its
   hint, exit 1.
2. The same with `NO_COLOR=1`, `CLICOLOR=0`, `TERM=dumb` and `env -u TERM`: the
   same layout, no colour, no bold.
3. The same with `AI_AGENT=x`, then `CLAUDE_CODE_CHILD_SESSION=1`, `CODEX_CI=1`
   and `CODEX_THREAD_ID=t`: the plain three-line blocks and `bilbo: ` on
   stderr. With only `CLAUDECODE=1`: the terminal view.
4. At 60 columns (`script -q /dev/null sh -c "stty cols 60; $B recall <word>"`):
   two-line snippets, no line wider than 60, the meta line broken under the
   path. At 200 columns (`stty cols 200`): no line wider than 100.
5. `check` clean and broken, `scope`, `library`, `library <corpus>`, `library
   show <ref>`, `device`, `device list`, `sync`, `history <note>`, `history
   <note> --diff <a> <b>`, `restore`, `new`, `index` and `setup --yes`: each
   shows its terminal view as [Terminal
   output](reference/commands.md#terminal-output) describes it.
6. `bilbo recal x`: a `■` line and indented usage lines, with `bilbo --help`
   in cyan after `see`.
7. The wizard (`bilbo setup`) with and without `NO_COLOR=1`: colour only
   without it; the boxes and marks are the same.
8. Dim text is readable in Terminal.app, iTerm2 and Ghostty, on a dark and a
   light theme. Note any theme where it is not.
9. A slow embedder (a fake that answers after 1.5 s, or a stalled one): `recall
   <word>` shows `Waiting for the embedder` with a spinner after half a second,
   the line is gone when the hits print and nothing of it stays in the
   scrollback. A quick embedder shows no flicker.
10. A note whose section is a fenced code block: `recall` shows its first lines
    as written, dim. A note holding an ESC byte in its text prints `\u{1b}`.
11. With `BILBO_HYPERLINKS=1` in iTerm2, Ghostty or WezTerm, the path opens the
    file; in Terminal.app it shows as plain text with no stray characters.
12. Through Claude Code's Bash tool, `bilbo recall <word>` prints the plain
   blocks, because that tool gives bilbo no terminal.

## Setup wizard

`keyword.tcl`, keyword search only:

```tcl
source lib.tcl
start keyword {$B setup}
want {Which embedder}
snd "\r"
want {Record note history}
snd "n"
want {Sync notes}
snd "n"
want {Apply these changes}
snd "y"
want {The report follows}
finish
```

`key.tcl <dummy key>`, a pasted key that the embedder check rejects, then
cancelled at the summary:

```tcl
source lib.tcl
set key [lindex $argv 0]
start key {$B setup}
want {Which embedder}
snd "jjj\r"
want {Model name}
snd "\r"
want {advanced settings}
snd "n"
want {API key come from}
snd "\r"
want {API key\r}
after 600
foreach c [split "$key\r" ""] { send -- $c }
want {answered 401}
want {What now}
snd "jj\r"
want {Record note history}
snd "n"
want {Sync notes}
snd "n"
want {Apply these changes}
snd "n"
want {Cancelled}
finish
```

`cancel.tcl`, Ctrl-C at the first prompt (repeat with `\033`, Esc):

```tcl
source lib.tcl
start cancel {$B setup}
want {Which embedder}
snd "\003"
want {Cancelled}
finish
```

Look for:

- `keyword`: `exit=0`, and `$T/keyword.out` holds only report lines
  (`store created: ...`, `embedder skipped: none configured`,
  `watch skipped: not chosen`, ...). Every prompt went to stderr.
- `key`: `exit=1`; the key field draws one `▪` per character; the key, or any
  part of it, is in neither `$T/key.raw` nor `$T/key.out`
  (`grep -c <key> "$T"/key.*`); `$T/conf/token` does not exist. Repeat with the
  whole key in one `send` instead of the per-character loop.
- `cancel`: `exit=1`, `Cancelled. Nothing changed.`, and `find "$T"` lists the
  same files before and after, apart from the script's own.
- After every run, `$T/<name>.stty` shows `echo`, never `-echo`.

## Local embedder

Needs `llama-server` on `PATH`, about 640 MB of download and 1 GB of memory,
and installs the embedder job (see the warning above). `local.tcl`:

```tcl
source lib.tcl
start local {$B setup}
want {Which embedder}
snd "j\r"
want {Keep the index fresh}
snd "jj\r"
want {Record note history}
snd "n"
want {Sync notes}
snd "n"
want {about 1 GB}
want {Apply these changes}
snd "y"
want {llama-server is ready}
want {1024-dimensional}
want {Run the first index now}
snd "y"
want {The report follows}
finish
```

Steps, each in the same world:

1. Run `local.tcl`. Look for the summary lines `Download
   Qwen3-Embedding-0.6B-Q8_0.gguf (639 MB)`, `llama-server keeps about 1 GB of
   memory in use` and `The first index of a large store takes a while`; a
   progress bar while downloading; the spinners `Starting llama-server and
   loading the model` and `Checking qwen3-embedding-0.6b at
   http://127.0.0.1:8737`. The report says `model installed`,
   `server installed: 127.0.0.1:8737` and `embedder ok: 1024 dimensions`.
2. Rerun with `\r` in place of `j\r` (local is now preselected). The summary
   says `Keep the model <path>`, the report `model kept` and `server kept`,
   and the job file's mtime does not change.
3. Resume: `$B setup --remove --yes`, cut the model to a part file
   (`head -c 600000000 <model> > <model>.part`, then delete the model), and
   run step 1 again. The bar starts at about 572 MiB, not at 0.
4. Update: `$B setup --yes --no-timer --no-watch --embedder-local
   --llama-server <a symlink to llama-server>`, then again with the original
   path. Both report `server updated` with `exit 0`, and the job runs the new
   path.
5. Remove: `$B setup --remove --yes` reports `server removed` and
   `model skipped: kept <path>`. Right after it,
   `launchctl print gui/$(id -u)/io.github.delucca.bilbo.embedder` exits 113
   (on Linux, `systemctl --user status` finds no bilbo embedder unit), and
   nothing listens on 8737.

## Device ceremony

Prepare the world with a store and a scope that syncs through a folder:

```sh
mkdir -p "$BILBO_HOME/notes" "$T/conf"
echo "scope.personal.sync = file://$T/sync" > "$BILBO_CONFIG"
```

`init.tcl <name>` reads the 12 words off the phrase screen and types back the
three it asks for. It keeps the throwaway phrase in `$T/phrase` for
`recover.tcl`; never do this with a real phrase.

```tcl
source lib.tcl
set name [lindex $argv 0]
start init-$name "\$B device init --name $name"
want {Recovery phrase}
# The note is whole once its fingerprint line is drawn.
expect -re {Owner fingerprint: [a-z0-9-]+} {} timeout {puts "TIMEOUT"; exit 2}
regsub -all {\x1b\[[0-9;?]*[A-Za-z]} $expect_out(buffer) {} screen
foreach {- n w} [regexp -all -inline {([0-9]+)\. ([a-z]+)} $screen] { set word($n) $w }
puts "words parsed: [array size word]"
set f [open $env(T)/phrase w]
foreach n [lsort -integer [array names word]] { puts -nonewline $f "$word($n) " }
close $f
want {Written down}
snd "y"
set done {}
expect {
  -re {Word ([0-9]+)[^0-9]} {
    set n $expect_out(1,string)
    if {$n ni $done} { lappend done $n; snd "$word($n)\r" }
    exp_continue
  }
  {Recovery phrase confirmed} {}
  timeout {puts "TIMEOUT"; exit 2}
  eof {puts "EOF before the phrase was confirmed"}
}
finish
```

`recover.tcl <phrase file> <name> [y|n]` types the 12 words and answers the
fingerprint question, if asked:

```tcl
source lib.tcl
lassign $argv file name answer
set f [open $file]; set words [read $f]; close $f
start recover-$name "\$B device recover --name $name"
set done {}
expect {
  -re {Word ([0-9]+)[^0-9]} {
    set n $expect_out(1,string)
    if {$n ni $done} { lappend done $n; snd "[lindex $words [expr {$n - 1}]]\r" }
    exp_continue
  }
  {Does it match} { if {$answer ne ""} { snd $answer; set answer "" }; exp_continue }
  timeout {puts "TIMEOUT"; exit 2}
  eof {}
}
finish
```

`plain.tcl <arguments>` runs one bilbo command in a pty and prints its screen
and stdout:

```tcl
source lib.tcl
log_user 1
start plain "\$B $argv"
expect eof {} timeout {puts "TIMEOUT"; close}
lassign [wait] pid id os code
set f [open $env(T)/plain.out]; puts -nonewline [read $f]; close $f
puts "exit=$code"
```

Steps:

1. `expect init.tcl rhosgobel`: `words parsed: 12`, `exit=0`.
   `$T/init-rhosgobel.out` holds three lines and nothing else:
   `owner created: <fingerprint>`, `device created: rhosgobel <id>` and
   `scope personal created: <id> manifest 1 epoch 1`. In the raw transcript
   the words appear only between `ESC[?1049h` and `ESC[?1049l`, the alternate
   screen; this prints 0:

   ```sh
   perl -0ne '@p = split /\e\[\?1049[hl]/; $n = 0;
     for $w (split " ", `cat "$ENV{T}/phrase"`) { $n += () = "$p[0]$p[2]" =~ /\b\Q$w\E\b/g }
     print "$n\n"' "$T/init-rhosgobel.raw"
   ```

   After the screen closes, only `Recovery phrase confirmed` is drawn. No
   other file holds a word
   (`for w in $(cat "$T/phrase"); do grep -rlw --exclude=phrase --exclude='*.raw' -e "$w" "$T"; done`
   prints nothing), `$XDG_STATE_HOME/bilbo/keys` is `drwx------`, and
   `owner.key`, `device.key` and `keys.lock` are `-rw-------`.
2. `$B device init </dev/null`: `owner kept`, `device kept`,
   `scope personal kept`, exit 0.
3. Start a relay for this owner on a loopback port and point the scope at it:

   ```sh
   FP=$(sed -n 's/^owner created: //p' "$T/init-rhosgobel.out")
   $B relay --data "$T/relay" --owner "$FP" --listen 127.0.0.1:0 2> "$T/relay.err" &
   RELAY=$!; sleep 1
   echo "scope.personal.sync = $(sed -n '1s/.*listening on //p' "$T/relay.err")" > "$BILBO_CONFIG"
   ```

   `$B device init </dev/null` reports
   `scope personal failed: changing the URL needs a terminal`, exit 1, and
   writes no manifest; `expect plain.tcl device init` reports
   `scope personal updated: <id> manifest 2 epoch 1`.
4. Recover into a second state folder against the same store:
   `XDG_STATE_HOME=$T/state2 expect recover.tcl "$T/phrase" bywater`. No
   fingerprint question (the local manifest vouches for the owner);
   `owner recovered`, `device created: bywater <id>`,
   `scope personal updated: <id> manifest 3 epoch 1`.
5. On bywater (`XDG_STATE_HOME=$T/state2`), `expect plain.tcl device revoke
   rhosgobel` reports `manifest 4 epoch 2`, and `$B device list` prints only
   bywater, marked `this`. On rhosgobel, `$B device` shows the scope's name as
   `-`. Stop the relay with `kill "$RELAY"`.
6. In a fresh world with an empty config (`: > "$BILBO_CONFIG"`) and
   `$BILBO_HOME/notes`, `expect recover.tcl <first world>/phrase fresh n`. The
   owner fingerprint is shown, the question defaults to No, and bilbo exits 1
   with `the fingerprint does not match; nothing was written`. The world holds
   no keys and no `keys.lock`.
7. `CLAUDE_CODE_CHILD_SESSION=1 expect plain.tcl device init --name x`: `bilbo
   device init needs a terminal: run it yourself, in a terminal, not through an
   agent`, exit 1, and nothing under `$XDG_STATE_HOME`.
7b. `CLAUDECODE=1 expect ... device init --name x` in a fresh world shows the
   recovery phrase: an IDE's terminal is a person.

`bilbo setup` runs the same ceremony when you answer yes to `Sync notes
between your devices?` on a device without keys; the same checks apply to its
transcript.

## Pairing

`bilbo pair` draws with `Terminal` on both devices, so this needs a person and
two terminals. Make two worlds as in [A throwaway world](#a-throwaway-world),
`$A` and `$B`, and run the Device ceremony's steps 1 and 2 in the first so it
holds keys and a scope that syncs through a folder. In the second, point
`--via` at the same folder, create `$BILBO_HOME/notes` and leave the config
empty. Use a terminal at least 100 columns wide unless a step says otherwise.

1. In terminal A, `bilbo pair`: an intro line, a box titled `On the new
   device, run` holding the command and `The code is <code>. It works once, for
   10 minutes.`, then a spinner `Waiting for the new device`.
2. In terminal B, run the command from the box. B shows `Looking for pairing
   <n>`, then a box titled `Fingerprint` with twelve digits, its name and id.
   A's spinner is gone and A shows `<name> <id> asks to join personal` and the
   question with the same digits, answered no at first.
3. Press `y` on A. A closes with `Paired` and prints `paired ...`; B
   shows `Waiting for the other device to confirm`, `Fetching the scopes`,
   `Paired with <A's name>` and prints `paired with ...`. No spinner line stays
   on either screen.
4. Repeat from step 1 in fresh worlds and answer no on A, then press Enter alone,
   then press Esc, then Ctrl-C: A closes with `Not paired` and `not confirmed;
   nothing was sent`; B closes with `Not paired` and `the other device declined;
   nothing was received` within a second, not after the 10 minutes. Neither
   world gains a device.
5. Narrow A to 60 columns (`stty cols 60`) and run `bilbo pair --via` with a
   folder path longer than the width: the command prints whole on its own line
   above a box titled `Pairing code`, with no border inside the command, and
   copying that line gives a working command.
6. Run B's command under `CLAUDE_CODE_CHILD_SESSION=1`: no box or spinner, and
   stderr holds `bilbo: fingerprint <digits> for <name> <id>; confirm on the
   device that showed the code`.
