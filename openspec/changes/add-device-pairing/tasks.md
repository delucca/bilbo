# Tasks

Run every command below from the repo root as `nix develop -c sh -c '<command>'`, with `CARGO_TARGET_DIR` set to the checkout's `target`. A new file under `src/` or `tests/` needs `git add -N` before any Nix command can see it. This change builds on `add-device-keys` and `add-sync`, both archived: `src/identity/keys.rs`, `src/identity/phrase.rs`, `src/identity/manifest.rs`, `src/sync/transport.rs` (with its `remove_mailbox`), `src/sync/scopes.rs`, and their test fixtures in `tests/fixtures/device/`.

## 1. Skeleton and library modules

- [x] 1.1 Add `spake2 = { version = "0.5.0-pre.0", default-features = false }` to `Cargo.toml` and update `Cargo.lock`. Confirm that `cargo tree -d -e normal` lists no duplicate it did not list before (`getrandom` 0.2 and 0.4, `syn` 2 and 3), and that the Nix package still builds. Verify with `cargo build --locked && cargo tree -d -e normal && nix build --no-link .#default`
- [x] 1.2 Add the verb's skeleton, so the parts below land in files of their own:
  - `src/identity/pake.rs` with `KeysRng`, the `rand_core` 0.10 adapter (`TryRng` with `Error = Infallible`, and `TryCryptoRng`) over `keys::random`, and the crate's own `test_asymmetric` vector (key `712295de…20c1`), through a fixed-bytes `TryRng` that yields each scalar's 32 little-endian bytes and then zeros;
  - the folder `src/identity/pair/`: `mod.rs` with `run(args, env, terminal, answer, limits, out, err)` and `Limits` (window 10 minutes, appear and manifest waits 2 minutes each, polls 250 ms on `file://` and 2 s on `https://`, sweep age 30 minutes), `show.rs`, `join.rs`, and the test-only `exchange.rs`;
  - `pub mod pake;` and `pub mod pair;` in `src/identity/mod.rs`;
  - the dispatch arm in `src/main.rs`, which passes `stdin().is_terminal() && stderr().is_terminal()`, the locked stdin, `Limits::default()` and a line printer for each stream, and the USAGE lines `bilbo pair [--scope <name>]... [--via <url>]` and `bilbo pair <code> --via <url> [--name <name>]` with one description line, copied into `tests/cli.rs` and `tests/recall.rs`;
  - in `tests/layout.rs`, `identity/pair/` in `VERBS`, `spake2` and `rand_core` in `identity/pake.rs` in `PLACEMENT`, `identity/pake.rs` among `base64`'s files (the boxes' encoding), and `identity/pair/exchange.rs` among the test-only files that may use a glob import.

  Verify with `cargo test --locked --bin bilbo identity::pake:: && cargo test --locked --test layout --test cli --test recall`
- [x] 1.3 Fill `src/identity/pake.rs`, with no I/O:
  - parsing a code: case, spaces or hyphens, a nameplate from 1 to 999, exactly three words, each a list word or its first four or more letters naming one word (a new `phrase::complete` in `src/identity/phrase.rs`), and an error naming the first unknown word;
  - the canonical code, the SPAKE2 password `bilbo-pair-1:<code>`, and a random code from `keys::random`;
  - SPAKE2 on both sides, with the identities `bilbo-pair-1 a` and `bilbo-pair-1 b`;
  - the transcript hash T over `bilbo-pair-1`, the nameplate and both SPAKE2 messages (`shared::hash::sha256`), and the HKDF-SHA256 outputs `b`, `c` and `fingerprint` with salt T (a new `keys::hkdf` in `src/identity/keys.rs`);
  - the `b.msg` and `c.msg` boxes (`keys::encrypt` and `decrypt`; the associated data is `bilbo-pair-1 <role>`, a newline, then T, with the role `b` or `c <result>`, so the plain `result` that picks the reader is under the AEAD), with B's signature over T (`SignKey::sign`, `keys::verify`);
  - the twelve-digit fingerprint;
  - the three message formats, with their format number, the six `result` values and the 4 KiB limit, the boxes and nonces in base64; a `plain_reply` for the box-less results; and the reader's checks on the payload: `keys::is_id` (new, beside `device_id`) on A's id and each scope id, `n` of 1 or more, scope names by the store's rule, `embedder` of `any` or `local`, a URL only as `https://` without whitespace, and distinct ids and names, each refused as malformed.

  Unit-test:
  - bilbo's golden key from two fixed seeds;
  - that both sides agree;
  - that one wrong word makes `b.msg`'s box fail to open;
  - that a changed SPAKE2 message fails to open either box;
  - that a `b.msg` whose signature is not over T, or not by the key it names, is refused;
  - an unknown format number;
  - the fingerprint's shape;
  - that a `c.msg` for 12 scopes fits in 4 KiB;
  - `phrase::complete` on a full word, four letters, five letters, a prefix of no word and a word that is not in the list;
  - `keys::hkdf` against RFC 5869's first SHA-256 vector.

  Verify with `cargo test --locked --bin bilbo identity::`
