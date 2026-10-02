# Design

## Context

The repo holds only the OpenSpec setup. There is no code, and `openspec/config.yaml` reads `Stack: <to decide>`. The legacy system lives in the maintainer's personal setup: an indexer in stdlib Python with a hand-rolled frontmatter reader, and a stdlib ULID-minting script. Both serve as reference for behavior, not as code to port. The proposal records the decisions on the store, the layout and the format.

The maintainer's machines get their tools from Nix and have no Rust toolchain installed. nixpkgs provides `cargo` and `rustc` 1.95.0.

## Goals / Non-Goals

**Goals:**
- A `bilbo` binary that later verbs (search, digest, ingest) extend instead of replacing.
- One strict reader of the note format, shared by `new` (which renders and re-reads) and `check` (which validates).
- No partial or clobbered note files, even with several agents working at once.

**Non-Goals:**
- A general YAML reader. The format is a fixed subset, and the reader accepts only that subset.
- Caching, indexing or watching the store. `check` reads every file on each run, which is fine at about 400 notes.

## Decisions

### Rust, one crate, one binary

`Cargo.toml` at the repo root, package and binary named `bilbo`, `edition = "2024"`, `rust-version = "1.95"`.

Why Rust over Python, the language of the current code: bilbo is meant to be installed by people other than its author, and it will run a daemon and a hook on every prompt. One static binary installs without Python or `uv` on the machine. Python's head start is smaller than it looks: with no backwards compatibility, scopes dropped and the frontmatter changed, the legacy indexer gets reworked whether ported or rewritten.

Why Rust over Go: the later index needs SQLite with FTS5 and sqlite-vec. rusqlite can build SQLite with FTS5 included, and sqlite-vec ships a Rust crate. Go only reaches sqlite-vec through cgo.

Hook latency is not the reason. In the legacy setup, the measured 1.54 s miss came from the embedder call, not from interpreter startup.

### Modules

This is the first code, so there is no existing module to extend. Each module owns one contract, so later changes extend a module instead of adding a parallel one:

- `src/main.rs`: verb dispatch, usage and help, exit codes, the `bilbo: ` stderr prefix (the `cli` spec).
- `src/store.rs`: resolving the root and listing the entries in `notes/` (the store root and notes layout requirements).
- `src/note.rs`: the note name (kind and topic), ULID minting and validation, the `created` form, reading the frontmatter and title into a note plus a list of problems, and rendering a new note.
- `src/new.rs` and `src/check.rs`: the two verbs, built on `store` and `note`.

`note.rs` returns every problem it finds, never only the first. `check` needs all of them, and `new` uses the same reader to re-validate what it renders.

### Dependencies: `jiff` only

| Need | Choice | Why not the alternative |
|---|---|---|
| Local time with the machine's UTC offset, for `created` | `jiff` 0.2.37 (`Zoned::now()`, `strftime("%Y-%m-%dT%H:%M%:z")`) | The standard library has no time zones. Calling `localtime_r` through `libc` needs `unsafe`, platform-specific code and a second dependency anyway. `chrono` works too, but jiff is maintained by the author of `regex` and has the more correct tz handling. |
| Calendar validation of `created` in `check` | `jiff` `strtime::parse` with the same format, then `.to_datetime()`, which rejects `2026-02-30`. A byte-shape check runs first, because `parse` accepts forms the contract forbids (seconds, `-00:00`, single-digit fields). | Comes free with the dependency `new` already needs. A hand-written leap-year table would duplicate it. |
| ULID | Written by hand: 48-bit milliseconds since the epoch plus 80 random bits from `/dev/urandom`, Crockford base32. About 20 lines, as the legacy minting script shows. | The `ulid` crate pulls in `rand`. `getrandom` only matters on Windows, which is a non-goal. |
| Arguments | Parsed by hand over `std::env::args` | Two verbs and one option. `clap` adds about a dozen transitive crates. Revisit when search adds filters. |
| Frontmatter | A strict line reader | `serde_yaml` is archived and its forks are young. A YAML reader would accept forms the contract forbids (flow lists, unquoted or single-quoted items, `sources: []`), so `check` would need a second validation layer on top of it. The strict reader is the validator. |
| Test temp folders | A helper over `std::env::temp_dir()` with a per-process, per-test unique name, removed on drop | The `tempfile` crate is a dev-dependency for about 15 lines of code. |

`Cargo.toml` uses the caret idiom (`jiff = "0.2.37"`), and `Cargo.lock` is committed. The lockfile is the pin.

### Atomic, no-clobber create

`new` writes the rendered note to `notes/.new-<id>.tmp`, calls `fsync`, lists `notes/` again for the topic, then `std::fs::hard_link`s it to `notes/<kind>-<topic>.md`. The temp file is removed on every path after its creation. `link` fails with `EEXIST` when the target exists, so of two racing runs exactly one wins. No reader ever sees a partial file.

- Why not `rename`: it silently replaces an existing note.
- Why not `OpenOptions::create_new` and then writing: the empty file is visible before its bytes are, and the later daemon could index it half-written.

The temp file starts with `.`, so `check` and later readers ignore it. A crash after the temp file is created leaves a harmless hidden file behind.

Topic uniqueness across kinds is checked by listing `notes/` before writing and again after the `fsync`, just before the link. Two runs that request the same topic under different kinds at the same instant can still both win. On macOS, `fsync` is `F_FULLFSYNC` and the two runs tend to finish it together: simultaneous starts ended with two notes in 1 to 6 of 20 test rounds. `check` reports both files. A lock file would close the window, but at the cost of a stale-lock failure mode, which is worse.

### One store path on every OS

The root is `~/.local/share/bilbo`, or `$XDG_DATA_HOME/bilbo`, on macOS as well as on Linux. It is not macOS's native `~/Library/Application Support/bilbo`.

- Agents reach the store through shell commands and paths they type themselves. A path with a space in it is a quoting trap on every command.
- One path on every machine keeps the docs, the skills and the hook config identical.
- Developer tools on macOS commonly do the same. `uv`, for example, keeps its data in `~/.local/share/uv` there.
- Anyone who wants the native location sets `BILBO_HOME`.

### Fixed kinds and defaults, no config file

The nine kinds are a constant in `note.rs`. The root comes only from environment variables. A config file arrives when something needs one (the embedder URL, in the search change).

### Toolchain

Every command in `tasks.md` runs inside `nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#clippy nixpkgs#rustfmt` (1.95.0). There is no `rust-toolchain.toml`: without rustup nothing reads it, and `rust-version` in `Cargo.toml` already states the minimum version.

## Risks / Trade-offs

- [An agent writes valid YAML that the strict reader rejects, such as a single-quoted source] → `check` names the line and the expected form, and the `note` skill (ported later) shows the exact shape. Strictness is the point: one form keeps `rg '^id: '` lookups and the later index simple.
- [A machine with no local time zone configured] → jiff falls back to UTC, and `created` gets `+00:00`, which is still valid.
- [The legacy notes (about 420) fail `check` as they stand] → Expected. The cutover change converts them.
- [A cross-kind topic race lets two notes share a topic] → `check` reports both files. See the atomic create decision.
- [Unix only: `/dev/urandom` and hard links] → Windows is a non-goal of the proposal.

## Migration Plan

None. Nothing uses bilbo yet. Rollback is reverting the commits.

## Open Questions

- How the repo gets its toolchain for good: a dev shell in the maintainer's Nix configuration, a `shell.nix`, or rustup on the machines. This doesn't change the code or the tasks, which assume the `nix shell` line above.
