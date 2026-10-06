# Design

## Context

See proposal.md for why. The relay client in `src/sync/remote/mod.rs` sends every request through one `ureq::Agent` built by `agent()`, with `timeout_connect` (10 s) and `timeout_global` (300 s) always set. `exchange()` turns any send or body error into a `Fault` through `fault()`, which maps `ureq::Error::Io(e)` to `Fault::Unreachable(e.to_string())`. So an `io::ErrorKind::Interrupted` from a socket read reaches the user as `relay <url> unreachable: Interrupted system call (os error 4)`.

Where the interrupted call comes from, checked against the sources in use (ureq 3.4.2, rustls 0.23.45, std):

- Connect is not exposed: std's `TcpStream::connect_timeout` retries `EINTR` from its `poll`.
- Writes are not exposed: ureq's `TcpTransport::transmit_output` uses `write_all`, which retries `Interrupted`.
- Reads are exposed: `TcpTransport::await_input` sets the read timeout (`SO_RCVTIMEO`) and calls `read` once, returning `Interrupted` as an error. On Linux a read on a socket with a receive timeout is never restarted after a signal handler runs, even with `SA_RESTART`, and a stop and continue can interrupt it with no handler at all. On macOS a handler installed without `SA_RESTART` interrupts it.
- Over `https://`, rustls sits on top of that read and its `complete_io` loops on `Interrupted`, so a TLS relay already rides it out. The exposed requests are the plain-HTTP ones, which the transport allows only to a loopback host: the unit and binary tests' fake relays, and a relay reached through `http://localhost`.

The relay server side already retries `Interrupted` in `src/relay/http.rs` and `src/relay/store.rs`.

Upstream ureq has the fix on its main branch, after 3.4.2 and in no release yet: `TcpTransport::await_input` retries a read interrupted by a signal whenever a timeout applies, with whatever is left of that timeout, and returns a timeout once the budget is spent. A read without a timeout still returns `Interrupted`. bilbo always sets a timeout, so every relay read gets the retry.

## Goals / Non-Goals

**Goals:**
- An interrupted relay read is retried inside the client, within the same time limit, as the `Signals during a request` requirement says.
- A test that reproduces the failure on the current dependency and passes on the fixed one, on Linux and macOS.

**Non-Goals:**
- Retrying in bilbo's own code, or covering the `file://` transport, which has no socket reads.
- Any change to `agent()`, `exchange()` or `fault()`.

## Decisions

- **Pin ureq to the upstream fix as a git dependency.** `Cargo.toml` takes `ureq` from its upstream repository at the upstream ureq commit `0ebb046e1cf269592f9edb3a55f489d508f47a1d`, with the same `json` feature. The full revision is the pin, and `Cargo.lock` records it with the git source. This adds no crate: it changes where an existing one comes from. Alternatives:
  - *Wait for a ureq release.* Leaves the false `unreachable` in place, and the test suite flaky, for an unknown time. bilbo is `publish = false`, so a git dependency costs no crates.io publish.
  - *Retry in bilbo.* A loop in `exchange()` on `Interrupted` would resend the whole request, not resume the read. A resent signed request needs a fresh nonce and signature, since the relay refuses a replayed nonce, and a resent PUT whose first copy landed takes the lost-answer path. It would duplicate, at a coarser grain, what ureq now does correctly at the read, and would have to be removed again once the release ships.
  - *Install signal handlers with `SA_RESTART`, or block signals on request threads.* Does not help on Linux, where a read with a receive timeout is not restarted either way, and bilbo does not own every handler in the process.
