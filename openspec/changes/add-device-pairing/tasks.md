# Tasks

Run every command below from the repo root as `nix develop -c sh -c '<command>'`, with `CARGO_TARGET_DIR` set to the checkout's `target`. A new file under `src/` or `tests/` needs `git add -N` before any Nix command can see it. Changes 3 and 4 must be applied first. This change builds on `src/keys.rs`, `src/phrase.rs`, `src/manifest.rs`, `src/transport.rs` (with its `remove_mailbox`) and their test fixtures.

## 1. Library modules

- [ ] 1.1 Add `spake2 = { version = "0.5.0-pre.0", default-features = false }` to `Cargo.toml` and update `Cargo.lock`. Confirm that `cargo tree -d -e normal` lists no duplicate it did not list before, and that the Nix package still builds. Verify with `cargo build --locked && cargo tree -d -e normal && nix build --no-link .#default`
- [ ] 1.2 Add `src/pake.rs`, with no I/O:
  - `KeysRng`, the `rand_core` 0.10 adapter (`TryRng` with `Error = Infallible`, and `TryCryptoRng`) over `keys::random`;
  - parsing a code: case, spaces or hyphens, 4-letter prefixes through `src/phrase.rs`, a nameplate from 1 to 999, exactly three words, and an error naming the first unknown word;
  - the canonical code and a random code;
  - SPAKE2 on both sides;
  - the transcript hash, and the HKDF outputs `b`, `c` and `fingerprint`;
  - the `b.msg` and `c.msg` boxes, with B's signature over the transcript;
  - the twelve-digit fingerprint;
  - the three message formats, with their format number, the five `result` values and the 4 KiB limit.

  Unit-test:
  - the crate's own `test_asymmetric` vector (key `712295de…20c1`), through a fixed-bytes `TryRng` that yields each scalar's 32 little-endian bytes and then zeros;
  - bilbo's golden key from two fixed seeds;
  - that both sides agree;
  - that one wrong word makes `b.msg`'s box fail to open;
  - that a changed SPAKE2 message fails to open either box;
  - an unknown format number;
  - the fingerprint's shape;
  - that a `c.msg` for 12 scopes fits in 4 KiB.

  Verify with `cargo test --locked --bin bilbo pake::`
- [ ] 1.3 Move `write_config` from `src/setup.rs` to `src/config.rs` unchanged, and add `config::set_keys`, which replaces a key's line in place or appends it, keeping every other line and comment as written, the `.bak` included. Unit-test a replacement, an append, comments and blank lines kept, a file that does not exist yet, and a file that cannot be written. Verify with `cargo test --locked --bin bilbo config:: && cargo test --locked --bin bilbo setup::`

## 2. `bilbo pair` (`device-pairing` spec)

- [ ] 2.1 Add `src/pair.rs` with `pair::run(args, env, terminal, answer, limits, out, err)`, its `mod` line, dispatch arm and both USAGE lines, and update the USAGE copies in `tests/cli.rs` and `tests/recall.rs`. `main` passes the terminal test, the locked stdin, `Limits::default()` and a line printer for each stream.
  - **A's checks, in this order, all before any write:** usage and config errors; no owner key; no syncing scope; an unknown or non-syncing `--scope`; more than 12 scopes; scopes on URLs that differ as written; a `--via` that no paired scope uses; an `https://` URL; the terminal, `CLAUDECODE` and `CODEX_THREAD_ID` rule.
  - **Showing a code:**
    - the sweep, then claiming a nameplate (20 tries, then `no free pairing number`);
    - the code and the instructions on stderr;
    - waiting for `b.msg` within the window;
    - `wrong-code`;
    - the owner check on an enrolled B (`other-owner`), and the name check against A's own name and every local manifest of the owner;
    - the question as a blocking read, then the window check;
    - adding B through `src/manifest.rs` with the epoch key A opens through its own `sealed` entry, on the latest local version, pending or not, or skipping a scope whose newest version lists B's id;
    - leaving the owner seed out of the payload for an enrolled B;
    - `c.msg` with each result;
    - removing an unanswered mailbox at expiry;
    - the `paired` line on stdout.

  Unit-test each refusal, the scope and `--via` rules with URLs compared as written, the owner check, and the name check. Verify with `cargo test --locked --bin bilbo pair::`
- [ ] 2.2 In `src/pair.rs`, add the joining side (B):
  - its refusals before the mailbox (no store, a bad code or `--via`, a missing folder, an `https://` URL);
  - for a B without keys, the pending device key in `<state>/bilbo/pair/` (0700 and 0600, `create_new`, reused); for an enrolled B, its device key from `keys/` and its owner key in `b.msg`'s box;
  - waiting for `a.msg` within the appear limit;
  - the format check;
  - creating `b.msg`, saying `already used` when it exists;
  - the fingerprint with B's name and id;
  - waiting for `c.msg`;
  - each `result`;
  - fetching versions 1 to n within the manifest limit, and checking the hash, the `prev` chain, each `owner` against the received signing seed, the signatures, B's listing and the sealed key;
  - the same-owner check against local manifests;
  - writing the manifests, then the config (the `sync` URL mapping and the `embedder` rule), then, for a B without keys, the keys under `keys.lock` through change 3's `keys.new/` folder (a leftover removed first) and `rename_new`, then removing `<state>/bilbo/pair/`;
  - the `<n> notes already carry scope` line for each paired scope that local notes already name;
  - removing the mailbox after every result but `wrong-code`;
  - the two stdout lines.

  Unit-test the URL mapping, the `embedder` rule, a broken chain, an `owner` that does not match the seed, a leftover `keys.new/`, and that a failed check or an unwritable config leaves `keys/` empty. Verify with `cargo test --locked --bin bilbo pair::`
