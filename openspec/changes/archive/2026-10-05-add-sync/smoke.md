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

## 10.1 Two stores through iCloud Drive, a conflict resolved by Claude Code

Run on rivendell on 2026-10-05 with `target/debug/bilbo` built at `0527c4d`. Two stores, A and B, each with its own `BILBO_HOME`, `XDG_STATE_HOME` and `BILBO_CONFIG`, take the `rivendell` and `bagend` keys and the shared `personal` manifest (versions 1 and 2) from `tests/fixtures/device/`; the same manifest bytes were placed in the folder. Both configs say `scope.personal.sync = file:///Users/delucca/Library/Mobile Documents/com~apple~CloudDocs/bilbo-smoke-20261005` (a path with a space, written literally) and `sync.poll_seconds = 5`. Each ran `bilbo watch` without `CLAUDECODE` or `CODEX_THREAD_ID`. The scripts, both watch logs and Claude Code's transcript are in the planning notebook's `work/add-sync/smoke101/`.

1. A ran `bilbo new plan release --scope personal` and `bilbo new decision deploy --scope personal`, then appended two passages to each. Both notes, with their text, were in B's store 6 seconds later. Each watcher printed only its start lines.
2. Concurrent edits, before either watcher polled. In `plan-release.md`, A changed `## Setup` and B changed `## Rollout`. In `decision-deploy.md`, both changed `## Target`, differently. Within 10 seconds `plan-release.md` was identical on both, holding both edits, with no markers. `decision-deploy.md` was identical on both and held one block, sides `9b46cd969c2b` (B's) and `b1fb38126e5c` (A's). Each watcher printed `bilbo: notes/decision-deploy.md: conflict in 1 passage; run bilbo check`.
3. Before resolving:

```
A$ bilbo check
notes/decision-deploy.md: conflict: 'Target' holds 2 sides; keep what is right, remove the markers
exit 1
A$ bilbo sync
scope personal file:///Users/delucca/Library/Mobile Documents/com~apple~CloudDocs/bilbo-smoke-20261005: 2 notes, pushed 2026-10-05T13:49-03:00, pulled 2026-10-05T13:49-03:00
device personal rivendell: this device
device personal bagend: behind by 1 segments
local: 0 notes sync nowhere
conflict notes/decision-deploy.md: 1 passage
exit 1
A$ bilbo history deploy
f2a778fce6a6 2026-10-05T13:49-03:00 merged decision-deploy.md [conflict]
9b46cd969c2b 2026-10-05T13:49-03:00 edited decision-deploy.md from bagend
b1fb38126e5c 2026-10-05T13:49-03:00 edited decision-deploy.md
8b21f0ff85a3 2026-10-05T13:48-03:00 added decision-deploy.md
```

   B's `bilbo sync` was the same, with `rivendell: behind by 1 segments` and B as this device.
4. Claude Code 2.1.289 ran as A, unattended: `claude -p "Use the bilbo note skill to keep the decision note on the deploy target (topic deploy) current. We decided today: deploy straight to production behind a flag. The staging-first plan with a 5% canary is superseded and should not stay in the note." --plugin-dir <checkout>/plugins/bilbo --setting-sources project --no-session-persistence --permission-mode bypassPermissions`, from a scratch folder. The transcript shows the `bilbo:note` skill load, then `bilbo recall`, a Read of the note, and an Edit removing the block. Then `bilbo check`, which printed `notes/decision-deploy.md: conflict: dropped 2 lines of 'Target', first "Deploy straight to production behind a flag."; restore them or run bilbo sync declare deploy "<why>"`: the agent had first reworded the kept line. It restored that line word for word, and ran `bilbo sync declare deploy '<the user's decision>'`, which printed `declared decision-deploy.md: 1 lines dropped on purpose`. A last `bilbo check` printed nothing. Its report named the kept and the dropped lines.
5. B received the resolution and the declaration within 2 seconds. `decision-deploy.md` was identical on both, holding only `Deploy straight to production behind a flag.` under `## Target`.

```
A$ bilbo check            -> exit 0, no output
B$ bilbo check            -> exit 0, no output
B$ bilbo sync
scope personal file:///Users/delucca/Library/Mobile Documents/com~apple~CloudDocs/bilbo-smoke-20261005: 2 notes, pushed 2026-10-05T13:49-03:00, pulled 2026-10-05T13:49-03:00
device personal rivendell: up to date
device personal bagend: this device
local: 0 notes sync nowhere
exit 0
B$ bilbo history deploy
d668cd4d4c43 2026-10-05T13:49-03:00 edited decision-deploy.md from rivendell
de3b0648487a 2026-10-05T13:49-03:00 edited decision-deploy.md from rivendell [dropped]
f2a778fce6a6 2026-10-05T13:49-03:00 merged decision-deploy.md [conflict]
b1fb38126e5c 2026-10-05T13:49-03:00 edited decision-deploy.md from rivendell
9b46cd969c2b 2026-10-05T13:49-03:00 edited decision-deploy.md
8b21f0ff85a3 2026-10-05T13:48-03:00 added decision-deploy.md from rivendell
```

   A's `bilbo sync` showed `bagend: behind by 3 segments`, since B acknowledges at its next push or within the hour.
6. The folder then held `scopes/<id>/manifest/{1,2}.json` and nine segments, six from A and three from B, 44 KB in all. A grep for the notes' words over it found nothing. Both watchers were stopped, and the folder was deleted from iCloud Drive, along with the scratch stores.

Verdict: pass. The merge ids agree on both devices, and the conflict line appears on both. The resolution and the declared drop reached B, and `bilbo check` is clean on both.
