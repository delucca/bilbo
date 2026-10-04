# Design

## Context

See proposal.md for why. What exists, after `add-library-store` and `add-library-reading` are archived, and constrains the approach:

- **Recall.** `recall::run` (`src/recall.rs`) parses `--kind`, `--limit` and `--`, loads the settings, refuses with `no store at <root>` when `notes/` is missing, reads the notes with `store::read_notes`, ranks with `rank::keyword`, adds the meaning order when an embedder is configured, merges both with `rank::fuse`, and prints three lines per hit. `--kind` is checked against `note::KINDS` while parsing, so an unknown kind is a usage error.
- **Passages and words.** `rank::passages` cuts a body at every heading outside fences, the title included, and splits a passage over 4,000 bytes at blank lines. A passage's `path` starts with the title, and its `line` is the heading line for the first part and the part's first line after that. `rank::keyword` is BM25 over every passage given, counting words in the heading path (title included) and the text. `rank::fuse(&keyword, &[])` keeps keyword order and keeps one hit per document. All of this is fixed by the `note-recall` spec.
- **Reading a file.** `store::read_notes` reads each file, takes `body_start` from `note::read` (which finds the closing `---` whatever the keys) and builds a `rank::Document`. A source's or a guide's frontmatter has other keys, which `note::read` reports as problems but still steps over.
- **The library modules from `add-library-store`.** `corpus::is_corpus_name` (with the reserved names) and `corpus::is_source_name`; `source::outline(lines, body_start)`, which returns each section's heading line, end line and heading path below the title, built on the same `rank::heading` and fence rule as `rank::passages`; `store::library_dir(root)`. `library-browse` already words the unknown-corpus error as `no corpus '<c>' in <root>/library`.
- **Speed today.** Plain `recall` has no budget in its spec. The archived `add-note-recall` design set one, 250 ms over about 6 MiB, checked by the ignored `recall_over_a_6_mib_store_is_fast` over `common::bench_store`. `bench.md` measures the real library on rivendell: 137 ms for 13.6 MB through the same code path, against 46.5 ms for the 5.0 MB of real notes.
- **The contract's search policy.** Plain `recall`, `index` and `digest` never read `<root>/library/`, and nothing embeds library text.

## Goals / Non-Goals

**Goals:**
- A lookup into any source, catalogs included, in one command, whose hit says where the section starts and ends.
- No change to plain `recall`'s output, ranking or speed, and none to `index` or `digest`.
- `recall --library` over a generated 14 MiB library answers in under 500 ms in a release build on rivendell, cold start, no cache. Plain `recall` over the 6 MiB bench store keeps its 250 ms with that library beside it.

**Non-Goals:**
- A persistent keyword index. The scan is linear in bytes and reaches 500 ms near 50 MB (`bench.md`). A postings cache under the cache folder, derived and never synced like the vectors, is the next step when the library grows that far.
- Ranking sources against notes in one list.

## Decisions

### No library hint in plain recall

Plain `recall` prints nothing about the library, not even when no note matches.

- **Cost.** A hint must scan the library. Through the shared path it takes plain `recall` from 46.5 ms to about 182 ms over today's notes and library, four times, on every call: the `recall` skill's queries and retries, and the `note` skill's same-subject check before every write. A dedicated presence scan would be a second word matcher to keep in step with `rank::words`, and still costs at least the 39 ms that reading the 13.6 MB takes.
- **Policy.** The contract binds plain `recall` to never read `<root>/library/`. A hint is such a read, so choosing one would mean amending that rule for all four changes.
- **Routing belongs to the skills.** A note question and a source question are different intents. The `recall` skill runs `--library` when the user asks about the library, and the `reference` skill runs it for lookups. The panel's evaluation rejected the digest pointer line for the same reason: the library should not crowd notes out (`evaluation.md`, "Recall, embedding and the digest").

Alternatives:
- A hint on every plain `recall` that matches in the library, such as `library: go (3 passages); use the reference skill`. Rejected for the cost and the policy above.
- A hint only when no note matches. It costs only on a miss, but the `note` skill's check misses on every new subject, and it still breaks the policy. It also turns "no notes match" into a mixed answer the skills must parse.

