# Tasks

Run every command below from the repo root as `nix develop -c sh -c '<command>'`, with `CARGO_TARGET_DIR` set to the checkout's `target`. A new file under `src/` or `tests/` needs `git add -N` before any Nix command can see it. Changes 1 and 2 are archived first: this change uses `swap::rename_new`, `sha2` and the `scope.*` keys.

## 1. Primitives and key files

- [ ] 1.1 Add `ed25519-dalek = "3.0.0"`, `hpke = { version = "0.14.1", default-features = false, features = ["x25519", "chacha", "alloc", "getrandom"] }`, `chacha20poly1305 = { version = "0.11.0", features = ["zeroize"] }`, `hkdf = "0.13.0"` and `getrandom = "0.4.3"` to `Cargo.toml`, and update `Cargo.lock`. Confirm the normal tree's duplicates are only `getrandom` (0.2 for `ring`, 0.4 new) and `syn` (already there), and that the Nix package builds. Verify with `cargo build --locked && [ "$(cargo tree -d -e normal --depth 0 | awk 'NF{print $1}' | sort -u | tr '\n' ' ')" = 'getrandom syn ' ] && nix build --no-link .#default`
- [ ] 1.2 Add `src/keys.rs`: random bytes, `sha256`, lowercase hex, RFC 4648 base32 without padding, the owner derivation (HKDF-SHA256, salt `bilbo-owner-1`, info `sign` and `box`, the `box` output passed to `PrivateKey::from_bytes` as it is), random device keys, the device id, scope id and owner fingerprint forms, Ed25519 sign and strict verify, HPKE base-mode seal and open with the info and aad of the `scope-manifest` spec, and XChaCha20-Poly1305 encrypt and decrypt with a random nonce. Hold every secret in `Zeroizing` or a zeroize-on-drop type. Unit-test the RFC 4648 base32 vectors, the `owner`, `owner_box` and fingerprint values pinned in the `Owner key` scenario, a changed word giving another fingerprint, seal and open round trips to a random and to the owner box key, opening with a wrong key, a changed aad or another recipient's id failing, and a tampered signature failing. Verify with `cargo test --locked --bin bilbo keys::`
- [ ] 1.3 Add `src/bip39-english.txt` (copied from `bitcoin/bips` `bip-0039/english.txt`) and `src/phrase.rs`: entropy to 12 words, words to entropy with the checksum, a word or its first 4 letters, case and spaces ignored, `check_word` as a plain `fn`, and 3 distinct random positions. Unit-test the list's SHA-256 (`2f5eed53…dbda`), that no two words share a 4-letter prefix, the four BIP39 vectors from design.md both ways, a prefix, an unknown word naming only its position, and a failed checksum. Verify with `cargo test --locked --bin bilbo phrase::`
- [ ] 1.4 Add the key files to `src/keys.rs`: `owner.key` (`sign`, `box_public`) and `device.key` (`name`, `sign`, `box`) as JSON with hex, written under `<state>/bilbo/keys.lock` into `keys.new/` (0700, `create_new` 0600 files, a leftover removed first) and moved into place with `swap::rename_new`. Reading refuses group or other bits, a folder holding one file, and a malformed key or name. Add `keys::host_name` and `keys::no_core_dump` (`setrlimit(RLIMIT_CORE, 0)`, and `prctl(PR_SET_DUMPABLE, 0)` on Linux) on `libc`, the name rule and sanitizing, and to `src/store.rs` `keys_dir`, `scopes_dir`, and `claudecode` and `codex_thread_id` in `Env`. Unit-test the modes, no box secret in `owner.key`, a refused second write, `keys.new` left over then removed, loose modes on the folder and on a file, a damaged identity, the core limit read back with `getrlimit`, and the sanitizing cases of the `Device name` scenarios. Verify with `cargo test --locked --bin bilbo keys:: && cargo test --locked --bin bilbo store::`

## 2. Sync URLs in the config (`config` delta)

