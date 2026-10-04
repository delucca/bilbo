# Tasks

Run every command below from the repo root as `nix develop -c sh -c '<command>'`, with `CARGO_TARGET_DIR` set to the checkout's `target`. A new file under `src/` or `tests/` needs `git add -N` before any Nix command can see it. This change builds on `add-device-keys` (manifest verification, fingerprints, device keys) and `add-sync` (the transport interface and `file://` writer in `src/transport.rs`, the two-store tests), which must be applied first.

## 1. HTTP framing (`src/http.rs`)

- [ ] 1.1 Add `httparse = "1.10.1"` to `Cargo.toml`, let cargo update the lock (the bilbo package's dependency list changes), and confirm `Cargo.lock` gains no new package. Verify with `cargo build && cargo build --locked && ! git diff --no-ext-diff Cargo.lock | grep -q '^+name'`
- [ ] 1.2 Add `src/http.rs` per design.md's framing and concurrency rules. Accept on a std `TcpListener`, one thread per connection, at most 256 at once; at the cap, answer 503 `busy` with `Retry-After` at once. The request line and headers must arrive within 10 seconds of the accept, and the body may idle at most 30 seconds. Parse with `httparse`, with headers capped at 16 KiB (431). Answer 400 to any `Transfer-Encoding`, to a repeated or non-decimal `Content-Length`, to a missing `Host`, to an `Expect` other than `100-continue`, and to a target that is not origin-form under `/v1/`. Answer 411 when a `PUT` has no length, and send `100 Continue` when it is expected. Cap the body per path before reading (413), and read it through a reader capped at `Content-Length`. On a refusal before the body is read: respond, shut down writes, drain for up to 2 seconds, then close. Every response carries `Content-Length`, `Connection: close` and `Bilbo-Time`. Unit-test each case over a raw `TcpStream` on `127.0.0.1:0`: 200 stalled clients that do not block a new request, the cap, the smuggling shape, the two lengths, the `100 Continue`, and that a client still sending reads the 413. Verify with `cargo test --locked --bin bilbo http::`

## 2. Request signatures and the client (`src/remote.rs`)

- [ ] 2.1 Add the signing text and its check to `src/remote.rs`: `bilbo-relay-1`, method, the target as received, time, nonce and body hash, with the four hex headers; verification takes the relay's clock as an argument and returns `signature` or `clock`. Unit-test a round trip, a changed body, a changed target, a missing header, a malformed hex field, and times at 300 and 301 seconds. Verify with `cargo test --locked --bin bilbo remote::sign`
- [ ] 2.2 Implement the transport interface of `src/transport.rs` for `https://` and loopback `http://` URLs in `src/remote.rs`, selected by scheme where that module selects `file://`. Build the `ureq` agent with `max_redirects(0)`, `http_status_as_error(false)`, a 10-second connect timeout and a 300-second global timeout. Sign every scope request with the device key, and use the owner key only for the three recovery operations. On 401 `clock`, retry once with a fresh nonce and keep the offset for the process. Count a 200 on create as created, and map 409 `exists` to the interface's already-exists outcome. Follow `more` so `list_after` returns everything after the seq. Make `remove_mailbox` a no-op. Keep `add-sync`'s outbox from re-creating or replacing a stored segment on a relay: Own segments survive and Damaged own segments apply to `file://` only. Drop `add-sync`'s not-supported line for these schemes and `add-device-pairing`'s `cannot reach https:// transports yet` refusal, and move their tests to the `sync-transport` and `device-pairing` deltas' `A relay URL` scenarios. Report 507 `quota` as its own failure. Turn every refusal into the messages of the `relay-transport` spec: 502, 503 or 504 without `Bilbo-Time` as unreachable, `invalid`, and a full mailbox. Unit-test against a fake loopback server: redirects, a missing `Bilbo-Time` on a 404 and on a 502, a repeated `clock`, the kept offset, each refusal message, an unreachable port, and a non-loopback `http://` URL that sends nothing. Verify with `cargo test --locked --bin bilbo remote::`
- [ ] 2.3 In `src/setup.rs`'s `sync` step, check a relay scope with an unsigned `GET <url>/v1/` through `src/remote.rs`, and report `ok` only on 200 with `{"relay":"bilbo","api":1}`, `failed: <url> is not a bilbo relay` on any other answer, and `failed: <url> is not reachable: <reason>` when nothing answers. Cover the `setup` delta's `A scope on a relay`, `A URL that is not a relay` and `A relay that is down` in `src/setup.rs`'s unit tests against a fake loopback server. Verify with `cargo test --locked --bin bilbo setup::`
- [ ] 2.4 Extend `add-sync`'s owner-scope fetch in `src/transport.rs` to relays: list the owner's scopes with an owner-signed `GET /v1/scopes/` through `src/remote.rs` and read each chain with owner-signed reads; the verification, the opening of `name` and the choice among duplicates stay `add-sync`'s.

  `bilbo device recover` in `src/device.rs` then fetches every syncing scope with no local manifest, on a relay as on a folder, before the device is added, and reports `unsealed` only when the transport holds no such scope. Cover every scenario of the `device-identity` delta in `tests/device.rs`, against a `bilbo relay` child on `127.0.0.1:0` and against a folder. Verify with `cargo test --locked --bin bilbo transport:: && cargo test --locked --test device`
- [ ] 2.5 In the setup wizard (`src/wizard.rs` and `src/setup.rs`), accept a relay URL beside a folder: `https://`, or `http://` to a loopback host, checked with the unsigned `GET /v1/` of task 2.3 and asked again on failure. After the confirmation, write the URL and copy in only the picked scope's chain through task 2.4's fetch. Cover the `setup` delta's `Turning sync on in the wizard` and `Applying sync from the wizard` scenarios with the scripted prompter, including `First device on a relay` and `Every device lost, through the wizard`. Verify with `cargo test --locked --bin bilbo wizard:: && cargo test --locked --bin bilbo setup::`

## 3. The relay (`src/relay.rs`, `relay-server` and `relay-api` specs)

- [ ] 3.1 Add `src/relay.rs`, its `mod` line, dispatch arm and USAGE line in `src/main.rs`, and update the USAGE copies in `tests/cli.rs` and `tests/recall.rs`. Implement:
  - flag parsing: `--data`; a repeatable `--owner` in `add-device-keys`' fingerprint form (six groups of four base32 characters), read in any case and with or without hyphens; `--listen`, defaulting to `127.0.0.1:8738`; and the three limits, each from 1 to 1,048,576;
  - the data folder: mode 0700, `.relay.lock` taken with `try_lock`, `.tmp/` cleared at start;
  - the bind, the startup line with the bound port, and the non-loopback warning;
  - a log sink passed in from `main`.

  Cover the `Start the relay`, `The listen address` and `The data folder` scenarios and the `cli` and `config` deltas in `tests/relay.rs` and `tests/cli.rs`, running the built binary as a child with `--listen 127.0.0.1:0`. Verify with `cargo test --locked --test relay && cargo test --locked --test cli`
- [ ] 3.2 Add the relay's own durable create-only storage in `src/relay.rs`, with `<data>/.tmp/` as its only temporary location. It is apart from the `file://` writer in `src/transport.rs`, which keeps `swap::rename_new` and its `.<device id>-<16 lowercase hexadecimal characters>.tmp` temporaries, and is not changed:
  - body to `.tmp/<random>`, then `sync_all`;
  - new parent folders synced;
  - `hard_link` to the final name (on `EEXIST`, compare bytes: 200 or 409 `exists`);
  - parent `sync_all` through std, `File::open(dir)?.sync_all()`;
  - the temporary file removed after an error at any step: ENOSPC or EDQUOT gives 507 `quota`, anything else 500 `internal`.

  Unit-test the writer with injected failures at each step. Add the `Durable writes` scenarios to `tests/relay.rs`: SIGKILL mid-body over a raw socket, then restart and retry; SIGKILL after a 201, then read; and on Linux a full size-limited tmpfs as `--data`. Run them on macOS as well. Verify with `cargo test --locked --bin bilbo relay::store && cargo test --locked --test relay`
- [ ] 3.3 Add routing and admission:
  - `GET /v1/` and 405;
  - the check order from the `relay-api` spec, with the nonce recorded only after its signature verifies, in a cache of 600 seconds and 100,000 entries;
  - the scope access rule: a listed device, own device folder only; the owner only for reading manifests, `GET /v1/scopes/` and manifest creates; 403 for an unknown scope as for a held one;
  - existence checked before contiguity, for manifests and segments;
  - manifest 1 admission and the n+1 chain through `src/manifest.rs`, including one `chain` entry per epoch from 1 to `epoch` minus 1 with manifest n-1's entries unchanged (change 3's Manifest validity), with a per-scope mutex and seq contiguity;
  - `manifest/latest` with `Bilbo-Manifest`, and both listings with the 1,000-entry page and `after`.

  Unit-test every `relay-api` scenario except the mailbox's against an in-process relay on `127.0.0.1:0` with an injected clock. Verify with `cargo test --locked --bin bilbo relay::`
