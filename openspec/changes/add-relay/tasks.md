# Tasks

Run every command below from the repo root as `nix develop -c sh -c '<command>'`, with `CARGO_TARGET_DIR` set to the checkout's `target`. A new file under `src/` or `tests/` needs `git add -N` before any Nix command can see it. This change builds on the archived `add-device-keys` (manifest verification, fingerprints, device keys), `add-sync` (the transport interface and the `file://` writer in `src/sync/transport.rs`, the owner-scope fetch in `src/sync/scopes.rs`, the two-store tests) and `add-device-pairing` (the mailbox exchange in `src/identity/pair/`). Until every caller of a new item exists, run clippy as `cargo clippy --locked --all-targets -- -D warnings -A dead_code`; the final gate (6.3) runs it with no allowance.

## 1. The dependency, the skeleton and HTTP framing

- [x] 1.1 Add `httparse = "1.10.1"` to `Cargo.toml`, let cargo update the lock (the bilbo package's dependency list changes), and confirm `Cargo.lock` gains no new package. Verify with `cargo build && cargo build --locked && ! git diff --no-ext-diff Cargo.lock | grep -q '^+name'`
- [x] 1.2 Add `src/relay/http.rs` per design.md's framing and concurrency rules, behind the `Handler` trait (`now`, `head`, `answer`) of task 1.3. Accept on a std `TcpListener`, one thread per connection inside a `std::thread::scope`, at most 256 at once; at the cap, answer 503 `busy` with `Retry-After` at once. Stop accepting once `stop` is set and a connection wakes the loop. The request line and headers must arrive within 10 seconds of the accept, and the body may idle at most 30 seconds. Parse with `httparse`, with headers capped at 16 KiB (431). Answer 400 `bad-request` to bytes that are not an HTTP request, any `Transfer-Encoding`, a repeated or non-decimal `Content-Length`, a missing `Host`, an `Expect` other than `100-continue`, a target that is not origin-form, and a `GET` with a body. Answer 411 when a `PUT` has no length. On `Head::Read`, send `100 Continue` when it is expected, then hand `answer` a reader capped at `Content-Length`. On `Head::Refuse` before the body is read: respond, shut down writes, drain for up to 2 seconds, then close. Every response carries `Content-Length`, `Connection: close` and `Bilbo-Time` from `Handler::now`. Unit-test each case over a raw `TcpStream` on `127.0.0.1:0` with a test handler: 200 stalled clients that do not block a new request, the cap, the smuggling shape, the two lengths, the `100 Continue`, and that a client still sending reads the 413. Verify with `cargo test --locked --bin bilbo relay::http::`
- [x] 1.3 Lay out the change's shared files once, so every later task owns whole files (the lead's `wave0.patch`):
  - `src/relay/`, a domain that is itself the verb: `mod.rs` with the flags (`--data`; a repeatable `--owner` read by a new `keys::parse_fingerprint`, any case, with or without hyphens; `--listen`, defaulting to `127.0.0.1:8738`; the three limits, each from 1 to 1,048,576), `run` and the test-only `start`, `State`, `Clock` and the sweeper's tick, and the order of start-up (the data folder, the bind and the startup line with the bound port and the non-loopback warning, then the walk); stubs of `http.rs`, `store.rs`, `admit.rs`, `route.rs`, `mailbox.rs` and `walk.rs` with their fixed signatures;
  - `relay` in `src/main.rs` (`mod` line, `relay::run` arm with a log sink that calls `print_stderr`, USAGE lines) and in the USAGE copies in `tests/cli.rs` and `tests/recall.rs`; `relay` in `tests/layout.rs`' `DOMAINS`, `relay/` in its `VERBS`, and `httparse` in `relay/http.rs` in its `PLACEMENT`;
  - `src/sync/remote/` with `mod.rs` and the stub `sign.rs`, its `mod` line in `src/sync/mod.rs`;
  - `transport::Keys` (the device, the owner key when held, and whether mailbox requests are signed) and `transport::open(url, &Keys)`, with every caller moved to it; `transport::{is_mailbox_name, manifest_number, segment_seq}` made public for the relay's target grammar;
  - `manifest::verify_next`, the secret-free check of one version after a scope's valid ones, and `shared::hash::Hasher`, a SHA-256 fed as a body streams;
  - `tests/common::Relay`, a `bilbo relay` child on `127.0.0.1:0` that reads the port from its startup line.

  Verify with `cargo test --locked --bin bilbo relay:: && cargo test --locked --bin bilbo identity::keys:: && cargo test --locked --bin bilbo identity::manifest:: && cargo test --locked --bin bilbo shared::hash:: && cargo test --locked --test layout && cargo test --locked --test cli && cargo test --locked --test recall`

