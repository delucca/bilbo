# Design

## Context

- bilbo has no identity today. Nothing signs or encrypts. The only hashing is `ring`'s SHA-256 for the model download, kept in `src/host/model.rs`, and `sha2` behind `src/shared/hash.rs` (`sha256_hex`, for history ids and library digests). `tests/layout.rs`'s `PLACEMENT` table keeps each crate in its files.
- `src/` is one folder per domain (`note/`, `search/`, `library/`, `citation/`, `setup/`, `host/`) with each verb inside the domain it serves, and `src/shared/` is the Shared Kernel, which admits a module only when two domains use it. Verbs never use each other, only `main` reaches a verb, and the domains' non-verb code forms no cycle. `tests/layout.rs` checks all of it, and clippy's `module_inception` rejects a module named like its domain (AGENTS.md, Architecture rules).
- `add-note-history` brought `src/host/swap.rs` with `rename_new` (a rename that refuses to replace) and the habit of locking with std's `File::lock` (`note::versions::lock`). `add-note-scope` brought the `scope.<name>.*` keys in `src/shared/config.rs`, with `sync = off` the only value (`config::Scope.sync` is a `&'static str`), and `bilbo scope` (`src/note/scope.rs`), which prints each scope's `sync` value as a column.
- `src/host/prompt.rs` is the only `cliclack` user. It holds the terminal port: the `Prompter` trait (`input`, `password`, `confirm`, `select`, `note`, ...) and `Terminal`, drawn by cliclack on stderr. The setup wizard (`src/setup/wizard.rs`) asks through it, and setup's unit tests have their own scripted prompters (`Script` in `src/setup/wizard.rs`, `Scripted` in `src/setup/driven.rs`). Setup decides whether to run the wizard from `stdin().is_terminal() && stderr().is_terminal()` (`src/setup/flags.rs`).
- `store::state_dir` (`src/shared/store.rs`) already resolves `<state>` (`$XDG_STATE_HOME`, else `$HOME/.local/state`). `config::url_problem` and `config::is_local` (`src/shared/config.rs`) already validate an embedder URL and tell a loopback host. `zeroize` 1.9.0 is already a dependency, for `Zeroizing`.
- `flake.nix` builds from a fileset that takes `./src` and `./tests` whole, and `buildRustPackage` in the pinned nixpkgs runs the tests with the release profile (`checkType ? buildType`, `buildType ? "release"`). A test hook gated on `debug_assertions` is therefore off under `nix flake check`.
- The contract for changes 3 to 6 is `work/sync-spec/CONTRACT.md` in the planning notebook, section "Identity and keys". This design fixes what it leaves open. The review of the first draft is `work/sync-spec/review-add-device-keys.md`, and its decisions are folded in here.

## Goals / Non-Goals

**Goals:**
- One identity per device and one owner per person, with formats that changes 4, 5 and 6 sign, seal and verify against without change.
- A phrase ceremony that an agent cannot watch or drive, and destructive forms an agent cannot run.
- Revocation that keeps a removed device out of later epochs, even when it keeps its keys folder.
- A user can create, back up and rehearse a recovery (recover, then revoke the stale device) before any note leaves the device.

**Non-Goals:**
- Hiding secrets from a program that runs as the same user. The key files are as safe as `~/.ssh/id_ed25519`.
- Clearing every copy of the phrase from memory. See "Zeroizing" for what is and is not covered.
- Rotating the owner. See "What revocation guarantees".

## Decisions

### Modules: a new `identity` domain

No existing module does crypto, and `ring` must stay in `host/model.rs`, so the primitives get a new home: a new domain, `src/identity/`, holding four library modules and the verb.

- **Why a domain of its own.** `shared/` admits a module only when two domains use it, and in this change only `device` uses these. `host/` holds adapters to what bilbo runs beside, not bilbo's own formats. The verb cannot be `device/device.rs` (`module_inception`), and a `device/` folder that is itself the verb, as `setup/` is, would put keys, phrase and manifest inside a verb, which no other code may use. `add-sync` (segments, the transport, the watcher), `add-device-pairing` (`bilbo pair`) and `add-relay` (manifest checks on the relay) all use them, so they must sit outside every verb. In a domain, each later verb uses `identity::keys` and `identity::manifest` as a verb may use any domain, and nothing moves.
- **`src/identity/keys.rs` (new, library).** The device's identity and every secret it touches. It is the only user of `ed25519-dalek`, `hpke`, `chacha20poly1305`, `hkdf` and `getrandom`, the second user of `sha2`, and a third user of `libc`. It holds:
  - random bytes, hex and base32, the fingerprint and device id;
  - the owner derivation and device key generation;
  - sign and verify, HPKE seal and open, XChaCha20-Poly1305 encrypt and decrypt;
  - the key files with their modes;
  - the host name and turning core files off.

  It returns plain values and `String` messages.
- **`src/identity/phrase.rs` (new, library).** The embedded word list (`src/identity/bip39-english.txt`), entropy to words, words to entropy with the checksum, the 4-letter prefix rule, and picking the 3 positions. It hashes through `shared::hash::sha256`, so it uses no crypto crate and does not depend on `keys`.
- **`src/identity/manifest.rs` (new, library).** The manifest struct, its canonical bytes, the validity checks that need no secret, the checks that need the epoch key, finding a scope by its sealed name, and writing the next version under the lock.
- **`src/identity/ceremony.rs` (new, library).** The phrase ceremony: showing and confirming a new phrase, reading a typed one, and the fingerprint question, all through the `Prompter`. It is not in the verb because `add-sync`'s setup `sync` step runs the same ceremony, and verbs never build on each other.
- **`src/identity/device.rs` (new, verb).** Argument parsing, the terminal rule, the order of checks, and the step report. It builds on `keys`, `phrase`, `manifest`, `ceremony`, `config`, `store` and `host::prompt`. What `add-sync`'s setup step must also run (the ceremony, writing keys, building each scope's next version) lives in the library modules, not here.
- **`src/identity/script.rs` (new, test-only).** The scripted `Prompter` that the unit tests of `ceremony` and `device` share.
- **Extended modules:**
  - `shared/config.rs`: sync URLs, and `Scope.sync` becomes an owned `String`, which `note/scope.rs` prints;
  - `shared/store.rs`: `scopes_dir`, `keys_dir`, and `claudecode` and `codex_thread_id` in `Env`;
  - `shared/hash.rs`: `sha256`, the bytes that `sha256_hex` prints, for ids, fingerprints and the phrase checksum;
  - `host/prompt.rs`: `Prompter::screen` and the alternate screen in `Terminal`;
  - `main.rs`: dispatch.
- **Domain edges.** `identity`'s library modules use only `shared` and `host` (`prompt`, `swap`). Later domains that use `identity` (the sync domain of `add-sync`, the relay) must never be used by `identity`'s library modules, or `domains_form_no_cycle` fails. Where a transport meets an identity, as in `add-sync`'s fetch before `recover` and its setup `sync` step, a verb joins the two.

### Crates, versions and why each

Resolved on 2026-10-03 in a scratch crate holding bilbo's `Cargo.toml` and `Cargo.lock` plus `add-note-history`'s `notify` 8.2.0 and `sha2` 0.11.0 (scratchpad `add-device-keys-full`), and again on 2026-10-05 against bilbo 0.11.0's own `Cargo.toml` and `Cargo.lock`, with the same result: the same 24 new lock entries and the same duplicate set, the five being the newest releases on crates.io. That second pass also built the Nix package with them, tests included, and its probe reproduced the three values of the `Owner key` scenario with `from_bytes`. With the five crates below, `cargo tree -d -e normal` adds one duplicate, `getrandom` 0.2.17 (from `ring`) beside 0.4.3. The other, `syn` 2 beside 3, is already in bilbo's lock. There is one `digest` (0.11.3), one `rand_core` (0.10.1) and one `x25519-dalek` (3.0.0, inside `hpke`). A second scratch program derived an owner key, signed and verified, sealed and opened an epoch key with HPKE to a random and to a derived box key, and encrypted a name with XChaCha20-Poly1305, all on Rust 1.95. Every crate's `rust-version` is 1.85 or lower.

| Crate | Version | For | Why not std or an existing dependency |
|---|---|---|---|
| `ed25519-dalek` | 3.0.0 | owner and device signatures | std has no signatures. `ring` has Ed25519 but is confined to `model.rs`, and its `agreement` X25519 is ephemeral-only, so it cannot open a box sealed to a long-lived key. |
| `hpke` | 0.14.1, `default-features = false`, features `x25519`, `chacha`, `alloc`, `getrandom` | sealing epoch keys (RFC 9180) | Hand-rolling X25519, HKDF and an AEAD into a sealed box is the code an RFC exists to replace. Default features would add ML-KEM, X-Wing, SHA-3 and the NIST curves. Its key types wrap `x25519-dalek` and clamp on `from_bytes`, so `x25519-dalek` needs no direct entry. |
| `chacha20poly1305` | 0.11.0, feature `zeroize` | the sealed name, the epoch chain, and change 4's segments (XChaCha20-Poly1305) | Already inside `hpke`. |
| `hkdf` | 0.13.0 | the owner derivation | Already inside `hpke`. |
| `getrandom` | 0.4.3 | every random byte | std has no stable OS randomness API in Rust 1.95. `hpke` and `crypto-common` use the same version. |
| `sha2` | 0.11.0 (already a dependency) | the HKDF hash | `Hkdf::<Sha256>` names the hash type, so `keys.rs` must import it. Plain SHA-256 (ids, fingerprints, `prev`, the phrase checksum) goes through `shared::hash`. |

New lock entries: 24, namely `aead`, `chacha20`, `chacha20poly1305`, `cipher`, `cmov`, `ctutils`, `curve25519-dalek`, `curve25519-dalek-derive`, `ed25519`, `ed25519-dalek`, `fiat-crypto`, `getrandom` 0.4, `hkdf`, `hmac`, `hpke`, `inout`, `poly1305`, `r-efi` (UEFI only), `rand_core`, `rustc_version` and `semver` (build script), `signature`, `universal-hash` and `x25519-dalek`. Licences are BSD-3-Clause (dalek) or MIT/Apache-2.0, all compatible with bilbo's Apache-2.0.

**`bip39` 3.0.0 rejected; the list is embedded.** bilbo needs the 2048 words and the 4-bit checksum, about 60 lines over `sha2`. The crate adds `bitcoin_hashes`, `hex-conservative`, `arrayvec`, `unicode-normalization` and `tinyvec`, and its main job, the PBKDF2 wallet seed, is unused. The list is `src/identity/bip39-english.txt` (13,116 bytes, `include_str!`), copied from `bitcoin/bips` `bip-0039/english.txt`, whose BIP header says `License: MIT`. A unit test pins its SHA-256, `2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda`.

`PLACEMENT` in `tests/layout.rs` becomes (paths below `src/`; AGENTS.md names no crate and points to the table):
- `cliclack` in `host/prompt.rs`;
- `libc` in `host/prompt.rs`, `host/swap.rs` and `identity/keys.rs`;
- `notify` in `note/watch.rs`;
- `ring` in `host/model.rs`;
- `sha2` in `shared/hash.rs` and `identity/keys.rs`;
- `unicode_normalization` in `shared/text.rs`, and `htmd` and `markup5ever_rcdom` in `library/html.rs`, as today;
- `ed25519_dalek`, `hpke`, `chacha20poly1305`, `hkdf` and `getrandom` in `identity/keys.rs`.

### The phrase and the owner key

- **Entropy.** 16 bytes from `getrandom`, in a `Zeroizing<[u8; 16]>`. The words are BIP39's: the 128 bits plus the first 4 bits of their SHA-256, cut into 12 groups of 11 bits.
- **Derivation.** `HKDF-SHA256(salt = "bilbo-owner-1", ikm = the 16 bytes)`. Expand with info `sign` gives the Ed25519 seed. Expand with info `box` gives 32 bytes used as they are as the X25519 private key: `hpke`'s `PrivateKey::from_bytes` clamps them. RFC 9180's `DeriveKeyPair` is not applied on top. That keeps `recover` a direct reproduction of the spec text, and a second labeled HKDF would add nothing to 32 uniform bytes. The draft's scratch program used `DeriveKeyPair`. The values the spec now pins were recomputed with `from_bytes`: for 16 zero bytes, the owner key is `12a801fe…7c79`, `owner_box` is `11985d02…8b60`, and the fingerprint is `yb4b-5aju-v6zb-x2nm-nc5x-ompf`.
- **Why not BIP39's PBKDF2 seed:** it stretches a passphrase, and there is none. 128 random bits need no stretching, and HKDF on the raw entropy avoids Unicode normalization of the words.
- **Typing words.** A word may be given by its first 4 letters, which are unique in the BIP39 list (checked in review), both when confirming at `init` and when entering at `recover`. Case and surrounding spaces are ignored.

### Fingerprints and ids

- **Owner fingerprint:** the first 24 characters of lowercase base32 (RFC 4648, no padding) of SHA-256 over the owner's Ed25519 public key, in six groups of four joined by `-`. That is 120 bits: finding another key with the same fingerprint costs about 2^120 work. Groups of four are what a person compares and types into `bilbo relay --owner`. Change 6 decides whether it also accepts the form without hyphens.
- **Comparison points.** A wrong word can still pass the 4-bit checksum (1 in 16), and the phrase is fixed at 12 words, so the fingerprint is the user's check that a phrase is the right one. It appears in three places:
  - `init` shows it on the phrase screen, to be written down beside the words;
  - `recover` shows the derived fingerprint and, when no local manifest vouches for it, asks the user to confirm it against the written copy or another device's `bilbo device`;
  - `bilbo device` prints it, for that comparison and for `bilbo relay --owner`.
- **Device id and scope id:** 26 base32 characters, as the contract fixes, printed whole because they also name folders on the transport.
- **Base32 and hex are hand-written** in `keys.rs`, about 40 lines with RFC 4648's test vectors. Neither is worth a crate.

### What an enrolled device keeps

```
<state>/bilbo/keys/          0700
  owner.key                  0600  {"format":1,"sign":"<64 hex seed>","box_public":"<64 hex>"}
  device.key                 0600  {"format":1,"name":"rivendell","sign":"<64 hex seed>","box":"<64 hex private key>"}
