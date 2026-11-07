# Memory strategies

An **arm** is one strategy being evaluated. Each directory contains its pinned environment,
store builder, integrity checks, and search adapter.

| Arm | Memory it searches |
|---|---|
| [bm25](bm25/) | Session chunks ranked by keyword matching. |
| [dense](dense/) | The same chunks ranked by embedding similarity. |
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
available. Building bm25, dense, mempalace, and auto-memory needs no LLM; other arms do.
Judging retrieval always uses an LLM.

## Files in an arm

- `compose.yaml` and `Dockerfile` define its environment and services.
- `build.py` creates the store; `check.py` validates it; `search.py` retrieves context.
- A `seed` file, where present, identifies a published store to continue from.

Store keys hash the corpus, embedder, applicable model settings, and arm files, with a few
explicit exclusions including `check.py` and `search.py`. Even an added documentation file
inside an arm changes its key. Search adapters instead contribute to the eval run key.

The [replay reference](../reference.md#the-stores) records each arm’s versions, settings,
search behavior, and deviations from a live installation.