## 2. Request signatures, the client, and the devices that use it

- [x] 2.1 Add the signing text and its check to `src/sync/remote/sign.rs`: `bilbo-relay-1`, method, the target as received, time, nonce and body hash, with the four hex headers; `read` decodes them, and `verify` takes the relay's clock as an argument and returns `Bad::Signature` or `Bad::Clock`, the time window first. Unit-test a round trip, a changed body, a changed target, a missing header, a malformed hex field, and times at 300 and 301 seconds. Verify with `cargo test --locked --bin bilbo sync::remote::sign::`
- [x] 2.2 Implement `Transport` for `https://` and loopback `http://` URLs in `src/sync/remote/mod.rs`, and pick it in `transport::open` by scheme where it picks `file://`, dropping `add-sync`'s not-supported line for these schemes. Build the `ureq` agent with `max_redirects(0)`, `http_status_as_error(false)`, a 10-second connect timeout, a 300-second global timeout, and no proxy for a loopback URL. Sign `GET /v1/scopes/` and manifest reads with the owner key when `Keys` holds it, every other scope request with the device key, except the create of a manifest version that does not list the device, which takes the owner key when `Keys` holds it, and mailbox requests only when `Keys::opener` is set. On 401 `clock`, retry once with a fresh nonce and keep the offset for the process, in a table keyed by URL. Map the trait as design.md's Modules section says: `keeps` is `true` (so `add-sync`'s Own segments survive and Damaged own segments stay `file://` only), `create` counts 201 and 200 as `Put::Created`, 409 `exists` as `Put::Exists` and 507 `quota` as `Put::Full`; `list_after` follows `more`; `probe` is the contiguous run after the cursor; `replace` refuses; `sweep`, `remove_mailbox` and `sweep_mailboxes` do nothing. Add `remote::identify`, the unsigned `GET <url>/v1/`, with `Probe::NotRelay` and `Probe::Unreachable`. Turn every refusal into the messages of the `relay-transport` spec, each naming the relay URL: 502, 503 or 504 without `Bilbo-Time` as unreachable, any other answer without it as not a relay, `invalid`, a full scope, a full mailbox, a redirect, an untrusted certificate. In `src/note/watch.rs`, print a relay URL's failure as `sync <name>: <message>`, not inside the folder line `<url> is not reachable: <why>`. Move `transport::tests::a_url_picks_its_transport_by_scheme`'s https case to the `sync-transport` delta's `A relay URL`. Unit-test against a fake loopback server: redirects, a missing `Bilbo-Time` on a 404 and on a 502, a repeated `clock`, the kept offset, each refusal message, an unreachable port, which key signs each request, and a non-loopback `http://` URL that sends nothing. Verify with `cargo test --locked --bin bilbo sync::remote:: && cargo test --locked --bin bilbo sync::transport:: && cargo test --locked --bin bilbo note::watch::`
- [x] 2.3 In `src/setup/syncing.rs`'s sync step (`reach`), check a relay scope with `remote::identify`, and report `ok` only on 200 with `{"relay":"bilbo","api":1}`, `failed: <url> is not a bilbo relay` on any other answer, and `failed: <url> is not reachable: <reason>` when nothing answers; drop the not-supported line. Cover the `setup` delta's `A scope on a relay`, `A URL that is not a relay` and `A relay that is down` in `src/setup/syncing.rs`'s unit tests against an in-process relay (`relay::start`) and a fake loopback server. Verify with `cargo test --locked --bin bilbo setup::syncing::`
- [x] 2.4 `bilbo device recover` in `src/identity/device.rs` (`fetch`) fetches every syncing scope with no local manifest, on a relay as on a folder: drop the `file://` filter, and open the transport with the phrase's owner key, so `scopes::list` lists the owner's scopes with an owner-signed `GET /v1/scopes/` and reads each chain with owner-signed reads; the verification, the opening of `name` and the choice among duplicates stay `add-sync`'s. It reports `unsealed` only when the transport holds no such scope. Drop `UNSEALED_RECOVER` if no path reaches it any more. `recover` needs a terminal, so cover every scenario of the `device-identity` delta in `src/identity/device.rs`'s unit tests with the scripted prompter, against `relay::start` and against a folder, where the `file://` scenarios are tested today; `tests/device.rs` adds the binary's refusals with a `tests/common::Relay` child. Verify with `cargo test --locked --bin bilbo identity::device:: && cargo test --locked --test device`
- [x] 2.5 In the setup wizard (`src/setup/wizard.rs` and `src/setup/syncing.rs`), ask where to sync: a folder, or a relay URL (`https://`, or `http://` to a loopback host) checked with `remote::identify` and asked again on failure; plain `http://` to another host is refused with no request sent. After the confirmation, write the URL and copy in only the picked scope's chain through `inspect` and `copy`, which read the relay with the owner key in hand. `Turned` names a folder only for a `file://` URL. Cover the `setup` delta's `Turning sync on in the wizard` and `Applying sync from the wizard` scenarios with the scripted prompter against an in-process relay, including `First device on a relay` and `Every device lost, through the wizard`. Verify with `cargo test --locked --bin bilbo setup::`
- [x] 2.6 Pair through a relay in `src/identity/pair/`: drop `show.rs`'s and `join.rs`'s `cannot reach https:// transports yet` refusals, open A's transport with `Keys::opener` set and B's without it, send each `https://` scope's URL in its grant and write it on B, and poll a relay every 2 seconds as `Limits::poll_https` says. Report a 507 `quota` on a mailbox message as `relay <url> has no room for a pairing now; try again later`. Replace the refusal tests with the `device-pairing` delta's `A relay URL`, and add `pair_through_relay` to `src/identity/pair/exchange.rs`: the one-process exchange against `relay::start` on `127.0.0.1:0`, with A's and B's mailbox requests checked for their signatures. Verify with `cargo test --locked --bin bilbo identity::pair:: && cargo test --locked --test pair`

## 3. The relay (`src/relay/`, `relay-server` and `relay-api` specs)

- [x] 3.1 Cover the `Start the relay`, `The listen address` and `The data folder` scenarios and the `cli` and `config` deltas in `tests/relay.rs` and `tests/cli.rs`, running the built binary through `tests/common::Relay` with `--listen 127.0.0.1:0`. Verify with `cargo test --locked --test relay && cargo test --locked --test cli`
- [x] 3.2 Add the relay's own durable create-only storage in `src/relay/store.rs`, with `<data>/.tmp/` as its only temporary location. It is apart from the `file://` writer in `src/sync/transport.rs`, which keeps `swap::rename_new` and its `.<device id>-<16 lowercase hexadecimal characters>.tmp` temporaries, and is not changed:
  - `Data::open`: the folder created with mode 0700 when missing, `.relay.lock` taken with `File::try_lock` (`another relay serves <data>`), a path that is not a folder named, `.tmp/` emptied;
  - `stage`: body to `.tmp/<random>`, hashed as it streams, then `sync_all`;
  - `Staged::link`: new parent folders created and synced with their parents, `hard_link` to the final name (on `EEXIST`, compare bytes: `Same` or `Other`), parent `sync_all` through std, `File::open(dir)?.sync_all()`;
  - the temporary file removed after an error at any step: ENOSPC or EDQUOT gives `Full` (507 `quota`), anything else `Failed` (500 `internal`).

  Unit-test the writer with injected failures at each step. In `tests/relay.rs`, add the `Durable writes` scenarios: SIGKILL mid-body over a raw socket, then restart and retry; SIGKILL after a 201, then read; and a full size-limited tmpfs as `--data` when `BILBO_TEST_TMPFS` names one, skipped otherwise. Run them on macOS as well. Verify with `cargo test --locked --bin bilbo relay::store:: && cargo test --locked --test relay`
- [x] 3.3 Add `src/relay/admit.rs`, the relay's rules on scopes with no HTTP in them:
  - the access rule: a listed device, writing only its own device folder; the owner only for reading manifests and `GET /v1/scopes/`; 403 `not-admitted` for an unknown scope as for a held one, and `invalid` for a scope the walk marked;
  - manifest 1 admission and the n+1 chain through `manifest::verify_next`, including one `chain` entry per epoch from 1 to `epoch` minus 1 with manifest n-1's entries unchanged (change 3's Manifest validity), the owner admitted and unchanged, and the request's key listed or the owner's;
  - seq contiguity, and `not-next` above the next n or seq;
  - the quotas: `--max-scopes` valid scopes per owner, `--max-scope-mb` with a body's length reserved under the scope's mutex before it streams and released when the create fails, `--max-object-mb`, and the 1 MiB manifest cap;
  - `Scopes::enrolled`, who may open a nameplate.

  Unit-test each rule with manifests built through `manifest::create` and `add_device`, and each `Limits` scenario's arithmetic with small limits. Verify with `cargo test --locked --bin bilbo relay::admit::`
