# Tasks

## 1. Failing tests

- [x] 1.1 In the tests of `src/sync/remote/mod.rs`, add the signal helper the design describes (a no-op `SIGUSR1` handler installed once through `libc::sigaction` without `SA_RESTART`, a worker thread signalled with `libc::pthread_kill` until it hands back its result, then released and joined) and `a_signal_does_not_fail_a_request`, which runs `reachable()` against a `Fake` that holds its answer about 200 ms and asserts `Ok(())` and at least one successful `pthread_kill`. Confirm it fails on the current ureq with `relay http://127.0.0.1:<port> unreachable: Interrupted system call (os error 4)`, on macOS and, where available, Linux: `nix develop -c cargo test --locked --bin bilbo sync::remote::tests::a_signal_does_not_fail_a_request`
- [x] 1.2 Add `a_signal_leaves_a_closed_connection_unreachable`, whose `Fake` sleeps about 200 ms and closes without answering, asserting the message starts with `relay <url> unreachable: ` and does not contain `Interrupted`; confirm it fails on the current ureq: `nix develop -c cargo test --locked --bin bilbo sync::remote::tests::a_signal_leaves_a_closed_connection_unreachable`

## 2. Pin the fix

- [x] 2.1 Change `ureq` in `Cargo.toml` to `{ git = "https://github.com/algesten/ureq", rev = "0ebb046e1cf269592f9edb3a55f489d508f47a1d", features = ["json"] }`, with a one-line comment beside it to move back to a crates.io release once one carries the fix, run `nix develop -c cargo update -p ureq`, and check that `Cargo.lock` moves only the `ureq` entry to the git source, with `ureq-proto` and the other entries unchanged: `git diff --no-ext-diff Cargo.lock`
- [x] 2.2 Confirm both tests from group 1 pass on the pinned ureq: `nix develop -c cargo test --locked --bin bilbo sync::remote::tests::a_signal`
- [x] 2.3 Add `cargoLock.outputHashes."ureq-3.4.2" = "sha256-BIj3Wb8r0WG4UPB8hSlHdjBp1u8JbLUHbDRdRNv79bc=";` to the package in `flake.nix`, the hash of the fetched source, and confirm the package builds: `nix build -L --no-link .#bilbo`
- [x] 2.4 Check that `openspec/config.yaml`'s Stack line still names `ureq` and needs no edit, and that `tests/layout.rs` needs none: `grep -n ureq openspec/config.yaml tests/layout.rs`

## 3. Verify

- [x] 3.1 Run the flake checks, which build the package and run its tests in the Nix sandbox: `nix flake check -L`
- [x] 3.2 Run the full verification: `nix develop -c cargo fmt --check && nix develop -c cargo clippy --locked --all-targets -- -D warnings && nix develop -c cargo test --locked`
