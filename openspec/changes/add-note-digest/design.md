# Design

## Context

bilbo already has what a digest ranks with:

- `rank.rs`: passages, words, BM25 (`keyword`) and reciprocal rank fusion (`fuse`);
- `config.rs`, `embed.rs` and `vectors.rs`;
- `recall`, which degrades to keywords when the embedder fails;
- `store::state_dir`, the state folder `setup` already uses for the timer's log;
- the `bilbo` plugin for Claude Code and Codex, which `bilbo setup` installs.

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

What the two tools do with a plugin's prompt hook was probed on rivendell on 2026-10-03, Claude Code 2.1.288 and Codex 0.155.1 (transcripts and source references in the planning notebook, `work/add-note-digest/probe/`):

- Both read `hooks/hooks.json` at the plugin root without a manifest field, in the same format. Codex's plugin validator rejects a `hooks` manifest field, so the default file is the only shape both validators accept.
- Both send `session_id` and `prompt` in the UserPromptSubmit payload. Claude Code's ids are UUIDs, Codex's are UUIDv7 thread ids.
- On exit 0, both add plain stdout to the model's context. Claude Code, through a real turn, answered from it. Codex records it as a developer message of kind `hooks.additional_context`.
- Both block the prompt on exit 2 and show stderr. A `bilbo` without the `digest` verb exits 2 on an unknown verb, which blocked every prompt in the probe.
- A missing command exits 127, which both show as a hook error on every prompt.
- Both move hook output past 10,000 characters to a file. Claude Code keeps 10,000 and moves 10,001. Codex moves more than 2,500 approximate tokens, counted as bytes / 4. The model then gets a preview and a path instead of the block.
- Codex runs a plugin hook only once it is trusted: a hash of the handler is stored in `config.toml` from Codex's review screen, which the TUI opens at startup. `codex exec` skips an untrusted hook in silence. No feature flag is needed: `plugin_hooks` is a removed flag, and plugin hooks load by default.
- No `codex` command trusts a hook. The TUI trusts through its app-server (`codex app-server`, JSON-RPC over stdio): `hooks/list` returns each hook's `key`, `currentHash` and `trustStatus`, and `config/batchWrite` upserts `hooks.state.<key>.trusted_hash`. Done that way in a throwaway `CODEX_HOME`, `codex exec` then ran the hook without `--dangerously-bypass-hook-trust`. The bilbo hook's key is `bilbo@bilbo:hooks/hooks.json:user_prompt_submit:0:0`, with no version in it. Its hash is SHA-256 over the handler's canonical JSON, so it changes only when `hooks.json` changes, not on a version bump. A `config.toml` in a read-only folder makes the write fail with Codex's own message.
- `timeout` is in seconds in both. Claude Code cancels a hook that runs past it in silence. Codex reports `hook timed out`.

## Goals / Non-Goals

**Goals:**
- With an embedder, admit a note on meaning only, so generic follow-ups stop pulling in notes that merely share words.
- Always give some answer inside 1.5 s, even when the embedder is slow, instead of dropping the digest.
- Make every empty or wrong digest explainable from a log, without logging by default.
- One binary call per prompt, no `jq`, and a hook that can never block or clutter a prompt.

**Non-Goals:**
- Keeping state in memory between prompts (a daemon). Reading the store per prompt fits the budget at 6 MiB.
- Ranking `decision` and `gotcha` above other kinds. `decision-graphiti-and-learn-not-built.md` lists it as the cheaper fix *if* stale answers show up, and the log is what would show them.

## Decisions

### Meaning-only gate with an embedder, a 3-word gate without

The legacy OR-gate (cosine ≥ 0.55 **or** 2 shared long words) is the likeliest cause of the off-topic digests, since a note with a low cosine gets in through the lexical arm alone. With an embedder, bilbo drops the lexical arm. Shared words still help ordering through fusion, but never admission.

Without an embedder, or when the query embed misses the budget, there is no meaning signal. The keyword gate then needs 3 distinct query words of 4+ letters or digits in one passage: stricter than the legacy 2, because nothing else filters. A short follow-up ("ok, do it") then admits nothing, which is the right answer for it.

