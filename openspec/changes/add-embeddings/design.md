# Design

## Context

This change starts after add-note-recall is archived. `recall` then reads the store on every run and ranks passages with BM25 in `src/rank.rs`, and passages are already capped at 4,000 bytes so they fit one embedder slot. `jiff` is the only dependency, and there is no config file: the add-note-store design said one arrives "when something needs one (the embedder URL, in the search change)".

The legacy `nbrecall` is the behavior reference. Its embedder is an OpenAI-shaped `POST /v1/embeddings` on bagend (llama-server, Qwen3-Embedding-0.6B Q8, 1,024 dimensions, `--parallel 1 -c 4096`). Its index is SQLite with sqlite-vec, filled by a timer every few minutes. It fuses FTS5 and vector ranks with reciprocal rank fusion (`k = 60`, 50 candidates per list) and applies a 0.35 similarity floor for search. Measured there (`report-notes-retrieval-eval.md`, `decision-query-embedding-during-indexing.md`):

- One 512-token chunk takes 8 s to embed on bagend's CPU.
- A 103-token query took 1.54 s on an idle slot, so a long query regularly missed the 1.5 s timeout.
- When the embedder failed, search exited 4 with no results, instead of falling back to keywords.

The legacy store is 424 notes and 4,827 heading sections. With the 4,000-byte cut that comes to about 5,000 passages, which is 20 MB of vectors at 1,024 `f32` dimensions.

## Goals / Non-Goals

**Goals:**
- Meaning ranking with no C dependency and no database. The cache is one file bilbo writes atomically and can always rebuild.
- `recall` never fails because of the embedder. It degrades to keywords and says so.
- A long first index (hours on a CPU embedder) can be interrupted and resumed without losing finished work.

**Non-Goals:**
- Approximate nearest-neighbor search. An exact scan of 5,000 vectors is a few milliseconds.
- Moving keyword ranking into a persistent index. BM25 per run stays as add-note-recall built it.

## Decisions

### A flat vector file, not SQLite with sqlite-vec

The cache is `<cache dir>/bilbo/<root key>.vectors`, where `<root key>` is the 16-hex-digit FNV-1a 64 hash of the store root's absolute path. Its layout, little-endian:

- the magic `BILBOVEC1\n`
- the model name's length (`u32`) and its bytes
- the dimension (`u32`) and the record count (`u32`)
- the records: an input key (`u64`) and the unit-normalized vector (`dims` × `f32`)

The input key is the FNV-1a 64 hash of the exact embedder input (heading path, newline, text). A renamed note keeps its vectors, and identical passages share one vector. Over about 10⁴ passages, the chance of an FNV-1a 64 collision is around 10⁻¹¹.

- **Why not SQLite with sqlite-vec,** which the add-note-store design expected: sqlite-vec's `vec0` does an exact brute-force scan too, so it buys no speed at this size. It costs a bundled C build of SQLite plus the sqlite-vec C code, two dependencies, and a schema to migrate. The keyword side doesn't need it either, since BM25 per run takes milliseconds.
- **Why not one file per note:** thousands of small files make the atomic-swap guarantee per file instead of per run, and pruning becomes a directory walk.
- **When to revisit:** past about 50,000 passages (a 200 MB file read on every query), or when the index has to be shared between machines.

The file is written whole to `<name>.tmp-<pid>` and renamed over the old one. Two concurrent `index` runs each produce a valid file, and the last rename wins. Both embedded from the same store, so the loser's work is at most duplicated, never corrupted. No lock file, for the same stale-lock reason as in add-note-store.

### Progress is saved every 30 seconds

`index` rewrites the cache after the batch that crosses 30 seconds since the last save, on any embedder failure, and at the end. The first index of the legacy store on bagend would take hours. Saving after every batch would rewrite up to 20 MB about 300 times, while saving every 30 s loses at most 30 s of embedding to a kill. A failure saves before exiting, which is what the spec's "progress survives" scenario tests.

### Batches of 16, 10-minute request timeout

