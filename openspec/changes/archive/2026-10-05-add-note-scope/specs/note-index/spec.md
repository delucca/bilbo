# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

### Requirement: Withheld passages
When the host of `embedder.url` is not `localhost`, `127.0.0.1` or `::1`, `bilbo index` SHALL NOT send a passage of a note whose embedder rule (the `note-scope` spec) is `local`, unless a note whose rule is `any` holds an identical passage. It SHALL drop the cached vectors of the passages it withholds. When it withholds any, stderr SHALL be `bilbo: withheld <n> passages from <embedder.url>: their scope allows only a loopback embedder`, where `<n>` counts distinct embedder inputs, an input also held by a note whose rule is `any` counting as sent. The stdout line and exit code SHALL be as without them. A request to a loopback `embedder.url`, from any verb, SHALL connect to it directly and never through a proxy that `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` or their lowercase forms name, since a loopback embedder is what lets a `local` passage be sent.

#### Scenario: A local scope and a remote embedder
- **WHEN** `embedder.url = http://bagend:8081`, `scope.work.embedder = local`, `scope.personal.embedder = any`, the store holds one note with `scope: work` and two passages, and one note with `scope: personal` and one passage, and the cache is empty
- **THEN** the embedder receives one input, stdout is `embedded 1, kept 0, dropped 0`, stderr is `bilbo: withheld 2 passages from http://bagend:8081: their scope allows only a loopback embedder`, and the exit code is 0

#### Scenario: Unassigned notes follow the strictest scope
- **WHEN** `embedder.url = http://bagend:8081`, `scope.work.embedder = local`, and a note with no `scope` key holds one passage
- **THEN** that passage is not sent and is counted in the withheld line

#### Scenario: Text shared with an `any` note is sent once
- **WHEN** `embedder.url = http://bagend:8081`, `scope.work.embedder = local`, `scope.personal.embedder = any`, and the store holds only two notes, one with `scope: work` and one with `scope: personal`, both titled `# Deploy` with the same one paragraph below the title
- **THEN** the embedder receives that input once, stdout is `embedded 1, kept 0, dropped 0`, and stderr is empty

#### Scenario: A loopback embedder gets everything
- **WHEN** `embedder.url = http://127.0.0.1:8737`, `scope.work.embedder = local`, and the store holds notes of every scope and unassigned ones
- **THEN** every passage that holds text is sent, and stderr is empty

#### Scenario: A proxy variable does not reroute a loopback embedder
- **WHEN** `embedder.url = http://127.0.0.1:8737`, `HTTP_PROXY` and `ALL_PROXY` name another server, `NO_PROXY` is unset, and a user runs `bilbo index`
- **THEN** every input reaches the embedder at `127.0.0.1:8737` and the proxy receives no request

#### Scenario: No scope asks for local
- **WHEN** `embedder.url = http://bagend:8081` and no scope sets `embedder = local`
- **THEN** every passage that holds text is sent, and stderr is empty

#### Scenario: A note moves into a local scope
- **WHEN** a note's two passages were embedded by `http://bagend:8081`, then its `scope` became `work`, which sets `embedder = local`, and an agent runs `bilbo index`
- **THEN** stdout reports `dropped 2`, and the cache holds no vector for those passages