- [x] 3.4 Add the start-up walk in `src/relay/walk.rs`: read the data folder as a `transport::Folder`, verify every chain with `sync::scopes::chain`, check that each device's seqs run from 1 with no gap and that the owner was passed with `--owner`, mark a failing scope `Invalid` or `NotAdmitted` with one log line naming its id and reason, and rebuild the byte totals, latest manifests and highest seqs. Files outside the tree's grammar are ignored and nothing is changed. Unit-test each case. Cover `Admitted and valid scopes` in `tests/relay.rs`: a hand-edited `2.json`, a seq gap, an owner dropped from the flags, two owners, and a copied `file://` folder. Verify with `cargo test --locked --bin bilbo relay::walk:: && cargo test --locked --test relay`
- [x] 3.5 Add the mailbox in `src/relay/mailbox.rs`:
  - a first message signed by a device of a valid, admitted scope (`Scopes::enrolled`);
  - later messages signed by the opener's key under the signed-request time and replay rules, except one unsigned message per nameplate;
  - at most 8 messages of 4 KiB per nameplate, and 32 open nameplates;
  - expiry 30 minutes after the first message, with `sweep` every 30 seconds and `Mailbox::open` removing expired nameplates by modification time at start and counting the rest;
  - requests that no known key signed limited per peer address to 60 a minute and 4 distinct nameplates in 10 minutes, with addresses held in memory and forgotten after 10 minutes idle.

  Unit-test every mailbox scenario with an injected clock and injected peer addresses. Verify with `cargo test --locked --bin bilbo relay::mailbox::`
