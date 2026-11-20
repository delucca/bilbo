# Memory strategies

An **arm** is one strategy being evaluated. Each directory contains its pinned environment,
store builder, integrity checks, and search adapter.

| Arm | Memory it searches |
|---|---|
| [bm25](bm25/) | Session chunks ranked by keyword matching. |
| [dense](dense/) | The same chunks ranked by embedding similarity. |
| [bilbo-next](bilbo-next/) | Session chunks ranked by bilbo's own keyword search, through the built bilbo. |
| [mem0](mem0/) | Memories extracted through mem0’s Claude Code plugin. |
| [graphiti](graphiti/) | A graph built from conversation turns. |
| [ai-memory](ai-memory/) | Observations and pages written by its hooks and consolidation. |
| [hindsight](hindsight/) | Facts and observations retained by its coding integration. |
| [claude-mem](claude-mem/) | Observations and summaries written by its hook-driven observer. |
| [bilbo-agent-v0_19](bilbo-agent-v0_19/) | Notes a replayed agent chooses to write with bilbo 0.19. |
| [bilbo-assisted-v0_19](bilbo-assisted-v0_19/) | The same, plus a user request to write a note at each session’s end. |
| [mempalace](mempalace/) | Memories collected by plugin hooks and a transcript sweep. |
| [auto-memory](auto-memory/) | Claude Code memory files reconstructed from recorded writes and reads. |

## Run an arm