- Alternative: keep the OR-gate with a BM25 score floor. Rejected, because BM25 scores depend on the query and the store, so no single floor transfers between stores.
- Alternative: no digest at all without an embedder. Rejected, because the keyword gate is cheap. If the log shows it noisy, the default can change without a spec change to the format.

`digest.min_similarity` is a setting for the same reason `embedder.min_similarity` is: 0.55 is calibrated for Qwen3-Embedding-0.6B.

A passage without a vector cannot pass the meaning gate, so a note written since the last `bilbo index` waits for the next run of the index timer. When nothing at all is indexed for the configured model, the digest skips the embedder and uses the keyword gate, recording `no passage is indexed; run bilbo index`.

### Ordering: recall's lists, restricted to the gate

The digest builds `recall`'s two lists and fuses them with `rank::fuse`:

- the keyword list, `rank::keyword` over every note, keeping only notes that passed the gate;
- the meaning list, every passage at or above `digest.min_similarity`, best first (empty under the keyword gate).

`fuse` returns one passage per note, which gives the line and snippet. A gated note that `fuse` drops, because its passages rank past the 50 candidates in the meaning list and it shares no query word, is appended in meaning order. Every note that passed the gate therefore appears exactly once, and `passed` in the log equals the number of gated notes.

### Shared steps move into the library modules

`recall`'s meaning step mixes four things the digest needs too. Following the `AGENTS.md` rule that verbs never build on each other, they move out of `recall.rs`, with no change to `recall`'s behavior:

- `vectors::lookup(cache, model, documents)`: each passage's cached vector and how many distinct inputs lack one;
- `rank::meaning(query, vectors, floor)`: every passage at or above `floor`, best first, with its similarity;
- `embed::query(embedder, text, timeout, dims)`: one request for the query vector, with the dimension check against the cache;
- `rank::snippet(passage)`: the 300-character snippet both outputs print.

`rank::shared(passage, words)`, the count of distinct words a passage holds, is new and serves only the keyword gate. It lives in `rank.rs` because that module owns word folding.

`embed`'s timeout message printed whole seconds, so the digest's 1.2 s budget read `within 1 s` and a 300 ms one `within 0 s`. It now prints `300 ms`, `5 s` or `1.2 s`; `recall` and `index` keep whole seconds and their text.

### The time budget

`digest` measures from its start:

1. Read the settings, stdin and the store, build passages and the keyword list, and load the vector cache.
2. Embed `query_prefix` plus the first 1,000 bytes of the query, with a timeout of the smaller of 1.2 s and 1.4 s minus the time spent so far. The last 100 ms are kept for the gate, the block, session memory and the log.
3. On a timeout or error, use the keyword gate and record the reason.

Building the keyword list before the embed means the fallback costs nothing extra once the embedder gives up.

Measured on rivendell (M-series, release build, 2026-10-03) over the generated 6.3 MiB store, about 10,800 passages, with a 1,024-dimension cache of 44 MB from the fake embedder:

- keyword-only `recall` (`recall_over_a_6_mib_store_is_fast`, 6.3 MiB): 57 to 61 ms, against 54 to 57 ms at the base, so moving its shared steps costs nothing measurable;
- `recall` with meaning: 81 to 89 ms with an instant embedder, and 188 to 193 ms with one answering in 100 ms;
- `digest` (`digest_over_a_6_mib_store_is_fast`): 195 to 200 ms with the 100 ms embedder, so about 100 ms outside the embed (276 ms when other builds loaded the machine);
- a stalled embedder (debug build, tiny store): the run ended at 1,215 ms with the keyword gate's answer.

Cutting the query to 1,000 bytes, against `recall`'s 2,000, halves the embed cost on a CPU embedder. At bagend's measured rate (103 tokens in 1.54 s) that still doesn't guarantee 1.2 s for a full 1,000 bytes. That is why the fallback exists: the digest degrades instead of vanishing. The local embedder (llama-server on Metal) answered its first query in 11 ms in add-local-embedder's smoke.