```

- **The owner's X25519 private key is never written.**
  - `init` derives it only to compute `box_public`.
  - `recover` derives it from the typed phrase to open the `owner` entry of each local manifest of that owner, then drops it.
  - It lives in a `Zeroizing` buffer or a zeroize-on-drop `hpke` key, and never outside one call of the verb.
- **What the seed alone allows.** The owner signing seed lets any enrolled device sign manifests, as the user decided: create a scope, add a device (with an epoch key it already holds through its own `sealed` entry), and revoke one. Creating a scope seals to `owner_box`, which needs only the public key. No step of `init` or `revoke` needs the box secret.
- **What a device can open.** Only its own `sealed` entry. So it finds a scope's name, and the epoch keys, only in manifests that list it. A manifest of its owner that does not list it shows `-`. `init` never adds this device to such a manifest. Only `recover`, with the phrase, does.

### What revocation guarantees

**Guaranteed.** Once the revoking version is confirmed on the transport (see "Pending versions"), the revoked device cannot read anything written under a later epoch. That holds even for an active thief who kept `<state>/bilbo/keys/` whole and uses the owner signing seed:
- `owner.key` holds no box secret, and the new epoch is sealed only to the remaining devices' box keys and to `owner_box`.
- The thief cannot seal a later epoch's key to itself, because it never learns it.
- It cannot mint an epoch of its own that a member accepts. A valid version's `chain` must hold an entry for every epoch from 1 to `epoch`-1 (Manifest validity, checked without secrets, so the relay enforces it too). Every member checks that each entry for an epoch whose key it already holds decrypts to that key (Chain check by a member). A rotation needs a correct entry for the current epoch, which only a holder of the current key can write.
- Every epoch is still sealed to `owner_box`, so the phrase alone recovers every scope.

**Not guaranteed.**
- The revoked device keeps every note and every epoch key it already had. Revocation never recalls data.
- Until the revoking version is confirmed, writers keep using the old epoch, which the revoked device knows.
- What remains is disruption. The revoked device still signs with the owner seed, so it can publish:
  - versions that members find invalid. Members reject them, and on a transport where create-only lets them take version `n+1` they block honest writers until the user acts;
  - valid versions that change `devices` or repoint `transport` without a new epoch. Change 4's watch announces every added device and epoch change, and a pinned URL that no longer matches the config stops sync.
- On a `file://` transport, the revoked device keeps write access to the folder through the cloud account. `revoke` prints a line saying to remove it there.
- The remedy against a disruptive thief stays a new owner, done by hand and written in the README:
  1. On a trusted device, move `<state>/bilbo/keys/` and `<root>/.bilbo/scopes/` aside.
  2. Run `bilbo device init`, which makes a new phrase, a new owner and new scope ids.
  3. Recover or pair the other devices, and restart the relay with the new fingerprint (change 6).

