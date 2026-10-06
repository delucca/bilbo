# note-digest Specification

## Purpose
`bilbo digest` is what a prompt hook runs: it finds the few notes that bear on the prompt an agent just received and prints them as a short block the hook injects, so relevant notes reach the agent without the agent asking.

## Requirements

### Requirement: Hook input
`bilbo digest` SHALL take no arguments and read one JSON object from stdin, using its string fields `session_id` and `prompt` and ignoring every other field. Input that is not such an object, a missing or empty field, or a `session_id` that is not 1 to 128 characters of `A-Z`, `a-z`, `0-9`, `.`, `_` and `-`, or is `.` or `..`, SHALL produce no digest.

#### Scenario: A Claude Code payload
- **WHEN** stdin is `{"session_id": "abc-123", "prompt": "how do embeddings get cached?", "cwd": "/tmp", "hook_event_name": "UserPromptSubmit"}` and a note passes the gate
- **THEN** stdout holds a digest block and the exit code is 0

#### Scenario: A Codex payload
- **WHEN** stdin is `{"session_id": "01a102b8-7eb6-7a50-b5e7-e58145afdffd", "turn_id": "t1", "prompt": "how do embeddings get cached?", "model": "m", "hook_event_name": "UserPromptSubmit"}` and a note passes the gate
- **THEN** stdout holds a digest block and the exit code is 0

#### Scenario: Garbage on stdin
- **WHEN** stdin is `not json`
- **THEN** stdout is empty and the exit code is 0

#### Scenario: A session id with a slash
- **WHEN** `session_id` is `../../etc`
- **THEN** stdout is empty, no file is written outside the cache and state folders, and the exit code is 0

### Requirement: The digest switch
When `digest.enable` is `off`, `bilbo digest` SHALL read and ignore stdin, print nothing, write no session file and no log line, delete nothing, and exit 0.

#### Scenario: The digest is off
- **WHEN** the config holds `digest.enable = off` and `digest.log = on`, and a hook sends a prompt that a note matches
- **THEN** stdout and stderr are empty, no file exists under `<cache folder>/bilbo/sessions/` or `<state folder>/bilbo/`, and the exit code is 0

#### Scenario: The digest is on by default
- **WHEN** the config does not set `digest.enable` and a note passes the gate
- **THEN** stdout holds a digest block

### Requirement: The query
The query SHALL be the prompt with surrounding whitespace trimmed, cut to its first 2,000 bytes at a character boundary. When the prompt starts with `/` or `$` and the rest of its first word is 1 or more characters of `A-Z`, `a-z`, `0-9`, `.`, `_`, `:` and `-`, the sigil SHALL be dropped and the query is that name and the rest of the prompt. When the first word starts with `/` or `$` and is not such a name, there SHALL be no digest.

#### Scenario: A command prompt
- **WHEN** the prompt is `/review:pr fix-login-timeout`
- **THEN** the query is `review:pr fix-login-timeout`

#### Scenario: A path is not a command
- **WHEN** the prompt is `/Users/a/notes/plan.md what is this?`
- **THEN** stdout is empty and the exit code is 0

### Requirement: The gate
With an embedder answering within the budget, a note SHALL pass the gate when one of its passages has a similarity of at least `digest.min_similarity` to the query, or when one of its passages that `bilbo index` withholds, under the `note-index` spec's Withheld passages, passes the keyword gate below. Sharing words alone SHALL NOT admit any other passage, and a passage `bilbo index` has not embedded for another reason cannot pass. Otherwise, a note SHALL pass only when one passage holds at least 3 distinct query words of 4 or more letters or digits, in its text or heading path. Words are `recall`'s words. A configured embedder that fails, misses the budget, or finds no passage indexed SHALL leave the keyword gate in charge. When `bilbo index` withholds every passage, the digest SHALL send no query and leave the keyword gate in charge without an error.

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

#### Scenario: Nothing indexed yet
- **WHEN** an embedder is configured, the vector cache is empty, and one passage holds 3 of the query's 4-letter words
- **THEN** that note passes the gate, and no request reaches the embedder

#### Scenario: A withheld note passes on keywords
- **WHEN** `embedder.url = http://embedder.example:8081` answers, `scope.work.embedder = local`, `scope.personal.embedder = any`, a note with `scope: personal` is embedded, `bilbo index` ran after the last change, the prompt is `why does the deploy pipeline stall on staging`, and a note with `scope: work` has a passage holding `deploy`, `pipeline` and `staging`
- **THEN** that note passes the gate

#### Scenario: A withheld note needs three words
- **WHEN** the same setup holds, and the work note's best passage holds only `deploy` and `staging`
- **THEN** that note does not pass the gate

