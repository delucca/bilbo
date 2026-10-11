# retrieval

Which memory strategy best remembers what happened in real coding sessions?

## Prerequisites

- [Nix](https://nixos.org/download) with flakes enabled. Every script runs in the eval's pinned shell.
- [Docker](https://docs.docker.com/get-started/get-docker/), which runs each strategy's services. The shell pins the
  Docker client; the engine is the host's.
- An OpenAI-compatible LLM endpoint, only to rebuild a store. Optionally, NVIDIA's
  [container toolkit](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/), so a rebuild embeds on the
  GPU.
- A [Hugging Face token](https://huggingface.co/settings/tokens), only to rebuild the corpus. Its account must accept
  [SWE-chat](https://huggingface.co/datasets/SALT-NLP/SWE-chat)'s terms.

## The shell

`flake.nix` and `flake.lock` pin the tools every script uses. From the repository root, open the shell with:

    nix develop ./evals/retrieval

or run one command in it:

    nix develop ./evals/retrieval --command evals/retrieval/scripts/fetch-corpus.sh

A script started outside the shell enters it on its own.

## The corpus

`dataset/corpus-1235aec0592a.tar.zst` holds 145 real coding sessions from five open-source projects, untouched:
one file per session, grouped by project, sorted by time. 143 are Claude Code's and 2 are Cursor's.

To check it, or rebuild it from SWE-chat:

    HF_TOKEN=hf_... evals/retrieval/scripts/fetch-corpus.sh

The script downloads each session at a pinned dataset revision and packs them the same way every time. Before it
finishes, it checks the archive against the sha256 pinned in the script; the file name starts with that hash. If the
archive is already there and valid, it says so and exits, with no token needed.

## The stores

A store is the memory one strategy builds from the corpus. It is committed as `dataset/store-<arm>-<key>.tar.zst`, so
a run needs no LLM. The key is a hash of what shapes the store: the corpus, the embedder's weights, the LLM, and every
file under `arms/<arm>/`. A changed file gives a new key, so a store is never overwritten by a different one.

To build one, or check it is there:

    BILBO_EVAL_LLM_URL=http://host.docker.internal:8080/v1 BILBO_EVAL_LLM_KEY=... BILBO_EVAL_LLM_MODEL=... \
      evals/retrieval/scripts/build-store.sh mem0

The script starts the arm's services with Docker Compose, feeds the sessions, packs the store and removes the
containers, failed or not. The LLM URL is the one the containers see: `host.docker.internal` reaches a server on this
machine. `BILBO_EVAL_LLM_HEADERS`, a JSON object, adds headers to every LLM call for a gateway that needs them.
The embedder runs on an NVIDIA GPU when Docker reports one and on the CPU otherwise; `BILBO_EVAL_GPU=1` or `0` forces
it. A build that stops resumes, when run again, from the last moment no session was open, and `--rebuild` starts it
over. `--sessions N` builds from the first N sessions only, a trial kept in the cache (`~/.cache/bilbo-evals`, or
`BILBO_EVALS_CACHE`) with the build's work files.

### mem0

mem0 2.2.1, open source, fed by its own Claude Code plugin (0.3.3, from the same release): each session is replayed
through the plugin as Claude Code's hooks would have called it, prompt by prompt, tool result by tool result, with the
transcript file growing line by line. The plugin decides what to keep and when to send it, as it does in real use. The
only change is where it sends: its request to mem0's hosted service goes to open-source mem0 as two `add` calls, one
for the project's memory (`agent_id`, with the plugin's project instructions) and one for the developer's
(`user_id`, with its personal instructions), as the hosted service splits them. The repository goes in metadata as
`app_id`, which open-source mem0 lacks as a field, and the plugin's categories, which only the hosted service reads,
are dropped. One store holds all five projects, and the sessions are replayed in one timeline, so the ones open at
the same time interleave as they happened. The two Cursor transcripts in the corpus are left out: the plugin never
sees them.

Each memory carries its session's time, and so does the extraction prompt, which mem0 would otherwise date today.
When a batch is longer than the embedder's context, only mem0's lookup of the memories it already holds embeds the
batch's start and end; the LLM reads all of it. mem0 runs its full hybrid search: Qdrant, BM25 through `fastembed` and
entity matching through spaCy, with Qwen3-Embedding-0.6B (the embedder bilbo uses) on llama.cpp. The build stops if
BM25 or entity matching would be off, which mem0 itself only warns about. The store holds a Qdrant snapshot per
collection, mem0's SQLite file, and `manifest.json` with the versions, counts and LLM tokens.

## How it works

1. Each strategy reads the sessions in the order they happened, the way the tool is normally fed, and builds its store
   once.
2. Each project has questions whose answers are somewhere in its sessions.
3. Each strategy searches its memory for every question, and its answers are scored against the answer key, along with
   what the strategy cost.

## Credits

The sessions come from SWE-chat (Baumann et al., *SWE-chat: Real-World AI Coding Sessions in the Wild*, COLM 2026),
released under ODC-BY 1.0.