- [ ] 2.1 Widen `scope.<name>.sync` in `src/config.rs` to the `Scope sync URLs` forms, reusing `url_problem` and `is_local`, with the credential error that does not repeat the URL. Replace change 2's "An unknown sync value" unit test, which used an `https://` URL, with the delta's scenarios, and add `device` to the verbs whose config errors the `Config location` scenarios check once the verb exists (5.1). Verify with `cargo test --locked --bin bilbo config::`

## 3. Manifests (`scope-manifest` spec)

- [ ] 3.1 Add `src/manifest.rs`: the struct with `deny_unknown_fields` and the member order of the spec, canonical bytes, the signed message `bilbo-manifest-1\n`, `verify_scope` with every check of `Manifest signature` and `Manifest validity` (canonical bytes, signature, `scope` equal to the folder, `n`, the same owner, `prev`, sorted unique devices with ids derived from `sign`, `sealed` keys equal to the ids and `owner`, `chain` holding every epoch from 1 to `epoch`-1 with version `n-1`'s entries byte-identical), `open`, which opens this device's entry (or `owner` with a phrase-derived key), the name and every chain entry, and checks each entry for an epoch whose key the previous valid version gave against that key, and the pinned-transport comparison (whole for `https://` and `http://`, scheme only for `file://`). Unit-test every scenario of `Manifest content`, `Pinned transport`, `Manifest signature`, `Manifest validity`, `Epoch key sealing`, `Epoch chain`, `Chain check by a member` (a revoked device's rotation with a wrong epoch 2 entry, and a chain with a gap) and `Sealed scope name`, including a non-canonical file, a copy in another scope's folder and a manifest of another owner. Verify with `cargo test --locked --bin bilbo manifest::`
- [ ] 3.2 Add writing to `src/manifest.rs`: the lock on the file `<root>/.bilbo/scopes/lock`, a hidden temporary file moved with `swap::rename_new` after removing stale temporaries under the lock, and the builders for a new scope (the owner's device union, sealed to `owner_box` from the public key), a URL change, adding a device, given the epoch key however the caller opened it (`recover` through `owner`, pairing in change 5 through its own entry), and a revocation (new epoch, chain extended, revoked entry gone, only on manifests listing this device). Every version written gets its `<n>.pending` marker. Add `usable_epoch` (the newest epoch of a confirmed version), and `lose(n, winner)`, which moves the pending file to `manifest/lost/<n>.json`, writes the winner as `<n>.json`, and re-applies the change as a pending `n+1`; change 4 calls it. Add `adopt(n, bytes)`, which writes a version copied from a transport as confirmed, with no marker; changes 4 and 5 call it. Unit-test the `Scope id`, `Manifest versions on disk`, `Versions that init writes`, `Versions that recover writes` and `Versions that revoke writes` scenarios, including that a revoked device's `device.key` and `owner.key` open nothing in the new version, the `Pending versions` scenarios (with the winning version given by the test), that `init` never re-adds this device, a version that already exists, and two writers racing for one lock. Verify with `cargo test --locked --bin bilbo manifest::`

## 4. The ceremony (`device-identity` spec, `cli` Output streams)

- [ ] 4.1 Add `Prompter::screen` to `src/wizard.rs`: `Terminal` enters the alternate screen on stderr and leaves it from a drop guard, and `Script` runs the work directly. Add the ceremony functions: show the numbered words and the owner fingerprint with `note`, confirm written down, ask 3 positions (words or prefixes) with a retry, show-again and cancel `select`; read 12 words with `check_word` and a whole-phrase retry on a failed checksum; and the fingerprint `confirm`, defaulting to no. Unit-test with `Script`: confirmed, a prefix, a wrong word then right, show again, cancel, an unknown word, a failed checksum, a declined fingerprint, and that no prompt text or warning holds a word of the phrase except the `note` that shows it. Verify with `cargo test --locked --bin bilbo wizard::`

## 5. `bilbo device` (`device-identity` spec, `cli` and `config` deltas)

- [ ] 5.1 Add `src/device.rs`, its `mod` line, dispatch arm and USAGE line, and update the USAGE copies in `tests/cli.rs` and `tests/recall.rs`. Implement show (exit 1 on a problem line, `manifest <n> pending` exiting 0), `list`, `init`, `recover` (unsealed scopes, the fingerprint check, finishing an interrupted recover on an enrolled device), and `revoke`, with `device::run(args, env, terminal, prompter)`. Include the terminal rule for `recover`, `revoke`, and `init`'s phrase and URL-change steps, with both agent markers; `keys::no_core_dump` before any phrase screen; every refusal before the phrase; the step report with `unsealed` (whose hint names `recover`, never `init`) and `-`, and the `file://` cloud-account line after `revoke`. Unit-test every `device-identity` scenario with `Script`, temporary roots and state folders, including `The phrase is kept nowhere` by scanning every written file for the words and the entropy hex. Verify with `cargo test --locked --bin bilbo device::`
- [ ] 5.2 Add the `#[ignore]` `device::tests::write_fixtures`, run it to write `tests/fixtures/device/` (key folders `rivendell` and `bagend` under the all-`abandon` owner with fixed device seeds, and a store whose `personal` scope, pinned to `file://`, has versions 1 and 2), and add `device::tests::fixtures_open`, which fails when the fixtures no longer verify and open. Add `tests/device.rs`. It copies the fixtures into temporary folders and sets modes, then covers through the built binary:
  - show with its exit codes;
  - `list`;
  - `init` on an enrolled device (rerun kept, a new scope, a URL change refused without a terminal);
  - the refusals of `init`, `recover` and `revoke` without a terminal, with `CLAUDECODE=1` and with `CODEX_THREAD_ID=x`;
  - `revoke` with a missing argument;
  - loose modes, a leftover `keys.new`;
  - a tampered, a reformatted, a misplaced and a foreign manifest, and two configs naming different folder paths for one `file://` scope;
  - the `cli` and `config` deltas' scenarios.

  Verify with `cargo test --locked --bin bilbo device::tests::fixtures_open && cargo test --locked --test device && cargo test --locked --test cli`
- [ ] 5.3 On macOS, with `expect`, in a scratch `XDG_STATE_HOME` and `BILBO_HOME`, record each run, without the phrase, in `openspec/changes/add-device-keys/smoke.md`:
  - run `bilbo device init > out.txt`, confirm the phrase, and check that `out.txt` holds only the step report and that the words and the fingerprint screen are gone from the scrollback;
  - rerun init;
  - change the scope's URL and run init in the terminal;
  - in a second state folder against the same store, run `bilbo device recover` with that phrase;
  - revoke the first device in the terminal;
  - run `recover` on a fresh store and decline the fingerprint.

  Verify with `test -s openspec/changes/add-device-keys/smoke.md && rg -q 'epoch 2' openspec/changes/add-device-keys/smoke.md && rg -q 'out.txt' openspec/changes/add-device-keys/smoke.md`
- [ ] 5.4 Update `AGENTS.md`: the dependency rule from design.md, `keys`, `phrase` and `manifest` in the library modules, and a gotcha that `tests/fixtures/device/` is regenerated with the ignored `write_fixtures` and that binary tests never feed the phrase through a hook. Update `README.md`:
  - `bilbo device` and its forms;
  - the ceremony, writing the fingerprint beside the phrase, and why agents are refused;
  - where keys live and their modes;
  - sync URL forms, with a `file://` path holding a space;
  - joining a second device by copying the store before `recover`;
  - what revocation guarantees and what it does not, in design.md's one sentence, removing a revoked device from a folder's cloud account, and the manual new-owner procedure;
  - that the terminal rule stops accidental misuse while the key files are the real boundary;
  - how to forget an identity.

  Verify with `rg -q 'src/keys.rs' AGENTS.md && rg -q 'bilbo device recover' README.md && rg -q 'keys/' README.md && rg -q -i 'revok' README.md`

## 6. Integration

- [ ] 6.1 Run the suite on Linux (CI's platform) in a memory-capped container, so `gethostname`, `prctl`, the modes and `renameat2` are exercised there. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked` inside the container
- [ ] 6.2 Run the full suite and the package check. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked && nix flake check -L`
