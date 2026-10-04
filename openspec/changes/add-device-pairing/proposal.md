# Proposal

## Why

After `add-device-keys`, the only way to bring a second device into the user's syncing scopes is to type the 12-word recovery phrase on it. That phrase is the master secret: it should come out of its hiding place only when every device is lost, not each time the user sets up a laptop. The sync design (the planning notebook's `design-bilbo-remote-sync.md`, decisions of 2026-10-03) keeps short-code pairing, `bilbo pair`, for everyday enrollment. This change adds it, over the transport the scopes already use.

## What Changes

- `bilbo pair` on an enrolled device (A) shows, on stderr, a one-time code such as `42-orbit-tunnel-velvet`: a nameplate number and three words from the BIP39 English list. The code works once and for 10 minutes.
- `bilbo pair <code> --via <url>` on the new device (B) joins. The URL is typed, because the new device has no identity and no config yet, and a `file://` folder has a different path on each machine.
- An enrolled device of the same owner can answer a code too, to join scopes it is not in. Devices hold no owner box secret, and a version under a new epoch needs a correct chain entry for the current one, so a device cannot add itself to a scope. Pairing (a member seals to it) and `bilbo device recover` (the phrase) are the two ways in, and `add-sync`'s advice to a device outside a scope names both.
- The two devices run SPAKE2 through the transport's mailbox, `pair/<nameplate>/`, as three create-only messages. A wrong code uses the code up, and both devices say so.
- Both devices show the same confirmation fingerprint, derived from the session key, and B shows its own name and id beside it. A enrolls B only after the user types `y` on A, having checked that B shows the same. A runs only in a terminal and not under an agent's markers, the rule change 3 applies to the phrase. The markers catch an agent running it by accident, since `env -u` removes them. The terminal is the boundary.
- A first writes, for each scope picked, a new manifest version that lists B with the epoch key sealed to B. Over the authenticated channel it then sends B the owner signing seed, the only owner secret a device holds, and each scope's name, id, embedder setting and the version and hash of that manifest. B gets each scope's epoch key only as A sealed it to B. It fetches every manifest version and verifies the chain from that hash before it writes anything. It then stores the manifests, sets `scope.<name>.sync` in its config, and writes its keys last. B keeps its device key aside until then, so an interrupted pairing resumes with the same device id instead of leaving a ghost device listed.
- Pairing refuses when A has no owner key (the phrase was never set), when no scope syncs, when B is enrolled with another owner, when B's name is taken among any of the owner's devices, when B's store holds another owner's scopes, and when B has no store. B's own checks run before it touches the mailbox where they can, so they do not use up the code.
- Pairing never reveals the recovery phrase. The phrase stays the fallback when no enrolled device is at hand.
- The mailbox layout and its removal on `file://` belong to add-sync's `sync-transport` spec. On `file://`, B removes the mailbox after reading the reply (except after a wrong code), A removes an unanswered one at expiry, and any `bilbo pair` sweeps mailboxes older than 30 minutes. The relay (change 6) expires them after 30 minutes itself, and lets only A sign its reply.

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
- Withholding the owner signing seed from a paired device. Every enrolled device holds it in v1, per change 3. design.md says what that means for scope selection.
- Pairing two people's devices, or sharing one scope between owners. Members are one person's devices.
- A `--yes` flag that skips the confirmation on A.
- Merging the new device's existing notes into the scopes. That is change 4's first-sync rule, which runs when `bilbo watch` next syncs.
- A setup step for pairing. Setup's `sync` step (change 4) stays as it is.
- The `https://` relay client. Until `add-relay`, `--via https://…` is refused.

## Impact

- New verb `src/pair.rs`, its `mod` line, dispatch arm and USAGE lines. A new library module, `src/pake.rs`, holds the code format, the SPAKE2 exchange, the key schedule, the fingerprint and the three message formats, with no I/O.
- `pair.rs` builds on change 3's key and manifest modules (owner key, device key, the keys-folder writer, manifest rewrite and chain verification, sealing, the BIP39 word list) and change 4's `src/transport.rs` (create, get, list, and `remove_mailbox`).
- `src/config.rs` gains a function that sets a few `scope.<name>.*` keys in the config file and keeps every other line as written, with the write-through-a-temporary-file helper moved from `src/setup.rs`.
- `main.rs` calls `pair::run(args, env, terminal, answer, limits, out, err)`, the shape of change 3's `device::run`. It passes the terminal test, stdin, the default time limits and a line printer for each stream.
- New dependency: `spake2` 0.5.0-pre.0, without default features. It adds no crate that changes 3 and 4 do not already bring (measured with `cargo tree`). `rand_core` gets a direct user, `src/pake.rs`, through its re-export.
- Tests: the whole exchange runs in one process in `src/pair.rs`'s unit tests: two threads, two folders joined by a delayed copier, a scripted answer and millisecond limits. `tests/pair.rs` covers through the binary only the refusals and usage errors. The shipped binary has no test hook.
- Migration: none. Devices enrolled with the phrase keep working, and pairing writes the same files the phrase route writes.
