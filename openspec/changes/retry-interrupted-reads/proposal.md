# Proposal

## Why

A signal that reaches bilbo while it waits for a relay's answer fails the request with `relay <url> unreachable: Interrupted system call (os error 4)`. The relay was up and answering; the user sees a false outage, a sync step that did nothing, or a `bilbo setup` that stops at the relay check. It was seen once, in the project's own unit tests against a loopback relay: a signal reached the process, and what sent it is unknown. Any relay reached over plain HTTP, which bilbo allows only on the same machine, is exposed the same way, and the fix covers a signal from any sender.

## What Changes

- A signal during a relay request no longer fails it: the transport keeps waiting for the answer within the request's time limit, and the request succeeds or fails on what the relay does.
- A relay that is really down, closes the connection or never answers is still reported as `relay <url> unreachable: <reason>`, within the limits that apply today.
- bilbo's HTTP client moves from its latest release to the upstream commit that retries a read interrupted by a signal, pinned by revision until a release carries that fix.

## Non-goals

- No retry loop of bilbo's own around relay requests, and no change to which requests are retried today (the clock-skew retry and the lost-answer retry stay as they are).
- No change to the 10-second connection limit or the 300-second request limit.
- No change to the relay server, which already retries interrupted reads and writes.
- No change to how a signal stops bilbo: a signal that ends the process still ends it.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `relay-transport` (`openspec/specs/relay-transport/spec.md`): adds a requirement that a signal received during a request does not fail it or report the relay unreachable, with scenarios for a relay that answers late and for relays that never answer or close the connection.

## Impact

- Dependencies: `ureq` becomes a git dependency pinned by revision in `Cargo.toml`; `Cargo.lock` records the git source; `flake.nix` gains a `cargoLock.outputHashes` entry so the Nix build can fetch it. No new crate.
- Code: two unit tests in `src/sync/remote/mod.rs` that interrupt a real relay read with signals. No change to the client code itself.
- Users: none visible beyond the false `unreachable` going away.
