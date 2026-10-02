# Proposal

## Why

Keyword recall misses a note when the query uses other words than the note does, or another language. On the legacy blind query set (424 notes, 2026-10-02), keyword ranking found the right note in the top 5 for 2 of 9 paraphrased queries and 3 of 7 cross-language ones. The legacy search, which adds embeddings, found 5 of 9 and 5 of 7. Agents paraphrase by default and write in English and Portuguese. Without meaning-based ranking, recall misses those notes, and so will the digest, which ranks on the same signals.

## What Changes

- A config file, `~/.config/bilbo/config` (`$XDG_CONFIG_HOME/bilbo/config` when set, `BILBO_CONFIG` overrides). It holds strict `key = value` lines. Its first keys describe the embedder: `embedder.url`, `embedder.model`, `embedder.token_file` or `embedder.token_env`, `embedder.query_prefix` and `embedder.min_similarity`. Without a config file, or without an `embedder.url`, bilbo stays keyword-only, exactly as add-note-recall left it.
- The embedder is any service that speaks the OpenAI-shaped `POST <url>/v1/embeddings`: OpenAI itself, Ollama, llama-server or a self-hosted box.
- `bilbo index`: embeds every passage the vector cache lacks, drops vectors for passages that no longer exist, and prints how many it embedded, kept and dropped. Passages are the ones `recall` already uses.
- A vector cache under `~/.cache/bilbo/` (`$XDG_CACHE_HOME/bilbo/` when set). It is a cache: deleting it loses nothing that `bilbo index` cannot rebuild, and a change of `embedder.model` invalidates it.
- `recall` fuses the keyword ranking with a meaning ranking: passages whose vectors are close enough to the query's vector. Its output does not change.
- `recall` degrades instead of failing. When the embedder is unreachable, slow or misconfigured, it ranks by keywords alone, says so in one stderr line, and still exits 0 with hits. Passages not yet indexed rank by keywords alone.

## Capabilities

### New Capabilities

- `config`: where bilbo reads its settings, the file format, and the embedder keys.
- `note-index`: `bilbo index` and the vector cache it maintains.

### Modified Capabilities

- `note-recall`: the Ranking requirement fuses keyword and meaning ranks when an embedder is configured. A new requirement covers falling back to keywords.
- `cli`: the Verb dispatch requirement adds `index`.

## Non-goals

- The digest and the prompt hook (the add-note-digest change).
- A daemon or a watcher. Something outside bilbo runs `bilbo index`: a launchd or systemd timer, a git hook, or the agent itself. The daemon comes later.
- Serving queries over HTTP, the hub role of the legacy setup, and sharing an index between machines.
- Reading tokens from the macOS keychain or a password manager. `token_file` and `token_env` cover the sops and `op` flows without bilbo running a command on every query.
- Running a model inside bilbo.
- Sources and the library.

## Impact

- New dependencies: `ureq` (HTTP and TLS) with its `json` feature, and `serde` for the request and response shapes. They are justified in design.md. There is still no SQLite.
- New modules: `src/config.rs`, `src/embed.rs`, `src/vectors.rs` and the verb `src/index.rs`. `src/rank.rs` gains fusion, and `src/recall.rs` loads the config and the cache.
- `tests/index.rs` is new, with a fake embedder: a `std::net::TcpListener` serving fixed vectors. `AGENTS.md` lists the new modules, the dependencies and the test file.
- Order: this change's `note-recall` delta modifies requirements that add-note-recall creates, so it applies and archives only after add-note-recall is archived.
