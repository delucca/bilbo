# What your agent does with bilbo

This guide shows what the bilbo plugin gives Claude Code and Codex, and what
each part does when you ask for it: the four skills, the compaction reminder and
the digest.

`bilbo setup` installs the plugin ([Set up](setup.md)). It gives your agent four
skills, `note`, `recall`, `reference` and `ingest`, a hook that runs the digest
on every prompt and a hook that reminds it to save notes after a compaction.

## Keep something for a later session

Ask the agent to keep it ("note this", "save this as a decision"). The `note`
skill also runs when the session settled something a later one would otherwise
work out again.

The skill:

1. Looks for the note on the subject first with `bilbo recall`, and updates that
   note instead of adding a second one.
2. When no note covers it, creates one with `bilbo new`. A note whose kind
   changes is renamed with `mv -n`.
3. Runs `bilbo check`, fixes the lines that name its own note, and reports the
   note's absolute path.

It never invents a source, and when `bilbo` is missing it says so and stops.

When `check` reports a [sync conflict](sync.md#conflicts) in that note, the
skill replaces each block with one passage that keeps every fact of both sides,
unless you said which side holds. Lines it dropped on purpose it declares with
`bilbo sync declare`, and the report names them. A conflict in a note it did not
touch is left alone.

## Find a note again

Ask the agent to "recall" something. The `recall` skill runs `bilbo recall` with
your words, retries twice in the note's likely wording when nothing matches, and
offers to open a hit. It searches through `bilbo` only: when the binary is
missing, it says so and stops.

When you ask what the library says or name a corpus, `recall` searches the
library instead. It only locates sources: it hands each hit to `reference` as a
pick, and never answers from the snippets.

## Answer a question from the library

Ask something like "what does the Go book say about X". The `reference` skill:

1. Lists the corpora with `bilbo library`, reads the guides, and posts its
   picks, the sources or sections that answer the question, before it reads
   anything. A catalog is only ever picked by section. A catalog, a bare name or
   a question no guide entry covers is looked up with `bilbo recall --library`.
2. Plans the picks with `bilbo library plan` and reads every slice through
   `bilbo library read`, never with a file tool.
3. Drafts one `bilbo:` citation per claim, runs `bilbo cite --plan` until every
   verdict is `ok`, and drops or narrows any claim its quote does not support.
4. Ends with the picks and cite's `citations:`, `coverage:` and `picked:` lines,
   copied as printed.

In Claude Code a plan of two to six partitions goes to one `general-purpose`
reader each, briefed by the skill's `references/reader.md`. Without the Agent
tool, as in Codex, it plans smaller slices and reads up to 100,000 tokens
itself.

See [Library](library.md#citations) for the citation syntax.

## Add a source to the library

Ask "ingest this page". The `ingest` skill:

1. Stages the URL with `bilbo library stage`.
2. Reads the capture around the suggested `keep` and every navigation suspect,
   cuts the document out with `--keep` ranges and lands it with `bilbo library
   land`.
3. Reads the landed source through `bilbo library plan` and `read`, writes its
   guide entry, and runs `bilbo check`.
4. Reports the source's path, id and kept ranges.

It never uses WebFetch: a page bilbo cannot fetch is saved by you and staged as
a file, and a PDF goes through `pdftotext -layout`. Files and PDF text are
labelled `capture: external`. It replaces an existing source only when you say
so.

## Save notes after a compaction

After each compaction, the plugin's `SessionStart` hook adds one line asking the
agent to save what the session settled, once the current task allows. It prints
nothing without `bilbo` on `PATH`.

## The digest

`bilbo digest` is what the plugin's prompt hook runs, in Claude Code and in
Codex, on every prompt. It reads the hook's JSON from stdin and prints the notes
that bear on the prompt, which the tool hands to the agent as context:

```text
<!-- bilbo digest: 2 of 3 notes -->
Notes that may bear on this prompt (open the file to read more):
- /Users/me/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md:11 (gotcha, 2026-10-06T14:39-03:00) SQLite needs a busy timeout > Fix: Set `busy_timeout = 5000` right after opening the connection.
- ...
(1 more passed; run bilbo recall for them)
```

A session's first digest lists up to 6 notes and later ones up to 3, never a
note the session already got, and never more than 9,000 bytes. When nothing
passes, it prints nothing.

### What passes

A note passes only when it is close to the prompt. With an embedder, one of its
passages must reach `digest.min_similarity`; sharing words is not enough.
Without one, or when the embedder fails, is slow or has indexed nothing, a
passage must hold at least 3 of the prompt's words of four letters or more. The
embedder gets at most 1.2 seconds, and the whole run stays within 1.5. A prompt
that starts with a path finds nothing.

### Failures never block a prompt

`bilbo digest` always exits 0, so a failure never blocks a prompt: it prints
nothing and says why in one line on stderr. It remembers what each session was
shown in a file named after the session id, under `sessions/` in bilbo's cache
folder, and deletes those files after 30 days. It never changes the store or the
vector cache.

### Turn it off or log it

Set `digest.enable = off` to turn the digest off: the hook prints nothing and
writes nothing. It is `on` by default.

With `digest.log = on`, each run appends one JSON line to `digest.jsonl` under
bilbo's state folder (`$XDG_STATE_HOME/bilbo`, else `~/.local/state/bilbo`): the
session, the first 500 characters of the prompt, how the notes were ranked, how
many passed, which were shown and any error. The prompts are plain text on disk,
so the file is mode 0600 and off by default.

The keys are in [Configuration](../reference/configuration.md#keys).

## See also

- [Embedders](embedders.md): how an embedder changes what passes.
- [Troubleshooting](../troubleshooting.md)
- [Security](../security.md)
