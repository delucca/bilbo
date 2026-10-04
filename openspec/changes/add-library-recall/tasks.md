# Tasks

Run every `cargo` command below from the repo root as `nix develop -c sh -c '<command>'`, with `CARGO_TARGET_DIR` set to the checkout's `target`. The plugin validators run outside the dev shell, as `AGENTS.md` shows; `$PLUGIN_CHECKS` stands for `claude plugin validate . && claude plugin validate plugins/bilbo && PYTHONDONTWRITEBYTECODE=1 python3 ~/.codex/skills/.system/plugin-creator/scripts/validate_plugin.py plugins/bilbo`. CLI tests run the built binary through `tests/common/` with a clean environment, so every library test sets `BILBO_HOME` and `HOME` inside its `TempDir`. This change builds on `add-library-store` and `add-library-reading`, which must be archived first.

## 1. Reading the library (`library-recall`: What library recall searches, The section of a hit)

- [ ] 1.1 Add `store::read_library(root, corpora) -> std::io::Result<Vec<Shelved>>` beside `read_notes`, as design.md's "Modules" section gives it: valid corpus folders only (all of them when `corpora` is empty), `guide.md` and valid source names only, hidden and unreadable entries skipped in silence, `body_start` from `note::read`, passages from `rank::passages` with the file stem as fallback title, sections from `source::outline`, and `<root>/.bilbo/` never opened. Add `Shelved::section(line) -> (start, end)` by the rule of The section of a hit, the title passage running from the title's line to the line before the first heading below the title. Unit tests in `src/store.rs`: a corpus with a guide and two sources in name order; `Go-Old/`, `Effective_Go.md`, `.draft.md` and a subfolder skipped; frontmatter words not in any passage; a source with a bad digest still read; the section of a level-3 heading, of a level-2 heading that holds subsections, of the second part of a split passage, of a title passage, and of a source with no heading below its title; a guide entry's section ending before the next `## `. Verify with `cargo test --locked --bin bilbo store::`

## 2. The options and the library branch (`library-recall`: Search the library, Library hit blocks, Library ranking, Narrow to corpora, Nothing in the library; `note-recall`: What recall searches, Kind filter, Options and the query, A missing store)

- [ ] 2.1 In `src/recall.rs`, parse `--library` (no value; `--library=<x>` is a usage error naming `--library`) and `--corpus <c>` or `--corpus=<c>` (repeatable; a bad or reserved name through `corpus::is_corpus_name` is a usage error naming it; implies `--library`). Refuse `--kind` with either, naming `--kind` and `--library`. Unit tests for each case in `recall::tests`, including `--` ending the options before `--library`. Verify with `cargo test --locked --bin bilbo recall::`
- [ ] 2.2 Add the library branch to `recall::run`: load the settings as today, resolve the root, refuse with `no library at <root>` when `store::read_library` finds no corpus, refuse with `no corpus '<c>' in <root>/library` for a named corpus with no folder, rank with `rank::keyword` and `rank::fuse(&keyword, &[])`, take `--limit`, and print each block as `<path>:<line>\t<source|guide>\t<reference>\t<start>-<end>`, the heading path without the title or `-`, and `rank::snippet`. Never call `vectors`, `embed` or the meaning ranking, and never read `notes/`. Nothing matching refuses with `no sources match`. Add to `USAGE` in `src/main.rs`:

  ```
         bilbo recall <query>... --library [--corpus <corpus>]... [--limit <n>]
  recall --library searches the sources and guides of the library by keyword instead of the notes; --corpus narrows it.
  ```

  Verify with `cargo build --locked && ./target/debug/bilbo --help | grep -q 'recall <query>... --library'`
- [ ] 2.3 Add `common::library(root, corpus, name, body)` and `common::guide(root, corpus, entries)` to `tests/common/mod.rs`, writing files in the `library-store` format with a correct digest. In `tests/recall.rs`, add one test per scenario of `library-recall` (Search the library through Nothing in the library) and of the four MODIFIED `note-recall` requirements: the exact source block and guide block with tabs and section lines; the title-passage hit with `-`; a section that holds its subsections; a split part whose line differs and whose section does not; the same section as `bilbo library show <ref>#<anchor>` prints; frontmatter, invalid entries and a capture not searched; an edited source still found; one block per source over 30 matching sections; the default limit; one and two corpora; unknown, bad and reserved corpora; `--kind` with `--library` and with `--corpus`; `--library=go`; the options before and after the words; nothing matches; no library; a library without notes, with no `no store` line; plain `recall` without `notes/` still refusing with `no store at <root>`; and a configured fake embedder that logs no request while stderr stays empty. Verify with `cargo test --locked --test recall`

## 3. Plain search leaves the library alone (`library-recall`: Plain search leaves the library alone)

