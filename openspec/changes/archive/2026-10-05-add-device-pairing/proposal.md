# Proposal

## Why

After `add-device-keys`, the only way to bring a second device into the user's syncing scopes is to type the 12-word recovery phrase on it. That phrase is the master secret: it should come out of its hiding place only when every device is lost, not each time the user sets up a laptop. The sync design (the planning notebook's `design-bilbo-remote-sync.md`, decisions of 2026-10-03) keeps short-code pairing, `bilbo pair`, for everyday enrollment. This change adds it, over the transport the scopes already use.

## What Changes

- `bilbo pair` on an enrolled device (A) shows, on stderr, a one-time code such as `42-orbit-tunnel-velvet`: a nameplate number and three words from the BIP39 English list. The code works once and for 10 minutes.
- `bilbo pair <code> --via <url>` on the new device (B) joins. The URL is typed, because the new device has no identity and no config yet, and a `file://` folder has a different path on each machine.
- An enrolled device of the same owner can answer a code too, to join scopes it is not in. Devices hold no owner box secret, and a version under a new epoch needs a correct chain entry for the current one, so a device cannot add itself to a scope. Pairing (a member seals to it) and `bilbo device recover` (the phrase) are the two ways in, and `add-sync`'s advice to a device outside a scope names both.
- The two devices run SPAKE2 through the transport's mailbox, `pair/<nameplate>/`, as three create-only messages. A wrong code uses the code up, and both devices say so.
- Both devices show the same confirmation fingerprint, derived from the session key, and B shows its own name and id beside it. A enrolls B only after the user types `y` on A, having checked that B shows the same. A runs only in a terminal and not under an agent's markers, the rule `add-device-keys` applies to the phrase. The markers catch an agent running it by accident, since `env -u` removes them. The terminal is the boundary.
- A first writes, for each scope picked, a new manifest version that lists B with the epoch key sealed to B. Over the authenticated channel it then sends B the owner signing seed, the only owner secret a device holds, and each scope's name, id, embedder setting and the version and hash of that manifest. B gets each scope's epoch key only as A sealed it to B. It fetches every manifest version and verifies the chain from that hash before it writes anything. It then stores the manifests, sets `scope.<name>.sync` in its config, and writes its keys last. B keeps its device key aside until then, so an interrupted pairing resumes with the same device id instead of leaving a ghost device listed.
- Pairing refuses when A has no owner key (the phrase was never set), when no scope syncs, when B is enrolled with another owner, when B's name is taken among any of the owner's devices, when B's store holds another owner's scopes, and when B has no store. B's own checks run before it touches the mailbox where they can, so they do not use up the code.
- Pairing never reveals the recovery phrase. The phrase stays the fallback when no enrolled device is at hand.
- The mailbox layout and its removal on `file://` belong to add-sync's `sync-transport` spec. On `file://`, B removes the mailbox after reading the reply (except after a wrong code), A removes an unanswered one at expiry, and any `bilbo pair` sweeps mailboxes older than 30 minutes. The relay (`add-relay`) expires them after 30 minutes itself, and lets only A sign its reply.

## Capabilities

### New Capabilities

- `device-pairing`: the `bilbo pair` verb on both sides, the code, the fingerprint and confirmation, one attempt per code, expiry, what the new device receives and where it lands, the refusals, and the mailbox on the transport.

### Modified Capabilities

- `cli`: Verb dispatch adds `pair`.
- `config`: Config location adds `pair` to the verbs that read settings.
- `device-identity` (added by `add-device-keys`): What recover writes. The `unsealed` hint also offers `bilbo pair` with a device that syncs the scope.
- `note-sync` (added by `add-sync`): Joining and leaving a scope names `bilbo pair` beside `bilbo device recover`, in its body and in the advice it prints to a device outside a scope.

## Non-goals

- Pairing over a LAN, mDNS, Bluetooth or a QR code. The transport the scopes already use is the only rendezvous.
- Withholding the owner signing seed from a paired device. Every enrolled device holds it in v1, per `add-device-keys`. design.md says what that means for scope selection.
- Pairing two people's devices, or sharing one scope between owners. Members are one person's devices.
- A `--yes` flag that skips the confirmation on A.
- Merging the new device's existing notes into the scopes. That is `add-sync`'s first-sync rule, which runs when `bilbo watch` next syncs.
- A setup step for pairing. Setup's `sync` step (`add-sync`) stays as it is.
- The `https://` relay client. Until `add-relay`, `--via https://…` is refused.

## Impact

- A new verb, `bilbo pair`, in the `identity` domain beside `bilbo device`: the folder `src/identity/pair/`, with `mod.rs` (arguments, the checks before the mailbox, the time limits, polling), `show.rs` (the device that shows the code), `join.rs` (the device that joins) and the test-only `exchange.rs` (the whole exchange in one process). Its `pub mod` line, dispatch arm and USAGE lines in `src/main.rs`.
- A new library module, `src/identity/pake.rs`, holds the code format, the SPAKE2 exchange, the key schedule, the fingerprint and the three message formats, with no I/O. It uses only `identity::keys`, `identity::phrase`, `shared::hash` and `shared::store` (the scope-name rule).
- `pair` builds on `add-device-keys`' modules: `identity/keys.rs` (the owner and device key files, `write_identity` under `keys.lock`, sealing, signing, the AEAD), which gains an HKDF helper and the pending device key of a joining device; `identity/phrase.rs` (the BIP39 list), which gains the lookup of a word by four or more letters; and `identity/manifest.rs` (`add_device`, `adopt`, `verify_scope`, `open`, `survey`). It builds on `add-sync`'s `sync/transport.rs` (create, get and `remove_mailbox`), which gains the sweep of mailboxes older than 30 minutes, and `sync/scopes.rs` (`chain`, a scope's versions from the transport).
- `src/shared/config.rs` gains `set_keys`, which sets a few `scope.<name>.*` keys in the config file and keeps every other line as written, with `write_config` moved there unchanged from `src/setup/apply.rs`.
- The advice to a device outside a scope names `bilbo pair`: the not-in-the-scope line of `sync/manifests.rs`, and the `unsealed` hint of `bilbo device recover` in `identity/device.rs`.
- `main.rs` calls `identity::pair::run(args, env, terminal, answer, limits, out, err)`, the shape of `add-device-keys`' `device::run` with a line printer for each stream, as `setup` and `watch` take one. It passes the terminal test, the locked stdin and `Limits::default()`.
- `tests/layout.rs`: `identity/pair/` joins `VERBS`, and `PLACEMENT` gains `spake2` and `rand_core` in `identity/pake.rs`, and `base64` (already used by segments) in `identity/pake.rs` too, for the mailbox messages.
- New dependency: `spake2` 0.5.0-pre.0, without default features. It adds no crate that `add-device-keys` and `add-sync` do not already bring (measured with `cargo tree`). `rand_core` gets a direct user, `src/identity/pake.rs`, through the `spake2::rand_core` re-export.
- Tests: the whole exchange runs in one process in `src/identity/pair/exchange.rs`: two threads, two folders joined by a delayed copier, a scripted answer and millisecond limits. `tests/pair.rs` covers through the binary only the refusals and usage errors. The shipped binary has no test hook.
- Migration: none. Devices enrolled with the phrase keep working, and pairing writes the same files the phrase route writes.