On bagend, 16 worst-case 1,000-token inputs take about 4 to 5 minutes. A 10-minute timeout covers that with margin, and it matches the legacy `INDEX_TIMEOUT` of 600 s. On OpenAI, 16 inputs per request is far below its limits and costs only round trips. Neither value is a setting until someone needs it to be.

### Query embedding: 5 s, 2,000 bytes, a prefix from the config

- **5 s for `recall`:** an agent waits on recall less tightly than on the digest. The legacy 1.5 s budget failed on long queries, and the digest change sets its own budget.
- **Query cut to 2,000 bytes:** the fix the legacy decision note chose, so a query can't fill a slot.
- **`embedder.query_prefix` replaces the legacy `QWEN_INSTRUCT`,** which was applied by sniffing the model name. Qwen3 embeddings want `Instruct: ...\nQuery: ` and OpenAI's models want nothing, so it is the user's setting, empty by default. The config format allows quoting so the prefix keeps its trailing space.

### Fusion

Reciprocal rank fusion over two lists of up to 50 passages each:

- the keyword list: BM25 order, passages holding a query word;
- the meaning list: cosine order, passages with a cached vector and a similarity of at least `min_similarity`.

A passage scores `1/(60 + rank)` for each list it appears in. A note keeps its best passage. These are the legacy constants, which scored 29 of 37 on the blind set. Ties go by path, then line, as before.

Both lists are built after the `--kind` filter, as in the legacy. Unlike the legacy, notes that only the keyword list holds past its 50th entry still print, after the fused hits and in keyword order. The `Query words` requirement says a passage holding a query word matches, so `--limit 100` has to print every keyword match. It also makes the no-embedder and fallback orders equal to the BM25 order exactly, which a unit test pins. Meaning candidates past the 50th are not hits.

The legacy floor was 0.35, calibrated for Qwen3-Embedding-0.6B on its 512-token chunks (the gold-hit minimum was 0.408 on the blind set). bilbo's default is 0.5, chosen after the blind-set sweep below: on bilbo's passages, 0.35 let a query with no word in common with any note, such as `wumpus`, bring in 30 to 40 meaning hits, while 0.5 left about 3 and scored as well. Other models compress cosines differently, which is why it is a setting.

Passages with no text (a title heading with nothing before the first `##`) get no vector. Embedded as just a heading, a short input scores high against any short query: `wumpus` drew 21 such hits, each with an empty snippet. Every other passage of the note carries the title in its heading path, so the meaning signal is not lost.

### Config file: a strict line reader, no TOML crate

`key = value` lines, `#` comments, optional double quotes, and `\n`, `\\` and `\"` escapes, read by about 60 lines in `src/config.rs`. This follows the frontmatter decision of add-note-store: a general TOML reader would accept tables, arrays and types the config doesn't have, and then need validation on top. The `toml` crate would also bring `serde`-driven parsing for six keys.

Settings resolution (`BILBO_CONFIG`, then `XDG_CONFIG_HOME`, then `HOME`) goes through `store::Env`, which gains `bilbo_config`, `xdg_config_home` and `xdg_cache_home`. Tests already run the binary with a clean environment, so every path stays in a temp folder.

### HTTP: `ureq` 3 with its `json` feature, and `serde`

| Need | Choice | Why not the alternative |
|---|---|---|
| HTTPS to OpenAI and HTTP to a LAN box | `ureq = { version = "3.4.2", features = ["json"] }` with its default `rustls` and `gzip` features. It is blocking, pure Rust, and has a global timeout per request (`Agent::config_builder().timeout_global(...)`). Its errors tell a timeout (`Error::Timeout`) from a status (`Error::StatusCode`). | The standard library has TCP but no TLS. Spawning `curl` puts the token in an argument or the environment of a child process, adds a process per query, and still leaves JSON to parse. `reqwest` pulls in `tokio` for a CLI with no async. |
| Request and response JSON | `serde = { version = "1.0.229", features = ["derive"] }` for two small structs. `serde_json` comes in through ureq's `json` feature. | Parsing `{"data": [{"embedding": [...], "index": n}]}` by hand means a float parser and a JSON tokenizer. That is error-prone code bilbo doesn't need to own. |

