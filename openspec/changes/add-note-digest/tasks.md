# Tasks

Run every command below inside `nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#clippy nixpkgs#rustfmt`, from the repo root, with `CARGO_TARGET_DIR` set to the checkout's `target`. Start only after add-embeddings is archived: `test -f openspec/specs/note-index/spec.md && test -f openspec/specs/config/spec.md`.

## 1. Settings

- [ ] 1.1 Extend `store::Env` with `xdg_state_home`, and `src/config.rs` with `digest.min_similarity` (0 to 1, default 0.55) and `digest.log` (`on` or `off`, default `off`). Unit-test the `Digest settings` scenarios. Verify with `cargo test config::`
- [ ] 1.2 Declare `serde_json` directly in `Cargo.toml` at the version `Cargo.lock` already holds, and confirm no new package entered the lockfile. Verify with `cargo build && git diff --no-ext-diff Cargo.lock | (! grep -E '^\+name = ')`

## 2. `bilbo digest` (`note-digest` spec)

- [ ] 2.1 Add `src/digest.rs`, its dispatch arm and USAGE line, with the always-exit-0 shape from design.md (the block plus an optional diagnostic, never a `Failure`). Parse the hook input (an object with `session_id` and `prompt`, unknown fields ignored, the session id pattern) and build the query (trim, cut to 2,000 bytes, the sigil rule). Cover the `Hook input`, `The query` and `Never fail the prompt` scenarios in `tests/digest.rs`, including `../../etc` writing nothing outside the cache. Verify with `cargo test --test digest`
- [ ] 2.2 Add the gate and limits: the meaning-only gate at `digest.min_similarity` when the embed answers, the 3-word keyword gate otherwise, `recall`'s ordering (moving any shared step into `rank.rs`), 6 or 3 notes, leaving out notes already shown in the session, and the 12,000-byte cap. Cover the `The gate` and `How many notes` scenarios against the fake embedder. Verify with `cargo test --test digest`
- [ ] 2.3 Render the block exactly as specified and write session memory (one path per line, temp file then rename, a 30-day cleanup on each run). Cover the `The digest block` and `Session memory` scenarios, with the cleanup scenario backdating a file's modification time. Verify with `cargo test --test digest`
- [ ] 2.4 Add the time budget: time from start, embed `query_prefix` plus the first 1,000 bytes with a timeout of the smaller of 1.2 s and what is left of 1.5 s, and fall back to the keyword gate. Cover `A stalled embedder` with the fake embedder's stall switch, asserting the run ends within 1.5 s. Verify with `cargo test --test digest`
- [ ] 2.5 Add the opt-in log: one `serde_json` line per run with `O_APPEND` to `<state>/bilbo/digest.jsonl`, the fields from the spec, the prompt cut to 500 characters, and nothing written when off. Cover `The digest log` scenarios and the read-only snapshot of the store and the vector cache. Verify with `cargo test --test digest`
- [ ] 2.6 Add the updated `cli` scenarios to `tests/cli.rs`: the verb list naming `digest`, and `bilbo digest --verbose` exiting 0 with one `bilbo: ` line. Verify with `cargo test --test cli`

## 3. Speed

- [ ] 3.1 Add an `#[ignore]` test in `tests/digest.rs` that uses add-note-recall's generated 6 MiB store with a fake embedder answering in 100 ms, and asserts a digest run finishes in under 1.5 s and spends under 300 ms outside the embed. Record the measured times on rivendell in design.md. Verify with `cargo test --release --test digest -- --ignored`

## 4. Docs

- [ ] 4.1 Update `AGENTS.md`: `src/digest.rs` (`note-digest` spec, the only verb that always exits 0), `tests/digest.rs`, `serde_json` among the dependencies, and the cache, state and session paths. Verify with `for f in src/*.rs tests/*.rs; do rg -qF "$f" AGENTS.md || echo "missing $f"; done | (! grep .) && rg -q serde_json AGENTS.md`

## 5. Integration

- [ ] 5.1 Smoke test the hook path with a real Claude Code payload shape: build, index a fresh store of three notes against a real embedder, pipe `{"session_id":"smoke","prompt":"<a paraphrase of one note>"}` into `bilbo digest` twice, and confirm the first run lists the note and the second does not. Record the commands and output in the change folder as `smoke.md`. Verify with `test -s openspec/changes/add-note-digest/smoke.md`
- [ ] 5.2 Run the full suite: formatting, lints and every test. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked`