The same sentence goes in the README and in changes 4 to 6: "After a confirmed revocation a revoked device, even one using the owner signing seed, cannot read anything written under later epochs; it can still disrupt by signing versions that members reject or that change the device list, which watch announces."

**What a later version would change.** Devices would hold no owner seed at all. The owner, behind the phrase, signs a short list of admin devices, and each manifest also carries the writing device's signature. A version written by a device absent from version n-1 is then invalid, so the thief cannot disrupt either. The manifest's `format` and the `bilbo-manifest-1` prefix are versioned for that move.

### The terminal rule, and keeping the phrase away from agents

`identity::device::run(args, env, terminal: bool, p: &mut impl Prompter)` returns the step lines. `main` passes `host::prompt::Terminal` and `terminal = stdin().is_terminal() && stderr().is_terminal()`. Unit tests pass the scripted prompter with `terminal = true`.

- **What needs a terminal:**
  - `init` when it creates a phrase or would change a scope's `transport`;
  - `recover`;
  - `revoke`.

  It also needs `CLAUDECODE` and `CODEX_THREAD_ID` unset or empty. Otherwise the form, or for `init` that step, refuses before generating or writing anything: "run `bilbo device <form>` yourself, in a terminal". The message is worded for an agent to relay to the user.
- **Why revoke and a URL change too.** Removing a device and moving where a scope may sync are decisions, not upkeep. A prompt-injected agent reaches for a one-line verb, and change 5 applies the same rule to all of `bilbo pair`. Creating the first manifest for a URL the config already names stays open to an agent: it gives no one new access. The config is the user's statement, and sync (change 4) follows the manifest's pinned URL only while the config agrees under the comparison in "The pinned transport".
- **What the rule is for.** It stops accidental and low-effort misuse, not a hostile agent. An agent with a shell can unset the markers and fake a pty with `script`, or simply copy `owner.key` and `device.key` (0600, same user) off the machine. The key files are the real boundary, and the README says so.
- **The agent markers.** Both were checked on 2026-10-03:
  - Claude Code's Bash tool sets `CLAUDECODE`, and gives neither stdin nor stderr a terminal.
  - A `codex exec` probe (codex-cli 0.155.1, throwaway `CODEX_HOME`, `-s read-only`) showed tool commands getting `CODEX_THREAD_ID`, `CODEX_SESSION_ID`, `CODEX_CI`, `CODEX_VERSION`, `CODEX_SANDBOX` and `CODEX_SANDBOX_NETWORK_DISABLED`, with no terminal on stdin or stderr. `CODEX_THREAD_ID` is the marker, because `CODEX_SANDBOX` is absent when Codex runs without a sandbox.

  The markers matter when an agent is given a pty, and for a user's `!` command whose output lands in a transcript. `Env` gains both fields so tests can set them.