`Cargo.toml` uses the caret idiom and `Cargo.lock` pins the build, as `AGENTS.md` says. Checked on 2026-10-02 against ureq 3.4.2's docs and a scratch build on rustc 1.95.0:

- The default `rustls` feature uses the `ring` provider, not `aws-lc-rs`, so the build needs a C compiler and no cmake.
- `Cargo.lock` grows from 23 to 109 entries, most of them Windows-only. About 40 crates compile on macOS, and a release build of the probe was 2.9 MB.
- `serde_json` is a dev-dependency for the fake embedder. It is already in the lock through ureq, so it adds no crate.
- ureq honors `HTTP_PROXY`, `ALL_PROXY` and `NO_PROXY`, like curl. That is left as it is.

### Modules

- `src/config.rs` (new): resolves the config path and reads the settings. It returns `Settings` or a `String` message. It never prints and never returns `Failure`, like `store` and `note`.
- `src/embed.rs` (new): `Client::new(embedder, var, timeout)` reads the token once and builds the agent, and `Client::embed(inputs) -> Result<Vec<Vec<f32>>, String>` sends one request of at most 16 inputs (`embed::BATCH`); `index` does the batching. It checks the response and normalizes the vectors. Its messages never include the token.
- `src/vectors.rs` (new): `key(input)`, `load(path) -> Cache` and `save(path, cache)`. A missing or unreadable cache loads as empty, since it's a cache.
- `src/rank.rs` (extended): `rank` splits into `keyword(query, documents)` and `fuse(keyword, meaning)`, plus an `input(passage)` that builds the embedder input, so `index` and `recall` agree on the key.
- `src/store.rs` (extended): `read_notes(notes)`, moved out of `recall`, so `index` and `recall` read exactly the same notes and passages.
- `src/index.rs` (new verb) and `src/recall.rs` (extended): both build on `config`, `embed`, `vectors`, `rank`, `store` and `note`, never on each other.

### Decided while planning the build

The specs left these choices open. The build plan (`work/add-embeddings/plan.md` in the bilbo-initial-launch notebook) settled them, and the user confirmed the ones marked as questions.

- **Only `recall` and `index` read settings.** `new` and `check` never open the config, so a broken config doesn't stop them.
- **A broken config makes `recall` exit 2 (asked).** It does not fall back. The fallback covers an embedder that fails at run time, and that includes a token file or variable that is missing or empty. A typo should fail loudly and be fixed once, not hide behind a warning line.
- **Order of checks.** Arguments come first, then settings, then the store. So a usage error never depends on the config, and `index` without an embedder never touches the store.
- **Not-indexed count.** `n` counts distinct embedder inputs across the whole store, `--kind` included, that have no vector for the configured model. When no passage has one, `recall` sends no query at all, since there would be nothing to rank.
- **Duplicates.** Identical passages (same heading path and text) share one key. `index` sends them once and counts them once, but they still rank as separate passages.
- **No change, no write.** When nothing is missing, nothing is dropped and the model is the same, `index` leaves the cache file's bytes and modification time alone.
- **The token is read late.** It is read once, just before the first request: by `index` only when something is missing, by `recall` only when it embeds the query.
- **A dimension change from the same model.** `index` exits 1 and tells the user to delete the cache file. `recall` falls back.
- **The root key.** It hashes the root as `store::root` resolves it, with `.`, doubled slashes and trailing slashes normalized but symlinks not resolved. `/tmp/a` and `/private/tmp/a` therefore get two caches, which at worst costs a re-embed.
- **The 30-second save.** A unit test of the fill loop covers it with a fake clock. No integration test waits 30 s.
- **Config format details:**
  - lines split on `\n`, with one trailing `\r` and a leading BOM dropped;
  - "spaces" means space and tab, and `key=value` without spaces is fine;
  - a `#` after a key is part of the value;
  - text after a closing quote is an error;
  - an empty value is an error, except for `embedder.query_prefix`;
  - keys other than `embedder.url` are still checked without a URL, so commenting out the URL is how you turn embeddings off.