### Sigil handling without skill-file lookups

The legacy hook checked `~/.claude/skills/<name>/SKILL.md` and three other paths to decide whether `/name` was a command. Those paths are tool-specific and machine-specific. bilbo accepts any first word that is a sigil plus `[A-Za-z0-9._:-]+`. That rejects paths (they hold `/`) and keeps every skill and command name, including OpenSpec's `opsx:apply`. An unknown command name becomes a query word, which is harmless.

### Session memory and log locations

- **Session memory** (`<cache>/bilbo/sessions/<session id>`): one note path per line. After the block is chosen and before it is printed, the old lines plus the new paths are written to `<session id>~<pid>` and renamed into place. `~` is not allowed in a session id, so a temporary name never collides with a session. It is cache-like: losing it only means a note may be shown again, and two prompts of one session racing can lose one run's paths the same way. Files untouched for 30 days, stale temporary files included, are deleted on each run. That is a directory scan of a few hundred entries at most, and its errors are ignored.
- **Digest log** (`<state>/bilbo/digest.jsonl`): XDG state, not cache, because it is history that can't be rebuilt. One `O_APPEND` write per run, so concurrent sessions don't interleave within a line, to a file created with mode 0600. It is off by default, because it stores the start of every prompt. The maintainer turns it on in dnix with `programs.bilbo.settings."digest.log" = "on"`.

The session id is checked against `[A-Za-z0-9._-]{1,128}`, and `.` and `..` are refused, before it touches a path, so a hostile payload can't escape the sessions folder.

### Hook input with `serde_json`

`serde_json` is already a direct dependency. The payload is parsed as a `serde_json::Value` that must be an object, and the two fields are read as strings, so Claude Code and Codex payloads both work and a JSON array is refused. Deriving a struct would accept an array as a struct too. The log line is written with `serde_json` as well, so prompt text with quotes and control characters stays valid JSON. Its keys come out in alphabetical order, the order `serde_json`'s map keeps without the `preserve_order` feature.

### Errors

`digest::run(args, input, env)` returns an `Outcome`: the block's lines (possibly none) and an optional diagnostic. It never returns `Failure`. `main` prints the lines to stdout and the diagnostic as one `bilbo: ` line, then exits 0. It is the only verb with that shape, and the `cli` delta records the exception.

- A config error is the diagnostic. Without settings there is no log.
- An error that stops the digest leaves no lines, and is both the diagnostic and the log's `error`: an argument, bad input, no store, an unreadable store or session file, an unwritable sessions folder, or a missing cache folder.
- An embedder failure only switches the gate. It is still the diagnostic and the log's `error`. stderr on exit 0 never reaches the model, in either tool.
- A failed log write never changes stdout. It becomes the diagnostic when there is no other.

### Delivery: the hook in the plugin

`plugins/bilbo/hooks/hooks.json` registers one UserPromptSubmit command hook:

```json
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "command -v bilbo >/dev/null 2>&1 || exit 0; bilbo digest; exit 0",
            "timeout": 5
          }
        ]
      }
    ]
  }
}
```