- **Where the phrase is drawn.** The `Prompter` trait gains one method, `screen(work)`, whose default runs `work` directly, so setup's scripted prompters need no change.
  - `Terminal` writes `ESC[?1049h` to stderr (the alternate screen), runs `work`, and writes `ESC[?1049l` from a drop guard, so an error or a cancel also leaves it.
  - Inside, `init` shows the words numbered in three columns and the owner fingerprint with `note`, then asks "Written down?" with `confirm`, then three `input` prompts ("Word 7").
  - On the main screen, after it, only "Recovery phrase confirmed" remains, so the words are not in the scrollback.
  - The scripted prompter of `src/identity/script.rs` keeps the default.
- **A wrong word.** A `warn` and a `select`: try again, show the phrase again, or cancel.
- **Recover's entry.** Twelve `input` prompts on the alternate screen, each checked by `phrase::check_word` (a plain `fn`, as `Prompter::input` wants). A failed checksum asks for the twelve again, with the typed words as defaults. Then comes the fingerprint step above, a `confirm` that defaults to no. Echo is on: the screen is cleared afterwards, and typing twelve words blind is worse.
- **No core file.** Before the screen opens, `keys::no_core_dump` sets `RLIMIT_CORE` to 0 with `setrlimit`, and on Linux also calls `prctl(PR_SET_DUMPABLE, 0)`, so a crash cannot write the phrase to `systemd-coredump` or a `core` file.
- **Never in output.** No step line, error or log names a word: errors give positions only. bilbo writes no log for `device`.
- **What remains.** A terminal recorder, `tmux capture-pane` while the screen is up, a screen reader, or a person behind the user. These are the same exposure as writing the phrase down.