- [ ] 3.1 Add tests through the built binary, each on a store whose `library/` holds sources that share the query's words: in `tests/recall.rs`, plain `recall` prints only the note's block with an empty stderr, and with only a source matching, stderr is exactly `bilbo: no notes match`; in `tests/index.rs`, with the fake embedder, `bilbo index` sends only the note's passage; in `tests/digest.rs`, a prompt whose words only a source holds gives an empty stdout. Each test also asserts the library's bytes and modification times are unchanged. Verify with `cargo test --locked --test recall --test index --test digest`

## 4. Speed (design.md: Goals, "The speed test")

- [ ] 4.1 Add `common::bench_library(dir, root)` as design.md describes, asserting 13.5 to 14.5 MiB. Add the ignored `recall_library_over_14_mib_is_fast` to `tests/recall.rs`: one warm-up run, then `recall embedder timeout decisao --library` under 500 ms, printing the size and the time to stderr as `recall_over_a_6_mib_store_is_fast` does. Write that library beside the notes in `recall_over_a_6_mib_store_is_fast` and keep its 250 ms. Record the times on rivendell in this change's `bench.md`. Verify with `cargo test --release --locked --test recall -- --ignored`

## 5. The recall skill (`agent-plugin`: The recall skill)

- [ ] 5.1 Update `plugins/bilbo/skills/recall/SKILL.md`:
  - The description adds that the skill also finds passages in the library and that answering from sources is the `reference` skill's, keeping it short (Codex counts its skill budget in tokens). `allowed-tools` stays as it is: `Bash(bilbo recall *)` covers `--library`.
  - Step 2 gains a second command in its own `bash` block, `bilbo recall --library [--corpus C]... [--limit N] -- '<the user's words>'`, and the rule for choosing it: the user asks what the library's sources say, asks to look a term up in the library, or names a corpus.
  - The exit-code table gains `bilbo: no sources match` (retry as for notes, in the source's likely wording) and `bilbo: no library at <root>` (show it and stop, without searching the notes instead).
  - Step 5 shows the library block and how to render it: `<path>:<line>  <kind>  <reference>  <start>-<end>  <heading path>`, the snippet under it.
  - A new step: for library hits, never answer from the snippets; run the `reference` skill with each hit as the pick `<corpus>/<name>#<heading path>`, a `-` heading path as `<corpus>/<name>`, and a guide hit as the pick of the source its entry names.

  In `tests/plugin.rs`, extend `recall_skill_drives_bilbo` with `bilbo recall --library`, `bilbo: no sources match`, `bilbo: no library at`, `reference`, `<corpus>/<name>#<heading path>` and `never answer from the snippets`. `skills_allow_every_command_they_run` must still pass. Verify with `cargo test --locked --test plugin && $PLUGIN_CHECKS`

## 6. The reference skill's lookup (`agent-plugin`: The reference skill, Reference picks)

- [ ] 6.1 Update `plugins/bilbo/skills/reference/SKILL.md` as design.md's "The reference skill's lookup path" gives it:
  - `allowed-tools` becomes `Bash(command -v bilbo), Bash(bilbo library *), Bash(bilbo cite *), Bash(bilbo recall *), Read, Agent(general-purpose), SendMessage`.
  - Step 3: a catalog, a bare name or a question no guide entry covers runs `bilbo recall --library --corpus <corpus>... -- '<term>'` in its own `bash` block; each kept hit becomes the pick `<corpus>/<name>#<heading path>`, a `-` heading path `<corpus>/<name>`, a guide hit the source its entry names. A table for its exit codes: 0, use the hits; 1 with `bilbo: no sources match`, name the query in the answer as finding nothing; any other 1 or 2, print bilbo's first stderr line and go on without that lookup. Replace `lookup only, not searched` with that wording.
  - A pick handed over by the `recall` skill goes into the picks message as is, with no second search.

  In `tests/plugin.rs`, change `reference_skill_frontmatter`'s exact `allowed-tools` string to the one above, and in `reference_skill_drives_bilbo` replace `lookup only, not searched` with `bilbo recall --library --corpus <corpus>... -- '<term>'`, `bilbo: no sources match` and `<corpus>/<name>#<heading path>`. `skills_allow_every_command_they_run` must pass for all three skills. Verify with `cargo test --locked --test plugin && $PLUGIN_CHECKS`

## 7. Docs

- [ ] 7.1 Update `README.md`: a usage row for `bilbo recall <query>... --library [--corpus <corpus>]... [--limit <n>]`, a short example of a library block with its four columns, the two refusals `no sources match` and `no library at <root>`, a sentence that plain `recall`, `index` and the digest never read the library, and in the plugin section, that the `recall` skill searches the library on request and hands library hits to `reference` as picks, and that `reference` looks up catalogs and bare names with `bilbo recall --library`. Verify with `rg -q 'recall <query>... --library' README.md && rg -q 'no sources match' README.md`

## 8. Integration

- [ ] 8.1 Run the full suite and the package check. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked && nix flake check -L`