- [x] 3.6 Add the log lines in `src/relay/route.rs`:
  - one per created object: `manifest` or `segment` with ids, n or seq and size, or `mailbox` with only the size;
  - one per 4xx refusal (other than 404) of a request whose signature verified under a key the relay knows, and one per 500 or 507;
  - one line a minute that counts every other 4xx refusal by reason, written by `route::tick`;
  - one per invalid scope at start (task 3.4);
  - none for reads.

  Cover the `What the relay logs` scenarios in `tests/relay.rs`. After a pairing exchange and a refused request, check that no line after the startup line holds the nameplate as one, the peer's address, or any body bytes. Check that 10,000 unsigned refusals give one counting line. Verify with `cargo test --locked --bin bilbo relay::route:: && cargo test --locked --test relay`
- [x] 3.7 Add the routing in `src/relay/route.rs`, `http::Handler` for `State`:
  - `GET /v1/`, the tree's grammar byte for byte (with `manifest/latest`, both listings, `?after=` and `scopes/`), and 405;
  - the check order from the `relay-api` spec: framing and grammar, method, the time window, the signature over the body's hash (after the body is staged, for a `PUT`), the nonce, recorded only after its signature verifies under a key the relay knows, in a cache of 600 seconds and 100,000 entries (503 `busy` beyond), then admission and the object;
  - reads, `manifest/latest` with `Bilbo-Manifest`, both listings with the 1,000-entry page and `after`, and `GET /v1/scopes/`;
  - creates: the body capped before it is read (413), reserved, staged, checked through `admit`, linked through `store`, existence checked before contiguity;
  - the mailbox's paths handed to `mailbox`;
  - every response's reason from the `relay-api` spec's list, and `Retry-After` on 429 and 503.

  Unit-test every `relay-api` scenario except the mailbox's against `relay::start` on `127.0.0.1:0` with an injected clock, and the handler directly with an injected peer address. Verify with `cargo test --locked --bin bilbo relay::route::`

## 4. End to end

