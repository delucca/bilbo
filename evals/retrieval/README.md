# retrieval

Which memory strategy best remembers what happened in real coding sessions?

## Prerequisites

- [Nix](https://nixos.org/download) with flakes enabled.
- To build or check a store: [Docker](https://docs.docker.com/get-started/get-docker/). The shell pins the client,
  Compose and buildx; the engine is yours. A buildx installed by Docker Desktop or OrbStack takes precedence.
- To build a store: an OpenAI-compatible LLM endpoint with `GET /models` and JSON mode. With NVIDIA's
  [container toolkit](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/), the embedder runs on the GPU.
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
needs no LLM. The key hashes what shapes a store: the corpus, the embedder, the LLM and the files under `arms/<arm>/`.
Change any of them and the key changes.

To get a store:

    evals/retrieval/scripts/build-store.sh mem0

Without `BILBO_EVAL_LLM_MODEL`, the script takes the store in `dataset.lock` built from the current corpus, embedder
and arm files, and asks for `BILBO_EVAL_LLM_MODEL` when there are several. A store already in `dataset/` is used as it
is, with a warning when it is not the one in the lock, as after a local build. When there is none, the script builds it,
which needs the LLM:

    BILBO_EVAL_LLM_URL=http://host.docker.internal:8080/v1 BILBO_EVAL_LLM_KEY=... BILBO_EVAL_LLM_MODEL=... \
      evals/retrieval/scripts/build-store.sh mem0

The script runs the arm's services with Docker Compose, feeds the sessions, packs the store and removes the
containers, even after a failure. Run it again after a stop and it resumes from the last point where no session was
open. An LLM usage limit only pauses it, for as long as the endpoint's `Retry-After` asks.

- `BILBO_EVAL_LLM_URL` is the URL the containers see. `host.docker.internal` reaches this machine, but on a Linux
  engine the server must listen on Docker's bridge (`172.17.0.1`) or on every interface, not on loopback alone.
- `BILBO_EVAL_LLM_HEADERS`, a JSON object, adds headers to every LLM call.
- The embedder uses an NVIDIA GPU when Docker reports one. `BILBO_EVAL_GPU=1` or `0` forces it.
- `--rebuild` builds the store again. `--sessions N` builds from the first N sessions only: a trial, kept with the
  build's work files in `~/.cache/bilbo-evals` or `BILBO_EVALS_CACHE`, and never published.
- `--check` then restores the store into fresh services and checks it with `arms/<arm>/check.py`, with no LLM.
- `--key` prints the store's key for the current inputs and stops. `publish.sh` uses it to refuse a stale store.

### mem0

mem0 2.2.1, open source, fed by its own Claude Code plugin (0.3.3, same release). Each session is replayed through the
plugin hook by hook, as Claude Code would have called it, so the plugin decides what to keep and when to send it. All
sessions share one store and one timeline: sessions that overlapped interleave as they did.

What the replay changes:

- The plugin's call to mem0's hosted service becomes two open-source `add` calls, split the way the hosted service
  splits them: project memory under `agent_id` and personal memory under `user_id`, each with the plugin's
  instructions. The repository goes in metadata as `app_id`. The plugin's categories are dropped, since only the
  hosted service reads them.
- Memories and the extraction prompt carry the session's date, not today's.
- When a batch is longer than the embedder's context, mem0's lookup of the memories it already holds embeds only the
  batch's start and end. The LLM still reads all of it.
- The two Cursor transcripts are left out. The plugin never sees them.

Search is mem0's full hybrid search: Qdrant, BM25 through `fastembed` and entity matching through spaCy, with
Qwen3-Embedding-0.6B, bilbo's embedder, on llama.cpp. mem0 only warns when BM25 or entity matching fails to load; the
build stops instead. A store holds a Qdrant snapshot per collection, mem0's SQLite file and a `manifest.json` with the
versions, counts and LLM tokens.

`--check` restores the store into a fresh Qdrant twice. The counts must match the manifest, every memory in mem0's
history must be in Qdrant with the same text, both restores must give the same points, and four searches in each
project's memory and in the personal memory must find memories.

The published store was built with Claude Opus 5.5, through a gateway that runs `claude -p` for each call, and the
embedder on an RTX 5070 Ti: 1,262 LLM calls, 24.1 million prompt tokens and 0.97 million completion tokens, in 2 h 51
min. It holds 2,576 memories.

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
