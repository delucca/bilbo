# Proposal

## Why

After `add-library-store` and `add-library-reading`, an agent finds a source by reading a corpus guide and picks sections from `bilbo library show`. That works for books, but not for a lookup: a lint name, a flag or an API in a catalog of 849 sections, or a term whose corpus the agent cannot guess. nbrecall's `--kind library` filled that role in the notebooks, and the dnix `recall` skill reached it with `--include-library` (`design-bilbo-library.md` in the planning notebook, "The library today"). bilbo has no way to search source text, and the `reference` skill's lookup path needs one.

## What Changes

- `bilbo recall <query>... --library` searches the library instead of the notes: every source passage and every guide entry, by keyword only. It never contacts the embedder and never reads the vector cache, even when an embedder is configured.
- `--corpus <corpus>` narrows the search to that corpus, may be given several times, and implies `--library`. An unknown corpus exits 1; a bad or reserved corpus name is a usage error.
- A library hit prints as a three-line block, like a note hit, with its own first line: `<absolute path>:<line>`, `source` or `guide`, the reference the `library` verb takes (`<corpus>/<name>` for a source, `<corpus>` for a guide), and the lines of the hit's section, `<start>-<end>`. The second line is the heading path below the title, which is an anchor `library show`, `library plan` and `cite` accept. From a hit, an agent reads from `<line>` to `<end>` and has the whole section.
- With `--library`, nothing matching prints `bilbo: no sources match`, and a store with no library prints `bilbo: no library at <root>`. Both exit 1.
- `--kind` filters notes, so `--kind` with `--library` or `--corpus` is a usage error.
- Plain `recall`, `index` and `digest` stay as they are and never read `<root>/library/`. Plain `recall` prints no hint about library matches (design.md, "No library hint in plain recall").
- The `recall` skill learns `--library`: it runs it when the user asks what the library says or names a corpus, retries on `no sources match` as it does for notes, and hands each library hit to the `reference` skill as the pick `<corpus>/<name>#<heading path>` instead of answering from snippets.
- The `reference` skill gains its lookup path. A catalog, a bare name (a lint, a flag, an API) or a question no guide entry covers goes through `bilbo recall --library [--corpus <c>] -- '<term>'`, and each hit becomes the pick `<corpus>/<name>#<heading path>`. A pick the `recall` skill hands over is taken the same way. A lookup that finds nothing is named in the answer with its queries. Its `allowed-tools` gains `Bash(bilbo recall *)`.
- An ignored speed test: `recall --library` over a generated 14 MiB library answers in under 500 ms in a release build, and plain `recall` keeps its 250 ms over the 6 MiB store with that library beside it.

## Capabilities

### New Capabilities

- `library-recall`: `recall --library` and `--corpus`: what they search, the hit block and its section lines, ranking, the corpus filter, the empty and missing cases, and the rule that plain search leaves the library alone.

### Modified Capabilities

- `note-recall`:
  - What recall searches: notes only without `--library` and `--corpus`, never `<root>/library/`; with either, the `library-recall` spec applies, and only the named requirements of this spec carry over.
  - Kind filter: `--kind` with `--library` or `--corpus` is a usage error.
  - Options and the query: `--library` and `--corpus` are options, with `--corpus=<corpus>` accepted.
  - A missing store: `no store at <root>` only without `--library` and `--corpus`.
- `agent-plugin`, building on `add-library-reading`'s text:
  - The recall skill: runs `bilbo recall --library` for library questions, acts on its two refusals, and hands library hits to the `reference` skill as picks.
  - The reference skill: `allowed-tools` covers `Bash(bilbo recall *)`.
  - Reference picks: the lookup path for catalogs, bare names and questions no guide entry covers, picks handed over by `recall`, and a lookup that finds nothing.

## Non-goals

- Embedding source passages or guide entries, or any library vector. Keyword only in v1, as the contract's search policy says.
- A library line in the digest, and a hint line in plain `recall`.
- A persistent keyword index or postings cache for the library. The scan meets its budget at today's size (design.md).
- Filtering by `source` or `guide`, or by a source's age. Hits name their kind, and `bilbo library <corpus>` prints a guide whole.
- Several blocks per source. A source shows once, at its best passage, as a note does.
- A `--json` output.
- Reading or planning a hit. That is `library show`, `library plan` and `library read` (`add-library-store`, `add-library-reading`).
- Changing the `note` skill's use of plain `recall`, or the `ingest` skill.

## Impact

- `src/recall.rs`: the two options, the library branch, and the library hit block.
- `src/store.rs`: `read_library`, beside `read_notes`, built on `corpus::is_corpus_name`, `corpus::is_source_name`, `note::read`, `note::lines`, `rank::passages` and `source::outline` from `add-library-store`.
- `src/main.rs`: one more USAGE line for `recall --library`, and one line saying what it searches.
- Tests: `tests/recall.rs` gains the library scenarios and the ignored speed test; `tests/common/mod.rs` gains a library writer and `bench_library`; `tests/plugin.rs` checks the recall skill's new strings.
- `plugins/bilbo/skills/recall/SKILL.md`: the library query, its exit codes, the hit block, and the hand-off to `reference`.
- `plugins/bilbo/skills/reference/SKILL.md`: the lookup in step 3 replaces `lookup only, not searched`, and `allowed-tools` gains `Bash(bilbo recall *)`; `tests/plugin.rs` updates the exact `allowed-tools` string and the `reference_skill_drives_bilbo` strings.
- **dnix edits** (`modules/ai/`, `modules/notebooks/`), moved here from `add-library-reading`'s cutover and made in one commit after this change is archived, then `just switch` on each host:
  - `skills/recall/SKILL.md`: drop `--include-library`; a library question goes to `bilbo recall --library` or the bilbo `reference` skill.
  - nbrecall: drop its `library` and `index` globs, so it no longer indexes `~/Notebooks/*/library/`.
  - `skills/review/references/lens-briefs.md`: variant B (the recall-grounded lens) moves from `nbrecall --kind library` to `bilbo recall --library [--corpus <c>]`, each hit becoming an anchor pick for `bilbo library plan`.
- `README.md`: the recall row and the recall skill's paragraph.
- No new dependency, no new verb, no new setting, and nothing new on disk.