- **Config value shapes:**
  - `min_similarity` is plain digits with an optional fraction (`0`, `0.35`, `1.0`): no sign, no exponent, no bare dot.
  - `embedder.url` must start with a lowercase `http://` or `https://` and have a host. A URL whose authority holds `@` is refused without being echoed, since it may carry a password.
- **USAGE (asked)** gains `config:` and `cache:` lines that name where both live.

### Testing without a real embedder

`tests/common/mod.rs` gains a fake embedder: a `std::net::TcpListener` on `127.0.0.1:0`, served from a thread. It answers `POST /v1/embeddings` with vectors from a table the test supplies (input substring to vector), records every request body and header, and can be told to:

- fail after N requests;
- answer a given status;
- answer with too few vectors;
- stall past a timeout.

No test needs the network or a model. The 10-minute index timeout is not exercised. A unit test checks that the agent is built with the timeout it is given.

### Measured on the legacy blind set

Run on 2026-10-02 with `work/add-embeddings/blindset.py` in the bilbo-initial-launch notebook. The corpus is the 426 legacy notes, copied flat. The embedder is Qwen3-Embedding-0.6B Q8_0 under llama.cpp build 9190 with Metal on rivendell: the same GGUF and build as bagend, with `-ub 4096`, because `-ub 512` crashed the Metal backend there. The query prefix is the legacy Qwen instruction.

| arm | success@5 |
|---|---|
| bilbo, keywords only (no config) | 24 of 39 |
| bilbo with embeddings, floor 0.35, title passages embedded | 32 of 39 |
| bilbo with embeddings, as shipped (floor 0.5, no vector for passages without text) | 33 of 39, and 31 of 37 on the legacy hybrid's queries |
| legacy hybrid | 29 of 37 |

- **Index:** `bilbo index` embedded 4,540 passages in 21 minutes. That makes an 18.6 MB cache, and 4,412 passages remain once the text-less ones are dropped.
- **Query time:** 92 ms median and 142 ms max per query with a warm embedder.
- **Cold embedder:** the first query after an hour idle took more than 5 s on the Metal server, and it fell back to keywords as specified.
- **Without a config,** the output is byte-identical to `origin/main` on every query.
- **Floor sweep** (passages without text excluded; `wumpus` / `asdkjh qwe` meaning hits):

  | floor | success@5 | nonsense hits |
  |---|---|---|
  | 0.35 | 32 | 38 / 32 |
  | 0.45 | 31 | 11 / 24 |
  | 0.5 | 33 | 3 / 3 |
  | 0.55 | 31 | 1 / 0 |
  | 0.6 | 29 | 1 / 0 |

  A one-query difference is within noise for 39 queries.

## Risks / Trade-offs

- [Nothing runs `bilbo index` on its own, so the cache drifts behind the store] → `recall` says how many passages are not indexed, and they still rank by keywords. On the maintainer's machines, a dnix timer runs `bilbo index`. The daemon comes later.
- [The query embed adds up to 5 s to `recall` when the embedder is slow] → It falls back after 5 s, never fails, and says why. A local embedder answers short queries in well under a second.
- [`min_similarity` set wrong for a different model floods or starves the meaning list] → It is documented as model-specific. The digest change's miss log will show it.
- [A 20 MB read on every `recall`] → About 10 ms from the page cache. Fine at this size, and revisited at the threshold above.
- [The token lands in a crash report or log] → It is never formatted into a message, and a unit test feeds a known token through every error path and checks it never appears.

## Migration Plan

- Ship with no embedder configured by default: every existing user stays keyword-only.
- On the maintainer's machines, a dnix change writes `~/.config/bilbo/config` pointing at bagend, with `query_prefix` set to the Qwen instruction, and adds a timer running `bilbo index`. That is dnix work, outside this repo.
- Rollback: remove the config file and bilbo is keyword-only again. The cache can be deleted at any time.
