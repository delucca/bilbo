# Tasks

Run every `cargo` command below from the repo root as `nix develop -c sh -c '<command>'`, with `CARGO_TARGET_DIR` set to the checkout's `target`. A new file under `src/` or `tests/` needs `git add -N` before any Nix command can see it. CLI tests run the built binary through `tests/common/` with a clean environment, so every library test sets `BILBO_HOME`, `HOME` and `XDG_STATE_HOME` inside its `TempDir`.

## 1. Hashing

- [x] 1.1 Add `sha2 = "0.11.0"` to `Cargo.toml`, run `cargo build` once so `Cargo.lock` records it (with `cargo update -p sha2 --precise 0.11.0` if the lock resolves another version), and write `src/hash.rs` with `pub fn sha256_hex(bytes: &[u8]) -> String` (lowercase hex) and its `mod` line. Unit tests: the empty input and `abc` against the FIPS 180-2 vectors (`e3b0c442…b855`, `ba7816bf…15ad`). Add `sha2` to the dependency list of the Stack line in `openspec/config.yaml`. Verify with `cargo tree -i sha2 --locked && cargo test --locked --bin bilbo hash::` and `rg -q 'sha2' openspec/config.yaml`

## 2. Sources (`library-store`: Source frontmatter, Fetched and origin, Kept ranges and capture label, Body digest, Source title, Outline, Derived sizes, Catalog, Anchors)

- [x] 2.1 Make `rank::heading` `pub(crate)`, with no change in behavior. Verify with `cargo test --locked --bin bilbo rank::`
- [x] 2.2 Write `src/source.rs`, a library module that never prints:
  - `read(text) -> Source`: the frontmatter keys and values, the body's byte offset and first physical line, and every problem with its line, in the messages of design.md's "The library check problems" table. It reuses `note::lines`, `note::fence_run`, `note::is_ulid` and the `Problem` type.
  - `render(&Frontmatter, body) -> String` in the key order `id`, `fetched`, `origin`, `digest`, `kept`, `capture`, with `# <title>` right after the closing `---`.
  - `digest(body) -> String` as `sha256:<hex>` through `hash::sha256_hex`.
  - `outline(lines, body_start) -> Vec<Section>`: heading line, end line, level, heading path below the title, bytes and tokens. Built on `rank::heading` and `note::fence_run`.
  - `tokens(bytes)` (`ceil(bytes / 2.5)`), `kb(bytes)` (`ceil(bytes / 1000)`), `is_catalog(body_bytes, &sections)` and `resolve(&sections, anchor) -> Resolved { One(i), Ambiguous(Vec<i>), Missing }`.
  - `parse_kept` and `format_kept`, merging ranges that touch.

  Unit tests cover every scenario of the requirements named in this group's heading, including the four catalog scenarios (the cut level, not the most common level), a `######` line, a fenced `## x`, the 1,000 and 1,001 byte sizes, the three anchor scenarios, and a body that differs only by a trailing newline. Verify with `cargo test --locked --bin bilbo source::`

## 3. Guides (`library-store`: Library layout, Corpus and source names, The guide, Guide entries, Stub and stale lines)

- [x] 3.1 Write `src/corpus.rs`, a library module that never prints:
  - `is_corpus_name`, which rejects `show`, `stage`, `land`, `plan` and `read`; and `is_source_name`, which rejects `guide`.
  - `read_guide(text) -> Guide`: frontmatter (`id` and `created` by the `note-store` rules, any other key unknown), title, lead lines, and entries with their heading line and prose lines. Every `##` outside fences is an entry.
  - `new_guide(id, created, corpus)`, `add_entry(text, name)` and `mark_stale(text, name, date)`. `mark_stale` replaces a stale line already under the heading.
  - `problems(root) -> Vec<(String, String)>`: every library problem in the table, plus each file's `id` for the shared-id rule.

  Unit tests cover every scenario of the requirements named in this group's heading, a guide with a fenced `## x`, a stale line written twice, and a guide that does not end with a newline. Verify with `cargo test --locked --bin bilbo corpus::`

## 4. Check (`store-check`: Report problems, Report every problem in one run, A missing store is a problem; `library-store`: Ids across notes and library, Captures)

- [x] 4.1 In `src/check.rs`, scan `<root>/library/` through `corpus::problems`, run `shared` over the note and library ids together, and refuse with `no store at <root>` only when both `notes/` and `library/` are missing. `check` never opens `<root>/.bilbo/captures/`. Add `store::library_dir`, `store::captures_dir` and `store::staging_dir`. In `tests/check.rs`, add tests for: a clean store with a library, an edited source (a `digest` line), a stub entry, a source with no entry, an entry with no source, a reserved corpus, a loose file in `library/`, a note and a source sharing an id (a line on each), a library-only store, neither folder, and a capture folder with a wrong name and one with no `capture.md` (no line). Keep the existing tests passing. Verify with `cargo test --locked --test check && cargo test --locked --bin bilbo check::`

## 5. Browsing (`library-browse`; `cli`: Verb dispatch; `config`: Config location)