- [x] 4.1 Run `add-sync`'s two-store sync scenarios against a `bilbo relay` child on `http://127.0.0.1:<port>`, alongside their `file://` runs. Check that the relay's data folder then reads as a `file://` transport with the same objects. Run `add-device-pairing`'s one-process exchange against a relay on `127.0.0.1:0` (task 2.6), in every build profile, since that change ships no test hook. Verify with `cargo test --locked --test relay -- sync_through_relay data_folder_is_a_file_transport && cargo test --locked --bin bilbo identity::pair::exchange::tests::pair_through_relay`

## 5. Service and docs

- [x] 5.1 Add `nixosModules.relay` to `flake.nix`:
  - the `services.bilbo-relay` options: `enable`, `package` (this flake's package for the host's system), `owners` (asserted non-empty), `listen`, `maxScopes`, `maxScopeMb` and `maxObjectMb`;
  - the hardened `bilbo-relay.service` from design.md, with `StateDirectoryMode = "0700"` and `UMask = "0077"`;
  - a check, on every system of `forAllSystems`, that evaluates a minimal `x86_64-linux` `nixosSystem` (with `fileSystems."/"`, `boot.loader.grub.enable = false` and `system.stateVersion`) and asserts `ExecStart` and `StateDirectoryMode`, and that with `owners = [ ]` the module's own assertion is the failing one, with a message naming `services.bilbo-relay.owners` (read from `config.assertions`).

  Verify with `nix flake check -L`
- [x] 5.2 Update `AGENTS.md`:
  - `relay/` among the domains, as a domain that is itself a verb like `setup/`, `relay::run` beside `setup::run` and `check::run` in the dispatch sentence, `relay/` in the verb list, and `relay` among the verbs that take a callback for their lines;
  - `sync/remote/` as the relay's client, and the `sync` sentence on what `relay/` may use;
  - the relay tests run the built `bilbo relay` on `127.0.0.1:0` through `tests/common::Relay`, and other domains' unit tests use the test-only `relay::start`;
  - the durability tests also run on macOS, and the full-disk test only where `BILBO_TEST_TMPFS` names a size-limited tmpfs.

  Verify with `rg -q 'relay/' AGENTS.md && rg -q 'BILBO_TEST_TMPFS' AGENTS.md`
- [x] 5.3 Update `README.md` with:
  - running a relay: the NixOS module, the plain systemd unit, flags and limits;
  - TLS through `tailscale serve --bg 8738`, with `tailscale cert <host>` once (or a first sync that reports unreachable), or through Caddy with `timeouts`, `max_header_size`, and the fact that it logs addresses only with a `log` directive;
  - that a public relay is exposed to connection exhaustion, which a one-person relay accepts;
  - setting `scope.<name>.sync` to the relay URL, using the fingerprint from `bilbo device`;
  - what the relay can and cannot see;
  - how to drop or repair a scope by hand, including a damaged segment: stop the relay, copy `<root>/.bilbo/scopes/<scope id>/out/<seq>.seg` from the writing device over it, start the relay;
  - recovering with every device lost: `bilbo device recover`, or the wizard, with the relay URL in the config;
  - pairing through a relay;
  - what revocation guarantees, in change 3's words.

  Verify with `rg -q 'bilbo relay' README.md && rg -q 'tailscale serve' README.md && rg -q 'services.bilbo-relay' README.md && rg -q 'max_header_size' README.md`

## 6. Integration

- [x] 6.1 Smoke test on bree, recorded in `openspec/changes/add-relay/smoke.md`:
  - in a NixOS guest, enable the module with the owner fingerprint from rivendell's `bilbo device`, then run `tailscale cert <host>` and `tailscale serve --bg 8738`;
  - point a scratch scope on rivendell and on a second store at `https://<guest>.<tailnet>.ts.net`, sync a note both ways, and pair a device;
  - restart the service and sync again;
  - check that a self-signed certificate is refused, with a local `openssl s_server` on loopback;
  - grep the journal for the scratch nameplate and for rivendell's tailnet address.

  Verify with `rg -q 'bilbo-relay.service' openspec/changes/add-relay/smoke.md && rg -q 'synced' openspec/changes/add-relay/smoke.md`
- [x] 6.2 Run the suite on Linux (CI's platform), in a container or on a Linux host as an ordinary user, with a size-limited tmpfs mounted and names it in `BILBO_TEST_TMPFS`, so `hard_link`, `sync_all` and a real ENOSPC run there. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked` there
- [x] 6.3 Run the full suite and the package check on macOS. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked && nix flake check -L`