### The hit block

The block keeps note recall's three lines, so the `recall` skill renders both the same way, and changes what the first two say:

```
<absolute path>:<line>	source	<corpus>/<name>	<start>-<end>
<heading path below the title, or ->
<snippet>
```

- **The kind column** says `source` or `guide`. Neither is a note kind, so a block is never mistaken for a note's.
- **The reference column** replaces `created`. It is the argument the `library` verb takes to show the file: `<corpus>/<name>` for `library show` and `library plan`, `<corpus>` for `bilbo library`. A guide has no `<corpus>/guide` reference, because `guide` is not a source name.
- **The section column** answers the `reference` skill's lookup: read lines `<line>` to `<end>`. The range is the one `library show` prints for that section, so the skill can also plan it by anchor. A title passage's range runs to the first heading below the title, which is the passage itself.
- **The heading path drops the title**, unlike a note block's. It is then an anchor as `library show`, `library plan` and `cite` take it, copied as printed. The title is one `library show` away, and the reference column already names the file.

How the section is found: the last outline section whose heading line is at or before the passage's `line`. Passages and outline sections start on the same heading lines, because both come from `rank::heading` with the same fence rule, so this is the section of the passage's own heading, also for a later part of a split passage.

Alternatives:
- The note block as is, with the title in the heading path and `fetched` in the third column. The skill would strip the title to get an anchor, and would need `library show` to find the section's end.
- A fifth column with the source's id, for `cite`. `library show` and `library plan` print the id, and the reading path goes through them before any citation, so the id would only lengthen every line.
- Printing tokens beside the section. `library show <ref>#<anchor>` prints them when the skill needs to choose between a Read and a plan.

### The reference skill's lookup path

`add-library-reading` reaches a catalog only by guessing a heading for `library show '<ref>#<name>'`, and names anything else `lookup only, not searched`. This change owns `recall --library`, so it also owns the step that uses it. The reference skill's step 3 changes:

- A catalog, a bare name (a lint, a flag, an API) or a question no guide entry covers: `bilbo recall --library --corpus <c>... -- '<term>'`, the corpora those the skill already chose, or none when no guide covers the question.
- Each hit it keeps becomes the pick `<corpus>/<name>#<heading path>`: the block's third column and second line, copied as printed. A `-` heading path gives `<corpus>/<name>`, and a guide hit gives the source its entry names. The picks then go through `library plan` and `library read` like any other, so the read log and `cite --plan` cover them.
- A pick the `recall` skill hands over is taken as is, with no second search.
- A lookup that finds nothing is named in the answer with its queries, replacing `lookup only, not searched`.

`allowed-tools` gains `Bash(bilbo recall *)`. The section lines in the block are not part of a pick: a plan cuts at the same section boundaries from the anchor. They serve an agent that only wants to Read a hit outside the `reference` skill.

Alternative: a fifth change for the skill. It would leave `review`'s variant B on nbrecall longer, and nobody would own the hand-off from `recall`.

### One block per file

A source shows once, at its best passage, as a note does, and `rank::fuse(&keyword, &[])` already does it. A catalog lookup names the lint, so the best passage is the lint's own section. When an agent needs more of one source, `library show <ref>#<anchor>` and `library plan` take over.

Alternative: up to three blocks per source. It suits a broad query into a book, but one large source would push other sources off the first page of 10, and the ranking rules would need a second cut.

### `--kind` with `--library` is a usage error

`--kind` filters notes by kind and rejects unknown kinds today. Reusing it for `source` and `guide` would make `--kind` mean two things, which the panel's evaluation flagged. There is no source or guide filter: a hit names its kind, and `bilbo library <corpus>` prints a guide whole.

Alternatives: `--kind source|guide` (overloads the note filter); ignoring `--kind` with `--library` in silence (hides a mistake).

### Library recall still reads the settings

`recall --library` loads the config like any `recall`, so a broken config file is the same usage error, and it never uses the embedder settings. Reading no settings for `--library` would need a change to the `config` spec's Config location, a requirement both this series and the sync series modify, for no gain an agent can see on a working setup.

### `--corpus` reads only its corpora

