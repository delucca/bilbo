# Spec Delta

## Purpose

`bilbo digest` is what a prompt hook runs: it finds the few notes that bear on the prompt an agent just received and prints them as a short block the hook injects, so relevant notes reach the agent without the agent asking.

## ADDED Requirements

### Requirement: Hook input
`bilbo digest` SHALL take no arguments and read one JSON object from stdin, using its string fields `session_id` and `prompt` and ignoring every other field. Input that is not such an object, a missing or empty field, or a `session_id` that is not 1 to 128 characters of `A-Z`, `a-z`, `0-9`, `.`, `_` and `-` SHALL produce no digest.

#### Scenario: A Claude Code payload
- **WHEN** stdin is `{"session_id": "abc-123", "prompt": "how do embeddings get cached?", "cwd": "/tmp", "hook_event_name": "UserPromptSubmit"}` and a note passes the gate
- **THEN** stdout holds a digest block and the exit code is 0

#### Scenario: Garbage on stdin
- **WHEN** stdin is `not json`
- **THEN** stdout is empty and the exit code is 0

#### Scenario: A session id with a slash
- **WHEN** `session_id` is `../../etc`
- **THEN** stdout is empty, no file is written outside the cache folder and the exit code is 0

### Requirement: The query
The query SHALL be the prompt with surrounding whitespace trimmed, cut to its first 2,000 bytes at a character boundary. When the prompt starts with `/` or `$` and the rest of its first word is 1 or more characters of `A-Z`, `a-z`, `0-9`, `.`, `_`, `:` and `-`, the sigil SHALL be dropped and the query is that name and the rest of the prompt. When the first word starts with `/` or `$` and is not such a name, there SHALL be no digest.

#### Scenario: A command prompt
- **WHEN** the prompt is `/opsx:apply add-note-recall`
- **THEN** the query is `opsx:apply add-note-recall`

#### Scenario: A path is not a command
- **WHEN** the prompt is `/Users/a/notes/plan.md what is this?`
- **THEN** stdout is empty and the exit code is 0

### Requirement: The gate
With an embedder answering within the budget, a note SHALL pass the gate only when its best passage has a similarity of at least `digest.min_similarity` to the query; sharing words alone SHALL NOT admit it. Otherwise, a note SHALL pass only when one passage holds at least 3 distinct query words of 4 or more letters, in its text or heading path. Words are `recall`'s words.

#### Scenario: Close in meaning
- **WHEN** an embedder answers, the best passage of `decision-note-store.md` has similarity 0.62 to the query and `digest.min_similarity` is 0.55
- **THEN** `decision-note-store.md` passes the gate

#### Scenario: Shared words do not override a weak similarity
- **WHEN** an embedder answers, a passage of `plan-specs.md` holds the query words `work` and `specs`, and its similarity to the query is 0.41
- **THEN** `plan-specs.md` does not pass the gate

#### Scenario: Keyword gate without an embedder
- **WHEN** no embedder is configured, the prompt is `why does the embedder livelock on long chunks`, and one passage holds `embedder`, `livelock` and `chunks`
- **THEN** that note passes the gate

#### Scenario: Two words are not enough without an embedder
- **WHEN** no embedder is configured and the best passage holds only two of the query's 4-letter words
- **THEN** that note does not pass the gate

### Requirement: How many notes
The notes that pass the gate SHALL be ordered as `recall` orders hits. Notes already shown earlier in the same session SHALL be left out. A session's first digest SHALL show at most 6 notes, a later one at most 3, and the whole block SHALL stay within 12,000 bytes, dropping notes from the end to fit. When no note is left to show, stdout SHALL be empty.

#### Scenario: A note is shown once per session
- **WHEN** `gotcha-slots.md` was shown in session `abc` and passes the gate again in session `abc`
- **THEN** the new digest does not list `gotcha-slots.md`

#### Scenario: A later prompt shows fewer notes
- **WHEN** session `abc` already had a digest and 5 notes pass the gate, none shown before
- **THEN** the digest lists 3 notes

#### Scenario: A new session starts fresh
- **WHEN** `gotcha-slots.md` was shown in session `abc` and passes the gate in session `def`
- **THEN** the digest for `def` lists `gotcha-slots.md`