### Zeroizing

- `Zeroizing` holds:
  - the entropy and the phrase string;
  - every HKDF output, the owner box secret included;
  - every epoch key;
  - typed words, wrapped as soon as `input` returns;
  - the hex text read from key files.
- `ed25519_dalek::SigningKey` zeroizes on drop through its default `zeroize` feature. `hpke`'s X25519 `PrivateKey` wraps `x25519_dalek::StaticSecret`, which `hpke` builds with `zeroize`.
- **Not covered:**
  - cliclack's and `console`'s line buffers;
  - the terminal emulator;
  - `serde_json`'s intermediate strings when it parses a key file (the hex is copied out and the parsed value dropped at once);
  - swap.

  This is best effort, not a guarantee, and the README says so.

### Key files

- **Outside the store**, so a copied or backed-up store never clones an identity. The owner file holds a derived seed and a public key, never the entropy and never the box secret, so it cannot print the phrase or open an `owner` entry.
- **Writing.** `init` and `recover`, and any later verb that writes an identity (the setup wizard's sync step, `bilbo pair` on a new device), hold `<state>/bilbo/keys.lock` (std `File::lock`), remove a leftover `keys.new/`, write both files into `keys.new/` (mode 0700, files opened `create_new` with mode 0600, fsynced), then `swap::rename_new` it to `keys`. An identity appears whole or not at all, and never replaces another.
  - The name is fixed, so a crashed run leaves at most one stale folder. The next writer removes it, and any other form reports it.
  - On an enrolled device, `recover` writes no key.
- **Reading.** Every form that reads the keys checks that the folder is 0700 and each file 0600, with no group or other bits. Otherwise it refuses, naming the path and the `chmod` that fixes it, as `ssh` does.
  - A folder holding one of the two files is a damaged identity, and so is a file with a malformed key or name. bilbo refuses and says to move the folder aside and run `recover`.
- **Removal.** `bilbo setup --remove` leaves the keys. The README says to delete `<state>/bilbo/keys/` and `<root>/.bilbo/scopes/` by hand to forget an identity.

### Manifest bytes, signature and validity

Example version 2 (hex shortened):

```json
{"format":1,"scope":"m3x…","n":2,"prev":"9f2c…","owner":"12a8…","owner_box":"1198…","devices":[{"id":"4ok…","name":"rivendell","sign":"3d40…","box":"de9e…"},{"id":"q7f…","name":"bagend","sign":"fc51…","box":"a1b2…"}],"transport":"file://","epoch":1,"sealed":{"4ok…":"<160 hex>","owner":"<160 hex>","q7f…":"<160 hex>"},"chain":[],"name":"<hex>","sig":"<128 hex>"}
```

- **Canonical bytes.** A serde struct with `deny_unknown_fields`, members in declaration order, `sealed` as a `BTreeMap`, compact output from `serde_json`, then a newline. Verification parses the file, re-serializes it, and treats any byte difference as invalid. So `prev`, the SHA-256 of the previous file's bytes, has one meaning.
- **Signed message:** `bilbo-manifest-1\n` followed by the canonical JSON without `sig`.
- **Checks that need no secret** (`manifest::verify_scope`, also run by the relay in change 6):
  - the canonical bytes and the signature;
  - `scope` equal to the folder name (the path segment, on a transport);
  - `n` equal to the file name;
  - the same `owner` as version 1;
  - `prev` chaining;
  - `devices` sorted by id with no duplicate, each id derived from its `sign` key;
  - `sealed`'s keys exactly the listed ids and `owner`;
  - `chain` holding one entry per epoch from 1 to `epoch`-1, in order, with every entry of version `n-1` byte-identical. Epoch numbers are plaintext, so the relay can enforce presence without a key.

  The folder check matters because on a relay the path and the content come from different parties.
- **Checks that need the epoch key** (`manifest::open`, run by a listed device): this device's entry opens, `name` opens, every `chain` entry opens, and the entry for each epoch whose key the device opened from an earlier valid version decrypts to that same key. An entry for an epoch the device never held is accepted unchecked. A version that fails them is invalid for that device. The last check is what keeps a revoked device from minting an epoch (see "What revocation guarantees"). It needs the epoch keys of the previous valid version, which `open` derives again from that version through this device's entry and the chain.
- **Sealing parameters**, now in the spec because changes 4 and 5 must match them byte for byte:
  - HPKE base mode with info `bilbo-epoch-1` and aad `<scope id>\n<epoch>\n<recipient>`;
  - the name under aad `bilbo-name-1\n<scope id>\n<epoch>`;
  - chain entry `k` under aad `bilbo-chain-1\n<scope id>\n<k>`, encrypted with epoch `k+1`'s key.

  Base mode suffices because the owner signature over the whole manifest says who sealed it. Random 192-bit XChaCha nonces make reuse negligible, and distinct aads keep one purpose's ciphertext from passing as another's.
- **Hex, not base64:** note history's ids are hex, manifests are a few kilobytes, and hex needs no decoder table. Change 4 picks the segment encoding.
- **Device names in the clear.** A relay sees host-like names such as `rivendell`. The contract lists `name` among the device fields. Encrypting it under the epoch is the alternative, at the price of the relay and an unenrolled device seeing only ids.

### The pinned transport

A `file://` folder has a different path on every machine (`/Users/a/Dropbox/bilbo` on a Mac, `/home/a/Dropbox/bilbo` on Linux), so pinning the full URL would leave every mixed pair permanently in disagreement, and `init` would flip the pin back and forth. `transport` therefore holds:
- only `file://` for a folder, the path staying each device's own config;
- the full URL for `https://` and loopback `http://`, where the relay is the same address for everyone and pinning it is what the contract's "allowed relay" means.

`bilbo device`'s problem line and `init`'s URL-change rule compare `https://` and `http://` URLs whole and `file://` URLs by scheme only. Moving the folder is therefore a config edit with no manifest write. Switching between a folder and a relay is a pinned change that needs a terminal.

### Finding a scope's manifest

A device opens the `name` of each local manifest that lists it, through its own `sealed` entry, and matches it against the config. There is no plaintext index file to keep in step. The cost is one HPKE open per scope per run, a fraction of a millisecond. Manifests that do not list this device show `-`, and so do all manifests on an unenrolled device.

### Which devices a new scope lists, and when versions are written

- **A new scope** lists this device and every device listed by the latest manifest of any other scope of the same owner. There is no separate device registry object: the contract's transport tree has none, and for one person's handful of devices the union is the registry. `revoke` rewrites every manifest that lists the device, so the union stays clean.
- **Only `init` mints scope ids.** `recover` never does. On a machine whose store has no manifest for a syncing scope, it reports `unsealed` and tells the user to bring in the scope's manifest (copy the store from an enrolled device) and run `recover` again. It never suggests `init`. Without a transport, a fresh id there would fork the scope the other device already has, and change 4 would have to reconcile two ids for one name. The way to join in change 3 is to copy the store from an enrolled device first, or to run `init` deliberately afterwards.
- **`recover` is also how a stopped recover is finished.** On an enrolled device it reads the phrase, checks it derives the stored owner, keeps the device key, and adds this device to each local manifest of its owner that does not list it. That includes one whose latest version dropped this device: with the phrase in hand, re-adding is the owner's decision. `init` never re-adds.
- **`revoke` acts on manifests that list both devices.** It needs the current epoch key to extend the chain, and opens it through its own entry. A manifest listing the target but not this device is reported `failed`.
- **Writes.** Writers hold `<root>/.bilbo/scopes/lock` (std `File::lock`, as `note::versions::lock` does for history). A version is written to a hidden temporary file in the same folder and moved with `swap::rename_new`, so an existing version is never replaced. Under the lock, a writer first removes any hidden temporary left in the `manifest/` folders by a crash: no other writer can be mid-write while it holds the lock, so every one it finds is stale. That is the same create-only rule change 4's transport applies, and the conflict two devices will meet when both write `n+1`.
- **Several scopes are not atomic.** A crash midway leaves some scopes updated. A rerun of `init`, `revoke` or `recover` finishes the rest, because each step checks the latest version first.
- **Every refusal comes before the phrase.** No store when a scope syncs, another owner's manifests, a taken name, bad permissions and an invalid manifest are all checked before anything is drawn.

### Pending versions

- **The race.** On a folder transport, two devices can each write their own version `n` of one manifest before either sees the other's. Create-only writes keep one of them on the transport. The other device already holds its own `n` locally, and may already have sealed or encrypted under its new epoch.
- **The rule.** Every version a device writes locally is pending until change 4's transport holds a version `n` with the same bytes, and every version copied from a transport is confirmed on arrival. There is no third way into a store, so pairing's A (writes `n+1`, pending) and B (copies from the transport, confirmed) are both covered. An empty `manifest/<n>.pending` beside it marks it, and change 4 removes the marker when it reads the bytes back. A marker file keeps the version itself immutable, and a crash leaves at worst a marker on a version that is already confirmed, which the next read-back clears.
  - A new version builds on the latest local version, pending or not, so a `recover`, `pair` or `revoke` after a pending revocation seals, chains and names under the pending epoch inside the manifest. The rule is about note data: `manifest::usable_epoch` returns the newest epoch introduced by a confirmed version, and change 4 encrypts segments only under it. A losing version therefore never has note data that only it can open.
- **When it loses.** The pending file moves to `manifest/lost/<n>.json`, kept and never deleted, so a forensic trail stays. The winning version takes `manifest/<n>.json`, and the device applies its change again (the added device, the new URL, the revocation) as a new pending `n+1` on top of the winner. This move is the one exception to "versions are never rewritten, renamed or deleted". `verify_scope` ignores `lost/`.
- **In change 3** there is no transport, so every version this device writes stays pending. `bilbo device` shows `manifest <n> pending`. That is not a problem line and the exit code stays 0: it says nothing has been published yet, which is true.
- **Cost for revocation.** Until a revoking version is confirmed, writers keep encrypting under the old epoch, which the revoked device knows. Revocation takes effect for new data when the transport confirms the version, not when `revoke` returns. The README says so.

### Exit code of `bilbo device`

It exits 1 when a scope line got a problem line: an invalid version, another owner's manifest, or a pinned `transport` that differs from the config's URL under "The pinned transport" (whole for `https://`, scheme only for `file://`). A pending version is not a problem either. That follows `bilbo check` and the `cli` Exit codes rule ("finds problems"). An unsealed scope (a sync URL with no manifest yet) is not a problem: it is the expected state before `init`, and it gets a hint on stderr with exit 0. A scope set to `off` whose manifest still pins a URL is narrowing, as the contract allows, and is not a problem.

### Sync URLs in the config

- `file://` plus an absolute path, taken literally. Percent signs are not decoded, so a folder with spaces such as `Mobile Documents` is written as it is. This is a deliberate departure from URL rules, because users copy paths, not URLs.
- `https://host[:port][/prefix]`, reusing `url_problem` for the credential and host checks.
- `http://` only when `is_local` holds.
- No query or fragment, because change 6 appends `/v1/...` paths to the prefix.
- Validation does no I/O. A missing folder is change 4's report.

### The host name

`keys::host_name` calls `libc::gethostname`. It sits in `keys.rs` because the name is part of the device identity, and `keys.rs` already holds the other `libc` call, `no_core_dump`. Alternatives:
- Running `hostname` through `host::command`: it is absent from the clean `PATH` the tests use, and from some minimal Linux images.
- Reading `/etc/hostname`: Linux only.
- `$HOSTNAME`: zsh and bash do not export it.

The sanitizing rule is pure and unit-tested apart from the call.

### Tests without a terminal, and without a test hook

- **Unit tests:**
  - in `phrase.rs`: the word list hash; the BIP39 vectors (16 bytes of `00` to `abandon` ×11 `about`, `7f` to `legal winner thank year wave sausage worth useful legal winner thank yellow`, `80` to `letter advice cage absurd amount doctor acoustic avoid letter advice cage above`, `ff` to `zoo` ×11 `wrong`); prefixes; checksum failure;
  - in `keys.rs`: the derivation pinned for the all-`abandon` phrase (`owner`, `owner_box` and the fingerprint); base32 vectors; seal, open and wrong-key failure; the modes of written files; `keys.new`; no box secret in `owner.key`;
  - in `manifest.rs`: canonical round trip, every validity check, the chain over three epochs, and a revoked device's two key files opening nothing in the new version;
  - in `ceremony.rs`: the ceremony with the scripted prompter;
  - in `device.rs`: every form with temporary roots and state folders, including "the phrase is kept nowhere", which scans every written file for the words and the entropy hex.
- **Golden fixtures** carry the binary tests.
  - `tests/fixtures/device/` holds two key folders, `rivendell` and `bagend`, under the all-`abandon` owner with fixed device seeds, and a store whose `personal` scope has versions 1 and 2.
  - `tests/device.rs` copies them into temporary folders and sets modes 0700 and 0600, since git keeps only the execute bit. It then runs show, `list`, `init` on an enrolled device, and the refusals of every terminal-only form without a terminal, with `CLAUDECODE` and with `CODEX_THREAD_ID`. It also covers loose modes, `keys.new`, tampered, misplaced and foreign manifests, and the exit codes.
  - The `#[ignore]` test `identity::device::tests::write_fixtures` regenerates them, and `identity::device::tests::fixtures_open` fails when a format change leaves them stale. The fixture keys are test keys, documented as such.
- **Why not a test hook in the binary:** `nix flake check` runs tests in release, where a `debug_assertions` hook would be off. Restore's and `scope set`'s hooks are function parameters that only unit tests pass, and the ceremony's unit tests do the same through the scripted prompter. A hidden way to feed the phrase to the binary is the thing this change refuses agents.
- **The deliberate gap.** Spec scenarios that need a real terminal cannot run in the suites:
  - the drawn phrase and its fingerprint;
  - the cleared scrollback;
  - `bilbo device init > out.txt`;
  - `revoke` and a URL change succeeding in a terminal.

  The scripted prompter covers their logic. The `Terminal` adapter itself is smoke-tested by hand with `expect` on macOS, as AGENTS.md asks after any adapter change, and task 5.3 records each of these runs in `smoke.md`.

### What later changes rely on

- **Change 4:**
  - It relies on `keys::{sign, verify, encrypt, decrypt, random}`, `manifest::{latest, verify_scope, open, write_next}`, the epoch numbering and the chain, and the create-only write.
  - It runs the ceremony from setup's `sync` step through the same `identity::ceremony` functions, so the `device-identity` Recovery phrase requirement and the `cli` Output streams exception go on its shared-MODIFIED list.
  - A device that holds the owner seed but is not listed cannot open a scope. Joining therefore takes `recover` (the phrase) or pairing, never the owner key alone.
  - It announces every adopted version that adds a device or changes the epoch, and treats a version that fails Manifest validity or Chain check by a member as invalid (see "What revocation guarantees").
  - It syncs a scope only while the config URL matches the pinned `transport` under "The pinned transport", and owns the transport-aware rule for an unsealed scope: whether `init` may mint an id once it can list the transport.
  - It confirms pending versions by reading their bytes back, moves a losing one to `lost/`, and re-applies its change as `n+1` (see "Pending versions").
- **Change 5:** adds a device by writing the next version through the same builder `recover` uses, given the epoch key however the caller opened it, sealing the epoch key the enrolled device opens through its own entry. The pairing payload carries only the owner signing seed and `owner_box` (public), never an owner box secret, which no enrolled device has. It may add `or bilbo pair` to the `unsealed` hint by modifying `device-identity` What recover writes, since the verb does not exist at change 3.
- **Change 6:** verifies a manifest from its plaintext fields with `manifest::verify_scope`, without decrypting, and admits by the fingerprint format above.

## Risks / Trade-offs

- [A revoked device keeps the owner signing seed] → After a confirmed revocation it cannot read later epochs, even while writing, but it can disrupt. Change 4 announces device and epoch changes and rejects invalid versions. The README has the manual new-owner procedure, and the format leaves room for admin delegation.
- [Losing the phrase and every device loses the scopes] → The ceremony says so before the words are shown. It is the price of end-to-end encryption, and the panel accepted it.
- [A mistyped phrase that passes the checksum derives another owner] → `recover` refuses it against any local manifest, and otherwise asks the user to compare the fingerprint.
- [An agent with a real pty under a harness other than Claude Code or Codex sees the phrase] → Only the terminal test guards it. Add that harness's marker beside the two checked ones when one is known.
- [`hpke` is pre-1.0 and the dalek 3 line is new (crates.io shows them last updated in July and September 2026)] → Exact pins through `Cargo.lock`, use limited to RFC 9180 base mode, and round trips in the unit tests.
- [Device names reveal host names to a relay operator] → The user's own relay in v1. Encrypting names is the alternative recorded above.
- [A crash between scopes leaves a partial revocation or recovery] → A rerun completes it. `bilbo device` shows each scope's epoch.
- [`file://` paths are literal] → A user who pastes `%20` gets a folder named with `%20`. Change 4 reports a missing folder, and the README shows a path with a space.
- [Revocation does not recall plaintext] → Stated in the spec and the README. It protects later epochs only.

## Migration Plan

Nothing to migrate. After an upgrade, nothing changes until the user sets a sync URL or runs `bilbo device init`. To roll back, set every `scope.<name>.sync` to `off`, delete `<state>/bilbo/keys/` and `<root>/.bilbo/scopes/`, and downgrade. An older bilbo rejects a URL in `scope.<name>.sync`, so the config must be reverted first.

## Decisions for the user to confirm

Each is recorded above with its alternatives.

1. Embed the BIP39 list instead of the `bip39` crate.
2. Derive the owner with HKDF over the raw 128 bits (salt `bilbo-owner-1`, info `sign` and `box`), with the `box` output used directly as the X25519 key, not BIP39's PBKDF2 seed nor RFC 9180's `DeriveKeyPair`.
3. The fingerprint is 24 base32 characters in six groups of four. It is shown with the phrase, confirmed at `recover` when nothing else vouches for it, and printed by `bilbo device`.
4. Devices keep the owner signing seed and box public key, never the box secret (lead decision B1).
5. `init` (phrase or URL change), `recover` and `revoke` run only in a terminal and never under `CLAUDECODE` or `CODEX_THREAD_ID`. There is no piped or file input. Creating a first manifest needs no terminal.
6. The ceremony runs on the alternate screen with core files off. Recover's words are echoed there, and 4-letter prefixes are accepted at both entry points.
7. Loose key permissions are refused, as `ssh` does, rather than warned about or fixed.
8. `init` is idempotent and is how a newly configured scope gets its manifest. There is no setup step in this change.
9. `recover` never mints a scope id. It finishes an interrupted recover when run again, and refuses a name another device holds. `revoke` refuses this device.
10. `bilbo device` exits 1 on a problem line and 0 on an unsealed scope.
11. Manifests use hex, strict canonical JSON, and device names in the clear.
12. A new scope lists the union of the owner's devices. There is no device registry object.
13. `file://` paths are literal, and a manifest pins only the scheme for a folder (the full URL for `https://`) (lead decision B2 of the final review).
14. The host name and the core-file limit go through `libc` in `keys.rs`.
15. Binary tests use golden fixtures, with no test hook.
16. Owner rotation is a manual procedure in v1.
17. A valid `chain` holds every epoch from 1 to `epoch`-1, and members check entries against the keys they hold, so a revoked device cannot read later epochs even when it writes (lead decision M1 of the final review).
