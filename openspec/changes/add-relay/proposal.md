# Proposal

## Why

After `add-sync`, two devices sync a scope only through a folder they both see (`file://`), which means a cloud-synced folder or a shared disk. The user decided on 2026-10-03 (planning notebook, `design-bilbo-remote-sync.md`, "Decisions taken") that sync also runs through a relay the user hosts: bilbo's own binary, run on a machine the user controls, keeping data on its disk, admitting only the user's own owner key. This change adds that relay and the `https://` transport that talks to it, so a laptop and a home server sync with no third-party folder in between.

## What Changes

- `bilbo relay`: a long-running verb that serves the transport tree over HTTP/1.1 under `/v1/`. It takes flags only: `--data <dir>` (required), `--owner <fingerprint>` (required, repeatable), `--listen <address:port>` (default `127.0.0.1:8738`), and three limits: `--max-scopes`, `--max-scope-mb` and `--max-object-mb`. It prints the address it listens on, then one line per write or refusal. It never logs content, nameplates or IP addresses.
- Admission with no shared secret. Every request on a scope carries an Ed25519 signature over its method, path, time, nonce and body hash. The relay accepts it only from a device that the scope's latest manifest lists, or from the scope's owner key. A scope enters the relay only through a manifest 1 that an `--owner` key signed. Manifest n+1 is accepted only when it follows n: same owner, `prev` matching, n contiguous, a chain entry for every earlier epoch, valid signature. At start the relay re-verifies every stored chain, so a copied or hand-edited folder is never served unchecked.
- Create-only storage, durable and crash safe. An object is written to a temporary file, flushed to disk, linked into place only if the name is free, and the folder is flushed before the relay answers. A re-sent identical object is accepted, a different one is refused. The data folder holds the same tree as a `file://` transport, so it can be copied or read as one.
- The pairing mailbox, the one unauthenticated area. Only an enrolled device can open a nameplate, and only it can write there after the new device's one unsigned answer. Messages are small, expire 30 minutes after the nameplate opens, and are rate-limited per peer address in memory, which also slows a stranger probing for open nameplates.
- Recovery through a relay: `bilbo device recover` and the setup wizard fetch the owner's manifests with owner-signed reads before enrolling, and the wizard takes a relay URL.
- The `https://` transport client used by `bilbo watch` and `bilbo pair`: it verifies certificates against the web PKI roots `ureq` already bundles, accepts `http://` only to a loopback host, never follows a redirect, signs every request with the device key, and turns each refusal into a message that names the relay and the reason.
- TLS is the job of a reverse proxy in front of the relay (`tailscale serve` or Caddy). The relay itself speaks plain HTTP and listens on loopback by default.
- A NixOS module, `nixosModules.relay`, with `services.bilbo-relay.*` options, which runs the relay as a hardened system service with its data under `/var/lib/bilbo-relay`. The README gives the same service as a plain systemd unit for other Linux hosts.

## Capabilities

### New Capabilities

- `relay-server`: the `bilbo relay` verb: flags, the data folder and its lock, durability, limits, what it logs, and the NixOS module.
- `relay-api`: the HTTP API under `/v1/`: paths, status codes, request signatures and replay protection, scope admission and the manifest chain, create-only objects, listings, and the pairing mailbox.
- `relay-transport`: the `https://` transport client: TLS and loopback rules, redirects, request signing, clock skew, and how relay refusals reach the user.

### Modified Capabilities

- `cli`: Verb dispatch adds `relay`.
- `config`: Config location adds `relay` to the verbs that run whatever the config holds.
- `sync-transport`: Transport URLs names a relay for `https://` and loopback `http://` URLs, so `add-sync`'s not-supported line no longer applies to them. Create-only writes says a relay takes a repeated create with the same bytes as done.
- `setup`: the Sync step checks a relay scope with an unsigned `GET <url>/v1/` and reports `failed: <url> is not a bilbo relay` when the identification is missing.
- `device-pairing`: The transport URL on the new device lets `--via` name a relay, so `add-device-pairing`'s `cannot reach https:// transports yet` refusal goes.
- `device-identity`: Recover fetches scopes from the transport, which `add-sync` added for `file://` folders, extends to relays (owner-signed listing and reads). What recover writes then reports `unsealed` only when the transport holds no such scope. So the phrase alone recovers every scope, on a folder or a relay, even when every device is lost.
- `setup`: Turning sync on in the wizard and Applying sync from the wizard accept a relay URL beside a folder, and copy in only the picked scope's chain.

## Non-goals

- A public relay run by the project. A default URL can come later with no protocol change.
- TLS inside the relay, and ACME. A proxy that already does both is one command away.
- Deleting data through the API: no `DELETE`, no remote purge, no expiry of scope data. The operator removes a scope by hand, as the README says.
- Accounts, quotas per person, billing, or a shared secret.
- Federation between relays, or one relay forwarding to another.
- A `bilbo setup` step for the relay. Setup configures a user's own machine; the relay runs on a server.
- A launchd agent for the relay. A relay on a laptop that sleeps is not a relay.
- Hiding object sizes or timing from the relay.
- Long polling or push. Clients poll, as with `file://`.
- Private certificate authorities. The client trusts only the bundled web PKI roots.

## Impact

- New verb `src/relay.rs`, with its `mod` line, dispatch arm and USAGE line in `src/main.rs`.
- New library modules: `src/http.rs` (HTTP/1.1 framing on std `TcpListener`) and `src/remote.rs` (the `https://` transport client and the request-signature rule both sides share). `remote.rs` implements the transport interface of `add-sync`'s `src/transport.rs` and is selected by URL scheme where `add-sync` selects `file://`.
- The relay stores objects through its own create path, `hard_link` with flushes in `<data>/.tmp/`. The `file://` writer in `src/transport.rs` (`add-sync`) keeps `swap::rename_new` and its `.<device id>-<16 lowercase hexadecimal characters>.tmp` temporaries, because hard links fail on exFAT, FAT and some SMB shares that users sync from. The owner-scope fetch that `add-sync` adds to `src/transport.rs`, called by `src/device.rs` (recover) and `src/wizard.rs` with `src/setup.rs`, gains the relay listing.
- Manifest and signature checks reuse `add-device-keys`' `src/manifest.rs` (verification, the chain) and `src/keys.rs` (Ed25519, the owner fingerprint, random bytes).
- New direct dependency: `httparse` 1.10.1, already in `Cargo.lock` through `ureq-proto`, so no new crate is built.
- `flake.nix`: `nixosModules.relay` and an evaluation check of it on the Linux systems.
- Tests: protocol tests as unit tests in `src/relay.rs` and `src/remote.rs` against a relay on `127.0.0.1:0`; `tests/relay.rs` runs the built `bilbo relay` as a child and syncs two stores through it.
- `AGENTS.md` and `README.md`: the relay, its deployment behind `tailscale serve` or Caddy, and the `httparse` rule.
- Migration: none. Nothing changes for a user who does not run a relay.
