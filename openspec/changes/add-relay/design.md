# Design

## Context

- After changes 1 to 5, a scope syncs through a transport URL. `add-sync` defines the transport operations and the `file://` transport, `add-device-keys` the manifest, the owner key and its fingerprint, the device keys and the `https://` form of `scope.<name>.sync`, and `add-device-pairing` the mailbox exchange. This change adds the server those `https://` URLs point at, and the client half that talks to it. The shared tree and its rules are in the planning notebook's `work/sync-spec/CONTRACT.md`.
- bilbo has no server code. Its only network code is the `ureq` 3.4.2 client in `src/embed.rs` and `src/model.rs`. `Cargo.lock` shows `ureq` built with `rustls` 0.23 on `ring` and with `webpki-roots` 1.0.9, the Mozilla roots compiled in. It also shows `ureq-proto` 0.6.4 pulling in `httparse` 1.10.1 and `http` 1.5.0.
- `config::is_local` (`src/config.rs`) already decides whether a URL's host is `localhost`, `127.0.0.1` or `::1`.
- The user's always-on machine is bree, a Proxmox node. Its guests are NixOS LXC containers, each a short file under `dnix/hosts/bree/guests/` that enables `services.tailscale` and the one service it exists for (`services.trademate`, for example). A relay there is one more such service.
- Load is small: all of `~/Notebooks`, 440 notes and 1,181 sources, is 13.7 MB. One user has a handful of devices.

## Goals / Non-Goals

**Goals:**
- A relay a user can deploy on bree in a few lines of Nix and one `tailscale serve` command.
- No secret shared between relay and devices: admission rests on the owner key the user already has.
- An answered write survives a crash or a power cut, and no crash leaves a partial object.
- The relay never sees plaintext and never logs content or addresses.
- The relay's data folder is a valid `file://` transport, so it can be backed up, copied, or read without the relay.

**Non-Goals:**
- Throughput beyond one person's devices. The relay serves one request per connection, one thread per connection.
- Hiding metadata from the operator, who is the user: scope ids, device ids, object sizes and timing are visible to the relay.

## Decisions

### A hand-written HTTP/1.1 server on std, with `httparse`

`src/http.rs` is a new library module: it accepts on a std `TcpListener`, parses the request line and headers with `httparse`, reads a `Content-Length` body, and writes a response with `Content-Length` and `Connection: close`. It handles one request per connection. The surface the relay needs is small: `GET` and `PUT`, `Content-Length` bodies only, no keep-alive, no TLS, no compression. That is about 300 lines.

Framing is strict, because smuggling and desync bugs live there (`relay-api` spec, Request framing and Request bodies):

- Any `Transfer-Encoding`, a repeated or non-decimal `Content-Length`, a missing `Host`, or an `Expect` other than `100-continue` is refused with 400. The relay reads exactly `Content-Length` bytes and closes, so it cannot be desynced itself, but it does not accept the smuggling shape as valid either.
- The target must be origin-form, start with `/v1/` and match the tree's grammar byte for byte. That means no percent-escapes, no `.` or `..` segments, and ids, seqs and names in their exact forms. The relay routes and verifies the signature on that same unmodified string, so nothing is normalized between the check and the use.
- `Expect: 100-continue` is answered with `100 Continue` once the checks that need no body pass. ureq never sends it, but Go's reverse proxy forwards it.
- A refusal before the body is read (411, 413, 403 and the like) writes the response, shuts down the write side, and drains the socket for up to 2 seconds before closing. Otherwise a client still sending would often see a reset instead of the answer, and the transport would misreport it as unreachable.

