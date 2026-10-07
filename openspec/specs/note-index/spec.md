# note-index Specification

## Purpose
`bilbo index` embeds the store's passages ahead of time and keeps them in a vector cache, so `recall` can rank by meaning without waiting on the embedder for anything but the query.

## Requirements

### Requirement: Index the store
`bilbo index` SHALL take the passages that hold text, of every note `recall` searches, except those Withheld passages names, send each passage the cache lacks to the embedder, keep the vectors of passages that still exist, drop the rest, and print one line to stdout, `embedded <n>, kept <n>, dropped <n>`, then exit 0. Identical passages, with the same heading path and text, SHALL be sent and counted once. When nothing is embedded or dropped, the cache SHALL be left unchanged. It takes no arguments.

#### Scenario: A first run
- **WHEN** the store holds two notes with three passages that hold text in all, the cache is empty and an agent runs `bilbo index`
- **THEN** the embedder receives three inputs, stdout is `embedded 3, kept 0, dropped 0` and the exit code is 0

#### Scenario: A heading without text
- **WHEN** a note's title heading has no text before its first `##` section
- **THEN** `bilbo index` sends no input for the title alone; the title still reaches the embedder in the heading path of every other passage of the note

#### Scenario: Nothing changed
- **WHEN** `bilbo index` ran and nothing in the store changed since
- **THEN** a second run sends no request to the embedder, prints `embedded 0, kept 3, dropped 0` and leaves the cache file's bytes and modification time unchanged

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

### Requirement: Withheld passages
When the host of `embedder.url` is not `localhost`, `127.0.0.1` or `::1`, `bilbo index` SHALL NOT send a passage of a note whose embedder rule (the `note-scope` spec) is `local`, unless a note whose rule is `any` holds an identical passage. It SHALL drop the cached vectors of the passages it withholds. When it withholds any, stderr SHALL be `bilbo: withheld <n> passages from <embedder.url>: their scope allows only a loopback embedder`, where `<n>` counts distinct embedder inputs, an input also held by a note whose rule is `any` counting as sent. The stdout line and exit code SHALL be as without them. A request to a loopback `embedder.url`, from any verb, SHALL connect to it directly and never through a proxy that `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` or their lowercase forms name, since a loopback embedder is what lets a `local` passage be sent.

#### Scenario: A local scope and a remote embedder
- **WHEN** `embedder.url = http://embedder.example:8081`, `scope.work.embedder = local`, `scope.personal.embedder = any`, the store holds one note with `scope: work` and two passages, and one note with `scope: personal` and one passage, and the cache is empty
- **THEN** the embedder receives one input, stdout is `embedded 1, kept 0, dropped 0`, stderr is `bilbo: withheld 2 passages from http://embedder.example:8081: their scope allows only a loopback embedder`, and the exit code is 0

#### Scenario: Unassigned notes follow the strictest scope
- **WHEN** `embedder.url = http://embedder.example:8081`, `scope.work.embedder = local`, and a note with no `scope` key holds one passage
- **THEN** that passage is not sent and is counted in the withheld line

#### Scenario: Text shared with an `any` note is sent once
- **WHEN** `embedder.url = http://embedder.example:8081`, `scope.work.embedder = local`, `scope.personal.embedder = any`, and the store holds only two notes, one with `scope: work` and one with `scope: personal`, both titled `# Deploy` with the same one paragraph below the title
- **THEN** the embedder receives that input once, stdout is `embedded 1, kept 0, dropped 0`, and stderr is empty

#### Scenario: A loopback embedder gets everything
- **WHEN** `embedder.url = http://127.0.0.1:8737`, `scope.work.embedder = local`, and the store holds notes of every scope and unassigned ones
- **THEN** every passage that holds text is sent, and stderr is empty

#### Scenario: A proxy variable does not reroute a loopback embedder
- **WHEN** `embedder.url = http://127.0.0.1:8737`, `HTTP_PROXY` and `ALL_PROXY` name another server, `NO_PROXY` is unset, and a user runs `bilbo index`
- **THEN** every input reaches the embedder at `127.0.0.1:8737` and the proxy receives no request

#### Scenario: No scope asks for local
- **WHEN** `embedder.url = http://embedder.example:8081` and no scope sets `embedder = local`
- **THEN** every passage that holds text is sent, and stderr is empty

#### Scenario: A note moves into a local scope
- **WHEN** a note's two passages were embedded by `http://embedder.example:8081`, then its `scope` became `work`, which sets `embedder = local`, and an agent runs `bilbo index`
- **THEN** stdout reports `dropped 2`, and the cache holds no vector for those passages

### Requirement: Human view of index
When stdout gets the `cli` spec's human view, `bilbo index` SHALL print `Embedded <n> passages`, `passage` for one, with the count in bold, then dim ` · kept <n> · dropped <n>`, numbers grouped by thousands, after `◆` when it embedded or dropped a passage and `◇` otherwise. Off a terminal it SHALL print the plain line, after the time the `cli` spec's Times in service logs requirement puts before it.

#### Scenario: New passages on a terminal
- **WHEN** a user runs `bilbo index` in a terminal and it embeds 49 passages and keeps 4,620
- **THEN** stdout is `◆  Embedded 49 passages · kept 4,620 · dropped 0`

#### Scenario: The timer's log keeps the line
- **WHEN** the timer runs `bilbo index` with stdout appended to its log
- **THEN** the log gains a line of the time, a space and `embedded 49, kept 4620, dropped 0`

#### Scenario: The journal gets the bare line
- **WHEN** systemd runs `bilbo index` with stdout to the journal and it embeds 49 passages and keeps 4,620
- **THEN** stdout is `embedded 49, kept 4620, dropped 0`