- **What the pin carries besides the fix.** The revision is four commits past 3.4.2. The others are a raised minimum rustls (0.23.45, the version `Cargo.lock` already has), new opt-in `timeout_per_read` and `timeout_per_write` settings that default to off, and a native-tls feature gating fix that bilbo, on the default rustls features, does not build. ureq's `Cargo.toml` at that revision still asks for `ureq-proto` 0.6.3, which the locked 0.6.4 satisfies, so no other entry in `Cargo.lock` should move. The crate still reports version 3.4.2.
- **The Nix build fetches the git crate through `outputHashes`.** `flake.nix` builds with `cargoLock.lockFile = ./Cargo.lock`, and `importCargoLock` refuses a git source without a hash. The package gains `cargoLock.outputHashes."ureq-3.4.2" = "sha256-...";`, filled from the hash Nix reports for `lib.fakeHash`. The ureq repository is a single crate, so one entry covers it. `allowBuiltinFetchGit = true` would skip the hash, but it fetches impurely at evaluation time; a fixed-output hash keeps the build reproducible. No file joins the `lib.fileset`.
- **Layout check unchanged.** `tests/layout.rs` places crates by the name `src/` uses, not by their source, and `ureq` has no `PLACEMENT` entry. Its scan stops at `#[cfg(test)] mod tests`, so the test's `libc::` calls in `src/sync/remote/mod.rs` stay outside `libc`'s allowed files without breaking the check. `openspec/config.yaml` names `ureq` in its Stack line, still true.
- **The test interrupts a real relay read with signals.** It extends the existing tests in `src/sync/remote/mod.rs` and reuses their `Fake` and `client()`; it needs no new module and no new crate (`libc` is already a dependency).
  - The fake holds its answer about 200 ms by sleeping in its answer closure, so the client is blocked in a socket read.
  - A no-op handler for `SIGUSR1` is installed once, through `libc::sigaction` without `SA_RESTART`, so the read is interrupted on both Linux and macOS. A probe of a timed loopback socket read on macOS returned `EINTR` with that handler and was restarted with `SA_RESTART`; Linux returns `EINTR` either way. The handler stays installed for the rest of the test process: restoring it would race with a concurrent test, and nothing else in the crate uses `SIGUSR1`.
  - A worker thread calls `reachable()` on a client of the fake. The test takes the worker's `pthread_t` from `JoinHandleExt::as_pthread_t` and sends it `SIGUSR1` with `libc::pthread_kill` every few milliseconds until the worker hands back its result over a channel. The worker then waits for a second message before it returns, so no signal can reach a thread that has exited; the test stops signalling, sends that message and joins.
  - `a_signal_does_not_fail_a_request` asserts `Ok(())` and that at least one signal was delivered. On ureq 3.4.2 it fails with `relay http://127.0.0.1:<port> unreachable: Interrupted system call (os error 4)`.
  - `a_signal_leaves_a_closed_connection_unreachable` uses a fake whose closure sleeps and returns an empty answer, so the connection closes unanswered. It asserts the message starts with `relay <url> unreachable: ` and does not contain `Interrupted`, which also fails on 3.4.2.
  - The never-answers scenario is not tested in bilbo: it needs the 300-second limit to pass. The upstream change carries unit tests for a retry that runs past its budget and returns a timeout, which `fault()` already maps to `timed out`.

## Risks / Trade-offs

- [A git dependency breaks the build if the revision disappears upstream] → the revision is a merged commit on ureq's main branch, and `Cargo.lock` and `outputHashes` pin its content, so a rewritten history fails loudly instead of building other code.
- [Unreleased code ships in a bilbo release] → the pin is four reviewed, merged commits past 3.4.2 and the full suite runs against it; the follow-up below moves back to crates.io.
- [The test depends on scheduling] → the worker is signalled repeatedly for the whole 200 ms hold. The test's count of delivered signals counts successful `pthread_kill` calls, not interrupted reads: a run where nothing was sent fails, so it guards the test's setup, but a run where every signal missed the read cannot be told apart from a pass. On the pinned ureq this cannot make the test flaky, since it is green either way; it weakens only the proof that the test is red on 3.4.2, so that proof is taken by running it there, now and again when the pin moves back to crates.io.
- [A git dependency needs the network] → a fresh `CARGO_HOME` must fetch the ureq repository before the first build or test, where a registry crate already in the local cache would need nothing; the same holds for the Nix build's fixed-output fetch. CI and the release jobs run with network, so only offline development on a clean machine pays it.
- [A signal handler in the unit-test process] → only `SIGUSR1`, only to the worker thread, and a no-op.

## Migration Plan

No migration: no stored data or config changes. Rollback is reverting `Cargo.toml`, `Cargo.lock` and `flake.nix`, which brings the false `unreachable` back.

Follow-up, not part of this change: once ureq publishes a release that contains the fix, set `ureq` back to that crates.io version, run `cargo update -p ureq`, and drop the `outputHashes` entry. The reminder lives as a one-line comment beside `ureq` in `Cargo.toml`, where the next person to touch the dependency sees it.