#### Scenario: Every passage withheld
- **WHEN** `embedder.url = http://embedder.example:8081`, `scope.work.embedder = local`, every note has `scope: work`, and one passage holds 3 of the query's 4-letter words
- **THEN** that note passes the gate, no request reaches the embedder, and stderr is empty

#### Scenario: An unembedded note in no local scope still needs meaning
- **WHEN** `embedder.url = http://embedder.example:8081` answers, no scope sets `embedder = local`, some passages are embedded, and a note written after the last `bilbo index` has a passage holding 3 of the query's 4-letter words
- **THEN** that note does not pass the gate

### Requirement: How many notes
The notes that pass the gate SHALL be ordered as `recall` orders hits. Notes already shown earlier in the same session SHALL be left out. A session's first digest SHALL show at most 6 notes, a later one at most 3, and the whole block SHALL stay within 9,000 bytes, dropping notes from the end to fit. When no note is left to show, stdout SHALL be empty, unless the Sync state in the digest requirement raises open conflicts.

#### Scenario: A note is shown once per session
- **WHEN** `gotcha-slots.md` was shown in session `abc` and passes the gate again in session `abc`
- **THEN** the new digest does not list `gotcha-slots.md`

#### Scenario: A later prompt shows fewer notes
- **WHEN** session `abc` already had a digest and 5 notes pass the gate, none shown before
- **THEN** the digest lists 3 notes

#### Scenario: A new session starts fresh
- **WHEN** `gotcha-slots.md` was shown in session `abc` and passes the gate in session `def`
- **THEN** the digest for `def` lists `gotcha-slots.md`

#### Scenario: A block too large for 6 notes
- **WHEN** the first prompt of a session passes 6 notes whose lines take about 2,000 bytes each
- **THEN** the block lists 4 notes, starts with `<!-- bilbo digest: 4 of 6 notes -->` and is no larger than 9,000 bytes

#### Scenario: Nothing passes
- **WHEN** no note passes the gate and no note has an open conflict
- **THEN** stdout is empty and the exit code is 0

### Requirement: The digest block
The block SHALL be, in order: a line `<!-- bilbo digest: <shown> of <left> notes -->`, where `<left>` counts the notes that passed the gate and were not shown before in the session; a line `Notes that may bear on this prompt (open the file to read more):`; one line per note, `- <absolute path>:<line> (<kind>, <created>) <heading path>: <snippet>`, with `recall`'s line, created, heading path and snippet, and the parenthesis extended with a label as Sync state in the digest says; when `<left>` exceeds `<shown>`, a line `(<left minus shown> more passed; run bilbo recall for them)`; and the `Sync conflicts wait in` line when that requirement adds it.

#### Scenario: A first digest
- **WHEN** the first prompt of a session passes 8 notes through the gate
- **THEN** the block starts with `<!-- bilbo digest: 6 of 8 notes -->`, lists 6 notes and ends with `(2 more passed; run bilbo recall for them)`

#### Scenario: No overflow line when everything fits
- **WHEN** 2 notes pass the gate in a session's first prompt
- **THEN** the block lists both and has no `more passed` line

### Requirement: Session memory
After choosing a block, `bilbo digest` SHALL record the notes it lists, and whether it raised sync conflicts, in a file named after the session under `<cache folder>/bilbo/sessions/`, where the cache folder is the one `bilbo index` uses, before printing the block. A session's first digest is one with no such file. Each run with the digest on SHALL delete files in that folder not modified for 30 days.

#### Scenario: The memory file
- **WHEN** session `abc` gets a digest listing two notes
- **THEN** `<cache folder>/bilbo/sessions/abc` exists and names both notes

#### Scenario: Nothing shown, nothing remembered
- **WHEN** no note passes the gate in session `abc` and no note has an open conflict
- **THEN** `<cache folder>/bilbo/sessions/abc` does not exist, and the next digest in `abc` is still its first

#### Scenario: Raised conflicts are remembered
- **WHEN** no note passes the gate in session `abc`, and its first digest prints only the `Sync conflicts` line
- **THEN** `<cache folder>/bilbo/sessions/abc` exists, and the next digest in `abc` is not its first

#### Scenario: Old sessions are cleaned up
- **WHEN** a session file was last modified 31 days ago and any digest runs
- **THEN** that file no longer exists

### Requirement: Time budget
`bilbo digest` SHALL finish within 1.5 seconds of starting when the store is no larger than 6 MiB. It SHALL embed `embedder.query_prefix` and only the first 1,000 bytes of the query, waiting at most for whatever is left of the budget after reading the store and no more than 1.2 seconds. When the embedder does not answer in time, it SHALL use the keyword gate.

#### Scenario: A stalled embedder
- **WHEN** the embedder accepts the connection and never answers, and one passage holds 3 of the query's 4-letter words
- **THEN** the digest lists that note and the run ends within 1.5 seconds

