# Tasks

Run every command below inside `nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#clippy nixpkgs#rustfmt`, from the repo root, with `CARGO_TARGET_DIR` set to the checkout's `target`. Start only after add-note-recall is archived: `test -f openspec/specs/note-recall/spec.md`.

## 1. Settings (`config` spec)

- [x] 1.1 Extend `store::Env` with `bilbo_config`, `xdg_config_home` and `xdg_cache_home`, read once in `main`. Unit-test that `from_process` picks them up. Verify with `cargo test store::`
- [x] 1.2 Add `src/config.rs`: path resolution (`BILBO_CONFIG`, absolute `XDG_CONFIG_HOME`, `HOME`), the strict line reader (comments, blank lines, quoting, `\n`, `\\` and `\"` escapes, unknown and repeated keys with file and line), the embedder keys with their defaults and checks, and `~/` expansion for `token_file`. Unit-test every `config` scenario that needs no verb. Verify with `cargo test config::`

## 2. Embedder client

- [x] 2.1 Pull ureq 3's current docs with `use-context7` (crypto provider of the default rustls build, `timeout_global`, the `Error` variants). Add `ureq = { version = "3.4.2", features = ["json"] }` and `serde = { version = "1.0.229", features = ["derive"] }` to `Cargo.toml`, and update the `Stack` line of `openspec/config.yaml` to name them. Verify with `cargo build && rg -q '^name = "ureq"' Cargo.lock && rg -q '^name = "serde"' Cargo.lock`
- [x] 2.2 Add the fake embedder to `tests/common/mod.rs`: a `TcpListener` on `127.0.0.1:0` in a thread, vectors from a substring table, recorded request bodies and headers, and switches to fail after N requests, answer a status, answer with too few vectors, or stall. Verify with `cargo test --test cli` (the helper compiles into every test crate)
- [x] 2.3 Add `src/embed.rs`: batches of at most 16, the bearer token from file or variable, the timeout passed in, checks that there is one vector per input and one length, unit normalization, and error messages that name the URL and status but never the token. Unit-test the token never appearing in any error message, using a known token string. Verify with `cargo test embed::`

## 3. Vector cache and `bilbo index` (`note-index` spec)

- [x] 3.1 Add `src/vectors.rs`: the FNV-1a 64 `key`, the root key, and the file layout from design.md, with `load` (missing or corrupt loads as empty) and `save` (write to `.tmp-<pid>`, then rename). Unit-test a round trip, a truncated file loading as empty, and a model mismatch reporting every vector as missing. Verify with `cargo test vectors::`
- [x] 3.2 Add `rank::input(passage)` (heading path, newline and text, cut to 4,000 bytes at a character boundary) and use it as the only source of embedder inputs and cache keys. Unit-test the `Embedder requests` heading-path scenario. Verify with `cargo test rank::`
- [x] 3.3 Add `src/index.rs`, plus its dispatch arm and USAGE line in `src/main.rs`: no embedder configured, a missing store, no arguments allowed, embed what is missing, keep and drop, save every 30 seconds, on failure and at the end, and the `embedded <n>, kept <n>, dropped <n>` line. Cover every `note-index` scenario in `tests/index.rs` against the fake embedder, including the resume after a failure halfway and the read-only snapshot of the store. Verify with `cargo test --test index`

## 4. Meaning in `recall` (`note-recall` delta)

- [x] 4.1 Add `rank::fuse`: reciprocal rank fusion with `k = 60` over up to 50 keyword and 50 meaning candidates, one hit per note, ties by path then line. Unit-test the `Agreement beats one signal` scenario on fixed ranks. Verify with `cargo test rank::`
- [x] 4.2 In `src/recall.rs`: load the settings and, with an embedder, the cache. Embed the query prefix plus the query, cut to 2,000 bytes, with a 5 s timeout. Rank cached passages by dot product above `min_similarity`, fuse with the keyword ranks, print the `embedder unavailable` and `passages not indexed` lines, and keep the output format unchanged. Cover the modified and added `note-recall` scenarios in `tests/recall.rs` against the fake embedder, including a stalled embedder falling back and a run with no config leaving stderr empty. Verify with `cargo test --test recall`
- [x] 4.3 Add the `cli` scenario for the verb list naming `index` to `tests/cli.rs`. Verify with `cargo test --test cli`

## 5. Docs

- [x] 5.1 Update `AGENTS.md`: the new modules and their contracts (`config`, `embed` and `vectors` never print and never return `Failure`), `tests/index.rs`, the fake embedder in `tests/common/mod.rs`, the dependency list (`jiff`, `ureq`, `serde`), and a line saying that something outside bilbo runs `bilbo index`. Verify with `for f in src/*.rs tests/*.rs; do rg -qF "$f" AGENTS.md || echo "missing $f"; done | (! grep .) && rg -q ureq AGENTS.md`

- [x] 5.2 Update `plugins/bilbo/skills/recall/SKILL.md` for the `agent-plugin` delta: exit 0 relays warning lines, exit 1 decides by the last stderr line, and the retry advice says meaning matching applies only with an embedder. Add the new phrases to `tests/plugin.rs`. Verify with `cargo test --locked --test plugin && claude plugin validate plugins/bilbo`

## 6. Integration

- [x] 6.1 Smoke test against a real embedder: run `ollama` or `llama-server` with an embedding model locally, or point at bagend. Write a config, create two notes in a fresh store, run `bilbo index` twice (the second prints `embedded 0`), then run a paraphrased `bilbo recall` that finds a note sharing no word with the query. Record the commands and the output in the change folder as `smoke.md`. Verify with `test -s openspec/changes/add-embeddings/smoke.md`
- [x] 6.2 Re-run the legacy blind set (`work/recall-arms/arms.py` in the bilbo-initial-launch notebook) with a `bilbo` arm against a converted copy of the legacy notes, indexed with Qwen3-Embedding-0.6B. Expect at least the legacy hybrid's 29 of 37, and record the number in design.md. Verify with `rg -q 'of 37' openspec/changes/add-embeddings/design.md`
- [x] 6.3 Run the full suite: formatting, lints and every test. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked`