From the repository root, with the [eval prerequisites](../README.md#run-a-small-eval):

```sh
evals/retrieval/scripts/build-store.sh bm25 --check
evals/retrieval/scripts/eval.sh bm25
```

`build-store.sh` uses a local archive, downloads a pinned one, or builds it if neither is
available. Building bm25, dense, bilbo-next, mempalace, and auto-memory needs no LLM;
other arms do. Judging retrieval always uses an LLM.

## Files in an arm

- `compose.yaml` and `Dockerfile` define its environment and services.
- `build.py` creates the store; `check.py` validates it; `search.py` retrieves context.
- A `seed` file, where present, identifies a published store to continue from.

Store keys hash the corpus, embedder, applicable model settings, and arm files, with a few
explicit exclusions including `check.py` and `search.py`. Even an added documentation file
inside an arm changes its key. Search adapters instead contribute to the eval run key.

The [replay reference](../reference.md#the-stores) records each arm’s versions, settings,
search behavior, and deviations from a live installation.

## bilbo-next's derive step

With `BILBO_EVAL_DERIVE` set (a JSON object; `arms/bilbo-next/build.py` documents its keys) the build of bilbo-next writes
notes between remembering and reindexing: it writes the `derive.*` settings into each store's `config.toml`, runs
`bilbo derive --calls <n> <the repository's entry ids>` while a run makes progress (it wrote a note, or the dry run after it
differs from the one before) until `bilbo derive --dry-run` says `due: 0 entries`, reads the `entry` lines of the dry run (every
entry must be `covered` or `passed over`), and then reindexes. `derive.tries` runs in a row that change nothing end the build.
The reindex must count the sessions plus the note entries. The provider is `anthropic` or `openai`. The way is the setting `derive.<provider>.via`, `api` when the settings name none, and the build writes it into every store's config; `openai` has `api` alone. The endpoint of a provider is an `https` URL, or an `http` URL at a literal loopback address (not a name), with a port that is a number or none, and holds no `@` anywhere (no user name, no password, no secret in its query or fragment). A file `STOP` in `/work/store` stops the loop.

**Lives.** A build is made of lives. A life makes every store from nothing and counts every request's asks from 0. The build
ends on a life in which the provider was asked nothing, and its stores and answers are the archive: a replay of it is that
life again, with the same stores and the same requests in the same order, each given the answer kept for it. A life that asked
and kept something is followed by another; a life that asked and kept nothing ends the build with an error that counts what
came back, since a replay could not repeat it. The price of the last life is one more pass over the kept answers, with no call.

**What is kept, and where.** `<cache>/answers/<arm>/<provider>-<way>/` (mounted at `/answers`) holds, for each request, its answers
as files `<n>` (a status, a line feed, the body) and the file `spent`. A request is named by the upstream's address, its path,
its headers but the key's and `host`, and its body, so two builds find each other's answers exactly when they send the same
bytes; the store's key is not in it. A trial, a full build, a rebuild and a build after a fix of the arm's scripts all find what
was paid for, and **no flag removes the folder**: `--rebuild` removes the work folder alone. An answer's file is synced before
it is renamed and its folder after, before the answer goes back; the `ask` line of `spent` is synced before the ask is sent.
The cache keeps a 200 (a cut answer and an unknown stop reason are 200s and are kept; the engine takes them as either's fault and charges them only when another call of the run is answered) and a 4xx that rejects the request (not 401, 402, 403, 404, 408, 409 or 429), except a 200 whose `status`
is `failed` with `rate_limit_exceeded`, `server_error` or `data_residency_mismatch`; it gives every other response back as it came,
with its `retry-after` and request id headers, and keeps nothing of it. A request's count moves only when an answer is given
back. A body that holds the key is never written and stops the build; no header is ever in a file. A build with a writer behind
removes the kept failures (any status but 200) of the folder of its provider and its way before its first life, since a rejection costs nothing
to ask again and a 4xx is not always about the request.

**One build at a time.** `build.py` takes an exclusive `flock` on `/answers/<provider>-<way>/lock` for its whole life, before it removes the kept failures and until it has copied the last life's answers; a second build of the same arm, provider and way (a trial and a full build have other store keys) says another build is using the answers and exits 1. A kill frees the lock. It holds between containers on one machine, which mount one folder under one kernel; two machines sharing a folder are not held apart.

**Spending.** `spent` holds `ask <name>` for every ask sent to the provider and `run <tokens in> <tokens out> <refused>` for every
run in which the cache asked. The ceilings (`calls`, `tokens`, `refusals`) are held against it, so they hold over every start of
every build through that folder, a trial and the full build together. **A new budget is given by raising the ceilings:** no build
lowers or empties `spent`. A build can pass the ceiling of tokens by one run, at most `calls_per_run` calls, each at most a window,
the earlier notes, the instruction and `derive.answer_tokens`; and the ceiling of refusals by one run's refusals. Every run, with a writer or with none, is asked for `calls_per_run` calls: the allowance is the cache's alone, which asks the provider nothing past the ceiling of calls, so a life that asked nothing runs exactly as a replay runs.
The manifest's `derive` keeps each run of the last life (`runs`: store, repository, exit and derive's nine numbers), each life of
this start (`lives`), `spent` (the folder's sums), `upstream` (the API's address, or the tool's name and version), the `derive.*` lines of `bilbo config` without the values of `headers` and
`environment`, the dry run's `kinds:` line and the totals.

A change of an instruction, of the frame of a request (`view::material`) or, by the way `tool`, of the tool's version changes every request, so answers kept before it no longer match and none is found: a paid build then asks again. An answer kept before the providers were folded is found once its folder has its new name, `anthropic-api` for `anthropic` and `anthropic-tool` for `claude`: a request's name holds no key's name. Give a folder its new name before the first build of the changed arm, and only while no folder of that name is there. A build before the rename finds none of the old answers, and with a writer it pays for each again. Any build that derives makes the folder of its provider and its way, and `mv` puts a folder inside one that is already there: the old answers would then sit one folder too deep, and none would be found.

**The numbers that bound a paid build:** 20 calls a run; ceilings of 1,000 calls, 10,000,000 tokens and 100 refusals;
`cache.timeout` 240 seconds, which must be under the store's `derive.timeout` (300 seconds by default): the cache's own answer then
reaches bilbo before bilbo stops waiting. `BILBO_EVAL_DERIVE` joins the store's key. **The cache alone holds the real key:** `BILBO_EVAL_DERIVE_KEY` goes to the cache's process
(as `BILBO_EVAL_CACHE_KEY`), by the way `tool` from there to the tool it starts, and to nothing else, and a build with a writer behind the cache stops at its start without it. By the way `api`, bilbo gets the
stand-in `replay-key-not-a-secret` in every life and in a replay, in the variable of the setting `derive.<provider>.api.key_env`, else `ANTHROPIC_API_KEY` for
`anthropic` and `OPENAI_API_KEY` for `openai`; the cache puts the real key in its place in the request it sends on (the whole of
`x-api-key`, or what follows `Bearer` alone in `authorization`), and a request that carries the stand-in in neither, or in any other place (another header, the path, the body), is not sent. 

`cache.py` stands at `derive.<provider>.api.endpoint` on `127.0.0.1`, one for the whole life, with a lock for each request. It asks the
provider in a thread of its own, with the standard client and the body read whole, and follows no redirect (a 3xx is given back as it
came and kept nowhere). **The deadline is for the caller:** an ask that is not whole by `cache.timeout` seconds gets the cache's 408 then, whatever
the ask is doing, and is abandoned: what it ends with is dropped and never kept, and the same request is not asked again while it
runs (a 408 and a `busy` line). A response is whole only if the client's own reading of the framing says so (a length it took, or chunks): a body short of it, a length header the client did not take (`-1`, a word) or no framing at all is a 502 and is kept nowhere. A 200 whose body the JSON decoder cannot read, for any reason, is an unreadable 200 and is kept. **The cache looks for the real key as plain bytes, and nowhere else:** in a body before it keeps or gives it back, kept or not, and in a kept answer before it is given back, and in each of the three header values it passes on (`retry-after`, `request-id`, `x-request-id`). A match ends it: nothing is written, nothing of the response is given back (the caller gets the cache's 502), and the build stops. A provider that sent the key back in another form (escaped, split, encoded) would not be seen; bilbo holds only the stand-in, so its own look for a key cannot see the real one either. For
another machine, copy the archive's `store/answers/` over `<cache>/answers/<arm>/` and build with `"cache": {"write": false}`.

**The way `tool`.** The image holds Claude Code 2.1.327, the version bilbo's guards were read from. The store's
`derive.anthropic.tool.program` is a file the build writes, `/tmp/claude`, which runs `tool.py` with the cache's address: it sends the
cache the arguments, the variables and the standard input bilbo started it with, and ends with the exit code and the output the
cache gives back, or with exit 70 and no output when the cache gives none (a miss, the deadline, a reply that holds the token).
bilbo reads that end as an unreadable reply, where it reads the API cache's own 408 as a timeout and its 404 as a stop. The
cache, started with `--tool`, starts the real program (`derive.anthropic.tool.program` of the settings, else the image's `claude`) in the
folder bilbo ran the file in, in a process group it stops when the program ends, with those variables, a home of its own in
place of `HOME`, the token in `CLAUDE_CODE_OAUTH_TOKEN`, and the three variables that keep the tool from updating itself and from
sending telemetry. **`BILBO_EVAL_DERIVE_KEY` is the token of `claude setup-token`**, and bilbo's environment holds no key and no
stand-in; a request that holds the token anywhere is not sent on. A request is named by the tool's name and version (what
`claude --version` prints), its arguments, its variables but `HOME`, `USER`, `PATH` and `TMPDIR`, and its standard input. An
answer's file is a status, a line feed and a JSON object of the exit code and the output. The status is 200 for a refusal, or
an end with exit 0, no error and a stop reason; else the one the API gave the tool (`api_error_status` of the reply), else 500;
and the cache keeps by it what it keeps of an API: a 200, and a 4xx that rejects the request. It gives every other reply back as
it came, keeps nothing of it, and prints the reply's `subtype` and the first 200 characters of its `result` in the build's log,
where a limit of the subscription or a failed login shows. An ask is one start of the tool, whatever the tool asks the API
again inside it. The deadline, the ceilings, `spent`, the lives and the record are the API cache's.

`selftest.py` checks what needs no store and no binary: `uv run python selftest.py` in `arms/bilbo-next`. `moved.py` compares
the chunk lists of two searches (a store with notes searched with `notes_budget` 0, and a store without) with no judge.