#### Scenario: A long prompt
- **WHEN** the prompt is 5,000 bytes long
- **THEN** the embedder receives `embedder.query_prefix` followed by the prompt's first 1,000 bytes

### Requirement: Never fail the prompt
`bilbo digest` SHALL exit 0 on every run. On an error that stops the digest (bad input, no store, an unreadable config, an unwritable cache folder or anything else that is not the embedder), stdout SHALL be empty. An embedder that fails or misses the budget SHALL NOT empty stdout; it only switches to the keyword gate. Whenever the run hits an error, stderr SHALL hold one line starting with `bilbo: ` that names it; otherwise stderr SHALL be empty.

#### Scenario: No store
- **WHEN** `<root>/notes/` does not exist
- **THEN** stdout is empty, stderr holds one line naming the root, and the exit code is 0

#### Scenario: A broken config
- **WHEN** the config file holds an unknown key
- **THEN** stdout is empty, stderr holds one line naming the key and the exit code is 0

#### Scenario: An unwritable cache folder
- **WHEN** a note passes the gate and `<cache folder>/bilbo/sessions/` cannot be created
- **THEN** stdout is empty, stderr holds one line naming that folder, and the exit code is 0

#### Scenario: An embedder that refuses
- **WHEN** the embedder answers 500 and one passage holds 3 of the query's 4-letter words
- **THEN** the digest lists that note, stderr holds one line naming the embedder's URL and 500, and the exit code is 0

### Requirement: The digest log
When `digest.log` is `on`, every run that could read the config and finds the digest on SHALL append one JSON object line to `<state folder>/bilbo/digest.jsonl`, where the state folder is `$XDG_STATE_HOME` when it is an absolute path, else `$HOME/.local/state`. The file SHALL be created readable and writable by its owner only. The line SHALL hold `time`, `session` (null when the input had no valid one), `prompt` (the trimmed prompt's first 500 characters, or null), `ranking` (`meaning`, `keywords` or `none` when no query was made), `passed` (how many notes passed the gate), `shown` (the listed paths), `elapsed_ms` and, when the run hit one, `error`. When `digest.log` is not `on`, nothing SHALL be written there.

#### Scenario: An empty digest is explained
- **WHEN** `digest.log` is `on`, the embedder stalls and no note passes the keyword gate
- **THEN** the new log line has `ranking` `keywords`, `passed` 0, an empty `shown` and an `error` naming the embedder timeout

#### Scenario: A digest that showed notes
- **WHEN** `digest.log` is `on` and a digest lists two notes in session `abc`
- **THEN** the new log line has `session` `abc`, `ranking` `meaning` or `keywords`, and `shown` naming both paths, and the file's mode is `0600`

#### Scenario: Logging is off by default
- **WHEN** the config does not set `digest.log` and a digest runs
- **THEN** no file exists under `<state folder>/bilbo/`

### Requirement: Digest leaves the store alone
`bilbo digest` SHALL NOT create, change, rename or delete anything under the store root, and SHALL NOT change the vector cache.

#### Scenario: The store and cache are left as found
- **WHEN** `bilbo digest` runs
- **THEN** every entry under the root and the vector cache file have the same bytes and modification time as before the run

### Requirement: Sync state in the digest
In a note's line, a note with an open conflict or undeclared dropped text SHALL show `(<kind>, <created>, conflict)`, and a note whose latest version is a `merged` version without conflict `(<kind>, <created>, auto-merged)`. A session's first digest SHALL end with `Sync conflicts wait in: <absolute path>, ... (run bilbo check)` when notes have an open conflict or undeclared dropped text, naming at most 3 and then `and <n> more`. When no note passes the gate, that line SHALL follow `<!-- bilbo digest: 0 of 0 notes -->` alone.

#### Scenario: A conflicted note is labelled
- **WHEN** `gotcha-nix.md` has an open conflict and passes the gate
- **THEN** its line holds `(gotcha, <created>, conflict)`

#### Scenario: An auto-merged note is labelled
- **WHEN** the latest version of `plan-release.md` is a `merged` version without conflict and the note passes the gate
- **THEN** its line holds `(plan, <created>, auto-merged)`

#### Scenario: Conflicts raised with nothing else to show
- **WHEN** a session's first prompt passes no note, and `gotcha-nix.md` has an open conflict
- **THEN** stdout is `<!-- bilbo digest: 0 of 0 notes -->` and `Sync conflicts wait in: <root>/notes/gotcha-nix.md (run bilbo check)`

#### Scenario: Raised once per session
- **WHEN** a session already had its first digest and `gotcha-nix.md` still has an open conflict
- **THEN** a later digest in that session has no `Sync conflicts` line

#### Scenario: Without sync
- **WHEN** no note has ever been merged
- **THEN** no line carries a `conflict` or `auto-merged` label and no block has a `Sync conflicts` line