- **Why `httparse`:** header parsing is where hand-written servers go wrong (folding, case, limits, smuggling). `httparse` is fuzzed, has no dependencies and is already compiled into bilbo through `ureq-proto`. Measured on 2026-10-03 in a scratch crate: adding `httparse = "1.10.1"` to `Cargo.toml` adds one dependency edge to `Cargo.lock` and no new package. It lives in `src/http.rs` only.
- **Why not `tiny_http`:** 0.12.0, its latest release, dates from 2022-10-06. It adds `ascii`, `chunked_transfer` and `httpdate`, and its TLS feature pins `rustls` 0.20, a second `rustls` beside `ureq`'s 0.23. It does more than the relay needs (chunked bodies, keep-alive, pipelining) and gives no control over the limits below.
- **Why not `rouille`:** 3.6.2, last released 2023-04-24, is a framework on top of `tiny_http`.
- **Why not `hyper`, `axum` or `astra`:** they bring `tokio` or `hyper`'s stack, an async runtime into a synchronous binary, for a server that answers a few devices.
- **Why not parse by hand too:** the parser is the part worth not writing.

Concurrency: one thread per connection, at most 256 at once. A counter in the accept loop enforces the cap, and at the cap the accept loop itself writes 503 `busy` with `Retry-After: 5` and closes, without waiting. Threads are cheap at this load, and a fixed pool of 16 with a queue was easy to exhaust: 16 clients each sending half a request line would hold every worker for the whole idle timeout.

- **Deadlines:** the request line and headers must arrive within 10 seconds of the accept. After that, the body may idle for at most 30 seconds between reads.
- **Size caps:** headers are capped at 16 KiB (431). The body is refused before it is read when its `Content-Length` passes the limit for its path (413).
- **Cheap checks first:** checks that cost nothing run before the Ed25519 verify (`relay-api` spec, Order of checks): framing, grammar, method and the time window. A stranger's junk therefore costs the relay a parse, not a signature check.
- **What is left:** on a public relay, 256 slow clients can still hold every connection for 10 seconds at a time. A one-person relay accepts that cost. The README tells a public deployment to set Caddy's `timeouts` and `max_header_size` too, so the proxy sheds such clients before they reach the relay.

`src/main.rs` stays the only writer of stderr. `relay::run` takes a log sink from `main`, a `&(dyn Fn(&str) + Sync)` that calls `print_stderr`, the way `setup::run` takes a line callback.

### TLS: a proxy in front, not inside the relay

