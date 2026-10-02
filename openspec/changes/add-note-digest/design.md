# Design

## Context

This change starts after add-embeddings is archived. By then bilbo has:

- `rank.rs`, which builds passages, words, BM25 and fusion;
- `config.rs`, `embed.rs` and `vectors.rs`;
- `recall`, which already degrades to keywords when the embedder fails.

The legacy digest is `nbrecall project`, called by a 35-line shell hook (dnix `modules/ai/common/hooks/nbrecall-project.sh`). The hook:

- parses the payload with `jq`;
- skips prompts that start with `/` or `$` unless a skill or command file of that name exists;
- calls `nbrecall project --session ID -- PROMPT`;
- swallows every error.

`nbrecall project` asks the shared index for 20 candidates. It admits a hit when its cosine is at least 0.55 **or** the hit shares at least 2 distinct query words of 4+ letters. It shows 6 notes on a session's first prompt and 3 on later ones, never repeats a note within a session, caps the block at 12,000 bytes, and keeps session state as JSON under `~/.cache/nbrecall/sessions/`.

Known failures, from `decision-graphiti-and-learn-not-built.md` and this session:

- **Off-topic digests on follow-ups.** 3 of 6 were fully off-topic. This session's digests list unrelated notes too: a NestJS decorator, Mutagent, an eval-spec format change. That pattern fits the lexical arm of the gate: two common 4-letter words such as `spec`, `work` or `test` are enough to admit a note, whatever the cosine.
- **Long prompts lose the digest.** A 103-token query took 1.54 s to embed on an idle bagend slot, past the 1.5 s timeout, and the whole digest was then dropped.
- **No log.** The hub logs no query text, so a bad or empty digest leaves no trace.

Measured on 2026-09-28 from rivendell's Claude transcripts (`research-jev-fit-in-dmacs-and-dev-process.md`, dmacs notebook):

- 316 digests injected 1,086 notes across 67 sessions.
- In 305 of 309 digests, more hits passed the gate than the cap could show. The OR-gate never filters; the cap of 6 or 3 does, so every prompt gets its top hits, relevant or not.
- Only 6.6% of injected notes were later named in a tool call. That is a proxy, since a snippet can help without being opened.

The same note leaves open whether per-prompt injection earns its place at all. The digest log in this change is what can answer that for bilbo.

The noise probes from the retrieval eval ("hello" 0.520, "quero cafe" 0.549, "ok thanks" 0.436) all sit under 0.55. The gold hits had a median of 0.649, and 32 of 38 cleared 0.55.

## Goals / Non-Goals

**Goals:**
- With an embedder, admit a note on meaning only, so generic follow-ups stop pulling in notes that merely share words.
- Always give some answer inside 1.5 s, even when the embedder is slow, instead of dropping the digest.
- Make every empty or wrong digest explainable from a log, without logging by default.
- One binary call per prompt, no `jq`, no shell logic in the hook.

**Non-Goals:**
- Keeping state in memory between prompts (a daemon). Reading the store per prompt fits the budget at 6 MiB.
- Ranking `decision` and `gotcha` above other kinds. `decision-graphiti-and-learn-not-built.md` lists it as the cheaper fix *if* stale answers show up, and the log is what would show them.

## Decisions

### Meaning-only gate with an embedder, a 3-word gate without

The legacy OR-gate (cosine ≥ 0.55 **or** 2 shared long words) is the likeliest cause of the off-topic digests, since a note with a low cosine gets in through the lexical arm alone. With an embedder, bilbo drops the lexical arm. Shared words still help ordering through fusion, but never admission.

Without an embedder, or when the query embed misses the budget, there is no meaning signal. The keyword gate then needs 3 distinct query words of 4+ letters in one passage: stricter than the legacy 2, because nothing else filters. A short follow-up ("ok, do it") then admits nothing, which is the right answer for it.

- Alternative: keep the OR-gate with a BM25 score floor. Rejected, because BM25 scores depend on the query and the store, so no single floor transfers between stores.
- Alternative: no digest at all without an embedder. Rejected, because the keyword gate is cheap. If the log shows it noisy, the default can change without a spec change to the format.

`digest.min_similarity` is a setting for the same reason `embedder.min_similarity` is: 0.55 is calibrated for Qwen3-Embedding-0.6B.

### The time budget

`digest` measures from its start:

1. Read the settings and the store, build passages and BM25, and load the vector cache. That is about 50 to 100 ms at 6 MiB in Rust (add-note-recall's speed task gives the measured number).
2. Embed `query_prefix` plus the first 1,000 bytes of the query, with a timeout of the smaller of 1.2 s and what is left of 1.5 s.
3. On a timeout or error, use the keyword gate and record the reason.

Cutting the query to 1,000 bytes, against `recall`'s 2,000, halves the embed cost on a CPU embedder. At bagend's measured rate (103 tokens in 1.54 s) that still doesn't guarantee 1.2 s for a full 1,000 bytes. That is why the fallback exists: the digest degrades instead of vanishing. A GPU or hosted embedder answers in tens of milliseconds.

### Sigil handling without skill-file lookups

The legacy hook checked `~/.claude/skills/<name>/SKILL.md` and three other paths to decide whether `/name` was a command. Those paths are tool-specific and machine-specific. bilbo accepts any first word that is a sigil plus `[A-Za-z0-9._:-]+`. That rejects paths (they hold `/`) and keeps every skill and command name, including OpenSpec's `opsx:apply`. An unknown command name becomes a query word, which is harmless.

### Session memory and log locations

- **Session memory** (`<cache>/bilbo/sessions/<session id>`): one note path per line, appended after each block, written through a temp file and a rename. It is cache-like: losing it only means a note may be shown again. Files untouched for 30 days are deleted on each run. That is a directory scan of a few hundred entries at most.
- **Digest log** (`<state>/bilbo/digest.jsonl`): XDG state, not cache, because it is history that can't be rebuilt. One `O_APPEND` write per run, so concurrent sessions don't interleave within a line. It is off by default, because it stores the start of every prompt. The maintainer turns it on in dnix.

The session id is checked against `[A-Za-z0-9._-]{1,128}` before it touches a path, so a hostile payload can't escape the sessions folder.

### Hook input with `serde_json`

`serde_json` already comes in through ureq's `json` feature. Declaring it directly, at the version `Cargo.lock` already pins, costs nothing new. The payload is parsed into a struct with two `Option<String>` fields, and unknown fields are ignored, so Claude Code and Codex payloads both work. The legacy hook reads the same two fields from both tools. The log line is written with `serde_json` too, so prompt text with quotes and control characters stays valid JSON.

### Errors

`digest` never returns `Failure` to `main`'s usual exit-code mapping. Its `run` returns the block (possibly empty) and an optional diagnostic. `main` prints the block, prints the diagnostic as one `bilbo: ` line, and exits 0. This is the only verb with that shape, and the `cli` delta records the exception.

### Modules

- `src/digest.rs` (new verb): input parsing, the query, the gate, limits, the block, session memory and the log.
- `src/config.rs` (extended): the two digest keys.
- `store::Env` (extended): `xdg_state_home`.

`digest` reuses `rank`, `embed`, `vectors`, `config`, `store` and `note`, and never calls `recall`. Following the `AGENTS.md` rule, anything both verbs need goes into `rank.rs`. If, for example, recall's candidate and fusion step is needed here, it moves there as one function.

## Risks / Trade-offs

- [0.55 cuts the weakest gold hits: 6 of 38 in the eval] → A missed note is still one `bilbo recall` away, and the block points there. Precision matters more than recall for injected context. The log shows whether the floor should move.
- [A CPU embedder often misses 1.2 s on long prompts] → The keyword gate takes over, and the log records `ranking: keywords` with the timeout, so the rate is measurable. A second query-only embedder instance on bagend (the fix in `decision-query-embedding-during-indexing.md`) is the dnix-side remedy.
- [The prompt log is sensitive] → Off by default, local only, under the user's state folder, and only the first 500 characters.
- [Reading the whole store on every prompt] → Bounded by add-note-recall's speed goal. A store far beyond 6 MiB needs the daemon, which is a separate change.

## Migration Plan

- On the maintainer's machines, a dnix change replaces the `nbrecall-project.sh` UserPromptSubmit hook with the command `bilbo digest`, for Claude Code and Codex, and sets `digest.log = on`. That happens after the cutover moves the notes into bilbo's store. Before that, the bilbo store is nearly empty and the legacy hook stays.
- Rollback: point the hook back at the legacy script. bilbo keeps nothing the legacy side needs.
