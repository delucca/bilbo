# retrieval

Which memory strategy best remembers what happened in real coding sessions?

## Prerequisites

- [Nix](https://nixos.org/download) with flakes enabled.
- To build or check a store: [Docker](https://docs.docker.com/get-started/get-docker/). The shell pins the client,
  Compose and buildx; the engine is yours. A buildx installed by Docker Desktop or OrbStack takes precedence.
- To build a store with an LLM (every arm but dense): an OpenAI-compatible LLM endpoint with `GET /models` and JSON
  mode.
- To run the embedder on the GPU: NVIDIA's
  [container toolkit](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/).
- To rebuild the corpus from SWE-chat or to publish: a [Hugging Face token](https://huggingface.co/settings/tokens).
  Rebuilding needs an account that accepted [SWE-chat](https://huggingface.co/datasets/SALT-NLP/SWE-chat)'s terms;
  publishing, write access to the dataset.

## The shell

`flake.nix` and `flake.lock` pin every tool the scripts use, and each script enters the shell on its own. To open it
from the repository root:

    nix develop ./evals/retrieval

## The data

The corpus and the stores are archives in `dataset/`, which git ignores. They are published in the Hugging Face dataset
[deluccadev/bilbo-retrieval](https://huggingface.co/datasets/deluccadev/bilbo-retrieval), and `dataset.lock` pins
each one to its sha256 and the commit that published it. The scripts use an archive already in `dataset/`, else
download the pinned one and check its sha256, else build it.

## The corpus

`dataset/corpus-1235aec0592a.tar.zst` holds 145 real coding sessions from five open-source projects, untouched: one
file per session, grouped by project. 143 are from Claude Code, 2 from Cursor.

To get it:

    evals/retrieval/scripts/fetch-corpus.sh

The archive is checked against the sha256 pinned in the script, which also starts its file name. When it is neither
in `dataset/` nor in `dataset.lock`, the script rebuilds it from SWE-chat at a pinned revision, which needs `HF_TOKEN`.
It is packed the same way every time, so a rebuild gives the same bytes.

## The stores

A store is the memory one strategy builds from the corpus, `dataset/store-<arm>-<key>.tar.zst`, so running the eval
needs no LLM. The key hashes what shapes a store: the corpus, the embedder, the LLM for an arm that uses one, and the
files under `arms/<arm>/`. Change any of them and the key changes.

| Arm | Strategy | LLM | Published store |
|---|---|---|---|
| [mem0](#mem0) | mem0, fed by its Claude Code plugin | yes | built with Claude Opus 5.5 |
| [dense](#dense) | the sessions in chunks, embedded | no | yes |
| [graphiti](#graphiti) | Graphiti's knowledge graph, fed one turn at a time | yes | not yet |
| [ai-memory](#ai-memory) | ai-memory, fed by its Claude Code hooks | yes | not yet |
| [hindsight](#hindsight) | Hindsight, fed by its Claude Code integration | yes | not yet |
| [claude-mem](#claude-mem) | claude-mem, fed by its Claude Code hooks | yes | not yet |

To get a store:

    evals/retrieval/scripts/build-store.sh mem0

Without `BILBO_EVAL_LLM_MODEL`, the script takes the store in `dataset.lock` built from the current corpus, embedder
and arm files, and asks for `BILBO_EVAL_LLM_MODEL` (and `BILBO_EVAL_LLM_SMALL_MODEL`) when there are several. A store
already in `dataset/` is used as it is, with a warning when it is not the one in the lock, as after a local build. When
there is none, the script builds it, which needs the LLM for every arm but dense:

    BILBO_EVAL_LLM_URL=http://host.docker.internal:8080/v1 BILBO_EVAL_LLM_KEY=... BILBO_EVAL_LLM_MODEL=... \
      evals/retrieval/scripts/build-store.sh mem0

The script runs the arm's services with Docker Compose, feeds the sessions, packs the store and removes the
containers, even after a failure. Run it again after a stop and it resumes from the arm's last checkpoint. An LLM usage
limit only pauses it, for as long as the endpoint's `Retry-After` asks.

- `BILBO_EVAL_LLM_URL` is the URL the containers see. `host.docker.internal` reaches this machine, but on a Linux
  engine the server must listen on Docker's bridge (`172.17.0.1`) or on every interface, not on loopback alone.
- `BILBO_EVAL_LLM_HEADERS`, a JSON object, adds headers to every LLM call.
- `BILBO_EVAL_LLM_SMALL_MODEL`, optional, is the model for an arm's lighter calls; only Graphiti reads it. When it
  names a model other than `BILBO_EVAL_LLM_MODEL`, it joins that arm's key and the store's line in `dataset.lock`.
- The embedder uses an NVIDIA GPU when Docker reports one. `BILBO_EVAL_GPU=1` or `0` forces it.
- `--rebuild` builds the store again. `--sessions N` builds from the first N sessions only: a trial, kept with the
  build's work files in `~/.cache/bilbo-evals` or `BILBO_EVALS_CACHE`, and never published.
- `--check` then restores the store into fresh services and checks it with `arms/<arm>/check.py`, with no LLM.
- `--key` prints the store's key for the current inputs and stops. `publish.sh` uses it to refuse a stale store.

Every arm embeds with Qwen3-Embedding-0.6B, bilbo's embedder, on llama.cpp.

### mem0

mem0 2.2.1, open source, fed by its own Claude Code plugin (0.3.3, same release). Each session is replayed through the
plugin hook by hook, as Claude Code would have called it, so the plugin decides what to keep and when to send it. All
sessions share one store and one timeline: sessions that overlapped interleave as they did. The build checkpoints
whenever no session is open.

What the replay changes:

- The plugin's call to mem0's hosted service becomes two open-source `add` calls, split the way the hosted service
  splits them: project memory under `agent_id` and personal memory under `user_id`, each with the plugin's
  instructions. The repository goes in metadata as `app_id`. The plugin's categories are dropped, since only the
  hosted service reads them.
- Memories and the extraction prompt carry the session's date, not today's.
- When a batch is longer than the embedder's context, mem0's lookup of the memories it already holds embeds only the
  batch's start and end. The LLM still reads all of it.
- The two Cursor transcripts are left out. The plugin never sees them.

Search is mem0's full hybrid search: Qdrant, BM25 through `fastembed` and entity matching through spaCy. mem0 only
warns when BM25 or entity matching fails to load; the build stops instead. A store holds a Qdrant snapshot per
collection, mem0's SQLite file and a `manifest.json` with the versions, counts and LLM tokens.

`--check` restores the store into a fresh Qdrant twice. The counts must match the manifest, every memory in mem0's
history must be in Qdrant with the same text, both restores must give the same points, and four searches in each
project's memory and in the personal memory must find memories.

The published store was built with Claude Opus 5.5, through a gateway that runs `claude -p` for each call, and the
embedder on an RTX 5070 Ti: 1,262 LLM calls, 24.1 million prompt tokens and 0.97 million completion tokens, in 2 h 51
min. It holds 2,576 memories.

### dense

The plain baseline: the sessions cut into chunks, each chunk embedded once, with no LLM. Searching it is exact nearest
neighbours over the vectors. The build checkpoints after every session.

A session's text is its main thread, from Claude Code or Cursor: user and assistant messages, tool calls with their
input, and tool results. Thinking and messages the harness injected are left out; compact summaries stay, labelled.
Consecutive messages pack into chunks of at most 512 tokens of the embedder's tokenizer, and a longer message becomes
windows of 512 tokens that overlap by 64.

A store holds `vectors.npy` (unit vectors as float16, row i for line i of `chunks.jsonl`), `chunks.jsonl` and a
`manifest.json`. llama.cpp reuses the start of the previous request, which would make a vector depend on the text
before it, so a one-token sentinel goes before every chunk, and a search must send it before every query. CPU and GPU
vectors differ only by floating-point noise, so the device is not in the key.

`--check` checks that the vectors, the chunks and the manifest agree and that every chunk lies within its session in
the corpus, then embeds five chunks again: each must point the way its stored vector does, at a cosine of at least
0.99.

The published store was built with the embedder on an RTX 5070 Ti, in 12 min: 28,115 chunks from 39,378 messages,
11.5 million tokens.

### graphiti

graphiti-core 0.30.2 on Neo4j 5.26, fed the way Graphiti's authors feed a conversation: each turn of a session is one
`message` episode, `Developer (user): ...` then `Claude Code (assistant): ...` with everything Claude Code said in
answer, at the turn's real time. Episodes are added one after another in the order they happened, across all five
projects, in the one group the Graphiti MCP server calls `main`; each session is one saga. Entity types are the ten
the MCP server ships, and everything else stays at Graphiti's defaults. The graph is the checkpoint: each episode is
written in one transaction, so a rerun skips those already there.

What the replay leaves out: tool calls and their results, subagents, compaction summaries, notices and the two Cursor
transcripts. A turn the user interrupted keeps the reply they saw.

Graphiti's generic OpenAI client is subclassed to send its lighter calls to `BILBO_EVAL_LLM_SMALL_MODEL` when set,
count tokens and wait out a 429. A store holds a `neo4j-admin` dump of the database and a `manifest.json` with the
versions, counts and LLM tokens by model and prompt.

`--check` loads the dump into a fresh Neo4j. The search indexes must come online, the counts must match the manifest,
every saga must start and end with its earliest and latest episode, the stored vectors of 20 entities and 20 facts
must match the embedder's, and four searches with no LLM must find facts and entities.

No store is published yet; it has only been built in trials of up to six sessions.

### ai-memory

[ai-memory](https://github.com/akitaonrails/ai-memory) 2.6.3, fed by its own Claude Code hooks. Each session is
replayed hook by hook into `ai-memory hook`, which spools each event into one `ai-memory serve` in the same container,
as on a developer's machine. All sessions share one store and one timeline, and the build checkpoints whenever no
session is open.

It is set up the way its docs tell a Claude Code user to get the best recall: assistant capture, LLM consolidation at
SessionEnd and at each compaction, and Qwen3's query instruction. Its scheduler reviews one finished session per
project an hour, too slow for a replay, so the build runs `ai-memory auto-improve` once per session itself. The forget
sweep is off, so no session page ages out. The manifest lists every setting that differs from a fresh install, with
the doc or source line that asks for it.

What the replay changes:

- Each event carries the time it happened, which ai-memory keeps for sessions and observations. Pages get the time of
  the build.
- Each project is a minimal git checkout with its GitHub remote, which is all the hook reads to name a project.
- The LLM sits behind a small gateway in the build that waits out a 429 and retries a 5xx. On its own, ai-memory gives
  up after three tries and keeps a rule-based page.
- The two Cursor transcripts are left out. The hooks never see them.

Search is ai-memory's own: SQLite full text, entities and links, plus vectors. A store holds the extracted
`ai-memory backup` (the wiki, `db/memory.sqlite` and `config.toml`) and a `manifest.json`. `ai-memory serve` starts on
a store only with `AI_MEMORY_EMBEDDING_BASE_URL` set.

`--check` reads the store as packed: SQLite's integrity, the counts against the manifest, the wiki file and vector of
every latest page, and up to five pages embedded again. It then serves a copy with no LLM and maintenance off, and
four searches in each project must find pages through the vectors.

No store is published yet; it has only been built in trials of up to six sessions.

### hindsight

[Hindsight](https://github.com/vectorize-io/hindsight) 0.10.3 on Postgres with pgvector, fed by its own Claude Code
integration (`hindsight-coding-agents` 0.8.0, run against a self-hosted server rather than the Cloud it defaults to).
Each session is replayed through the integration's Stop hook, which reads the transcript when a turn ends and sends what
is new; the integration's setup engine configures each repository's bank first. All sessions share one timeline, with
one bank per repository, and the build checkpoints whenever no session is open.

It is set up the way the integration sets up a Claude Code user: its coding missions, a conversation strategy with
12,000-character chunks, knowledge labels, observations in one shared scope and five knowledge pages. Extraction is
verbose, the documented setting for more detail per fact, in place of the integration's default, concise. The five
similarity thresholds Hindsight calibrated for its own embedder are matched to Qwen3's, as its docs ask when the model
changes, and Qwen3's query instruction is on, which also acts at build time when consolidation recalls. Pages refresh
hourly when stale; the build runs that schedule on the sessions' own clock. The manifest lists every setting that
differs from a fresh install, with the doc or source line that asks for it.

What the replay changes:

- Git history, the codebase survey and imports of past sessions are off: they read things outside the corpus. Every
  retain carries the session's time, which the hook takes from the transcript.
- The session-start and prompt hooks are not run: they write no memory. The setup engine runs once per bank.
- A Stop fires where Claude Code's own record of it sits in the transcript, and otherwise where a reply ends. A resumed
  session's copied history is written to the transcript but fires nothing.
- Every session of a repository goes to that repository's bank. Two readaloud sessions start in `/home/noah/projects`
  and `/home/noah`; live, the integration would name banks after those directories.
- After each Stop the build waits until the bank has finished the retain, its consolidation and any page refresh, as the
  minutes before the next prompt allow live.
- The LLM sits behind a small gateway in the build that waits out a 429 and retries a 5xx. On its own, Hindsight retries
  three times with short backoffs, or parks the whole retain and extracts it again later.
- The two Cursor transcripts are left out. The hook never sees them.

Search is Hindsight's recall: semantic, keyword, graph and temporal retrieval fused and reranked by a local
cross-encoder, with no LLM. It is not what the integration gives an agent per prompt, which is an LLM reflect with page
and observation fallbacks; the manifest's `recall.prompt_time` lists that path. A store holds a `hindsight-admin backup`
of the database (facts, observations, entities, links, pages, operations and LLM traces, each with a 100-character
preview of its prompt and answer) and a `manifest.json`.

`--check` restores the backup into a fresh database, rebuilds the per-bank vector indexes and starts the server with no
LLM. The counts, documents, pages and schema version must match the manifest, and three facts per bank must come back
when searched by their own text. Up to 20 memories per bank are embedded again as Hindsight embedded them: observations
and facts whose entity names can be rebuilt exactly must match their stored vectors to a cosine of 0.99, while a fact
whose stored entity names differ from the extracted ones is rebuilt approximately, must reach 0.97 and must still be its
own nearest stored vector.

No store is published yet; it has only been built in trials of up to two sessions.

### claude-mem

[claude-mem](https://github.com/thedotmack/claude-mem) 13.35.0, the npm package pinned by sha256, fed by its own
Claude Code hooks. Each session is replayed hook by hook through the plugin's hook command, which hands each event to
one claude-mem worker in the same container, as on a developer's machine. claude-mem's observer, an LLM, writes
observations from the tool calls and a summary at the end of each turn. All sessions share one store and one timeline,
and the build checkpoints whenever no session is open.

It is set up the way its docs set up a custom OpenAI-compatible observer, with the context window of its default
Claude observer, and Chroma embeds through the shared embedder (`CLAUDE_MEM_CHROMA_EMBEDDING_FUNCTION=openai` with
`OPENAI_BASE_URL`). Everything else stays at its default. The manifest lists every setting that differs from a fresh
install, with the doc or source line behind it.

What the replay changes:

- claude-mem dates everything by the clock when it receives it. A small Bun preload gives the worker and every hook a
  clock that reads the time of the event being replayed, so rows carry the sessions' dates. The observer's 3-minute
  idle timer follows the same clock, so a new observer conversation starts where a real pause would have started one.
- Each event waits until the observer is done with the one before. During a usage limit the build waits for the LLM, as
  mem0's does. Once the LLM's wait has passed, claude-mem's own rate-limit breaker, which counts 90 seconds of the
  replayed clock, may still hold the observer: the clock then steps forward, never past the next event. Inside a burst
  of events there is no room for that, so the observer stays held until the timeline passes the breaker's window, and
  its backlog is then observed in one generation. The manifest counts these waits and steps.
- A transcript line Claude Code wrote again (one session re-writes its earlier history) is written to the transcript
  but fires no hook. The mem0 and ai-memory arms replay those lines.
- When claude-mem drops a batch itself, after a second reply that is not XML, the manifest lists it
  (`dropped_by_claude_mem`) and the build goes on.
- Each project is a minimal git checkout with its GitHub remote, worktrees included, which is all claude-mem reads to
  name a project.
- The LLM sits behind a small gateway in the build that waits out a 429 and retries a 5xx, keeping each call alive;
  when the LLM asks for a wait of more than about a minute it answers with a rate limit, which claude-mem retries for
  free and which ends that session's observer conversation: claude-mem's resume starts a new one, briefed again from
  memory. On its own, claude-mem drops a batch after two failed sends.
- Image-only prompts are sent, as Claude Code sends them, and claude-mem stores `[media prompt]`. The mem0 and
  ai-memory arms skip them.
- The two Cursor transcripts are left out. The hooks never see them.

Search is claude-mem's own: Chroma first, SQLite full text when Chroma finds nothing. It embeds a query like a
document: the `openai` function takes no query instruction, which Qwen3 would take (ai-memory sends one). A store
holds `claude-mem.db`, `chroma/`, `chroma-sync-state.json`, `settings.json` and a `manifest.json`. Serving one needs
`OPENAI_BASE_URL` and `OPENAI_API_KEY` for the embedder, and a search sees rows older than 90 days only when it sends
`dateStart`. claude-mem keeps only the first 4,000 characters of a prompt (`prompt-storage.ts`), so no prompt is
too long for the embedder.

`--check` reads the store as packed: SQLite's integrity and full-text tables, the counts against the manifest, every
row's Chroma documents, and five documents embedded again. It then serves a copy with no LLM, and four searches in each
project must find observations through Chroma.

No store is published yet; it has only been built in trials of up to two sessions.

## Publishing

Only the dataset's owner publishes. The script uploads archives from `dataset/` in one commit, with `dataset-card.md`
as the dataset's card, and writes their lines in `dataset.lock`; then commit `dataset.lock`. Set the token first, so
it stays out of your shell history:

    read -rs HF_TOKEN && export HF_TOKEN
    evals/retrieval/scripts/publish.sh evals/retrieval/dataset/store-mem0-<key>.tar.zst

It refuses a trial, a stale store, or a store whose manifest does not match its name. Building never uploads.

## How it works

1. Each strategy reads the sessions in the order they happened, fed the way the tool normally is, and builds its store
   once.
2. Each project has questions whose answers are somewhere in its sessions.
3. Each strategy searches its memory for every question. Its answers are scored against the answer key, along with
   what the strategy cost.

## Credits

The sessions come from SWE-chat (Baumann et al., *SWE-chat: Real-World AI Coding Sessions in the Wild*, COLM 2026),
released under ODC-BY 1.0.
