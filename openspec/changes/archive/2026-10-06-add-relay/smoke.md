# add-relay smoke test

## 6.1 A relay on bagend behind `tailscale serve`, two stores and a pairing

Run on 2026-10-06 (UTC) between rivendell (macOS, aarch64) and bagend (NixOS 26.05 LXC guest 106 on bree, x86_64). The evidence is in the planning notebook's `work/add-relay/smoke/`.

- **Owner.** A throwaway owner: the fixture `rivendell` keys from `tests/fixtures/device/` (owner `yb4b-5aju-v6zb-x2nm-nc5x-ompf`, a public test seed), so no real key was admitted.
- **Relay.** dnix, uncommitted for the run: `bilbo.nixosModules.relay` in `nixosConfigurations.bagend`'s modules, and `services.bilbo-relay = { enable = true; owners = [ "yb4b-5aju-v6zb-x2nm-nc5x-ompf" ]; }` in `hosts/bree/guests/bagend.nix`. It was built on gondolin with `--override-input bilbo git+file:///Users/delucca/Developer/delucca/bilbo?ref=add-relay`. `nix store diff-closures` against the running system showed only `bilbo: ∅ → 0.14.0` and `unit-bilbo-relay.service: ∅ → ε`. bagend was switched to it.
- **Clients.** A release build of the branch on rivendell, in scratch worlds A and B (`HOME`, `BILBO_HOME`, `XDG_STATE_HOME` and a config each), run under `env -u CLAUDECODE -u CODEX_THREAD_ID`. A paired in an `expect` pseudo-terminal. `bilbo setup` was not run, because it would install the watcher into the real login session; its relay check is covered by `setup::syncing`'s unit tests.

### The service

`bilbo-relay.service` started under the module's full hardening (`DynamicUser`, `ProtectSystem=strict`, `SystemCallFilter=@system-service`, `MemoryDenyWriteExecute`, `ProtectProc=invisible` and the rest) inside the unprivileged LXC container:

```
$ systemctl is-active bilbo-relay
active
$ journalctl -u bilbo-relay -b
bagend systemd[1]: Started bilbo relay.
bagend bilbo[3428598]: bilbo: relay listening on http://127.0.0.1:8738
$ curl -s http://127.0.0.1:8738/v1/
{"relay":"bilbo","api":1}
$ stat -c '%a %U' /var/lib/private/bilbo-relay
700 bilbo-relay
```

### TLS

`tailscale cert bagend.tailbc380a.ts.net` was run once in a temporary folder, which was deleted afterwards. Then `tailscale serve --bg 8738` published `https://bagend.tailbc380a.ts.net (tailnet only)`. From rivendell, `curl -s https://bagend.tailbc380a.ts.net/v1/` returned `{"relay":"bilbo","api":1}`.

### A's first sync

A held `scope.relaysmoke.sync = https://bagend.tailbc380a.ts.net`, the scope from `bilbo device init` at manifest 1, and one note. A's watch printed `bilbo: syncing relaysmoke through https://bagend.tailbc380a.ts.net`. The relay logged:

```
bilbo: refused 403 not-admitted GET manifest 6yb6fuxikwowz4pyvcgusbolne
bilbo: manifest 6yb6fuxikwowz4pyvcgusbolne gr2q7gf5lh6pzfdnurnkvputhp 1 1125
bilbo: segment 6yb6fuxikwowz4pyvcgusbolne gr2q7gf5lh6pzfdnurnkvputhp 1 1105
```

The 403 is the owner-signed read of a scope the relay did not hold yet, which the client reads as absent (design.md, Modules) before it creates manifest 1.

### Pairing through the relay

```
bilbo: pairing code 704-differ-else-reunion
bilbo: on the new device, run: bilbo pair 704-differ-else-reunion --via https://bagend.tailbc380a.ts.net
[B] bilbo: fingerprint 4301 2541 9248 for smoke-b 3n5vq2bnxl2gxx5jt7j4sd7325; confirm on the device that showed the code
bilbo: fingerprint 4301 2541 9248
bilbo: pair smoke-b 3n5vq2bnxl2gxx5jt7j4sd7325 into relaysmoke? compare the fingerprint on that device, then type y to confirm
paired smoke-b 3n5vq2bnxl2gxx5jt7j4sd7325: relaysmoke
[B] paired with rivendell: relaysmoke
[B] bilbo watch starts syncing them within one cycle
```

- **Timing.** From the code to both exits took 5 seconds, with A's watch running throughout.
- **B's config.** It holds `scope.relaysmoke.sync = https://bagend.tailbc380a.ts.net`.
- **Devices.** `bilbo device list` on B lists `rivendell` and `smoke-b` (this). A reports `scope relaysmoke ... manifest 2 epoch 1 2 devices https://bagend.tailbc380a.ts.net`.
- **First attempt.** The first run (code `319-…`) stopped at A's question after 5 minutes, because the harness gave B a `BILBO_CONFIG` that named a missing file and B exited 2 before the mailbox. Only `a.msg` reached the relay. Its nameplate stays read-only until it expires, as the mailbox rules say.

### Notes synced both ways

Both watchers ran with `sync.poll_seconds = 5`.

- **A to B.** B's watch had A's note, with its text, 2 seconds after it started.
- **B to A.** A note written on B reached A in 4 seconds.

The relay logged `manifest ... 2` (the version adding `smoke-b`), and one segment per device and change.

### Restart

```
$ systemctl restart bilbo-relay
bagend bilbo[3430459]: bilbo: relay listening on http://127.0.0.1:8738
bagend bilbo[3430459]: bilbo: segment 6yb6fuxikwowz4pyvcgusbolne 3n5vq2bnxl2gxx5jt7j4sd7325 2 1153
bagend bilbo[3430459]: bilbo: segment 6yb6fuxikwowz4pyvcgusbolne gr2q7gf5lh6pzfdnurnkvputhp 3 1153
```

One more note from each side synced within 6 seconds of the restart. Both stores then held the same four notes. The start-up walk found the stored scope valid and logged nothing for it.

### What the journal holds

The whole journal of the unit, 20 lines, was grepped:

| Pattern | Matches |
|---|---|
| `pair/704` | 0 |
| `pair/319` | 0 |
| ` 704` | 0 |
| ` 319` | 0 |
| `704-` | 0 |
| `differ-else-reunion` | 0 |
| rivendell's tailnet address `100.117.37.47` | 0 |

Mailbox writes appear only as `mailbox <bytes>`. Behind `tailscale serve`, the relay sees only `127.0.0.1`.

### The data folder is a `file://` tree

`/var/lib/private/bilbo-relay` held `scopes/<scope>/manifest/<n>.json`, `scopes/<scope>/devices/<device>/<20-digit seq>.seg` and `pair/<nameplate>/<name>.msg`, with nothing else outside `.tmp/` and `.relay.lock`.

### A certificate the client does not trust

`openssl s_server` served a self-signed P-256 certificate on `127.0.0.1:8443`. A world C with `scope.tlscheck.sync = https://localhost:8443` printed:

```
bilbo: syncing tlscheck through https://localhost:8443
bilbo: sync tlscheck: relay https://localhost:8443: certificate not trusted: Other(OtherError(CaUsedAsEndEntity))
```

The reason is rustls's own text for a variant the client does not put into words. The client puts the common ones into words: wrong host name, expired, unknown issuer.

### Afterwards

The scratch worlds were deleted. `tailscale serve reset` was run on bagend, and bagend was switched back to its previous system without the module. `/var/lib/private/bilbo-relay` was removed. Whether bagend keeps a relay with the user's own fingerprint after the release is the user's decision.