- [ ] 2.3 In `src/pair.rs`'s unit tests, run the whole exchange in one process.
  - **Setup:**
    - A and B run on two threads with two `Env`s in temporary folders.
    - A's keys and manifests come from change 3's fixtures in `tests/fixtures/device/`.
    - A gets `terminal = true` and a scripted answer, and both get millisecond `Limits`.
    - Each side has its own transport folder (`/…/a/sync` and `/…/b/sync`), with a copier thread that moves new files across after a configurable delay. The copier also deletes on the far side what one side removes.
  - **Cover every `device-pairing` scenario that needs both sides:**
    - joining, a loosely typed code, the fingerprints, declined, end of input;
    - a wrong word, the retry, a second answer;
    - each expiry, a late manifest and one that never arrives;
    - after pairing, a manifest that does not verify, and pairing again after an interrupted pairing;
    - the config scenarios, both phrase scenarios (the opaque mailbox through the copier's view of every file);
    - narrowing, the name checks, a store of another owner, a newer format;
    - an enrolled device joining another scope, and one enrolled with another owner;
    - notes that already carry the scope;
    - the folder with another path, checking that no manifest version is written for it;
    - the mailbox cleanups, including a stale and a fresh mailbox with back-dated modification times.

  Verify with `cargo test --locked --bin bilbo pair::`
- [ ] 2.4 Cover in `tests/pair.rs`, through the built binary with a clean environment, what needs no terminal and no waiting:
  - A's refusals, including `An agent shows a code` with stdin not a terminal, with `CLAUDECODE=1` and with `CODEX_THREAD_ID` set (tests run without a terminal, so this is what every A run reaches once the earlier checks pass);
  - B's refusals before the mailbox;
  - the usage errors;
  - the `cli` delta's `Pair is a verb`, and the `config` delta's `Pair reads the config`.

  Verify with `cargo test --locked --test pair && cargo test --locked --test cli`
- [ ] 2.5 Smoke test on two machines with real terminals, recorded in `openspec/changes/add-device-pairing/smoke.md`:
  - pair bagend from rivendell through a folder a cloud service syncs, with a different path on each machine;
  - check the fingerprints, names and ids by eye;
  - decline once, then use a wrong word once and retry the right code;
  - after pairing, check that `bilbo device list` on both machines shows both devices, and that `bilbo device` on bagend reports the scope valid;
  - then pair bagend into a second scope while it is enrolled, and check that its keys did not change.

  Verify with `rg -q 'paired with' openspec/changes/add-device-pairing/smoke.md`

## 3. Docs

- [ ] 3.1 Update `AGENTS.md`:
  - `spake2` lives in `src/pake.rs`;
  - `rand_core` is used directly there, through `spake2::rand_core`;
  - so the dependency rule reads `cliclack` in `src/wizard.rs`; `libc` in `src/wizard.rs`, `src/swap.rs` and `src/keys.rs`; `ring` in `src/model.rs`; `notify` in `src/watch.rs`; `sha2` in `src/versions.rs` and `src/keys.rs`; `ed25519-dalek`, `hpke`, `chacha20poly1305`, `hkdf` and `getrandom` in `src/keys.rs`; `base64` in `src/segment.rs`; `spake2` and `rand_core` (through `spake2::rand_core`) in `src/pake.rs`;
  - `pake` joins the library modules;
  - `write_config` now lives in `src/config.rs`;
  - the pairing exchange is tested in one process through `pair::run`'s injected terminal flag, answer and limits, never through a hook.

  Verify with `rg -q 'src/pake.rs' AGENTS.md && rg -q 'rand_core' AGENTS.md`
- [ ] 3.2 Update `README.md` with a pairing section:
  - both commands, with an example code;
  - comparing the fingerprint, name and id;
  - that `--via` is the folder's path on the new device;
  - one attempt per code and the 10 minutes;
  - why it needs a terminal, and that the terminal rule stops accidental misuse while the key files' modes are the last line;
  - pairing an enrolled device into another scope;
  - the phrase as the fallback;
  - that `--scope` picks which scopes a device can read, and change 3's sentence on what a revoked or stolen device can and cannot do;
  - that whoever can write a shared folder can stop a pairing but not read it.

  Verify with `rg -q 'bilbo pair' README.md && rg -q -- '--via' README.md`

## 4. Integration

- [ ] 4.1 Run the suite on Linux (CI's platform) in a memory-capped container. Pairing exercises there:
  - the modes of the pending key and the keys folder under a Linux umask;
  - the default device name from `gethostname`;
  - `renameat2` for the keys folder move;
  - the sweep's back-dated modification times.

  Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked` inside the container
- [ ] 4.2 Run the full suite and the package check. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked && nix flake check -L`