- [ ] 3.4 Add the start-up walk and the limits. The walk verifies every chain with `manifest::verify_scope`, checks contiguous seqs and admitted owners, marks a failing scope `invalid` or `not-admitted` with one log line, rebuilds the byte totals, latest manifests and highest seqs, and counts the open nameplates under `pair/` after deleting expired ones. Add `--max-scopes` per owner (counting valid scopes only), `--max-scope-mb` with `Content-Length` reserved under the per-scope mutex before streaming, `--max-object-mb`, and the 1 MiB manifest cap. Unit-test each `Limits` scenario with small limits. Cover `Admitted and valid scopes` in `tests/relay.rs`: a hand-edited `2.json`, a seq gap, an owner dropped from the flags, and a copied `file://` folder. Verify with `cargo test --locked --bin bilbo relay::limits && cargo test --locked --test relay`
- [ ] 3.5 Add the mailbox:
  - name rules;
  - a first message signed by a device of a valid, admitted scope;
  - later messages signed by the opener's key under the signed-request time and replay rules, except one unsigned message per nameplate;
  - at most 8 messages of 4 KiB per nameplate, and 32 open nameplates;
  - expiry 30 minutes after the first message, with a sweeper every 30 seconds and a clean-up by modification time at start;
  - unsigned requests limited per peer address to 60 a minute and 4 distinct nameplates in 10 minutes, with addresses held in memory and forgotten after 10 minutes idle.

  Unit-test every mailbox scenario with an injected clock and injected peer addresses. Verify with `cargo test --locked --bin bilbo relay::mailbox`
