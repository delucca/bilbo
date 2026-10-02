# Tasks

Run every command below inside `nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#clippy nixpkgs#rustfmt`, from the repo root, with `CARGO_TARGET_DIR` set to the checkout's `target` (see AGENTS.md, Architecture).

## 1. Note reader (`note-store` contract, extended for recall)

- [ ] 1.1 In `src/note.rs`, give `Note` a `created: Option<String>` (set only when the value passes `is_created`) and a `body_start` (the physical line after the closing `---`, or 1 when the frontmatter is missing or never closes). Keep every existing problem message unchanged. Unit-test both fields on a valid note, a note with `created: 2026-10-02`, a note with no frontmatter and one with an unclosed frontmatter. Verify with `cargo test note::`

## 2. Ranking (`src/rank.rs`)

- [ ] 2.1 Add `words(text)`: runs of `char::is_alphanumeric`, lowercased, with the Latin-1 Supplement and Latin Extended-A accent table from design.md, dropping words under 2 characters. Unit-test `Decisão` to `decisao`, `ß` to `ss`, `snake_case` to two words, a one-letter word dropped, and a letter outside the table passing through. Verify with `cargo test rank::`
- [ ] 2.2 Add `passages(lines, first_line, title)`: one passage per ATX heading outside fenced code blocks, the heading path from the title down, text before the first heading under the title alone, and parts of at most 4,000 bytes cut at blank lines, or at a character boundary for one long paragraph, each with its own start line. Unit-test the `Passages` scenarios of the `note-recall` spec, a 9,000-byte section, a 5,000-byte single paragraph with multi-byte characters at the cut, and a note with no title. Verify with `cargo test rank::`
- [ ] 2.3 Add `rank(query_words, documents)`: BM25 (`k1 = 1.2`, `b = 0.75`) over every passage's heading path plus text, one result per document at its best passage, passages without a query word dropped, ties by path then line. Unit-test the `Ranking` scenarios, deduplication of repeated query words, and that a word in the heading path alone matches. Verify with `cargo test rank::`

## 3. `bilbo recall` (`note-recall` spec)

- [ ] 3.1 In `src/recall.rs`, parse the arguments: query words, `--kind` (repeatable, checked against the nine kinds), `--limit` (default 10, a whole number of 1 or more), `--` ending options, unknown options and a query without words as usage errors. Add the dispatch arm and USAGE line in `src/main.rs`, and make `bilbo recall --help` print help. Cover the `Options and the query`, `Kind filter`, `Limit` and `Query words` usage scenarios in `tests/recall.rs`. Verify with `cargo test --test recall`
- [ ] 3.2 Search the store: `bilbo: no store at <root>` when `notes/` is missing, skip hidden, badly named, non-file and unreadable entries, read files lossily, rank, and render each hit as `<absolute path>:<line>\t<kind>\t<created or ->`, the heading path and a 300-character snippet with whitespace collapsed, blocks separated by one blank line. Return `Failure::Refused("no notes match")` when nothing matches. Cover the remaining `note-recall` scenarios in `tests/recall.rs`, including the read-only snapshot of bytes and modification times, as `tests/check.rs` does. Verify with `cargo test --test recall`
- [ ] 3.3 Add the updated `cli` scenarios to `tests/cli.rs`: the usage message names `new`, `check` and `recall`, and `bilbo recall wumpus` on a store with no match exits 1. Verify with `cargo test --test cli`

## 4. Speed

- [ ] 4.1 Add an `#[ignore]` test in `tests/recall.rs` that generates a store of 450 notes and about 6 MiB of mixed English and Portuguese text with headings, runs `bilbo recall` with a three-word query and asserts it finishes in under 250 ms after one warm-up run. Record the measured time on rivendell in the design.md Context section. Verify with `cargo test --release --test recall -- --ignored`

## 5. Docs

- [ ] 5.1 Update `AGENTS.md`'s Architecture section: `src/rank.rs` (passages, words, ranking; never prints, never returns `Failure`, shared by verbs), `src/recall.rs` (`note-recall` spec), the `Note` fields from 1.1, and `tests/recall.rs` among the CLI test files. Verify with `for f in src/*.rs tests/*.rs; do rg -qF "$f" AGENTS.md || echo "missing $f"; done | (! grep .)`

## 6. Integration

- [ ] 6.1 Smoke test the built binary in a fresh store: two notes from `bilbo new`, a body appended to each, a `recall` that finds one, a `recall --kind` that filters it out, and a `recall` with no match exiting 1. Verify with `cargo build && export BILBO_HOME="$(mktemp -d)/store" && p=$(./target/debug/bilbo new decision flat-layout) && printf '\n## Layout\n\nOne flat folder.\n' >> "$p" && ./target/debug/bilbo new plan other && ./target/debug/bilbo recall flat folder | grep -q 'decision-flat-layout.md' && ! ./target/debug/bilbo recall flat --kind plan && ! ./target/debug/bilbo recall wumpus`
- [ ] 6.2 Run the full suite: formatting, lints and every test. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked`
