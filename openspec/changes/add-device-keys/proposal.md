# Proposal

## Why

Remote sync (the planning notebook's `design-bilbo-remote-sync.md`, "Decisions taken (2026-10-03)") encrypts each scope end to end, admits only what the owner key signed, and turns sync on only after the user holds a recovery phrase. Sync, pairing and the relay (changes 4 to 6) all sign, seal and verify with the same identity, so it comes first and alone: the user can create it, back it up and rehearse a recovery before any note leaves the device.

## What Changes

- **Recovery phrase.** 12 words from the BIP39 English list. `bilbo device init` shows it once, with the owner fingerprint to write down beside it, in a terminal only, and asks for 3 of its words back before it writes anything. bilbo never stores it, never prints it to stdout, and refuses to show or read it without a terminal or under an agent (`CLAUDECODE`, `CODEX_THREAD_ID`).
- **Owner key.** Derived from the phrase: an Ed25519 signing key and an X25519 box key. Every enrolled device keeps the signing seed, so any of them can sign, and the box public key. No device keeps the box secret: only the phrase opens the owner's copy of an epoch key. Its fingerprint, `xxxx-xxxx-xxxx-xxxx-xxxx-xxxx`, is what `bilbo relay --owner` will take.
- **Device key.** Each device has its own random Ed25519 and X25519 pair, a name (by default the host name) and an id derived from its signing key. Secrets live in `<state>/bilbo/keys/` (folder 0700, files 0600), never under the store root, so a copied store does not copy an identity.
- **Scope manifests.** For every scope whose `scope.<name>.sync` is a URL, bilbo creates a scope id and a signed, versioned manifest under `<root>/.bilbo/scopes/<scope_id>/manifest/<n>.json`. It lists the devices, pins the relay URL (only the scheme for a folder, whose path differs per machine), and seals the scope's epoch key to each device and to the owner, so the phrase alone opens every scope. The scope's name is stored encrypted.
- **New verb `bilbo device`.**
  - `bilbo device` shows this device, the owner fingerprint and each syncing scope's manifest.
  - `bilbo device list` lists the enrolled devices.
  - `bilbo device init [--name <name>]` creates the phrase, the owner key and the device key, then the missing manifests. Run again, it only brings the manifests in line with the config, and needs no terminal.
  - `bilbo device recover [--name <name>]` rebuilds the owner key from a typed phrase on a new or wiped device, checks its fingerprint, and adds that device to the scopes the store already holds. It never creates a scope: a syncing scope with no local manifest stays unsealed. Run again, it finishes a recover that stopped half way.
  - `bilbo device revoke <name|id>` removes a device from every manifest and rotates each scope to a new epoch key, chained so later devices can still read older epochs. Like `recover`, and like `init` when it changes a scope's URL, it runs only in a terminal.
- **What revocation guarantees.** After a confirmed revocation a revoked device, even one using the owner signing seed, cannot read anything written under later epochs; it can still disrupt by signing versions that members reject or that change the device list, which watch announces. A valid manifest's chain must hold every earlier epoch, and every member checks the entries for the epochs it holds, so no one can mint an epoch without the current key. The watch that announces is `add-sync`'s.
- **Config.** `scope.<name>.sync` accepts `file:///<absolute path>` and `https://<host>[:port][/prefix]`, and `http://` only to a loopback host, as well as `off`.

What a user can observe after this change: the ceremony, the key files and their modes, the fingerprints, the manifests on disk and their validity (`bilbo device` exits 1 on a bad one), a recovery that adds a device, and a revocation that bumps the epoch. Nothing is sent anywhere, and a scope with a URL is still not synced.

## Capabilities

### New Capabilities

- `device-identity`: the recovery phrase and its ceremony, the owner key and fingerprint, device keys, ids and names, where secrets live, and the `bilbo device` verb (show, `list`, `init`, `recover`, `revoke`).
- `scope-manifest`: scope ids, the manifest's location, fields, signature and encryption, epoch keys and their chain, and how init, recover, a URL change and revocation write new versions.

### Modified Capabilities

- `config`: Config location adds `device` to the verbs that read settings (on top of `add-note-scope`'s copy). Scope settings (added by `add-note-scope`) lets `scope.<name>.sync` be a URL, gains the "Sync takes a URL" scenario, and its "An unknown sync value" scenario now uses a value that is neither `off` nor a URL. A new requirement, Scope sync URLs, gives the URL forms.
- `cli`: Verb dispatch adds `device` (on top of `add-note-scope`'s copy). Output streams extends the wizard's exception to the phrase prompts of `device init` and `device recover`.

## Non-goals

- Any transport. Nothing is uploaded or fetched, and nothing reads the URL's folder or host. `add-sync` adds the `file://` transport, `add-relay` the `https://` one.
- Pairing a second device with a short code. `add-device-pairing` adds `bilbo pair`. Here a second device joins only through `recover`.
- A setup step or wizard question. `add-sync` appends the `sync` step, which will run this ceremony.
- Keeping the owner signing seed off ordinary devices, so a revoked device can still disrupt a scope (not read it). design.md says what a later version changes. Also out: a second owner, and sharing a scope with another person.
- Renaming a device or a scope. A scope renamed in the config is a new scope with a new id.
- Recalling data a revoked device already holds. Revocation protects only what later epochs seal.
- Removing keys with `bilbo setup --remove`. Keys outlive the store on purpose; design.md says how to delete them by hand.
- Teaching the agent plugin about devices. Agents are refused the ceremony and need no skill for it.

## Impact

- New library modules: `src/keys.rs` (every crypto primitive, random bytes, key files and their modes, the host name, core files off), `src/phrase.rs` (the embedded BIP39 list, encoding, checksum and prefixes) and `src/manifest.rs` (format, canonical bytes, verification, sealing, the chain, and writing versions under a lock).
- New verb `src/device.rs`, its `mod` line, dispatch arm and USAGE line.
- `src/wizard.rs` gains the phrase ceremony on the existing `Prompter` trait, and the `Terminal` adapter gains the alternate screen it is drawn on.
- `src/config.rs` widens `scope.<name>.sync`. `src/store.rs` gains the scopes and keys paths, and the `CLAUDECODE` and `CODEX_THREAD_ID` variables in `Env`.
- New dependencies: `ed25519-dalek` 3.0.0, `hpke` 0.14.1 (X25519 and ChaCha20-Poly1305 only), `chacha20poly1305` 0.11.0, `hkdf` 0.13.0 and `getrandom` 0.4.3, all in `src/keys.rs`. `sha2` gains a second user, `src/keys.rs`, and `libc` a third, also `src/keys.rs`. No `bip39` crate: the 2048-word list is embedded as `src/bip39-english.txt`.
- Tests: `tests/device.rs` through the built binary, with golden key and manifest fixtures in `tests/fixtures/device/`; the ceremony is unit-tested in `src/device.rs` and `src/wizard.rs` with the scripted prompter, and smoke-tested by hand in a terminal.
- `flake.nix`'s `lib.fileset` already takes `./src` and `./tests` whole, so the word list and the fixtures join the build without an edit.
- Migration: none. A config without a sync URL behaves exactly as before.