- [ ] 3.6 Add the log lines:
  - one per created object: `manifest` or `segment` with ids, n or seq and size, or `mailbox` with only the size;
  - one per 4xx refusal (other than 404) of a request whose signature verified;
  - one line a minute that counts every other 4xx refusal by reason;
  - one per invalid scope at start;
  - none for reads.

  Cover the `What the relay logs` scenarios in `tests/relay.rs`. After a pairing exchange and a refused request, check that no line after the startup line holds the nameplate as one, the peer's address, or any body bytes. Check that 10,000 unsigned refusals give one counting line. Verify with `cargo test --locked --test relay`

## 4. End to end

- [ ] 4.1 Run `add-sync`'s two-store sync scenarios against a `bilbo relay` child on `http://127.0.0.1:<port>`, alongside their `file://` runs. Check that the relay's data folder then reads as a `file://` transport with the same objects. Run `add-device-pairing`'s one-process exchange (`pair::run` with an injected answer, as that change's `src/pair.rs` unit tests run it) against a relay on `127.0.0.1:0`, in every build profile, since that change ships no test hook. Verify with `cargo test --locked --test relay -- sync_through_relay data_folder_is_a_file_transport && cargo test --locked --bin bilbo pair::tests::pair_through_relay`

## 5. Service and docs

