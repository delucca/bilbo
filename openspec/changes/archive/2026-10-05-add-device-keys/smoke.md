# add-device-keys smoke test

Run on rivendell (macOS) on 2026-10-05 with `target/release/bilbo` built at `3cc172c` of the add-device-keys worktree, driven by `/usr/bin/expect` in a pseudo-terminal. `$S` is a scratch folder; each world `$S/tN` sets `HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME`, `BILBO_HOME` and `BILBO_CONFIG` inside it. The lead's session runs under Claude Code, so every run that must pass the terminal rule ran under `env -u CLAUDECODE -u CODEX_THREAD_ID`. The scripts read the words off the screen into an expect variable and typed them back; no word of any phrase was written to a file, and none appears below. Fingerprints and ids are of throwaway identities. The scripts are in the planning notebook's `work/add-device-keys/smoke/`.

## 1. `bilbo device init > out.txt`, phrase confirmed

Config: `scope.personal.sync = file://$S/t3/sync`.

```
$ expect init.tcl init3 rivendell      # spawns: bilbo device init --name rivendell > $S/init3.out
words parsed: 12
confirmed
EXIT=0
$ cat init3.out
owner created: w6ii-xacc-k6av-ykh3-x3k4-ktxc
device created: rivendell whj5l4zn34jceyo2c5j2pk3vag
scope personal created: 2jy2a7ti5hammmhwj5qdjqokeq manifest 1 epoch 1
```

- `out.txt` (here `init3.out`) holds the three step lines and nothing else.
- The raw pty transcript (`log_file -a`) shows `ESC[?1049h ESC[H ESC[2J` before the numbered words and the owner fingerprint, and `ESC[2J ESC[?1049l` after the three asked words. A script split the transcript at those two sequences: 12 numbered words inside, 0 of them outside. After the screen only `Recovery phrase confirmed` is drawn.
- A scan of every file under the world (8 files) for each of the 12 words: 0 hits.
- Modes:

```
drwx------ state/bilbo/keys
-rw------- state/bilbo/keys/owner.key
-rw------- state/bilbo/keys/device.key
-rw------- state/bilbo/keys.lock
-rw-r--r-- store/.bilbo/scopes/2jy2a7ti5hammmhwj5qdjqokeq/manifest/1.json
-rw-r--r-- store/.bilbo/scopes/2jy2a7ti5hammmhwj5qdjqokeq/manifest/1.pending
```

Verdict: pass.

A first attempt answered every redraw of a `Word N` prompt, so surplus words sat in the pty's input and the tty echoed them on the main screen after bilbo exited. That was the script typing ahead, not bilbo drawing; the script was fixed to answer each position once, and the run above is the fixed one in a fresh world. A user who types ahead would see the same echo.

## 2. Rerun init, no terminal

```
$ bilbo device init </dev/null
owner kept: w6ii-xacc-k6av-ykh3-x3k4-ktxc
device kept: rivendell whj5l4zn34jceyo2c5j2pk3vag
scope personal kept: 2jy2a7ti5hammmhwj5qdjqokeq
exit=0
$ bilbo device </dev/null
device	rivendell	whj5l4zn34jceyo2c5j2pk3vag
owner	w6ii-xacc-k6av-ykh3-x3k4-ktxc
scope	personal	2jy2a7ti5hammmhwj5qdjqokeq	manifest 1 pending	epoch 1	1 devices	file://
exit=0
```

Verdict: pass.

## 3. Change the scope's URL

Config changed to `scope.personal.sync = https://relay.example.net`.

```
$ bilbo device init </dev/null
owner kept: w6ii-xacc-k6av-ykh3-x3k4-ktxc
device kept: rivendell whj5l4zn34jceyo2c5j2pk3vag
scope personal failed: changing the URL needs a terminal
exit=1
$ ls manifest/
1.json 1.pending
$ expect plain.tcl device init          # in a pty
owner kept: w6ii-xacc-k6av-ykh3-x3k4-ktxc
device kept: rivendell whj5l4zn34jceyo2c5j2pk3vag
scope personal updated: 2jy2a7ti5hammmhwj5qdjqokeq manifest 2 epoch 1
EXIT=0
```

`2.json`'s `transport` is `https://relay.example.net`, its `epoch` 1, with `2.pending` beside it. Verdict: pass.