`--corpus` may repeat, like `--kind`, and implies `--library`. Recall then reads only the named corpus folders, which is faster than reading the whole library and filtering. Word rarity is measured over the passages read, which is what a user searching one corpus means by rare. An unknown corpus exits 1 and a bad name exits 2, as `bilbo library <corpus>` does.

### Two refusals of its own

`no sources match` and `no library at <root>` differ from `no notes match` and `no store at <root>`, so the `recall` skill can tell a library miss from a notes miss by the last stderr line, as it already does. A store with a library and no `notes/` searches the library: change 1 made such a store valid.

### Modules

- `src/store.rs` gains `read_library(root, corpora) -> io::Result<Vec<Shelved>>`, beside `read_notes`. A `Shelved` holds the path, the kind (`Source` or `Guide`), the reference, the `rank::Document` and the outline sections' `(start, end, path)`. It lists corpus folders through `store::entries`, checks names with `corpus::is_corpus_name` and `corpus::is_source_name`, takes `body_start` from `note::read`, passages from `rank::passages` with the file stem as fallback title, and sections from `source::outline`.
- `src/recall.rs` gains the two options in `parse`, a check that `--kind` is not combined with them, and a library branch in `run` that skips `config`'s embedder, `vectors` and `embed` entirely. It formats the library block.
- `src/main.rs` gains one USAGE line and one help line. The dispatch arm already prints `Output`.
- No new module and no new dependency: everything reuses `rank`, `note`, `store`, `corpus` and `source`.

### The speed test

`tests/common/mod.rs` gains `bench_library(dir, root)`, which writes 8 corpora of generated sources into `<root>/library/` with valid frontmatter and guides: a few large books with three heading levels, one catalog with hundreds of short level-2 sections, and many short sources, about 14 MiB in all, asserted between 13.5 and 14.5 MiB. The ignored `recall_library_over_14_mib_is_fast` asserts under 500 ms for `recall --library`. The ignored `recall_over_a_6_mib_store_is_fast` gains that library beside the notes and keeps its 250 ms. Both run with the existing `cargo test --release --test recall -- --ignored`, so `AGENTS.md` needs no new command.

## Risks / Trade-offs

- [The library grows past the budget] → Time is linear in bytes; the ignored test catches it at 14 MiB, and a postings cache is the known next step.
- [A word in a source's title matches every passage of that source] → Inherited from note recall, where the title is part of each heading path. One block per file keeps it to one block, and the other words decide which passage it shows.
- [A short guide entry outranks the source it describes] → BM25 favors short passages, so a topic word in an entry can come first. The entry names the source, so the hit still leads to it, and the skill hands guide hits on as their source.
- [Split sources before migration] → Until `migrate.py` runs, the user's library lives in the notebooks and `--library` finds nothing there. The cutover (`add-library-reading`) moves it.
- [`add-library-reading` changes the reference skill's text again before archiving] → The two MODIFIED requirements copy its current text and every scenario. Re-read it before archiving this change.
- [The dnix edits lag] → Until they land, the dnix `recall` skill still passes `--include-library` to nbrecall and `review`'s variant B still reads nbrecall. Both keep working on the notebook libraries until the cutover removes them, so nothing breaks in silence; it only stays on the old path.

## Migration Plan

Nothing on disk changes. Plain `recall` behaves as before. Rollback is reverting the code; no file needs cleaning.

## Decisions to confirm

1. No library hint in plain `recall`, not even on a miss.
2. The hit block: kind `source` or `guide`, the `library` reference instead of `created`, the section lines as a fourth column, and the heading path without the title.
3. One block per file.
4. `--kind` with `--library` or `--corpus` is a usage error; no source or guide filter.
5. `recall --library` still reads the settings and fails on a broken config, though it never uses them.
6. `--corpus` repeats, reads only its corpora, and measures word rarity over them.
7. The refusals `bilbo: no sources match` and `bilbo: no library at <root>`.
8. The budget: under 500 ms over a generated 14 MiB library, in an ignored test, with no keyword cache in this change.
9. This change owns the `reference` skill's lookup path and `Bash(bilbo recall *)` in its `allowed-tools`, rather than a fifth change.