- [ ] 5.1 Add `nixosModules.relay` to `flake.nix`:
  - the `services.bilbo-relay` options: `enable`, `package`, `owners` (asserted non-empty), `listen`, `maxScopes`, `maxScopeMb` and `maxObjectMb`;
  - the hardened `bilbo-relay.service` from design.md, with `StateDirectoryMode = "0700"` and `UMask = "0077"`;
  - a check on the Linux systems that evaluates a minimal `nixosSystem` (with `fileSystems."/"`, `boot.loader.grub.enable = false` and `system.stateVersion`) and asserts `ExecStart` and `StateDirectoryMode`, and a second one that fails with the module's own message when `owners = [ ]`.

  Verify with `nix flake check -L`
- [ ] 5.2 Update `AGENTS.md`:
  - `httparse` lives in `src/http.rs`, so the dependency rule reads `cliclack` in `src/wizard.rs`; `libc` in `src/wizard.rs`, `src/swap.rs` and `src/keys.rs`; `ring` in `src/model.rs`; `notify` in `src/watch.rs`; `sha2` in `src/versions.rs` and `src/keys.rs`; `ed25519-dalek`, `hpke`, `chacha20poly1305`, `hkdf` and `getrandom` in `src/keys.rs`; `base64` in `src/segment.rs`; `spake2` and `rand_core` (through `spake2::rand_core`) in `src/pake.rs`; `httparse` in `src/http.rs`;
  - `http` and `remote` join the library modules;
  - the relay tests run the built `bilbo relay` on `127.0.0.1:0` and read the port from its first stderr line;
  - the durability tests also run on macOS.

  Verify with `rg -q 'httparse' AGENTS.md && rg -q 'src/http.rs' AGENTS.md`
- [ ] 5.3 Update `README.md` with:
  - running a relay: the NixOS module, the plain systemd unit, flags and limits;
  - TLS through `tailscale serve --bg 8738`, with `tailscale cert <host>` once (or a first sync that reports unreachable), or through Caddy with `timeouts`, `max_header_size`, and the fact that it logs addresses only with a `log` directive;
  - that a public relay is exposed to connection exhaustion, which a one-person relay accepts;
  - setting `scope.<name>.sync` to the relay URL, using the fingerprint from `bilbo device`;
  - what the relay can and cannot see;
  - how to drop or repair a scope by hand, including a damaged segment: stop the relay, copy `<root>/.bilbo/scopes/<scope id>/out/<seq>.seg` from the writing device over it, start the relay;
  - recovering with every device lost: `bilbo device recover`, or the wizard, with the relay URL in the config;
  - what revocation guarantees, in change 3's words.

  Verify with `rg -q 'bilbo relay' README.md && rg -q 'tailscale serve' README.md && rg -q 'services.bilbo-relay' README.md && rg -q 'max_header_size' README.md`

## 6. Integration

- [ ] 6.1 Smoke test on bree, recorded in `openspec/changes/add-relay/smoke.md`:
  - in a NixOS guest, enable the module with the owner fingerprint from rivendell's `bilbo device`, then run `tailscale cert <host>` and `tailscale serve --bg 8738`;
  - point a scratch scope on rivendell and on a second store at `https://<guest>.<tailnet>.ts.net`, sync a note both ways, and pair a device;
  - restart the service and sync again;
  - grep the journal for the scratch nameplate and for rivendell's tailnet address.

  Verify with `rg -q 'bilbo-relay.service' openspec/changes/add-relay/smoke.md && rg -q 'synced' openspec/changes/add-relay/smoke.md`
- [ ] 6.2 Run the suite on Linux (CI's platform) in a container whose `--data` tests use a size-limited tmpfs, so `hard_link`, `sync_all` and a real ENOSPC run there. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked` inside the container
- [ ] 6.3 Run the full suite and the package check. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked && nix flake check -L`