#### Scenario: Nothing passes
- **WHEN** no note passes the gate
- **THEN** stdout is empty and the exit code is 0

### Requirement: The digest block
The block SHALL be, in order: a line `<!-- bilbo digest: <shown> of <left> notes -->`, where `<left>` counts the notes that passed the gate and were not shown before in the session; a line `Notes that may bear on this prompt (open the file to read more):`; one line per note, `- <absolute path>:<line> (<kind>, <created>) <heading path>: <snippet>`, with `recall`'s line, created and snippet; and, when `<left>` exceeds `<shown>`, a line `(<left minus shown> more passed; run bilbo recall for them)`.

#### Scenario: A first digest
- **WHEN** the first prompt of a session passes 8 notes through the gate
- **THEN** the block starts with `<!-- bilbo digest: 6 of 8 notes -->`, lists 6 notes and ends with `(2 more passed; run bilbo recall for them)`

#### Scenario: No overflow line when everything fits
- **WHEN** 2 notes pass the gate in a session's first prompt
- **THEN** the block lists both and has no `more passed` line

### Requirement: Session memory
After printing a block, `bilbo digest` SHALL record the notes it listed in a file named after the session under `<cache folder>/bilbo/sessions/`, where the cache folder is the one `bilbo index` uses. A session's first digest is one with no such file. Each run SHALL delete session files not modified for 30 days.

#### Scenario: The memory file
- **WHEN** session `abc` gets a digest listing two notes
- **THEN** `<cache folder>/bilbo/sessions/abc` exists and names both notes

#### Scenario: Old sessions are cleaned up
- **WHEN** a session file was last modified 31 days ago and any digest runs
- **THEN** that file no longer exists

### Requirement: Time budget
`bilbo digest` SHALL finish within 1.5 seconds of starting when the store is no larger than 6 MiB. It SHALL embed only the first 1,000 bytes of the query, waiting at most for whatever is left of the budget after reading the store and no more than 1.2 seconds. When the embedder does not answer in time, it SHALL use the keyword gate.

#### Scenario: A stalled embedder
- **WHEN** the embedder accepts the connection and never answers, and one passage holds 3 of the query's 4-letter words
- **THEN** the digest lists that note and the run ends within 1.5 seconds

### Requirement: Never fail the prompt
`bilbo digest` SHALL exit 0 on every run. On any error (no store, an unreadable config, an unwritable cache folder or anything else), stdout SHALL be empty and stderr SHALL hold at most one line starting with `bilbo: `.

#### Scenario: No store
- **WHEN** `<root>/notes/` does not exist
- **THEN** stdout is empty and the exit code is 0

#### Scenario: A broken config
- **WHEN** the config file holds an unknown key
- **THEN** stdout is empty, stderr holds one line naming the key and the exit code is 0

### Requirement: The digest log
When `digest.log` is `on`, every run SHALL append one JSON object line to `<state folder>/bilbo/digest.jsonl`, where the state folder is `$XDG_STATE_HOME` when it is an absolute path, else `$HOME/.local/state`. The line SHALL hold `time`, `session`, `prompt` (its first 500 characters), `ranking` (`meaning`, `keywords` or `none`), `passed`, `shown` (the listed paths), `elapsed_ms` and, when the run hit one, `error`. When `digest.log` is not `on`, nothing SHALL be written there.

#### Scenario: An empty digest is explained
- **WHEN** `digest.log` is `on`, the embedder stalls and no note passes the keyword gate
- **THEN** the new log line has `ranking` `keywords`, `passed` 0, an empty `shown` and an `error` naming the embedder timeout

#### Scenario: Logging is off by default
- **WHEN** the config does not set `digest.log` and a digest runs
- **THEN** no file exists under `<state folder>/bilbo/`

### Requirement: Digest leaves the store alone
`bilbo digest` SHALL NOT create, change, rename or delete anything under the store root, and SHALL NOT change the vector cache.

#### Scenario: The store and cache are left as found
- **WHEN** `bilbo digest` runs
- **THEN** every entry under the root and the vector cache file have the same bytes and modification time as before the run
