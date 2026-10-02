# Spec Delta

## Purpose

`bilbo index` embeds the store's passages ahead of time and keeps them in a vector cache, so `recall` can rank by meaning without waiting on the embedder for anything but the query.

## ADDED Requirements

### Requirement: Index the store
`bilbo index` SHALL take the passages of every note `recall` searches, send each passage the cache lacks to the embedder, keep the vectors of passages that still exist, drop the rest, and print one line to stdout, `embedded <n>, kept <n>, dropped <n>`, then exit 0. It takes no arguments.

#### Scenario: A first run
- **WHEN** the store holds two notes with three passages in all, the cache is empty and an agent runs `bilbo index`
- **THEN** the embedder receives three inputs, stdout is `embedded 3, kept 0, dropped 0` and the exit code is 0

#### Scenario: Nothing changed
- **WHEN** `bilbo index` ran and nothing in the store changed since
- **THEN** a second run sends no request to the embedder and prints `embedded 0, kept 3, dropped 0`

#### Scenario: An edited and a deleted note
- **WHEN** one passage of a note was edited and another note was deleted since the last run
- **THEN** the run embeds only the edited passage and drops the vectors of the old passage and of the deleted note's passages

#### Scenario: An extra argument
- **WHEN** an agent runs `bilbo index --rebuild`
- **THEN** bilbo prints a message naming `--rebuild` as unknown to stderr and exits 2

### Requirement: Embedder requests
Each request SHALL be `POST <embedder.url>/v1/embeddings` with a JSON body `{"model": <embedder.model>, "input": [<texts>]}`, an `Authorization: Bearer <token>` header when a token is configured, and at most 16 inputs. Each input is the passage's heading path, a newline and its text, cut to at most 4,000 bytes at a character boundary. A response SHALL be used only when it holds one vector per input, all of one length.

#### Scenario: The input carries the heading path
- **WHEN** a passage under `## Layout` in the note titled `# Note store` holds `One flat folder.`
- **THEN** its input is `Note store > Layout`, a newline and `One flat folder.`

#### Scenario: A malformed answer
- **WHEN** the embedder answers 200 with two vectors for three inputs
- **THEN** bilbo stores none of them, prints a message naming the URL to stderr and exits 1

### Requirement: No embedder configured
When the settings hold no `embedder.url`, `bilbo index` SHALL print `bilbo: no embedder configured; set embedder.url in <config path>` to stderr and exit 1, without reading the store.

#### Scenario: A keyword-only setup
- **WHEN** no config file exists and an agent runs `bilbo index`
- **THEN** stderr names the default config path, stdout is empty and the exit code is 1

### Requirement: Progress survives a failure
When the embedder fails during a run (unreachable, no answer within 10 minutes, a non-2xx status or a malformed answer), `bilbo index` SHALL keep every vector it already received, print the reason to stderr and exit 1. The next run SHALL embed only what is still missing.

#### Scenario: The embedder dies halfway
- **WHEN** a run must embed 100 passages and the embedder stops answering after 64 of them, four full requests
- **THEN** the exit code is 1, and the next run, with the embedder back, prints `embedded 36, kept 64, dropped 0`

### Requirement: The vector cache
The cache SHALL live under `$XDG_CACHE_HOME/bilbo/` when `XDG_CACHE_HOME` is an absolute path, else `$HOME/.cache/bilbo/`, with one cache per store root. It SHALL record the model, and vectors from another model SHALL count as missing. It SHALL be replaced atomically, so a reader sees the old or the new cache, never a partial one. Deleting it SHALL lose nothing that `bilbo index` cannot rebuild.

#### Scenario: A model change re-embeds everything
- **WHEN** the cache holds 3 vectors from `model-a` and `embedder.model` is now `model-b`
- **THEN** `bilbo index` prints `embedded 3, kept 0, dropped 3`

#### Scenario: Two stores do not share vectors
- **WHEN** `bilbo index` runs with `BILBO_HOME=/tmp/a` and then with `BILBO_HOME=/tmp/b`
- **THEN** the second run drops nothing indexed for `/tmp/a`

#### Scenario: A deleted cache
- **WHEN** the cache folder is deleted and `bilbo index` runs
- **THEN** every passage is embedded again and the exit code is 0

### Requirement: Index leaves the store alone
`bilbo index` SHALL NOT create, change, rename or delete anything under the store root. It writes only under the cache folder.

#### Scenario: The store is left as found
- **WHEN** `bilbo index` runs on a store
- **THEN** every entry under the root has the same bytes and modification time as before the run, and no new entry exists

### Requirement: A missing store
When `<root>/notes/` does not exist, `bilbo index` SHALL print `bilbo: no store at <root>` to stderr and exit 1, sending nothing to the embedder.

#### Scenario: Wrong BILBO_HOME
- **WHEN** `BILBO_HOME` names a folder with no `notes/` inside it and an embedder is configured
- **THEN** stderr is `bilbo: no store at <that folder>`, the embedder receives no request and the exit code is 1
