# add-sync smoke test

## 8.4 The setup wizard turns sync on with a new phrase

Run on rivendell (macOS) on 2026-10-05 with `target/debug/bilbo` built at `623f3c6` of the add-sync worktree, driven by `/usr/bin/expect` in a pseudo-terminal, under `env -i` (so without `CLAUDECODE` and `CODEX_THREAD_ID`). Each world `$W` sets `HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME`, `BILBO_HOME` and `BILBO_CONFIG` inside it, and `PATH` puts a fake `launchctl` (logs and succeeds) before `/usr/bin:/bin`, so the machine's real `io.github.delucca.bilbo.watch` agent is never touched (`launchctl list` showed the same pid before and after). `claude` and `codex` are not on that `PATH`. The script reads the words off the screen into an expect variable and types three back; no word was written to a file, and none appears below. The scripts are in the planning notebook's `work/add-sync/smoke84/`.

Answers: no embedder, record history yes, sync yes, scope `personal` (the default), folder `$W/sync`, no phrase from another device, written down yes, the three asked words, apply yes.

```
$ run.sh w3 wizard.tcl          # spawns: bilbo setup > $W/wizard.out
words parsed: 12
EXIT=0
$ cat $W/wizard.out
store created: $W/store/notes
config written: $W/conf/config
key skipped: no embedder
model skipped: not local
server skipped: not local
embedder skipped: none configured
claude skipped: not found
codex skipped: not found
hook skipped: no codex plugin
timer skipped: no embedder
watch installed: watching $W/store/notes
sync ok: personal through file://$W/sync (0 notes)
$ cat $W/conf/config
# bilbo config, written by bilbo setup 0.12.0 on 2026-10-05
scope.personal.sync = file://$W/sync
```

- The summary before `Apply these changes?` listed: create the store folder, write the config, search by keywords only, record note history (launchd agent `io.github.delucca.bilbo.watch`), `Sync personal through file://$W/sync`, create the folder `$W/sync`, create the device keys in `$W/state/bilbo/keys`, create the scope `personal`.
- The raw transcript holds `ESC[?1049h` before the numbered words and the owner fingerprint, and `ESC[?1049l` after the three asked words. All 12 words sit inside that span. Outside it, one 4-letter list word appears once, as `this` in the embedder menu's `Ollama on this machine`: menu text, not a leak.
- A scan of every file in the world (9 files, the transcript excluded) for each of the 12 words: 0 hits.
- `$W/sync` exists, mode `0700`, and is empty: setup writes nothing to the transport.
- Modes: `state/bilbo/keys` `drwx------`; `keys.lock`, `keys/owner.key`, `keys/device.key` `-rw-------`. The store holds `.bilbo/scopes/<id>/manifest/1.json` and `1.pending`.
- The fake `launchctl` logged `bootout --wait gui/501/io.github.delucca.bilbo.watch` then `bootstrap gui/501 $W/home/Library/LaunchAgents/io.github.delucca.bilbo.watch.plist`.

Verdict: pass.

Declining at the summary (`decline.tcl`, the same answers and a new phrase, then no): exit 1, `wizard.out` empty, and the world holds only the folders the runner made (`cache`, `conf`, `home`, `state`): no config, no keys, no store, no folder.

Verdict: pass.

The first two attempts answered every redraw of a prompt, so the extra keystrokes sat in the pty and answered later questions (the phrase question got `Yes`); both stopped before the summary and wrote nothing. The script was fixed to answer each prompt once. The transcripts, which hold the throwaway phrases, were deleted.
