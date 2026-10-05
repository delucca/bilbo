# add-device-pairing smoke test

## 2.7 Pairing bagend from rivendell through a Syncthing folder

Run on 2026-10-05 between rivendell (macOS, aarch64) and bagend (Linux, x86_64). The transport was `work/add-device-pairing/smoke-sync/` inside the planning notebook, which Syncthing syncs between the two machines: `file:///Users/delucca/Notebooks/20261002T132305Z--bilbo-initial-launch__personal/work/add-device-pairing/smoke-sync` on rivendell and `file:///srv/notebooks/20261002T132305Z--bilbo-initial-launch__personal/work/add-device-pairing/smoke-sync` on bagend, a different path on each machine. Syncthing's watcher delay there is 10 seconds.

- **Binaries.** Run 1 used `target/debug/bilbo` at `1461e72` on rivendell and `nix build path:<tree>#default` of the same tree on bagend. Runs 2 to 6 used `e4bda39`, built the same way, after the final review's fixes (none touch the paths run 1 took).
- **A (rivendell).** Each world sets `HOME`, `BILBO_HOME`, `XDG_STATE_HOME` and `BILBO_CONFIG` under a scratch folder. A ran under `env -u CLAUDECODE -u CODEX_THREAD_ID`, in an `expect` pseudo-terminal that answered A's question. A took the fixture `rivendell` keys and the fixture `personal` manifest (versions 1 and 2, listing `rivendell` and `bagend`) from `tests/fixtures/device/`. Its config syncs `personal` and `shared` through the folder. `bilbo device init` created `shared` (version 1, pending).
- **B (bagend).** A fresh world with an empty store, no keys and no config file, with `XDG_CONFIG_HOME` set. B ran over `ssh` with no terminal. Its lines below carry `[B]`.
- **Fingerprints.** Each run compares, by eye, the fingerprint and B's name and id that both machines printed.

### Run 1: B's default name is taken

The fixture manifest already lists a device named `bagend`, which is B's host name.

```
bilbo: pairing code 163-visit-hobby-discover
bilbo: on the new device, run: bilbo pair 163-visit-hobby-discover --via file:///Users/delucca/Notebooks/.../smoke-sync
bilbo: the code works once, for 10 minutes
[B] bilbo: fingerprint 1586 4405 6125 for bagend hnqbpsqg6tehuqm6w37j5opsyt; confirm on the device that showed the code
bilbo: a device named bagend is already enrolled; pair again with --name on the new device
[B] bilbo: the name bagend is taken; run bilbo pair again with --name
[B] B_EXIT=1
```

A exited 1 and asked nothing. An earlier attempt of this run (code `283-…`) died with the driver script, before B read `c.msg`. B's pending key from that attempt was reused: the same id `hnqbpsqg6tehuqm6w37j5opsyt` appears in every run below.

### Run 2: declined on A

`--name bagend-pair` on B, `n` on A.

```
[B] bilbo: fingerprint 3712 0638 1915 for bagend-pair hnqbpsqg6tehuqm6w37j5opsyt; confirm on the device that showed the code
bilbo: fingerprint 3712 0638 1915
bilbo: pair bagend-pair hnqbpsqg6tehuqm6w37j5opsyt into personal? compare the fingerprint on that device, then type y to confirm
n
bilbo: not confirmed; nothing was sent
[B] bilbo: the other device declined; nothing was received
```

Both machines showed the same fingerprint, name and id. Both exited 1.

### Run 3: one wrong word, then the right code

B typed `764-day-client-zoo` for `764-day-client-entry`.

```
[B] bilbo: fingerprint 2770 0556 4346 for bagend-pair hnqbpsqg6tehuqm6w37j5opsyt; confirm on the device that showed the code
bilbo: the other device used a wrong code; this code is used up, run bilbo pair again
[B] bilbo: wrong code; run bilbo pair again on the other device for a new one
$ (bagend) bilbo pair 764-day-client-entry --via file:///srv/notebooks/.../smoke-sync --name bagend-pair
[B] bilbo: code 764 was already used; run bilbo pair again on the other device
```

The retry answered at once, because the mailbox `pair/764/` is kept after a wrong code. It was still there on both machines at the end.

### Run 4: paired

```
bilbo: pairing code 80-essence-style-slush
[B] bilbo: fingerprint 0601 1448 1014 for bagend-pair hnqbpsqg6tehuqm6w37j5opsyt; confirm on the device that showed the code
bilbo: fingerprint 0601 1448 1014
bilbo: pair bagend-pair hnqbpsqg6tehuqm6w37j5opsyt into personal? compare the fingerprint on that device, then type y to confirm
y
paired bagend-pair hnqbpsqg6tehuqm6w37j5opsyt: personal
[B] paired with rivendell: personal
[B] bilbo watch starts syncing them within one cycle
```

- **Timing.** From the code to both exits took 3 min 25 s (23:38:57 to 23:42:22 UTC), all of it Syncthing carrying `a.msg`, `b.msg`, `c.msg` and manifest version 3 across.
- **Device list.** `bilbo device list` on both machines lists `bagend`, `bagend-pair` and `rivendell`, marking `this` on each.
- **Scope on bagend.** `bilbo device` on bagend reports `scope personal ho5qmsdzujmpbxetyxwzdgqz64 manifest 3 epoch 1 3 devices file://`, valid. Rivendell holds the same version as `manifest 3 pending` until its watch confirms it.
- **B's config.** It holds only `scope.personal.sync = file:///srv/notebooks/.../smoke-sync`, B's own path.
- **B's keys.** They are `keys/owner.key` and `keys/device.key`, `-rw-------`, in a `drwx------` folder. `<state>/bilbo/pair/` is gone.
- **Mailbox.** B removed `pair/80/`.

**Watch on bagend.** A wrote `plan-pairing-smoke.md` in `personal` (`bilbo new plan pairing-smoke --scope personal`, then a passage), and both machines ran `bilbo watch`: A with `sync.poll_seconds = 5`, B with the default 30. The note, with its text, was in bagend's store 95 seconds after the watchers started. B's watch printed only its start lines:

```
bilbo: watching /home/delucca/scratch/bilbo-pair-smoke/B/store/notes
bilbo: syncing personal through file:///srv/notebooks/.../smoke-sync
```

### Run 5: the enrolled bagend-pair joins `shared`

`bilbo pair --scope shared` on A; `bilbo pair <code> --via …` on bagend, with no `--name`.

```
[B] bilbo: fingerprint 8228 4682 7341 for bagend-pair hnqbpsqg6tehuqm6w37j5opsyt; confirm on the device that showed the code
bilbo: fingerprint 8228 4682 7341
bilbo: pair bagend-pair hnqbpsqg6tehuqm6w37j5opsyt into shared? compare the fingerprint on that device, then type y to confirm
y
paired bagend-pair hnqbpsqg6tehuqm6w37j5opsyt: shared
[B] paired with rivendell: shared
```

- **Keys unchanged.** `sha256sum` of bagend's `device.key` (`21999dcb…baae`) and `owner.key` (`c3211cf8…4555`) were the same before and after.
- **Scope.** `bilbo device` on bagend now also reports `scope shared eefc6iszyjpem46inzwf3o2zru manifest 2 epoch 1 3 devices file://`.
- **Config.** B's config gained `scope.shared.sync` with its own path.

### Run 6: two pairings at once

Two `bilbo pair --scope personal` ran on rivendell, the second while the first still waited. They showed `163-wagon-weather-track` and `32-chef-bus-wrist`, two nameplates. Both were stopped with Ctrl-C.

Verdict: pass. The smoke folder and both scratch worlds were deleted afterwards.