- [x] 1.4 Move `write_config` from `src/setup/apply.rs` to `src/shared/config.rs` unchanged, with its test, and add `config::set_keys`, which replaces a key's line in place or appends it, writes each value through `config::quote`, and keeps every other line and comment as written, the `.bak` included. Unit-test a replacement, an append, comments and blank lines kept, a file that does not exist yet, and a file that cannot be written. Verify with `cargo test --locked --bin bilbo shared::config:: && cargo test --locked --bin bilbo setup::`
- [x] 1.5 Add `sweep_mailboxes(now, age)` to the `Transport` trait in `src/sync/transport.rs`: on `file://` it removes each `pair/<nameplate>/` whose `a.msg` was last modified more than `age` before `now`, and nothing under `scopes/`. Unit-test a stale and a fresh mailbox with back-dated modification times, a mailbox with no `a.msg`, and a name outside the layout. Verify with `cargo test --locked --bin bilbo sync::transport::`

## 2. `bilbo pair` (`device-pairing` spec)

- [x] 2.1 In `src/identity/pair/mod.rs`, read both forms and run every check that comes before the mailbox:
  - **Usage:** a first argument that does not start with `-` is a code; `--scope` (repeatable) and `--via` for A; `--via` (required) and `--name` for B; any other argument, or a missing value, is a usage error.
  - **A's checks, in this order, all before any write:** usage and config errors; no owner key; no syncing scope; an unknown or non-syncing `--scope`; more than 12 scopes; scopes on URLs that differ as written; a `--via` that no paired scope uses; an `https://` URL; the terminal, `CLAUDECODE` and `CODEX_THREAD_ID` rule.
  - **B's checks before the mailbox:** usage and config errors; a code that is not a number and three list words; a remote `http://` URL; an `https://` URL; no `<root>/notes/`; a `--via` folder that does not exist.
  - the polling helper both sides use, on the transport's interval and its own monotonic clock.

  Unit-test each refusal and the scope and `--via` rules with URLs compared as written. Verify with `cargo test --locked --bin bilbo identity::pair::`
- [x] 2.2 In `src/identity/pair/show.rs`, add the showing side (A):
  - the sweep, then claiming a nameplate (20 tries, then `no free pairing number`);
  - the code and the instructions on stderr;
  - waiting for `b.msg` within the window;
  - `wrong-code`, and a `b.msg` of a newer format;
  - the owner check on an enrolled B (`other-owner`), and the name check against A's own name and every device the owner's local manifests list under another id;
  - the question as a blocking read, then the window check;
  - adding B through `manifest::add_device` with the epoch key A opens through its own `sealed` entry, on the latest local version, pending or not, or skipping a scope whose newest version lists B's id; creating each new version on the transport, create-only;
  - leaving the owner seed out of the payload for an enrolled B;
  - `c.msg` with each result;
  - removing an unanswered mailbox at expiry;
  - the `paired` line on stdout.

  Unit-test each result against a B scripted through `pake`, the owner check and the name check. Verify with `cargo test --locked --bin bilbo identity::pair::`
- [x] 2.3 In `src/identity/pair/join.rs`, add the joining side (B):
  - for a B without keys, the pending device key in `<state>/bilbo/pair/device.key` (folder 0700, file 0600, `create_new`, reused by the next run), through new functions in `src/identity/keys.rs`; for an enrolled B, its device key from `keys/` and its owner key in `b.msg`'s box;
  - the sweep, then waiting for `a.msg` within the appear limit;
  - the format check;
  - creating `b.msg`, saying `already used` when it exists;
  - the fingerprint with B's name and id;
  - waiting for `c.msg`;
  - each `result`;
  - fetching versions 1 to n through `sync::scopes::chain` within the manifest limit, and checking the hash, the `prev` chain, each `owner` against the received signing seed, the signatures, B's listing and the sealed key;
  - the same-owner check against local manifests;
  - writing the manifests through `manifest::adopt` under `manifest::lock`, then the config through `config::set_keys` (the `sync` URL mapping and the `embedder` rule), then, for a B without keys, the keys through `keys::write_identity` (owner seed from the payload, `owner_box` from the manifests), then removing `<state>/bilbo/pair/`;
  - the `<n> notes already carry scope` line for each paired scope that local notes already name;
  - removing the mailbox after every result but `wrong-code`;
  - the two stdout lines.

  Unit-test the URL mapping, the `embedder` rule, a broken chain, an `owner` that does not match the seed, a leftover `keys.new/`, and that a failed check or an unwritable config leaves `keys/` empty, each against an A scripted through `pake`. Verify with `cargo test --locked --bin bilbo identity::pair:: && cargo test --locked --bin bilbo identity::keys::`