- [x] 5.1 Write `src/library.rs` with `run(args, env) -> Result<Vec<String>, Failure>` for `bilbo library`, `library <corpus>` and `library show`, plus the option rules of Library options. Add its `mod` line, the `library` dispatch arm (stdout lines from the result, warnings to stderr) and these USAGE lines in `src/main.rs`:

  ```
         bilbo library [<corpus>]
         bilbo library show <corpus>/<name>|<id>[#<anchor>] [--depth <n>]
         bilbo library stage <file> --origin "<url|doc>: <value>" [--fetched <YYYY-MM-DD>]
         bilbo library land <stage> <corpus>/<name> --keep <a>-<b>[,<c>-<d>]... [--title <text>] [--replace]
  library lists the corpora, prints a corpus's guide with the facts of each source, or a source's outline; stage and land add a source.
  ```

  The verb reads no settings. Verify with `cargo build --locked && ./target/debug/bilbo --help | grep -q 'bilbo library land'`
- [x] 5.2 Write `tests/library.rs` with the browsing scenarios: two corpora and their exact rows, the singular `1 source`, an invalid folder skipped, no library, a guide with its facts lines, a source with no entry, an entry with no source (`missing`), a bad `fetched` (`-`), a catalog marked, the unknown, bad and reserved corpus errors, show by name and by id with equal output, a note's id, a malformed reference, the small-source rows of the Show a source scenario, a headingless source, a legacy source without a capture folder, `#<anchor>` narrowing, an ambiguous anchor, `--depth 1` and `--depth 0`, `--depth=1` before the reference, `--json` unknown, `show` with no reference, a config file with an unknown key (stderr empty), and a `snapshot` before and after all three forms. In `tests/cli.rs`, expect `library` among the verbs in the usage message. Verify with `cargo test --locked --test library && cargo test --locked --test cli`

## 6. Stage and land (`library-ingest`)

- [x] 6.1 Implement `library stage <file>` in `src/library.rs`: read, normalize, write `capture.md` and `stage.json` (origin, fetched, label `external`, SHA-256 of `capture.md`) under `store::staging_dir`, and print the stage output. Exit codes as Stage refusals says. Verify with `cargo test --locked --test library stage`
- [x] 6.2 Implement `library land` in `src/library.rs`, following design.md's "Land: order of writes and concurrency": the lock on `<root>/library/.lock` through `std::fs::File::lock`, the target checks, the stage hash check, the capture folder and its `landed` line, the body and its demotion, the source through a temp file with a hard link (new) or a rename (`--replace`), the guide through `corpus::new_guide`, `add_entry` or `mark_stale`, the duplicate-origin warnings, and the removal of the stage folder. Extend `wants_help` in `src/main.rs` so the value of `library land --title` is never read as `-h`. Verify with `cargo test --locked --test library land`
- [x] 6.3 Add the stage and land scenarios to `tests/library.rs`: CRLF normalized; origin and fetched carried to the source; the stage output for a page with navigation and for text with no title; a PDF refused; no `--origin`; a bad origin type; a first source in a new corpus, with its guide and the four stdout lines; the whole capture kept (no `kept`); two `--keep` cuts; touching ranges merged; a range past the end; overlapping ranges; kept lines byte for byte (trailing spaces, tabs, a fence); the title from the capture; the demotion and its stderr line; no title; an empty title; a new entry; an entry that outlived its source; a re-ingest with new text (same id, stale line, old prose kept); a re-ingest with the same text (guide bytes unchanged); a taken name; nothing to replace; the capture folder and its `landed` line, and the stdout line `capture folder:`; the same text landed twice (one folder, two lines); an existing capture folder left as it was, with a file added by hand; a different origin with no warning; an edited capture refused with nothing written; an unknown stage; `go/guide` and `read/errors` refused; the duplicate-origin warning; two `land` children adding `go/a` and `go/b` at once (both entries kept); two `land` children adding `go/errors` at once (one exits 1); and `bilbo check` clean after a land, an Edit that replaces the stub with prose, and a `show` that prints `capture folder:`. Verify with `cargo test --locked --test library`

## 7. Docs

- [x] 7.1 Update `README.md`: a "Library" section under "Usage" that describes a corpus, a source, the guide, the five forms of `bilbo library`, the `TODO` and `stale` lines `check` fails on, and how to move or delete a source by hand (`mv` or `rm`, then the guide edit, then `bilbo check`). The `check` text says it covers the library too. Verify with `rg -q 'bilbo library land' README.md && rg -q 'guide.md' README.md`
- [x] 7.2 Update `AGENTS.md`: add `source`, `corpus` and `hash` to the library modules, and "`sha2` in `src/hash.rs`" to the dependency rule. Add a gotcha: `land` and `check` treat any hidden entry in `<root>/library/` as absent, which is where the `land` lock lives. Verify with `rg -q 'src/hash.rs' AGENTS.md && rg -q 'corpus' AGENTS.md`

## 8. Integration

- [ ] 8.1 Run the full suite and the package check. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked && nix flake check -L`