## 4. Recover into a second state folder against the same store

World `t3b`: the same store and config, its own `XDG_STATE_HOME`.

```
$ expect recover.tcl init3.raw bagend    # spawns: bilbo device recover --name bagend, types the 12 words
EXIT=0
```

After the screen:

```
◇  Owner fingerprint
│  w6ii-xacc-k6av-ykh3-x3k4-ktxc
owner recovered: w6ii-xacc-k6av-ykh3-x3k4-ktxc
device created: bagend bfoe245do27hwtib5gflkrub6w
scope personal updated: 2jy2a7ti5hammmhwj5qdjqokeq manifest 3 epoch 1
```

- No fingerprint question: the local manifest is signed by that owner, so it vouches.
- The 12 typed words were echoed only inside the alternate screen (0 outside it in the transcript).
- A scan of every file under both worlds (16 files): 0 hits.

Verdict: pass.

## 5. Revoke the first device, in a terminal

```
$ expect plain.tcl device revoke rivendell     # on bagend
scope personal updated: 2jy2a7ti5hammmhwj5qdjqokeq manifest 4 epoch 2
EXIT=0
$ bilbo device            # on bagend
device	bagend	bfoe245do27hwtib5gflkrub6w
owner	w6ii-xacc-k6av-ykh3-x3k4-ktxc
scope	personal	2jy2a7ti5hammmhwj5qdjqokeq	manifest 4 pending	epoch 2	1 devices	https://relay.example.net
exit=0
$ bilbo device list       # on bagend
bagend	bfoe245do27hwtib5gflkrub6w	this
$ bilbo device            # on rivendell
device	rivendell	whj5l4zn34jceyo2c5j2pk3vag
owner	w6ii-xacc-k6av-ykh3-x3k4-ktxc
scope	-	2jy2a7ti5hammmhwj5qdjqokeq	manifest 4 pending	epoch 2	1 devices	https://relay.example.net
exit=0
```

The revoked device can no longer open the scope's name, so its line shows `-`. The scope is a relay scope, so no cloud-account line is printed (the unit tests cover the `file://` line). Verdict: pass.

## 6. Recover on a fresh store, fingerprint declined

World `t4`: empty store, empty config, no keys.

```
$ expect recover.tcl init3.raw fresh n
EXIT=1
```

After the screen:

```
◇  Owner fingerprint
│  w6ii-xacc-k6av-ykh3-x3k4-ktxc
◆  Does it match the fingerprint written down with the phrase, or shown by `bilbo device` on another device?
│  ○ Yes  / ● No
◇  Does it match the fingerprint written down with the phrase, or shown by `bilbo device` on another device?
│  No
bilbo: the fingerprint does not match; nothing was written
```

The question defaults to No. Afterwards the world holds only `store/notes`: no keys, no `keys.lock`, no `.bilbo`. Verdict: pass. A stray `n` echoed after exit was again the script answering a redraw twice.

## 7. An agent marker with a real terminal

```
$ expect agent.tcl        # sets CLAUDECODE=1, spawns: bilbo device init --name x
bilbo: bilbo device init needs a terminal: run it yourself, in a terminal, not through an agent
EXIT=1
```

Nothing is written under the world's state folder. Verdict: pass.

## 8. The setup wizard still draws and cancels

`Terminal` gained only `screen`; its prompt methods are unchanged. So of the archived `add-setup` and `add-local-embedder` expect runs, only the part that needs no install was repeated: `bilbo setup` in a throwaway world (`CLAUDE_CONFIG_DIR` and `CODEX_HOME` inside it), up to the first prompt, then Esc.

```
┌  bilbo setup
●  Notes go to $S/t6/store/notes. Set BILBO_HOME to put them elsewhere.
◆  Which embedder should recall use?
│  ● No embedder (keyword search only)
│  ○ Local embedder, run by bilbo
│  ○ Ollama on this machine
│  ○ OpenAI
│  ○ Another OpenAI-compatible URL
■  Which embedder should recall use?
└  Cancelled. Nothing changed.
bilbo: setup cancelled; nothing changed
EXIT=1
```

Nothing was created in the world. The launchd timer, the local llama-server and the masked API key paths were not rerun: they install real launchd jobs, and none of their code changed. Verdict: pass.

## Cleanup

The scratch folder `$S` was deleted after the run.
