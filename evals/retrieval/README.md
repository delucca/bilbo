# retrieval

Which memory strategy best remembers what happened in real coding sessions?

## Prerequisites

- [Nix](https://nixos.org/download) with flakes enabled.
- To build or check a store: [Docker](https://docs.docker.com/get-started/get-docker/). The shell pins the client,
  Compose and buildx; the engine is yours. A buildx installed by Docker Desktop or OrbStack takes precedence.
- To build a store with an LLM (every arm but dense and mempalace): an OpenAI-compatible LLM endpoint with
  `GET /models` and JSON mode, and tool calls for bilbo's arms. The auto-memory arm needs none: it runs
  `claude -p` with its own login.
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
| [graphiti](#graphiti) | Graphiti's knowledge graph, fed one turn at a time | yes | built with Opus 5.5 and luna |
| [ai-memory](#ai-memory) | ai-memory, fed by its Claude Code hooks | yes | built with Claude Opus 5.5 |
| [hindsight](#hindsight) | Hindsight, fed by its Claude Code integration | yes | not yet |
| [claude-mem](#claude-mem) | claude-mem, fed by its Claude Code hooks | yes | not yet |
| [bilbo-agent-v0_19](#bilbo-019) | notes the agent writes with bilbo's note skill | yes | from L3, Claude Opus 5.5 |
| [bilbo-assisted-v0_19](#bilbo-019) | the same, plus a `/note` at each session's end | yes | from L3, Claude Opus 5.5 |
| [mempalace](#mempalace) | MemPalace, fed by its Claude Code plugin hooks | no | not yet |
| [auto-memory](#auto-memory) | Claude Code's auto memory, written by Claude Code itself | yes | not yet |

To get a store:

    evals/retrieval/scripts/build-store.sh mem0

Without `BILBO_EVAL_LLM_MODEL`, the script takes the store in `dataset.lock` built from the current corpus, embedder
and arm files, and asks for `BILBO_EVAL_LLM_MODEL` (and `BILBO_EVAL_LLM_SMALL_MODEL`) when there are several. A store
already in `dataset/` is used as it is, with a warning when it is not the one in the lock, as after a local build. When
there is none, the script builds it, which needs the LLM for every arm but dense and mempalace:

    BILBO_EVAL_LLM_URL=http://host.docker.internal:8080/v1 BILBO_EVAL_LLM_KEY=... BILBO_EVAL_LLM_MODEL=... \
      evals/retrieval/scripts/build-store.sh mem0

The script runs the arm's services with Docker Compose, feeds the sessions, packs the store and removes the
containers, even after a failure. Run it again after a stop and it resumes from the arm's last checkpoint. An LLM usage
limit only pauses it, for as long as the endpoint's `Retry-After` asks (auto-memory waits for the reset time Claude Code
reports).

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

Every arm but auto-memory embeds with Qwen3-Embedding-0.6B, bilbo's embedder, on llama.cpp.

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

The published store splits the models as Graphiti's library does: Claude Opus 5.5 extracts entities and facts and
merges duplicate entities, and `gpt-6-luna` makes the small calls (fact deduplication and contradiction, dates, entity
attributes and summaries). It was built through a gateway that runs `claude -p` or `codex exec` for each call, with the
embedder on an RTX 5070 Ti: 18,797 LLM calls (2,437 Opus, 16,360 luna), 139 million prompt tokens and 3.0 million
completion tokens, in 11 h 2 min, 1 h 27 min of it waiting out the account's usage limit. It holds 828 episodes, 4,564
nodes and 8,705 facts, 2,069 of them invalidated by later ones.

### ai-memory

[ai-memory](https://github.com/akitaonrails/ai-memory) 2.6.3, fed by its own Claude Code hooks. Each session is
replayed hook by hook into `ai-memory hook`, which spools each event into one `ai-memory serve` in the same container,
as on a developer's machine. All sessions share one store and one timeline, and the build checkpoints whenever no
session is open.

It is set up the way its docs tell a Claude Code user to get the best recall: assistant capture, LLM consolidation at
SessionEnd and at each compaction, and Qwen3's query instruction. Its scheduler reviews one finished session per
project an hour, too slow for a replay, so the build runs `ai-memory auto-improve` once per session itself. Its review
also reads the `decisions/` and `gotchas/` pages it writes, as the docs advise, so it patches a lesson instead of
proposing it again. The forget sweep is off, so no session page ages out. The manifest lists every setting that differs
from a fresh install, with the doc or source line that asks for it.

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

The published store was built with Claude Opus 5.5, through a gateway that runs `claude -p` for each call, and the
embedder on an RTX 5070 Ti: 358 LLM calls, 22.8 million prompt tokens and 1.2 million completion tokens, in 4 h 38
min, 1 h 27 min of it waiting out the account's usage limit. It holds 33,590 observations and 370 pages: 142 session
pages, all consolidated by the LLM, and 228 more from 234 approved auto-improve proposals. kimi-cli's pages are mostly
in Chinese, the language its developer wrote in, since no `_prompts/consolidation.md` pins one.

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

### bilbo 0.19

[bilbo](https://github.com/delucca/bilbo) 0.19.0, the release binary. bilbo keeps what the agent of a session chooses
to write with its `note` skill, and these sessions never had bilbo, so a writer model plays that agent. The writer is
the L3 eval's, copied into `l3/` byte for byte (the manifest lists their sha256). Two arms run it, named for bilbo's
version, since a later bilbo may write and search another way:

- `bilbo-agent-v0_19`: after each exchange, one call asks the model whether the agent would use the note skill now.
  When it says yes, the model runs the skill as shipped, with Bash, Read and Edit, against the real binary.
- `bilbo-assisted-v0_19`: the same, plus a bare `/note` from the user after the last exchange of every session.

Each project is written into its own store, its sessions one after another in the order they started; then the five
are merged into one store, as one developer's bilbo holds every project, and that store is what search reads. A run
sees the skill's `SKILL.md` from the v0.19.0 tag and up to 32k tokens of its session; the yes-or-no call sees 16k.
After each run the build runs `bilbo check`, as the harness does, and `bilbo index`, as bilbo's timer would. The build
checkpoints after every exchange, and a session that stopped half way starts again from its beginning.

What the replay changes:

- An exchange is one prompt and everything up to the next, rendered as L3 rendered it: thinking and system reminders
  are dropped, and each tool input and result is cut to 800 characters. The two Cursor transcripts are skipped before
  rendering.
- The yes-or-no call is the harness's: a real agent decides while it works. Each arm runs its own pass, so a build
  of both draws it twice, where L3 drew one pass per project for both rows.
- Only the note skill is listed, and the prompt hook (`bilbo digest`) is off, since it only reads. After a compaction
  the writer sees the nudge of bilbo's compaction hook.
- The writer can open only bilbo's notes, never the project's files, and runs only the skill's allowed commands. The
  build indexes after every run, where bilbo's timer runs every 15 minutes.
- Sessions that overlapped in one project run one after another, not interleaved.
- A project's writer never saw another project's notes.
- A note's `created` is the exchange's time in UTC, and its `id` is a hash of the exchange, so a replay writes the
  same bytes. bilbo would write the local time and a time-ordered ULID.
- The writer is a simulated agent, so a notes store measures that simulation as much as it measures bilbo.

Search is bilbo's own `bilbo recall`: BM25 over the notes' passages, fused with the embedder's vectors, one hit per
note. Once the stores are merged, the result is indexed in one pass, right after a request that leaves the embedder
nothing to reuse, so its vectors depend only on its notes. A store holds every project's notes in `notes/`, its
vectors in `cache/bilbo/`, the writer's journals and traces in `writer/<project>/`, bilbo's `config` and a
`manifest.json`, which lists each project's notes. bilbo finds a vector file by a hash of the store's path: they were
built at `/work/store`, and a search from another path renames them, as the manifest says.

`--check` runs `bilbo check` on the store, checks the notes, the journals and the manifest against each other,
renders the sessions again from the corpus, and checks that every passage has a unit vector from the embedder. It
then indexes the smallest project's notes again, whose vectors must point the same way at a cosine of at least 0.99.
Four searches, and one made of the title of each project's first note, must run on the vectors; the title search must
find that note.

The published stores were not built here. They are the bilbo stores of the L3 eval, written there on 2026-10-09 and
10 with Claude Opus 5.5, and this arm's build resumed from them with no LLM call. Each project was written on its own,
then the five were merged into the one store and indexed with this eval's embedder.

How they were written, as the L3 records show:

- The L3 gateway ran `claude -p` for each call. It flattened every chat into one text prompt, and the model answered
  with one JSON object that named its tool call, so the tool loop is a simulation of one over that gateway.
- The two stores share L3's one yes-or-no pass per project. A fresh build of both arms draws two.
- The writer's own searches ran on L3's embedder, llama.cpp b9190, on a GPU for four projects and a CPU for readaloud,
  not on this eval's. The final vectors come from this eval's embedder.
- readaloud is L3's dev project, where the 800-character cap was chosen. Its rows came from the same frozen code.
- `ingest_seconds` is L3's wall time, usage-limit pauses included, not the time of this resume.
- `versions` describes the image that packed the stores. L3 wrote with the x86_64 release binary of bilbo 0.19.0
  (sha256 in each project's provenance).

A replay of the gateway's records, with each `bilbo recall` answered as it was then, regenerated every request behind
the stores from this arm's writer (`l3/`) and this corpus, 3,252 for bilbo-agent-v0_19 and 3,772 for
bilbo-assisted-v0_19, and every note byte for byte. bilbo-agent-v0_19 holds 106 notes and bilbo-assisted-v0_19 120,
and the sessions written are 143 of 145: the two Cursor transcripts are left out.

### mempalace

[MemPalace](https://github.com/MemPalace/mempalace) 3.10.0, fed by its own Claude Code plugin hooks. Each session is
replayed hook by hook into `mempalace hook run`, as Claude Code calls the plugin's Stop, PreCompact and SessionEnd
hooks, with the transcript as Claude Code had written it by then. The hooks decide what to keep: every 15 `user`
records of the transcript (tool results count) and at the end they write a diary checkpoint, and with each save, and
before a compaction, they mine the whole transcript again into the `sessions` wing, in drawers of up to 800
characters. All sessions share one palace and one timeline, and the build checkpoints whenever no session is open.

On top of the hooks it runs `mempalace sweep`, which MemPalace's README recommends "periodically" for per-message
recall: at each quiet point, once for every transcript that ended since the last one. It stores one drawer per `user`
and `assistant` message, whole, with no wing and no room, so a search filtered by wing does not see them.

It is set up the way its docs set up a Claude Code user: the marketplace plugin with its defaults, the embedder swapped
for the eval's through MemPalace's OpenAI-compatible embedding setting, and no LLM, since the plugin's silent save
mode, the default, computes everything from the transcript. The plugin's `/mempalace:init` is not run: it writes project
entity files and config defaults that neither mining nor search reads. The manifest lists every setting that differs
from a fresh install, with the doc or source line that asks for it.

What the replay changes:

- Each hook runs under libfaketime at the time its event happened, and each sweep at the time of the last event
  before its quiet point, so the dates MemPalace files (`filed_at`, the diary checkpoints) are the session's, in UTC,
  not the build's. `authored_at` is MemPalace's own: the transcript's latest timestamp.
- The plugin's mine runs detached in the background on a developer's machine; the build waits for each one, so no save
  is ever skipped because an earlier mine of the transcript is still running.
- An input longer than the embedder's context goes to the sweep's embedder cut to its first tokens, as MemPalace's own
  embedders cut at 2,048 or 256 tokens, and the drawer keeps the whole message. The manifest says where the limit is
  and how many inputs were cut. The hooks, their mines and the diary are far under the context and are not cut.
- The sweep waits up to an hour for each embeddings request instead of MemPalace's 120 s, which a CPU embedder needs.
  The full store is meant to be built on a GPU: the sweeps alone would take about 52 hours on a laptop CPU.
- The embedder identity is recorded with `mempalace palace set-embedder`, as MemPalace asks on every open of a palace
  its hooks created.
- Each session ends once, at its last line. A session resumed after an exit does not get the SessionEnd that exit
  fired (12 resumes in 9 sessions), so a few diary checkpoints and mines are missing; the final drawers are the same.
- The two Cursor transcripts are left out. The hooks never see them.
- What a live agent saves through MemPalace's MCP tools is not reproduced: the corpus agents never had it, so the
  knowledge graph stays empty. `mempalace mine --extract general` is off: it adds drawers beside the exchange ones and
  the hooks never run it.

What MemPalace keeps from a transcript is its own. Before chunking, its normalizer drops thinking blocks, omits the
results of Read, Edit and Write, keeps the first and last 20 lines of Bash output and 20 Grep or Glob matches, cuts
any other tool result to 2,048 characters and turns each tool call into one line. Its chunker drops the lines after a
`---` rule inside a turn, and cuts a transcript with fewer than three lines starting with `>` by paragraph instead of
by exchange. The sweep keeps each message whole, with tool inputs and results in full.

Search is `search_memories`, MemPalace's blend of cosine similarity and BM25 over the top vector candidates. A store
holds the palace directory (ChromaDB), `hallways.json`, the config directory with MemPalace's write log and a
`manifest.json` with the versions, settings and counts. Open it with the same `MEMPALACE_EMBEDDING_*` settings as
the build: MemPalace refuses a palace under another embedder kind, but opens it silently under another model id.

`--check` opens the store with the embedder and no LLM. The embedding function must be MemPalace's OpenAI-compatible
one with unit vectors of 1,024 dimensions, and the counts by kind, wing and room must match the manifest. The sessions
must be the first ones of the corpus, each with the chunks MemPalace's own normalizer and chunker make of its
transcript, dated with its latest timestamp and filed within its time, and with one sweep drawer per message. Every
drawer must carry the metadata MemPalace writes. Up to five transcript drawers, a diary drawer and two sweep drawers
(one cut, when the build cut any) embedded again must agree with their stored vectors at a cosine of at least 0.99.
Four searches in the `sessions` wing, in each diary wing and in none must find drawers, and an unfiltered search must
return a sweep drawer.

No store is published yet; it has only been built in a trial of two sessions.

### auto-memory

Claude Code 2.1.296's own auto memory, written by Claude Code itself. Each session is replayed by resuming a copy of
its transcript headless, at the points where Claude Code would have reviewed it (its end, and before each compaction),
with one closing message; the model and Claude Code's background extraction then write `MEMORY.md` and topic files
under the project's memory directory, the same name the developer's machine used, under the build's config directory.
Every later point of that project starts with that memory: `MEMORY.md` is loaded, and topic files come through Read or
the recall selector. Points run in the order they happened, and each is a checkpoint.

What the replay changes:

- The memory the developer's Claude Code wrote mid-session is applied to the memory directory before the point it
  falls in: successful `Write`, `Edit` and `MultiEdit` calls on a memory file and the Bash writes of three known
  shapes, as the tool input had them. A recorded command is never run, an `Edit` whose text is absent is skipped, and no
  `Read` seeds the start, so each project's memory starts empty, as for every arm.
- The closing message, `let's pause here for a bit`, is the replay's; the developer said nothing there. At a point
  that ends mid-task or on a question, the model is asked to pause there. If the model saves memory itself in answer
  to it, Claude Code skips the background review of that point; the manifest counts those points.
- The checkout is empty, and without the project's `CLAUDE.md` or `AGENTS.md`, so the model writes memory from the
  transcript alone.
- Claude Code reviews a point once, not after every turn of the session.
- The model is told the build's date, not the session's, so a relative date in a session resolves against it, and the
  memory dates are the build's.
- Auto dream, Claude Code's periodic consolidation, never runs under `claude -p`, so it never runs here.
- With an OAuth login Claude Code puts the account's email address in the context. After each point the build scans
  the memory directories the point touched and its record for the account's identifiers, and the whole store again
  before it is written; on a hit it stops and packs nothing.
- Files Claude Code writes in the build carry the build's time and its own frontmatter stamp. Applied recorded files
  have neither stamp: their content is as their authors' sessions wrote it.
- The two Cursor transcripts are left out. Claude Code never saw them.

The build runs with the account's own feature flags and stops unless the background extraction runs under `claude -p`
on every turn; the manifest records them. Auth comes from exactly one of `CLAUDE_CODE_OAUTH_TOKEN` or
`ANTHROPIC_API_KEY` in the caller's environment; the model is `BILBO_EVAL_LLM_MODEL`, and `BILBO_EVAL_LLM_URL` and
`_KEY` are not read. Claude Code waits out a usage limit itself (`CLAUDE_CODE_RETRY_WATCHDOG`), and the build stops on
an error no timer cures, such as a request that needs usage credits.

A store holds each project's memory directory as Claude Code left it, `points.jsonl` with one record per point and a
`manifest.json` with the versions, flags, counts and LLM usage, the background extraction's input and cache tokens
included (its output tokens are not reported locally). The replay's own
transcript entries stay in the work folder. Search is whatever the run step gives the model: `MEMORY.md` is loaded up
to its first 200 lines or 25,000 characters, and topic files are read on demand.

`--check` reads the store as packed, with no LLM: every memory file against the manifest's hashes, `MEMORY.md`'s size
and links and the topic files' frontmatter against the manifest's numbers, and the per-point records against the counts.

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