- [x] 2.4 In `src/identity/pair/exchange.rs`, run the whole exchange in one process.
  - **Setup:**
    - A and B run on two threads with two `Env`s in temporary folders.
    - A's keys and manifests come from `add-device-keys`' fixtures in `tests/fixtures/device/`; a scope that does not list B yet is built in the test with `manifest::create` over A's identity.
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
  - **And the `note-sync` delta's Paired into the scope afterwards** up to what needs no watcher: B's keys unchanged and the latest version listing B.

  Verify with `cargo test --locked --bin bilbo identity::pair::`
- [x] 2.5 Cover in `tests/pair.rs`, through the built binary with a clean environment, what needs no terminal and no waiting:
  - A's refusals, including `An agent shows a code` with stdin not a terminal, with `CLAUDECODE=1` and with `CODEX_THREAD_ID` set (tests run without a terminal, so this is what every A run reaches once the earlier checks pass);
  - B's refusals before the mailbox;
  - the usage errors;
  - the `cli` delta's `Pair is a verb`, and the `config` delta's `Pair reads the config`.

  Verify with `cargo test --locked --test pair && cargo test --locked --test cli`
- [x] 2.6 Name `bilbo pair` in the advice to a device outside a scope:
  - the not-in-the-scope line of `src/sync/manifests.rs` (`note-sync` delta), which now names the scope's URL;
  - `recover`'s `unsealed` hint in `src/identity/device.rs` (`device-identity` delta), leaving `init`'s hint as the `scope-manifest` spec words it.

  Update their unit tests and `tests/sync.rs`. Verify with `cargo test --locked --bin bilbo sync::manifests:: && cargo test --locked --bin bilbo identity::device:: && cargo test --locked --test sync`
- [ ] 2.7 Smoke test on two machines with real terminals, recorded in `openspec/changes/add-device-pairing/smoke.md`:
  - pair bagend from rivendell through a folder a cloud service syncs, with a different path on each machine;
  - check the fingerprints, names and ids by eye;
  - decline once, then use a wrong word once and retry the right code;
  - after pairing, check that `bilbo device list` on both machines shows both devices, that `bilbo device` on bagend reports the scope valid, and that bagend's `bilbo watch` syncs the scope within one cycle;
  - then pair bagend into a second scope while it is enrolled, and check that its keys did not change.

  Verify with `rg -q 'paired with' openspec/changes/add-device-pairing/smoke.md`

## 3. Docs

- [x] 3.1 Update `AGENTS.md`:
  - `identity/pair/` joins the verb list, and the sentence on verbs that call into `sync` from `note` and `identity`;
  - `pake` joins `identity`'s library modules that never use a domain that uses `identity`;
  - `pair`, like `setup` and `watch`, takes callbacks for its lines, and `main` prints each one;
  - a gotcha: the pairing exchange is tested in one process through `pair::run`'s injected terminal flag, answer and limits, never through a hook, and `tests/pair.rs` reaches only what needs no terminal.

  Verify with `rg -q 'identity/pair/' AGENTS.md && rg -q 'pake' AGENTS.md`
- [x] 3.2 Update `README.md` with a pairing section:
  - both commands, with an example code;
  - comparing the fingerprint, name and id;
  - that `--via` is the folder's path on the new device;
  - one attempt per code and the 10 minutes;
  - why it needs a terminal, and that the terminal rule stops accidental misuse while the key files' modes are the last line;
  - pairing an enrolled device into another scope;
  - the phrase as the fallback;
  - that `--scope` picks which scopes a device can read, and `add-device-keys`' sentence on what a revoked or stolen device can and cannot do;
  - that whoever can write a shared folder can stop a pairing but not read it.

  Verify with `rg -q 'bilbo pair' README.md && rg -q -- '--via' README.md`

## 4. Integration

- [x] 4.1 Run the suite on Linux (CI's platform) in OrbStack's Docker, memory-capped, as `add-sync` did: the `nixos/nix` image with the checkout mounted, flakes enabled, and `CARGO_TARGET_DIR` inside the container. Pairing exercises there:
  - the modes of the pending key and the keys folder under a Linux umask;
  - the default device name from `gethostname`;
  - `renameat2` for the keys folder move;
  - the sweep's back-dated modification times.

  Verify with `docker run --rm -m 8g -v "$PWD":/src -w /src nixos/nix sh -c 'nix --extra-experimental-features "nix-command flakes" develop -c sh -c "export CARGO_TARGET_DIR=/tmp/target && cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked --no-fail-fast"'`
- [ ] 4.2 Run the full suite and the package check. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked && nix flake check -L`