The relay speaks plain HTTP and listens on `127.0.0.1:8738` (next to the local embedder's 8737). TLS comes from a reverse proxy:

- **On bree, `tailscale serve`:** `tailscale serve --bg 8738` publishes `https://<host>.<tailnet>.ts.net` with a Let's Encrypt certificate that Tailscale renews, reachable only from the tailnet. The guests already run `services.tailscale`. The tailnet needs HTTPS certificates turned on once in the admin console.
- **On a public host, Caddy:** `relay.example.org { reverse_proxy 127.0.0.1:8738 }` gets and renews its certificate by ACME. Its `timeouts` and `max_header_size` should be set as well (README).
- **The first request through `tailscale serve`:** the certificate is issued on that request, which can take longer than the client's 10-second connect timeout. The README says to run `tailscale cert <host>` once, or to expect the first sync to report the relay unreachable and the next one to succeed.

Alternatives considered:
- **Terminate TLS in the relay with `rustls`.** `rustls` is already compiled in. But the relay would need certificate and key flags, a reload when the certificate renews, and still an ACME client, because the client trusts only public roots (below). A proxy already does all three.
- **No TLS at all, relying on end-to-end encryption.** Content would stay sealed and requests signed, but every scope id, device id, size and timing would cross the network in the clear, and the contract allows plain `http://` only to loopback.

A relay told to listen on a non-loopback address still serves, with a warning, because a proxy may sit on another host. Clients will not reach it without TLS anyway.

### The client trusts the bundled web PKI roots only

`src/remote.rs` builds its `ureq` agent with the default `rustls` verifier and `webpki-roots`, `max_redirects(0)`, `http_status_as_error(false)`, a 10-second connect timeout and a 300-second global timeout. Tailscale's and Caddy's certificates both chain to those roots.

- **Why no redirects:** a signature covers the path, so a redirect would fail anyway, and following one would send the signed request somewhere the user did not configure.
- **Alternatives:** `rustls-platform-verifier`, to trust the system store, adds a crate and its platform dependencies. A `relay.ca_file` key for a private CA can come later without a protocol change. A self-signed certificate is refused, with no fallback.
- **`http://` to loopback** reuses `config::is_local`, as the contract says. It serves the tests and a relay reached through an SSH tunnel.

### Requests are signed by the device, over a fixed text

Every request under `/v1/scopes/` carries four headers, and so does a signed mailbox message. All four are hex, so no base64 dependency is needed:

```
Bilbo-Key:       <64 hex: Ed25519 public key>
Bilbo-Time:      <Unix seconds>
Bilbo-Nonce:     <32 hex: 16 random bytes>
Bilbo-Signature: <128 hex: Ed25519 signature>
```

The signature covers these lines joined by `\n`:

```
bilbo-relay-1
<METHOD>
<the request target as received, which begins with /v1/>
<Bilbo-Time>
<Bilbo-Nonce>
<hex SHA-256 of the body>
```

- **Domain separation:** the first line keeps a request signature from ever verifying as a manifest or segment signature made with the same key.
- **The target as received, no host:** the client signs the target it expects the relay to receive, which begins with `/v1/`. A proxy may strip a prefix (`https://example.org/bilbo` reaches the relay as `/v1/...`) or rewrite the host. The relay never searches inside a target for `/v1/`: a target that does not begin with it is refused (400). Replaying a request to another relay of the same owner gains nothing: creates are idempotent and reads return ciphertext that the signer could read anyway.
- **Time and nonce:** a 300-second window, and a per-key nonce cache for 600 seconds, held in memory and bounded at 100,000 entries (503 `busy` beyond it, which signed traffic from one person never reaches). A nonce is recorded only after its signature verifies, so a stranger cannot poison the cache. The same rules cover a signed mailbox message, so a captured nameplate opener cannot reopen its nameplate later. A restart forgets the cache, so a request captured in the 5 minutes before a restart could be replayed once. Its effect would be a read of ciphertext or an idempotent create.
- **Clock skew:** the relay puts its time in `Bilbo-Time` on every answer. On 401 `clock`, the client retries once with that time and a fresh nonce, and keeps the offset for the rest of the process, so a laptop with a wrong clock still syncs and pays the retry once. A man in the middle cannot abuse it, since TLS protects the answer.
- **Which key:** the relay derives the device id from `Bilbo-Key` (`add-device-keys`' rule, SHA-256 then base32) and looks it up in the scope's latest manifest. When the key instead matches the scope's `owner`, the request acts as the owner.

Alternatives considered:
- **Bearer tokens issued by the relay.** That is a shared secret, which the user ruled out, and one more thing to store and leak.
- **TLS client certificates.** The proxy terminates TLS, so the relay would never see them.
- **HTTP Message Signatures (RFC 9421).** A general format with component selection and structured fields, where bilbo needs one fixed form that both ends hold in a single function.

### Owner-signed requests exist for recovery

A device restored from the phrase is in no manifest yet. To write manifest n+1 it needs n's bytes for `prev`, and to find its scopes it needs a list. So the owner key may do three things only: read a scope's manifests, list its owner's scopes (`GET /v1/scopes/`), and create manifests. It can never list device folders, read segments or create them. Once the recovered device is listed, it reads segments with its own device key. This gives no new power: under the contract's v1 trade-off every enrolled device holds the owner key, so any device could already sign a manifest that lists itself. The design says so. A later version that keeps the owner key off ordinary devices would also narrow what an owner-signed request means here.

The client is `bilbo device recover` and the setup wizard (`device-identity` "Recover fetches scopes from the transport", and `setup` "Applying sync from the wizard"). They hold the phrase-derived keys for the length of the command, which the watcher never does. For each syncing scope with no local manifest, the client lists the owner's scopes with `GET /v1/scopes/`. It reads each chain from `manifest/latest` down to 1, verifies it with `manifest::verify_scope`, opens the `owner` entry and the sealed `name`, and copies in the chain named like the config's scope. Only then does it add the device as manifest n+1, which the watcher publishes. `add-sync` already does the same for a `file://` folder through `read_dir` of `scopes/`; this change adds the relay listing behind the same fetch.

- **Two scopes with one name:** the client takes the one listing more devices, on a tie the lower id, the rule `add-sync` uses when a device meets two scopes with one name.
- **Only the picked scope:** the wizard copies in only the scope the user picked, so a second device joins that scope alone. `recover` by hand joins every scope the config gives a sync URL.
- **A scope the transport does not hold:** this is the one case `unsealed` remains, and there `bilbo device init` is the right advice. The user's decision that setup takes the relay endpoint therefore holds, and so does the contract's promise that the phrase alone recovers every scope, every device lost included.

### Manifest admission reuses the keys change's verification

The relay reads only a manifest's plaintext fields: `scope`, `n`, `prev`, `owner`, `devices[].id` and `devices[].sign`, and `sig`. It verifies with `add-device-keys`' `src/manifest.rs`, which checks plaintext fields only and never decrypts (`manifest::verify_scope` over a scope's versions, or the same check on one version against its predecessor; `manifest::latest` at start-up). That check covers `sig` over `bilbo-manifest-1` and the compact JSON without `sig`, `prev` as the hex SHA-256 of version n-1's file, and the same `owner` in every version. It compares `--owner` with the fingerprint that change defines (24 base32 characters of the SHA-256 of the owner's Ed25519 key, as six groups of four), computed from `owner` by `src/keys.rs`, which also holds `keys::verify`, SHA-256, hex and base32. Manifest files are canonical, and verification re-serializes them, so a non-canonical upload is refused with 422 `manifest`, and `prev` over the stored bytes is the same as over the canonical form. `--owner` is accepted in any case, with or without hyphens. On top of that it checks:

1. `scope` and `n` match the path.
2. n is 1 with `prev` null and an admitted owner, or n-1 is the latest stored, `prev` is the SHA-256 of n-1's stored bytes, and `owner` is unchanged.
3. Every device's `id` is derived from its `sign` key.
4. `chain` holds one entry per epoch from 1 to `epoch` minus 1, in order, with every entry of version n-1 unchanged. That is change 3's Manifest validity rule. Epoch numbers and entries are plaintext, so the relay can enforce it without a key, and a version that rotates the epoch while skipping or rewriting an entry never reaches other devices through the relay.
5. The request's key is listed in the new manifest, or is the owner.

A per-scope mutex serializes manifest and segment creates within a scope, and the create-only link below settles any race that slips past it. An owner change is always refused: the phrase is the identity, and a new phrase means a new scope.

Existence is checked first, for manifests and segments alike. With manifest 4 stored, a manifest 4 or 3 is answered 200 when its bytes match and 409 `exists` when they do not. Only a create above the latest goes on to the chain checks, where a gap is 409 `not-next`.

A stored segment is never created again. `add-sync`'s Own segments survive and Damaged own segments apply to `file://` only, because a cloud tool can lose or truncate a folder's files and the relay cannot. A segment an operator removes by hand from the data folder stays missing, and the not-next rule stays: a device's create below its highest seq answers 409 `not-next`.

**Forks at one n (context; the rule belongs to the client changes).** When two devices write manifest n at once, the first create on the transport wins, and the relay answers the other 409 `exists`. The losing device moves its own local copy of n aside, never deleting it. It then takes the relay's n and re-applies its change as n+1. The relay needs nothing more than strict create-only manifests, which it has. The team lead set this rule for the cross-change pass. The client side lives in `add-device-keys` and `add-sync`.

### Storage: the `file://` tree, created by link

```
<data>/
  .relay.lock                              held by the running relay
  .tmp/                                    bodies being received
  scopes/<scope_id>/manifest/<n>.json
  scopes/<scope_id>/devices/<device_id>/<seq>.seg
  pair/<nameplate>/<name>.msg
```

A create:

1. Stream the body to `.tmp/<random>` and check its length.
2. `sync_all` the file.
3. Create the parent folders, and `sync_all` each one that was new, along with its parent.
4. `std::fs::hard_link` the temporary file to the final name. On `EEXIST`, compare the bytes and answer 200 or 409 `exists`.
5. `sync_all` the parent folder, remove the temporary file, and answer.

Why `hard_link`: it is atomic and refuses an existing name on every POSIX filesystem, ZFS and NFS included, with no `renameat2` flag. `swap::rename_new` needs `RENAME_NOREPLACE`, which some filesystems a Proxmox guest might use refuse. A crash between steps leaves only a file under `.tmp/`, which start-up deletes.

- **Folders are flushed with std only:** `File::open(dir)?.sync_all()`. On macOS std's `sync_all` is `fcntl(F_FULLFSYNC)`; a filesystem that refuses it on a folder fails the write like any other step below. The durability tests also run on macOS.
- **Errors at any step:** ENOSPC often surfaces at `sync_all`, not at `write`. An error at any step therefore removes the temporary file. ENOSPC or EDQUOT answers 507 `quota`; anything else answers 500 `internal`.
- **Quota reservations:** before streaming a body, the relay reserves its `Content-Length` against `--max-scope-mb` under the per-scope mutex, and releases the reservation if the create fails. Two concurrent 16 MiB uploads cannot overshoot the cap.

`add-sync`'s `file://` writer in `src/transport.rs` stays as it is, on `swap::rename_new` with its own temporaries, `.<device id>-<16 lowercase hexadecimal characters>.tmp`, in the target folder (its design, "The `file://` transport"). A folder users sync from may sit on exFAT, FAT or some SMB shares, where hard links fail. The relay does not call that writer. Its data folder is a POSIX filesystem it controls (`/var/lib/bilbo-relay` under the NixOS module), so it uses the `hard_link` and flush sequence above in its own code path in `src/relay.rs`. Each writer has one temporary location: the target folder for `file://`, and `<data>/.tmp/` for the relay. Both produce the same tree.

Start-up walk: the relay walks `scopes/` once and builds, per scope, the latest manifest, the byte total and each device's highest seq, and keeps them in memory. A scope of 1 GiB holds tens of thousands of files, which takes well under a second to walk.

The walk trusts nothing on disk, because a copied `file://` folder is a documented way in. It verifies every chain with `manifest::verify_scope`: signatures, `n` against the file name, `prev`, an unchanged owner, and device ids derived from keys. It also checks that each device's seqs run from 1 with no gap, and that the owner was passed with `--owner`. A scope that fails is logged once with its id and reason. It answers 403 `invalid` (or `not-admitted` when only its owner is missing), does not count toward `--max-scopes`, and its devices cannot open nameplates. Its folder is left alone for the operator to repair. Files outside the tree's grammar are ignored. The walk also counts the open nameplates under `pair/` and deletes the expired ones, so the 32-nameplate limit holds across a restart.

### The mailbox: only an enrolled device opens a nameplate

The mailbox is the one place a device with no manifest entry can write. Pairing needs that, and nothing else does.

- **Who opens:** the first message of a nameplate must be signed by a device that the latest manifest of a valid, admitted scope lists. A stranger therefore cannot create nameplates and fill the disk.
- **Who writes next:** every later message must be signed by the opener's key, under the same time and replay rules as any signed request, except one unsigned message per nameplate, which is the new device's answer. A stranger gets at most one write into an open nameplate. They can neither fill its slots nor pre-empt the opener's reply (`c.msg`), and another enrolled device cannot write there either. The rule names no slot, so the relay does not depend on the pairing message names. `add-device-pairing` asked for it.
- **Finding a nameplate:** reads are unsigned, because B has no listed key, and nameplates run only from 1 to 999. Without a limit, a stranger could poll every nameplate in minutes and find the open one. Unsigned mailbox requests are therefore limited per peer address to 60 a minute and 4 distinct nameplates in 10 minutes. B, which polls one nameplate every 2 seconds, uses half of the first limit and one of the four. Probing all 999 nameplates within one code's 10 minutes then takes about 250 addresses. The opener signs its polls, so they never count.
- **The hole that remains:** a stranger who does find an open nameplate can create one junk `b.msg`. That uses up the code, under `add-device-pairing`'s one-attempt rule, and the pairing must be restarted. This is the cost of a new device that cannot authenticate. It leaks no key, and the one-attempt rule bounds it. magic-wormhole's mailbox server has the same exposure, with the difference that it allocates nameplates itself.
- **What `add-device-pairing` must follow:** the enrolled device, the one that shows the code, writes first. Its draft does: A writes `a.msg` and `c.msg`, B writes `b.msg`, each at most 4 KiB, under a nameplate from 1 to 999. The relay accepts message names of 1 to 16 characters of `[a-z0-9-]` and up to 8 per nameplate, a superset of the contract's `a|b`, so a later message format needs no relay change.
- **Limits:** 4 KiB per message, 32 open nameplates, and 30 minutes from the first message. Thirty minutes matches the age at which `bilbo pair` sweeps a `file://` mailbox, and leaves B time to read `c.msg` after a pairing that used its full 10 minutes. A sweeper thread checks every 30 seconds, and start-up deletes nameplates older than 30 minutes by modification time.
- **Rate, behind a proxy:** peer addresses are counted in memory and forgotten after 10 minutes idle. Behind a proxy every request has the proxy's address, so the unsigned limits become global. An abuser on the tailnet, or anyone in front of a public Caddy, can then stall pairing for up to 10 minutes. They cannot store anything or reach a scope, and the user retries. The per-address scenarios are tested with injected addresses, and cannot be observed behind a proxy.
- **Why reads stay unauthenticated:** reading a mailbox message reveals one SPAKE2 message, which is public by design. Reading `c.msg` reveals only ciphertext under the session key.

### Limits per scope

| Limit | Default | Why |
|---|---|---|
| `--max-scopes` | 16 per owner | A person's scopes number a few. Per owner, so one of two owners cannot take every slot. |
| `--max-scope-mb` | 1024 | About 70 times today's whole corpus. |
| `--max-object-mb` | 16 | A bootstrap segment of a large store must fit. `add-sync` sizes its segments under it. |
| manifest size | 1 MiB | Fixed: a manifest lists a few devices. |

Signed requests are not rate-limited. Only the owner's devices can get past the signature check, and abuse would need a stolen key, which the quotas still bound. Unsigned junk is cheap to refuse, by the check order above.

### What the relay logs

The relay logs one line per create: kind, scope id, device id, seq or n, and bytes, or only `mailbox <bytes>` for a mailbox message, which has no scope or device to name. It logs one line per 4xx refusal (other than 404) of a request whose signature verified, with its reason. Every other 4xx refusal, unsigned or forged, is folded into one line a minute with counts by reason, so a flood cannot fill the journal. It also logs its start-up lines and each invalid scope found at start. It logs nothing for reads: a new device's bootstrap reads thousands of objects.

- **Never logged:** bodies, headers and signatures; peer addresses, which are personal data and are kept only in the rate limiter's memory (the startup line names only the listen address); nameplates, which are half of a pairing code.
- **Ids are logged:** scope and device ids are random, and the operator is their owner.
- **Proxy logs:** `tailscale serve` keeps no access log. Caddy logs requests only with a `log` directive, and the README says so.

### Running it: a NixOS module

`flake.nix` gains `nixosModules.relay`. It defines `systemd.services.bilbo-relay` with `ExecStart = bilbo relay --data /var/lib/bilbo-relay --owner … --listen …`, plus `DynamicUser`, `StateDirectory = bilbo-relay`, `StateDirectoryMode = "0700"`, `UMask = "0077"`, `ProtectSystem = strict`, `ProtectHome`, `PrivateTmp`, `NoNewPrivileges`, `RestrictAddressFamilies = AF_INET AF_INET6`, `CapabilityBoundingSet = ""`, `SystemCallFilter = "@system-service"`, `SystemCallArchitectures = "native"`, `ProtectKernelTunables`, `ProtectKernelModules`, `ProtectControlGroups`, `RestrictNamespaces`, `RestrictRealtime`, `LockPersonality`, `MemoryDenyWriteExecute`, `ProtectProc = "invisible"`, `Restart = on-failure` and `RestartSec = 10`. On bree, a guest file then holds:

```nix
services.bilbo-relay = { enable = true; owners = [ "<fingerprint>" ]; };
```

plus a one-time `tailscale serve --bg 8738`, which tailscaled persists.

Alternatives considered:
- **A setup step or `bilbo relay --install`.** Setup configures a user's own login session. A relay is a system service, and a systemd user unit stops at logout unless lingering is on.
- **A launchd agent.** A laptop that sleeps makes a poor relay.
- **A container image.** One more artifact to build, pin and publish, for no user who asked.
- **Flags only.** They remain the base. The README gives the equivalent plain systemd unit for a non-Nix Linux host.

The flake's check evaluates a `nixosSystem` that enables the module on `x86_64-linux` and asserts the unit's `ExecStart` and `StateDirectoryMode`, the same way the home-manager check does without building a VM. The evaluated system carries the minimum NixOS asserts on (`fileSystems."/"`, `boot.loader.grub.enable = false`, `system.stateVersion`), so the module's own assertion is what fails when `owners` is empty.

### Modules

- `src/http.rs` (library): framing, limits, deadlines and the connection cap. Its handler gets the parsed request and a reader capped at `Content-Length`, so a 16 MiB body streams to disk instead of sitting in memory, and returns a response. It never prints.
- `src/remote.rs` (library): the `https://` transport behind the transport interface in `add-sync`'s `src/transport.rs`, selected by URL scheme where that module selects `file://`, plus the request-signature text and its verification, which both sides call. `ureq` gains this third user. Mapping onto that interface (`list_devices`, `list_after`, `get`, `create`, `highest_manifest`, `remove_mailbox`):
  - `list_after` returns everything after the seq, so `remote.rs` follows `more` through the 1,000-entry pages itself;
  - `create` maps 201 and 200 (identical bytes already there) to created, and 409 `exists` to the interface's already-exists outcome, never to a failure;
  - `highest_manifest` reads `manifest/latest`;
  - `remove_mailbox` does nothing, because the relay expires nameplates itself;
  - 507 `quota` is a failure of its own kind, so `add-sync` can report a full scope apart from an unreachable relay. `add-sync` keeps segments to 8 MiB, under the 16 MiB object cap.
- `src/relay.rs` (verb): flags, the data folder and lock, routing, admission, quotas, the mailbox and its sweeper, and log lines handed to `main`'s sink. It builds on `http`, `remote`, and `add-device-keys`' `keys` and `manifest` modules. Its create path is its own, apart from the `file://` writer.

### Tests run a relay on loopback

- **Protocol tests:** unit tests in `src/relay.rs` start the server in-process on `127.0.0.1:0` with an injected clock and an injected peer address, and sign requests with keys from `add-device-keys`' test helpers. That covers windows, replays, check order, admission, the chain, create-only, listings, limits, mailbox rules, per-address limits and expiry without sleeping. Malformed and smuggling-shaped requests go over a raw `TcpStream`.
- **Client tests:** `src/remote.rs` unit tests run the client against the same in-process relay, and against a small fake on loopback that answers redirects, missing `Bilbo-Time` and repeated 401 `clock`.
- **CLI tests:** `tests/relay.rs` runs the built `bilbo relay --listen 127.0.0.1:0` as a child, reads the port from its first stderr line, and covers flags, the lock, the log's content, and kill-and-restart durability (SIGKILL mid-body, then a retry). It then runs `add-sync`'s two-store sync scenarios with `scope.<name>.sync = http://127.0.0.1:<port>` in place of `file://`.
- **Disk-full tests:** on Linux, a size-limited tmpfs as `--data` produces a real ENOSPC. Elsewhere, an injected write failure stands in.
- Every test stays on loopback, so the suite stays offline. TLS is exercised in the bree smoke run, not in CI.
- **Segments are opaque on purpose:** the relay does not check a segment envelope's plaintext `scope`, `device` and `seq` against the path. The request signature already binds the device to its folder, readers verify every segment's own signature, and the envelope format belongs to `add-sync`.

## Risks / Trade-offs

- [A replay within 5 minutes of a restart] → It can only read ciphertext or repeat an idempotent create, and TLS keeps requests from being captured in the first place.
- [A stolen device signs as the owner] → The relay cannot tell it from the user; this is the contract's v1 trade-off. In change 3's words: "After a confirmed revocation a revoked device, even one using the owner signing seed, cannot read anything written under later epochs; it can still disrupt by signing versions that members reject or that change the device list, which watch announces." A valid manifest's chain must hold every earlier epoch, and every member checks the entries for the epochs it holds, so no one can mint an epoch without the current key. The relay applies the part of that check that needs no secret, so a version with a missing or altered chain entry never reaches other devices through it. The remedy for disruption is change 3's new-owner procedure, which ends with restarting the relay with the new fingerprint.
- [A segment damaged on the relay's disk] → Readers stop at that seq and retry, and the writer never learns. The README gives the repair: stop the relay, copy `<root>/.bilbo/scopes/<scope id>/out/<seq>.seg` from the writing device over the damaged file, and start it again. Readers pick it up at their next poll.
- [The relay keeps everything forever] → Bounded by `--max-scope-mb`. Pruning on the transport, if `add-sync` needs it, is an additive `DELETE` in API 1 or a new `/v2/`. Until then, the README says how to drop a scope by hand: stop the relay, remove `scopes/<scope_id>`, start it.
- [A proxy that strips or adds a prefix] → The client signs the target the relay receives, which begins with `/v1/`, so both work.
- [Connection exhaustion on a public relay] → Bounded by the 256-connection cap and the 10-second header deadline, and by the proxy's own timeouts when set. A one-person relay accepts the rest.
- [One request per connection] → A bootstrap opens a fresh loopback connection per object behind the proxy, which keeps its own client connections alive. At a few thousand objects that costs well under a second.
- [`tailscale serve` reachable only inside the tailnet] → That is the intended reach for one person's devices. Tailscale Funnel or Caddy serves a relay that devices outside the tailnet must reach.
- [Behind a proxy, the mailbox limits are global] → An abuser inside the tailnet could stall pairing for up to 10 minutes. Nothing is stored, and the nameplate caps still bound the disk.
- [Private CAs are refused] → Out of scope for v1. A `relay.ca_file` key can come later.

## Migration Plan

Nothing to migrate: a user who runs no relay sees no change. To deploy, add the module to a NixOS host (or run the plain unit from the README), run `tailscale serve --bg 8738` once, then set `scope.<name>.sync = https://<host>.<tailnet>.ts.net` on each device. To roll back, stop the service. Devices report the relay unreachable and keep working locally, and the data folder stays a `file://` tree that can be served again or copied.