- `command -v bilbo … || exit 0` turns a missing binary into silence instead of a hook error on every prompt. Claude Code runs the command with `/bin/sh -c`, Codex with `$SHELL -lc`, and both shells have `command -v`.
- `; exit 0` keeps bilbo's exit code from the tool. `bilbo digest` already exits 0, but a `bilbo` older than the plugin exits 2 on the unknown verb, and exit 2 blocks the prompt in both tools. The probe saw exactly that. A plugin added by hand from the repository's default branch (`claude plugin marketplace add delucca/bilbo`, as the `agent-plugin` spec's install scenario does) or through `--plugin-source` can be newer than the installed binary, so this skew is expected, not hypothetical.
- `timeout` 5 s is a backstop above the 1.5 s budget, leaving room for Codex's login shell to start. A hang past it is a bilbo bug: Claude Code then drops the hook in silence and Codex reports it.
- No `${CLAUDE_PLUGIN_ROOT}`: the command needs nothing from the plugin folder, and Codex's trust hash covers the command text, which then never changes between plugin versions.
- No `additionalContextLimit`: the 9,000-byte block stays under Codex's default of 2,500 approximate tokens (10,000 bytes).
- The block cap drops from the legacy 12,000 bytes to 9,000. Above 10,000 characters both tools replace the output with a preview and a file path, so a 12,000-byte block would have reached the model as neither digest nor notes. 9,000 bytes keeps room under both limits. A typical block of 6 notes is about 3 KB.

`bilbo setup` already installs the plugin from the package's `share/bilbo` or from the release tag, and the Nix package copies all of `plugins/`.

### Codex hook trust through the app-server

The user asked (2026-10-03) for `bilbo setup` to trust the hook in Codex, so Codex users get the digest without a review step. That overrides a safety review Codex puts in front of every new hook, at the user's explicit request. The trust is narrow: it covers one hook, whose command the plugin pins and `tests/plugin.rs` checks, and which runs only `bilbo` from PATH.

How: after the codex step leaves `bilbo@bilbo` installed, setup starts `<codex> app-server` and sends, one JSON object per line, `initialize`, the `initialized` notification and `hooks/list` (with `cwds` set to `$HOME`). It keeps the entries whose `pluginId` is `bilbo@bilbo`. When each is `trusted`, the line says `kept`. Otherwise it sends one `config/batchWrite` with `keyPath` `hooks.state`, mergeStrategy `upsert`, and `{<key>: {"trusted_hash": <currentHash>}}` for each entry that is `untrusted` or `modified`. Then it closes stdin and waits for the process to exit. Requests carry ids 1, 2, 3 in order. Replies are matched by id, and anything else on stdout (notifications) is skipped. Every call has a 30 s limit.

`--remove` sends `config/read` and deletes every `hooks.state` key that starts with `bilbo@bilbo:`, one `config/batchWrite` edit per key (`keyPath` `hooks.state."<key>"`, value `null`, mergeStrategy `replace`). Codex leaves an empty `[hooks.state]` table, which is harmless. It runs whether or not the plugin is still installed, because `config/read` does not need the plugin.

Why the app-server and not a hand-written TOML edit:
- Codex computes the key and the hash itself, so bilbo copies none of Codex's hashing. The probe reproduced the hash by hand (`sha256` over the sorted, compact JSON of `{event_name, matcher, hooks: [normalized handler]}`), but that formula is Codex's internal detail.
- Codex writes its own `config.toml` with its own TOML library, so an existing `[hooks.state]` table, comments and the user's formatting survive, and an unwritable file fails with Codex's message. bilbo has no TOML crate and would otherwise have to edit one table in a file it does not own.
- No new dependency: `std::process` and `serde_json`. Nothing hashes, so `ring` stays in `model.rs`, as `AGENTS.md` asks.

Risks: the app-server protocol is marked experimental. It is what Codex's own TUI and IDE extension use, and the methods bilbo calls (`initialize`, `hooks/list`, `config/batchWrite`, `config/read`) are in the generated schema of 0.155.1. If a later Codex renames them, the hook line says `failed` with Codex's error and the digest falls back to Codex's review screen; nothing else breaks. Starting the app-server creates Codex's runtime files in a `CODEX_HOME` that has none, as any `codex` run does. A rerun only reads (`hooks/list`) and writes nothing.

`agents.rs` gains the request bodies and their parsing as pure functions, tested on responses recorded from Codex 0.155.1 under `tests/fixtures/agents/`. `command.rs` gains the conversation itself (`command::Rpc`), a child with piped stdin and stdout, read by a thread so a call can time out. The fake `codex` answers `app-server` with shell builtins: it counts requests to echo their ids, reports the hook as trusted when its state file holds the fake hash, and records every `config/batchWrite` in its log.

### The digest switch

`digest.enable = off` makes `bilbo digest` read and ignore stdin, print nothing, touch no file and exit 0. Claude Code cannot disable one hook in a plugin, so without the key, a user who wants `recall` but no digest would have to drop the whole plugin. Reading stdin before exiting keeps the hook from getting a broken pipe. Neither tool fails on one, but a clean exit costs nothing. With the digest off, no log line is written either, because there is no run to explain.

### Settings: config, setup and home-manager

`config::Settings` gains `digest: config::Digest { enable, min_similarity, log }`, parsed with the embedder keys, so every verb that reads settings rejects a bad digest value. The keys stand alone: a config with only `digest.log = on` is valid and keyword-only.

`setup` rewrites a config when the wizard changes the embedder, and it wrote embedder keys only, so a `digest.log = on` would have vanished into `config.bak`. The rewrite now appends the old file's digest lines as written, in `config::KEYS` order (`Settings.digest_lines`, which `config::parse` collects), so an explicit value that equals today's default stays pinned if the default ever moves. The home-manager module's `settings` gains the three keys, in `config::KEYS` order.

### Modules

- `src/digest.rs` (new verb): input parsing, the query, the gate, limits, the block, session memory and the log.
- `src/config.rs` (extended): the three digest keys, and `digest_lines` (the digest lines a file wrote, for rewrites).
- `src/rank.rs`, `src/vectors.rs`, `src/embed.rs` (extended): the shared steps above. `src/recall.rs` calls them.
- `src/setup.rs` (extended): rewrites keep the digest settings; the `hook` step, in install and remove.
- `src/agents.rs` (extended): the app-server requests and parsing. `src/command.rs` (extended): `Rpc`, the conversation with a child process.
- `src/main.rs`: the `digest` arm and USAGE line.
- `plugins/bilbo/hooks/hooks.json` (new), checked by `tests/plugin.rs`.
- `flake.nix`: the module's three keys and a check line.
- `tests/common/fakes.rs`: the fake `codex` answers `app-server`.

`digest` never calls `recall`.

## Risks / Trade-offs

- [0.55 cuts the weakest gold hits: 6 of 38 in the eval] → A missed note is still one `bilbo recall` away, and the block points there. Precision matters more than recall for injected context. The log shows whether the floor should move.
- [A CPU embedder often misses 1.2 s on long prompts] → The keyword gate takes over, and the log records `ranking: keywords` with the timeout, so the rate is measurable. A second query-only embedder instance on bagend (the fix in `decision-query-embedding-during-indexing.md`) is the dnix-side remedy.
- [The prompt log is sensitive] → Off by default, local only, mode 0600 under the user's state folder, and only the first 500 characters.
- [Reading the whole store on every prompt] → About 100 ms at 6 MiB, measured. A store far beyond 6 MiB needs the daemon, which is a separate change.
- [Codex users who installed the plugin by hand never trusted the hook, and `codex exec` never runs it] → Codex asks at TUI startup; `bilbo setup` trusts it.
- [Setup's trust depends on Codex's app-server, an experimental interface] → A failure is one `hook failed` line and leaves Codex's review in place. See the trust section above.
- [The plugin hook reaches every user of the plugin, whether they want a digest or not] → Without notes passing the gate it prints nothing, and `digest.enable = off` turns it off.
- [A stderr line on every prompt that falls back to keywords] → Neither tool shows stderr on exit 0 outside its transcript view.

## Migration Plan

- The plugin carries the hook, so a bilbo release with this change brings the digest to every machine where `bilbo setup` installed the plugin.
- On rivendell the legacy `nbrecall-project.sh` hook comes from managed settings. Until the cutover moves the notes into bilbo's store, bilbo's store is nearly empty, so its digest prints nothing and the legacy one keeps working. At cutover, dnix drops the legacy hook and sets `programs.bilbo.settings."digest.log" = "on"`. Activation's `bilbo setup` then trusts the hook in Codex when it is given `codex`.
- Rollback: disable the bilbo plugin, or pin the previous release. bilbo keeps nothing the legacy side needs.
